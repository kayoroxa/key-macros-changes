//! Processo residente: ícone na bandeja, supervisiona o kanata, recebe os
//! `push-msg` pelo TCP e executa as ações. Não usa nenhum framework de UI
//! (só Win32), por isso fica com poucos MB de RAM.

use crate::{actions, msg, paths, win};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: &str = "KeyMacrosDaemon";
/// Mensagens que o editor pode mandar para o daemon (PostMessage).
pub const WM_KM_TOGGLE_PAUSE: u32 = WM_APP + 10;
pub const WM_KM_RELOAD: u32 = WM_APP + 11;
pub const WM_KM_SET_PAUSED: u32 = WM_APP + 12; // wparam = 1 pausa, 0 ativa

const WM_KM_EVENT: u32 = WM_APP + 1;
const WM_KM_TRAY: u32 = WM_APP + 2;
const TIMER_POLL: usize = 1;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ID_STATUS: usize = 100;
const ID_EDITOR: usize = 101;
const ID_PAUSE: usize = 102;
const ID_RELOAD: usize = 103;
const ID_FOLDER: usize = 104;
const ID_LOG: usize = 105;
const ID_AUTOSTART: usize = 106;
const ID_EXIT: usize = 107;

enum Ev {
    Layer(String),
    Connected(bool),
    CfgChecked(Result<(), String>),
    Notify { title: String, text: String, error: bool },
}

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static QUEUE: Mutex<VecDeque<Ev>> = Mutex::new(VecDeque::new());

fn post(ev: Ev) {
    QUEUE.lock().unwrap().push_back(ev);
    let h = MAIN_HWND.load(Ordering::SeqCst);
    if h != 0 {
        unsafe { PostMessageW(h as HWND, WM_KM_EVENT, 0, 0) };
    }
}

fn notify_err(title: &str, text: impl Into<String>) {
    post(Ev::Notify { title: title.into(), text: text.into(), error: true });
}

struct State {
    hwnd: HWND,
    cfg: PathBuf,
    port: u16,
    kanata: Option<Child>,
    job: HANDLE,
    paused: bool,
    error: Option<String>,
    layer: String,
    connected: bool,
    icons: [HICON; 3], // ativo, pausado, erro
    tcp: Arc<Mutex<Option<TcpStream>>>,
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

// ---------------------------------------------------------------------------
// Ícone desenhado em código (não precisa de arquivo .ico)

const ICON_N: i32 = 32;

/// Desenha o ícone (quadrado arredondado com um "K") em RGBA 32x32.
pub fn icon_rgba(rgb: [u8; 3]) -> Vec<u8> {
    const N: i32 = ICON_N;
    let mut px = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let (cx, cy) = ((x as f32 - 15.5).abs(), (y as f32 - 15.5).abs());
            let r = 7.0f32;
            let dx = (cx - (15.5 - r)).max(0.0);
            let dy = (cy - (15.5 - r)).max(0.0);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let stem = (9..13).contains(&x) && (7..25).contains(&y);
            let arm_x = 13.0 + (y as f32 - 16.0).abs() * 0.8;
            let arm = (7..25).contains(&y) && (x as f32) >= arm_x - 0.5 && (x as f32) < arm_x + 3.5;
            let c = if stem || arm { [255, 255, 255] } else { rgb };
            let i = ((y * N + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    px
}

fn make_icon(rgb: [u8; 3]) -> HICON {
    const N: i32 = ICON_N;
    // DIB espera BGRA
    let mut px = icon_rgba(rgb);
    for p in px.chunks_mut(4) {
        p.swap(0, 2);
    }
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = N;
        bmi.bmiHeader.biHeight = -N;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hdc = GetDC(std::ptr::null_mut());
        let color = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ReleaseDC(std::ptr::null_mut(), hdc);
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
        let mask_bits = vec![0u8; (N * N / 8) as usize];
        let mask = CreateBitmap(N, N, 1, 1, mask_bits.as_ptr().cast());
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

// ---------------------------------------------------------------------------
// Bandeja

fn copy_w(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n
}

fn status_text(s: &State) -> String {
    if let Some(e) = &s.error {
        let first = e.lines().next().unwrap_or("");
        return format!("Erro: {first}");
    }
    if s.paused {
        return "Pausado — teclado normal".into();
    }
    if s.kanata.is_none() {
        return "Iniciando…".into();
    }
    if !s.connected {
        return "Ativo (conectando…)".into();
    }
    format!("Ativo — camada: {}", s.layer)
}

fn tray_update(s: &State, add: bool) {
    let mut n = nid(s.hwnd);
    n.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    n.uCallbackMessage = WM_KM_TRAY;
    n.hIcon = if s.error.is_some() { s.icons[2] } else if s.paused { s.icons[1] } else { s.icons[0] };
    copy_w(&mut n.szTip, &format!("KeyMacros\n{}", status_text(s)));
    unsafe { Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &n) };
}

fn balloon(hwnd: HWND, title: &str, text: &str, error: bool) {
    let mut n = nid(hwnd);
    n.uFlags = NIF_INFO;
    copy_w(&mut n.szInfoTitle, title);
    copy_w(&mut n.szInfo, if text.is_empty() { " " } else { text });
    n.dwInfoFlags = if error { NIIF_ERROR } else { NIIF_INFO };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) };
}

// ---------------------------------------------------------------------------
// Autostart (HKCU\...\Run)

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

fn autostart_enabled() -> bool {
    let k = win::wide(RUN_KEY);
    let v = win::wide("KeyMacros");
    let mut len = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut len) == 0 }
}

