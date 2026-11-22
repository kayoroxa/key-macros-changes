#![windows_subsystem = "windows"]
//! KeyMacros.exe (fica na raiz do projeto).
//!
//! A cada clique:
//!   1. vê se algum arquivo do código mudou depois do último build (dist\keymacros.exe);
//!   2. se mudou: compila (abre um console mostrando o cargo), fecha o app antigo,
//!      copia o novo para dist\ e reabre;
//!   3. garante que o app está na bandeja e abre a janela do editor.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn find_window(class: Option<&str>, title: Option<&str>) -> HWND {
    let c = class.map(wide);
    let t = title.map(wide);
    unsafe {
        FindWindowW(
            c.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            t.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
        )
    }
}

fn msg(text: &str) {
    let t = wide("KeyMacros");
    let m = wide(text);
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_ICONERROR) };
}

/// Sobe a partir da pasta do exe até achar o Cargo.toml do keymacros.
fn project_root() -> Option<PathBuf> {
    let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    loop {
        let toml = d.join("Cargo.toml");
        if toml.is_file() && std::fs::read_to_string(&toml).map(|t| t.contains("name = \"keymacros\"")).unwrap_or(false) {
            return Some(d);
        }
        d = d.parent()?.to_path_buf();
    }
}

fn newest(path: &Path, best: &mut SystemTime) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.is_dir() {
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                newest(&e.path(), best);
            }
        }
    } else if let Ok(m) = meta.modified() {
        if m > *best {
            *best = m;
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn wait_gone(class: Option<&str>, title: Option<&str>) {
    let h = find_window(class, title);
    if h.is_null() {
        return;
    }
    unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
    let t = Instant::now();
    while !find_window(class, title).is_null() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn copy_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let t = Instant::now();
    loop {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            // o processo antigo pode demorar um instante para soltar o arquivo
            Err(e) if t.elapsed() < Duration::from_secs(8) => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }
}

fn pause_console() {
    println!("\nPressione Enter para fechar.");
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

const DAEMON_CLASS: &str = "KeyMacrosDaemon";
const EDITOR_TITLE: &str = "KeyMacros — Editor";

fn main() {
    let Some(root) = project_root() else {
        msg("Não achei o projeto keymacros (Cargo.toml). Deixe o KeyMacros.exe dentro da pasta do projeto.");
        return;
    };
    let dist = root.join("dist");
    let app = dist.join("keymacros.exe");

    let mut src_time = SystemTime::UNIX_EPOCH;
    for p in ["src", "config/exemplo.kbd", "Cargo.toml", "Cargo.lock"] {
        newest(&root.join(p), &mut src_time);
    }
    let needs_build = mtime(&app).is_none_or(|t| src_time > t);

    if needs_build {
        unsafe {
            use windows_sys::Win32::System::Console::*;
            AllocConsole();
            // sem "Edição rápida": um clique no console não pausa mais o build
            let h = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(h, &mut mode) != 0 {
                SetConsoleMode(h, (mode | ENABLE_EXTENDED_FLAGS) & !ENABLE_QUICK_EDIT_MODE);
            }
        }
        println!("KeyMacros: o código mudou desde o último build — compilando...\n");
        let ok = Command::new("cargo").args(["build", "--release"]).current_dir(&root).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            println!("\nERRO: o build falhou. O app antigo continua como estava.");
            pause_console();
            return;
        }
        let was_running = !find_window(Some(DAEMON_CLASS), None).is_null();
        println!("\nBuild ok. Atualizando dist\\ ...");
        wait_gone(None, Some(EDITOR_TITLE));
        wait_gone(Some(DAEMON_CLASS), None);
        let _ = std::fs::create_dir_all(&dist);
        if let Err(e) = copy_retry(&root.join("target/release/keymacros.exe"), &app) {
            println!("ERRO ao copiar keymacros.exe: {e}");
            pause_console();
            return;
        }
        // o build não muda a hora do exe se nada foi recompilado: marca como atualizado
        let _ = std::fs::File::options().write(true).open(&app).and_then(|f| f.set_modified(SystemTime::now()));
        let kanata = root.join("tools/kanata/bin/kanata.exe");
        if !dist.join("kanata.exe").exists() && kanata.exists() {
            let _ = std::fs::copy(&kanata, dist.join("kanata.exe"));
        }
        let _ = std::fs::copy(root.join("README.md"), dist.join("README.md"));
        let _ = std::fs::copy(root.join("config/exemplo.kbd"), dist.join("exemplo.kbd"));
        if !dist.join("keymacros.kbd").exists() {
            let _ = std::fs::copy(root.join("config/keymacros.kbd"), dist.join("keymacros.kbd"));
        }
        println!("Pronto{}.", if was_running { " — reabrindo o app" } else { "" });
        std::thread::sleep(Duration::from_millis(600));
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    }

    if find_window(Some(DAEMON_CLASS), None).is_null() {
        if let Err(e) = Command::new(&app).current_dir(&dist).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            msg(&format!("Não consegui abrir {}: {e}", app.display()));
            return;
        }
        let t = Instant::now();
        while find_window(Some(DAEMON_CLASS), None).is_null() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // com --startup (atalho na pasta Iniciar do Windows) só deixa na bandeja, sem abrir o editor
    if std::env::args().any(|a| a == "--startup") {
        return;
    }
    let _ = Command::new(&app).arg("editor").current_dir(&dist).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
