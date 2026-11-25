//! Executa os comandos recebidos do kanata.

use crate::msg::{Cmd, WinFilter};
use crate::win;
use std::time::{Duration, Instant};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_RETURN, VK_SHIFT};

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Retorna Ok(true) se a ação "deu certo" (usado pelo `first`).
pub fn exec(cmd: &Cmd, notify: &dyn Fn(&str)) -> Result<bool, String> {
    match cmd {
        Cmd::App { filter, run, args, dir, maximize, launch, wait_ms } => {
            app(filter, run.as_deref(), args.as_deref(), dir.as_deref(), *maximize, *launch, *wait_ms)
        }
        Cmd::Run { cmd, args, dir } => win::shell_execute(cmd, args.as_deref(), dir.as_deref()).map(|_| true),
        Cmd::BrowserTab { matches, search, url, filter, run } => browser_tab(matches, search, url, filter, run),
        Cmd::NextWindow { skip } => Ok(next_window(skip)),
        Cmd::Type(t) => {
            win::type_text(t);
            Ok(true)
        }
        Cmd::Topmost => {
            win::toggle_topmost(win::foreground());
            Ok(true)
        }
        Cmd::Maximize => {
            win::maximize(win::foreground());
            Ok(true)
        }
        Cmd::Minimize => {
            win::minimize(win::foreground());
            Ok(true)
        }
        Cmd::CloseWindow => {
            win::close(win::foreground());
            Ok(true)
        }
        Cmd::SleepPc => {
            unsafe { windows_sys::Win32::System::Power::SetSuspendState(false, false, false) };
            Ok(true)
        }
        Cmd::Lock => {
            unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() };
            Ok(true)
        }
        Cmd::Shutdown => {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("shutdown")
                .args(["/s", "/t", "0"])
                .creation_flags(0x0800_0000)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(true)
        }
        Cmd::Notify(t) => {
            notify(t);
            Ok(true)
        }
        Cmd::Wait(ms) => {
            sleep(*ms);
            Ok(true)
        }
        Cmd::First(subs) => {
            for s in subs {
                if exec(s, notify)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Cmd::Seq(subs) => {
            let mut all = true;
            for s in subs {
                all &= exec(s, notify)?;
            }
            Ok(all)
        }
    }
}

/// Traz a janela para frente. Devolve false se o Windows não deixou.
fn focus(w: &win::WinInfo, maximize: bool) -> bool {
    let ok = win::activate(w.h());
    if maximize {
        win::maximize(w.h());
    }
    ok
}

/// Foca o app. Se ele já estiver em primeiro plano e tiver várias janelas,
/// vai para a próxima (como o ciclo do Telegram no AHK).
fn app(f: &WinFilter, run: Option<&str>, args: Option<&str>, dir: Option<&str>, maximize: bool, launch: bool, wait_ms: u64) -> Result<bool, String> {
    let wins = win::find(f);
    if !wins.is_empty() {
        let fg = win::foreground() as isize;
        let idx = match wins.iter().position(|w| w.hwnd == fg) {
            Some(i) => (i + 1) % wins.len(),
            None => 0,
        };
        let w = &wins[idx];
        if focus(w, maximize) {
            return Ok(true);
        }
        // Minimizada e não dá para restaurar (programa elevado): relançar o programa faz a
        // instância que já está aberta se mostrar sozinha. Segue para o caminho de abrir.
        if !(win::is_minimized(w.h()) && launch && run.is_some()) {
            return Err(format!("não consegui trazer \"{}\" para frente (detalhes em keymacros-foco.log).", f.exe.as_deref().unwrap_or("janela")));
        }
    }
    if !launch {
        return Ok(false);
    }
    let Some(run) = run else { return Ok(false) };
    win::shell_execute(run, args, dir)?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(wait_ms) {
        sleep(100);
        if let Some(w) = win::find(f).into_iter().find(|w| !win::is_minimized(w.h())) {
            // dá um instante para a janela terminar de nascer
            sleep(50);
            focus(&w, maximize);
            return Ok(true);
        }
    }
    Ok(true)
}

/// Porte do OpenEdgeSite do AHK: foca o navegador, lê a URL atual pelo
/// Ctrl+L/Ctrl+C (restaurando o clipboard) e, se não estiver no site,
/// usa a busca de abas (Ctrl+Shift+A).
fn browser_tab(matches: &[String], search: &str, url: &str, f: &WinFilter, run: &str) -> Result<bool, String> {
    let Some(w) = win::find(f).into_iter().next() else {
        win::shell_execute(run, Some(url), None)?;
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            sleep(100);
            if let Some(w) = win::find(f).into_iter().next() {
                focus(&w, true);
                break;
            }
        }
        return Ok(true);
    };
    focus(&w, true);
    sleep(150);
    let backup = win::clip_backup();
    win::clip_clear();
    let seq = win::clip_seq();
    win::chord(&[VK_CONTROL], b'L' as u16);
    sleep(100);
    win::chord(&[VK_CONTROL], b'C' as u16);
    let t = Instant::now();
    while win::clip_seq() == seq && t.elapsed() < Duration::from_secs(1) {
        sleep(20);
    }
    let current = win::clip_text().to_lowercase();
    win::clip_restore(&backup);
    if !matches.iter().any(|m| current.contains(&m.to_lowercase())) {
        win::chord(&[VK_CONTROL, VK_SHIFT], b'A' as u16);
        sleep(150);
        win::type_text(search);
        sleep(150);
        win::chord(&[], VK_RETURN);
    }
    Ok(true)
}

/// Porte do ActivateNextDynamicWindow: vai para a próxima janela que não
/// é de um app "catalogado" (que já tem atalho próprio).
fn next_window(skip: &[WinFilter]) -> bool {
    let all: Vec<win::WinInfo> = win::list_windows()
        .into_iter()
        .filter(|w| !w.cloaked && !skip.iter().any(|f| win::matches(w, f)))
        .collect();
    if all.is_empty() {
        return false;
    }
    let fg = win::foreground() as isize;
    let idx = match all.iter().position(|w| w.hwnd == fg) {
        Some(i) => (i + 1) % all.len(),
        None => 0,
    };
    focus(&all[idx], true);
    true
}