fn set_autostart(on: bool, cfg: &Path) -> Result<(), String> {
    let k = win::wide(RUN_KEY);
    let v = win::wide("KeyMacros");
    unsafe {
        let mut hk: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_SET_VALUE, &mut hk) != 0 {
            return Err("não consegui abrir a chave Run".into());
        }
        let r = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = win::wide(&format!("\"{}\" --cfg \"{}\"", exe.display(), cfg.display()));
            RegSetValueExW(hk, v.as_ptr(), 0, REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32)
        } else {
            RegDeleteValueW(hk, v.as_ptr())
        };
        RegCloseKey(hk);
        if r != 0 && on {
            return Err(format!("erro {r}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// kanata

fn create_job() -> HANDLE {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job
    }
}

fn start_kanata(s: &mut State) {
    if s.kanata.is_some() {
        return;
    }
    if let Err(e) = paths::check_config(&s.cfg) {
        s.error = Some(e.clone());
        tray_update(s, false);
        balloon(s.hwnd, "Config com erro — teclado continua normal", &e, true);
        return;
    }
    let Some(exe) = paths::kanata_exe() else {
        s.error = Some("kanata.exe não encontrado".into());
        tray_update(s, false);
        return;
    };
    let log = paths::log_path(&s.cfg);
    if std::fs::metadata(&log).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&log);
    }
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log);
    let mut cmd = Command::new(exe);
    cmd.arg("--cfg").arg(&s.cfg).arg("--port").arg(format!("127.0.0.1:{}", s.port)).arg("--nodelay");
    cmd.env("NO_COLOR", "1").creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    match out {
        Ok(f) => {
            let f2 = f.try_clone().ok();
            cmd.stdout(f);
            if let Some(f2) = f2 {
                cmd.stderr(f2);
            }
        }
        Err(_) => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            unsafe { AssignProcessToJobObject(s.job, child.as_raw_handle() as HANDLE) };
            s.kanata = Some(child);
            s.error = None;
            s.layer = "base".into();
        }
        Err(e) => {
            s.error = Some(format!("falha ao iniciar kanata: {e}"));
        }
    }
    tray_update(s, false);
}

fn stop_kanata(s: &mut State) {
    if let Some(mut c) = s.kanata.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    s.connected = false;
}

fn tcp_send(s: &State, line: &str) -> bool {
    let mut g = s.tcp.lock().unwrap();
    if let Some(stream) = g.as_mut() {
        if stream.write_all(format!("{line}\n").as_bytes()).is_ok() {
            return true;
        }
    }
    false
}

fn reload(s: &mut State) {
    if s.paused {
        return;
    }
    match paths::check_config(&s.cfg) {
        Err(e) => {
            balloon(s.hwnd, "Config com erro — mantive a anterior", &e, true);
        }
        Ok(()) => {
            s.error = None;
            if s.kanata.is_some() && tcp_send(s, r#"{"Reload":{}}"#) {
                balloon(s.hwnd, "KeyMacros", "Config recarregada", false);
            } else {
                stop_kanata(s);
                start_kanata(s);
            }
            tray_update(s, false);
        }
    }
}

fn state_file(cfg: &Path) -> PathBuf {
    cfg.with_file_name("keymacros-estado.txt")
}

/// Lembra se o teclado estava ativo. Sem arquivo = pausado (seguro).
fn save_active(cfg: &Path, active: bool) {
    let _ = std::fs::write(state_file(cfg), if active { "ativo" } else { "pausado" });
}

fn was_active(cfg: &Path) -> bool {
    std::fs::read_to_string(state_file(cfg)).map(|t| t.trim() == "ativo").unwrap_or(false)
}

fn set_paused(s: &mut State, paused: bool) {
    s.paused = paused;
    save_active(&s.cfg, !paused);
    if paused {
        stop_kanata(s);
        s.error = None;
    } else {
        start_kanata(s);
    }
    tray_update(s, false);
}

fn poll_kanata(s: &mut State) {
    let exited = match s.kanata.as_mut() {
        Some(c) => c.try_wait().ok().flatten(),
        None => None,
    };
    // SEGURANÇA: nunca religa o kanata sozinho. Se ele saiu (botão de pânico
    // Ctrl esq.+Espaço+Esc, crash, Gerenciador de Tarefas...), fica pausado com
    // o teclado normal até a pessoa clicar em "Ativar".
    if let Some(status) = exited {
        s.kanata = None;
        s.connected = false;
        s.paused = true;
        save_active(&s.cfg, false);
        let log = paths::strip_ansi(&std::fs::read_to_string(paths::log_path(&s.cfg)).unwrap_or_default());
        let last_lines: Vec<&str> = log.lines().rev().take(40).collect();
        if last_lines.iter().any(|l| l.contains("LControl+Space+Escape")) {
            balloon(s.hwnd, "Saída de emergência", "kanata encerrado (Ctrl+Espaço+Esc). Teclado normal. Para voltar: ícone → Ativar teclado.", false);
        } else {
            let errs: Vec<&str> = last_lines.iter().rev().filter(|l| l.contains("ERROR")).copied().collect();
            let tail = errs.iter().rev().take(2).rev().copied().collect::<Vec<_>>().join("\n");
            s.error = Some(format!("kanata parou ({status}). {tail}"));
            balloon(s.hwnd, "kanata parou — teclado normal", &format!("Nada foi religado. Veja o log. {tail}"), true);
        }
        tray_update(s, false);
    }
}

// ---------------------------------------------------------------------------
// Threads

fn tcp_thread(port: u16, writer: Arc<Mutex<Option<TcpStream>>>, tx: Sender<msg::Cmd>) {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    loop {
        let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
            std::thread::sleep(Duration::from_millis(400));
            continue;
        };
        let _ = stream.set_nodelay(true);
        *writer.lock().unwrap() = stream.try_clone().ok();
        post(Ev::Connected(true));
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if let Some(m) = v.get("MessagePush").and_then(|m| m.get("message")) {
                match msg::parse_json(m) {
                    Ok(cmd) => {
                        let _ = tx.send(cmd);
                    }
                    Err(e) => notify_err("push-msg inválido", format!("{m}: {e}")),
                }
            } else if let Some(n) = v.get("LayerChange").and_then(|m| m.get("new")).and_then(|n| n.as_str()) {
                post(Ev::Layer(n.to_string()));
            } else if let Some(e) = v.get("Error").and_then(|m| m.get("msg")).and_then(|n| n.as_str()) {
                notify_err("kanata", e);
            }
        }
        *writer.lock().unwrap() = None;
        post(Ev::Connected(false));
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn action_log(cfg: &Path, line: &str) {
    let p = cfg.with_file_name("keymacros-actions.log");
    if std::fs::metadata(&p).map(|m| m.len() > 256_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} {line}");
    }
}

fn watcher_thread(cfg: PathBuf) {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut last = mtime(&cfg);
    loop {
        std::thread::sleep(Duration::from_millis(800));
        let now = mtime(&cfg);
        if now != last {
            std::thread::sleep(Duration::from_millis(250));
            last = mtime(&cfg);
            post(Ev::CfgChecked(paths::check_config(&cfg)));
        }
    }
}

// ---------------------------------------------------------------------------
// Janela / menu

fn show_menu(hwnd: HWND) {
    let Some((status, paused)) = with(|s| (status_text(s), s.paused)) else { return };
    unsafe {
        let m = CreatePopupMenu();
        let add = |id: usize, text: &str, flags: u32| {
            let w = win::wide(text);
            AppendMenuW(m, MF_STRING | flags, id, w.as_ptr());
        };
        add(ID_STATUS, &status, MF_GRAYED);
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        add(ID_EDITOR, "Abrir editor de macros", 0);
        add(ID_PAUSE, if paused { "Ativar teclado (iniciar kanata)" } else { "Pausar (teclado normal)" }, 0);
        add(ID_RELOAD, "Recarregar config", 0);
        add(ID_FOLDER, "Abrir pasta da config", 0);
        add(ID_LOG, "Ver log do kanata", 0);
        add(ID_AUTOSTART, "Iniciar com o Windows", if autostart_enabled() { MF_CHECKED } else { 0 });
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        add(ID_EXIT, "Sair", 0);
        SetMenuDefaultItem(m, ID_EDITOR as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(m, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
    }
}

pub fn open_editor(cfg: &Path) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("editor").arg("--cfg").arg(cfg).spawn();
    }
}

fn on_command(id: usize) {
    let Some(cfg) = with(|s| s.cfg.clone()) else { return };
    match id {
        ID_EDITOR => open_editor(&cfg),
        ID_PAUSE => {
            with(|s| set_paused(s, !s.paused));
        }
        ID_RELOAD => {
            with(reload);
        }
        ID_FOLDER => {
            let _ = Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", cfg.display())).spawn();
        }
        ID_LOG => {
            let _ = win::shell_execute("notepad.exe", Some(&format!("\"{}\"", paths::log_path(&cfg).display())), None);
        }
        ID_AUTOSTART => {
            if let Err(e) = set_autostart(!autostart_enabled(), &cfg) {
                with(|s| balloon(s.hwnd, "Iniciar com o Windows", &e, true));
            }
        }
        ID_EXIT => unsafe {
            DestroyWindow(MAIN_HWND.load(Ordering::SeqCst) as HWND);
        },
        _ => {}
    }
}

/// Libera páginas não usadas do working set (o que aparece no Gerenciador de Tarefas).
fn trim_ram() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

fn drain_events() {
    loop {
        let ev = QUEUE.lock().unwrap().pop_front();
        let Some(ev) = ev else { break };
        with(|s| match ev {
            Ev::Layer(l) => {
                s.layer = l;
                tray_update(s, false);
            }
            Ev::Connected(c) => {
                s.connected = c;
                tray_update(s, false);
            }
            Ev::CfgChecked(Ok(())) => reload(s),
            Ev::CfgChecked(Err(e)) => balloon(s.hwnd, "Config com erro — mantive a anterior", &e, true),
            Ev::Notify { title, text, error } => balloon(s.hwnd, &title, &text, error),
        });
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KM_EVENT => drain_events(),
        WM_KM_TRAY => match lp as u32 {
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => on_command(ID_EDITOR),
            _ => {}
        },
        WM_COMMAND => on_command(wp & 0xFFFF),
        WM_TIMER if wp == TIMER_POLL => {
            with(poll_kanata);
            // devolve ao Windows a memória que não está em uso (a cada 30 s)
            static TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if TICKS.fetch_add(1, Ordering::Relaxed) % 30 == 2 {
                trim_ram();
            }
        }
        WM_KM_TOGGLE_PAUSE => {
            with(|s| set_paused(s, !s.paused));
        }
        WM_KM_SET_PAUSED => {
            with(|s| set_paused(s, wp != 0));
        }
        WM_KM_RELOAD => {
            with(reload);
        }
        WM_DESTROY => {
            with(|s| {
                stop_kanata(s);
                let n = nid(s.hwnd);
                Shell_NotifyIconW(NIM_DELETE, &n);
            });
            PostQuitMessage(0);
        }
        _ => {
            let tc = with(|s| s.taskbar_created).unwrap_or(0);
            if tc != 0 && msg == tc {
                // o Explorer reiniciou: recoloca o ícone
                with(|s| tray_update(s, true));
                return 0;
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
    }
    0
}

pub fn find_daemon() -> HWND {
    let c = win::wide(CLASS);
    unsafe { FindWindowW(c.as_ptr(), std::ptr::null()) }
}

pub fn run(cfg: PathBuf, port: u16, start_paused: bool) -> i32 {
    unsafe {
        let name = win::wide("Local\\KeyMacrosDaemon");
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // já está rodando: só abre o editor
            open_editor(&cfg);
            return 0;
        }
    }
    let start_paused = start_paused || !was_active(&cfg);
    if let Err(e) = paths::ensure_config(&cfg) {
        win::message_box("KeyMacros", &format!("Não consegui criar a config em {}:\n{e}", cfg.display()), MB_ICONERROR);
        return 1;
    }

    let (tx, rx) = channel::<msg::Cmd>();
    let tcp = Arc::new(Mutex::new(None));
    {
        let tcp = tcp.clone();
        std::thread::spawn(move || tcp_thread(port, tcp, tx));
    }
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            for cmd in rx {
                win::mask_win_alt();
                let r = actions::exec(&cmd, &|t| post(Ev::Notify { title: "KeyMacros".into(), text: t.into(), error: false }));
                action_log(&cfg, &format!("{cmd:?} -> {r:?}"));
                if let Err(e) = r {
                    notify_err("Erro na macro", e);
                }
            }
        });
    }
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || watcher_thread(cfg));
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = win::wide(CLASS);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, cls.as_ptr(), cls.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        MAIN_HWND.store(hwnd as isize, Ordering::SeqCst);
        let tc = win::wide("TaskbarCreated");
        let state = State {
            hwnd,
            cfg: cfg.clone(),
            port,
            kanata: None,
            job: create_job(),
            paused: start_paused,
            error: None,
            layer: "base".into(),
            connected: false,
            icons: [make_icon([37, 99, 235]), make_icon([120, 120, 128]), make_icon([220, 38, 38])],
            tcp,
            taskbar_created: RegisterWindowMessageW(tc.as_ptr()),
        };
        STATE.with(|s| *s.borrow_mut() = Some(state));
        with(|s| tray_update(s, true));
        SetTimer(hwnd, TIMER_POLL, 1000, None);

        let ahk = win::running_exes().into_iter().any(|e| e.starts_with("autohotkey"));
        if ahk && !start_paused {
            balloon(hwnd, "AutoHotkey está rodando", "Os atalhos do AHK e do kanata podem conflitar. Feche o AHK ou pause o KeyMacros.", true);
        }
        if start_paused {
            balloon(hwnd, "KeyMacros pausado", "Teclado normal. Clique com o botão direito no ícone → Ativar teclado.", false);
        } else {
            with(start_kanata);
        }

        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    0
}
