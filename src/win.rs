//! Helpers Win32: janelas, processos, envio de teclas, clipboard.

use crate::msg::WinFilter;

use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Marca nos eventos que o KeyMacros injeta.
pub const EXTRA_INFO_TAG: usize = 0x4B4D_4143; // "KMAC"
/// Tecla "não atribuída" usada para mascarar Win/Alt (igual ao #MenuMaskKey vkE8 do AHK).
const VK_MASK: u16 = 0xE8;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Expande `%VAR%` (ex.: `%LocalAppData%`).
pub fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = wide(s);
    let mut buf = vec![0u16; 4096];
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 || n as usize > buf.len() {
        return s.to_string();
    }
    from_wide(&buf)
}

#[derive(Debug, Clone)]
pub struct WinInfo {
    pub hwnd: isize,
    pub title: String,
    pub class: String,
    /// nome do executável em minúsculas (ex.: `claude.exe`)
    pub exe: String,
    pub path: String,
    pub cloaked: bool,
}

impl WinInfo {
    pub fn h(&self) -> HWND {
        self.hwnd as HWND
    }
}

pub fn process_path(pid: u32) -> String {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn info(h: HWND) -> WinInfo {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    let path = process_path(pid);
    let exe = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    let mut cloaked: u32 = 0;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, (&mut cloaked as *mut u32).cast(), 4) };
    WinInfo { hwnd: h as isize, title: window_text(h), class: class_name(h), exe, path, cloaked: cloaked != 0 }
}

/// Janelas "de verdade" (as que aparecem no Alt+Tab): visíveis, com título,
/// sem dono e que não são tool windows. Ordenadas por HWND (ordem estável).
pub fn list_windows() -> Vec<WinInfo> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let v = &mut *(lp as *mut Vec<isize>);
        v.push(h as isize);
        1
    }
    let mut hs: Vec<isize> = vec![];
    unsafe { EnumWindows(Some(cb), &mut hs as *mut _ as LPARAM) };
    let mut out = vec![];
    for h in hs {
        let h = h as HWND;
        unsafe {
            if IsWindowVisible(h) == 0 || !GetWindow(h, GW_OWNER).is_null() {
                continue;
            }
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW != 0 {
                continue;
            }
        }
        let i = info(h);
        if i.title.is_empty() {
            continue;
        }
        // "fantasmas" de apps UWP suspensos
        if i.cloaked && matches!(i.class.as_str(), "ApplicationFrameWindow" | "Windows.UI.Core.CoreWindow") {
            continue;
        }
        out.push(i);
    }
    out.sort_by_key(|w| w.hwnd);
    out
}

fn ci_contains(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

pub fn matches(w: &WinInfo, f: &WinFilter) -> bool {
    if let Some(e) = &f.exe {
        if !w.exe.eq_ignore_ascii_case(e) {
            return false;
        }
    }
    if let Some(t) = &f.title {
        if !ci_contains(&w.title, t) {
            return false;
        }
    }
    if let Some(t) = &f.title_ends {
        if !w.title.to_lowercase().trim_end().ends_with(&t.to_lowercase()) {
            return false;
        }
    }
    if let Some(c) = &f.class {
        if !w.class.eq_ignore_ascii_case(c) {
            return false;
        }
    }
    true
}

/// Janelas que batem com o filtro, na ordem do ciclo:
/// `prefer` primeiro, depois as visíveis na área de trabalho atual, depois o resto.
pub fn find(f: &WinFilter) -> Vec<WinInfo> {
    let mut v: Vec<WinInfo> = list_windows().into_iter().filter(|w| matches(w, f)).collect();
    v.sort_by_key(|w| {
        let pref = f.prefer.as_ref().is_some_and(|p| ci_contains(&w.title, p));
        (!pref, w.cloaked, w.hwnd)
    });
    v
}

pub fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

fn key_input(vk: u16, up: bool) -> INPUT {
    let sc = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let ext = matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C | 0x5D | 0x6F | 0x90 | 0xA3 | 0xA5);
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if ext {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: sc, dwFlags: flags, time: 0, dwExtraInfo: EXTRA_INFO_TAG } },
    }
}

fn send(inputs: &[INPUT]) {
    if !inputs.is_empty() {
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }
}

pub fn is_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Se Win ou Alt estiverem pressionados, toca uma tecla neutra para o
/// Windows não abrir o menu Iniciar / barra de menus ao soltar.
pub fn mask_win_alt() {
    if [VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU].iter().any(|&k| is_down(k)) {
        send(&[key_input(VK_MASK, false), key_input(VK_MASK, true)]);
    }
}

/// Toca um atalho, ex.: `chord(&[VK_CONTROL], 'L' as u16)`.
pub fn chord(mods: &[u16], vk: u16) {
    let mut v = vec![];
    for &m in mods {
        v.push(key_input(m, false));
    }
    v.push(key_input(vk, false));
    v.push(key_input(vk, true));
    for &m in mods.iter().rev() {
        v.push(key_input(m, true));
    }
    send(&v);
}

/// Digita texto via unicode (não depende do layout ABNT2/US).
pub fn type_text(s: &str) {
    let mut v = vec![];
    for ch in s.chars() {
        if ch == '\n' {
            v.push(key_input(VK_RETURN, false));
            v.push(key_input(VK_RETURN, true));
            continue;
        }
        let mut buf = [0u16; 2];
        for &u in ch.encode_utf16(&mut buf).iter() {
            for up in [false, true] {
                v.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT { wVk: 0, wScan: u, dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: EXTRA_INFO_TAG },
                    },
                });
            }
        }
    }
    send(&v);
}

/// Traz a janela para frente de forma confiável (o Windows bloqueia
/// SetForegroundWindow de processos em segundo plano).
pub fn activate(h: HWND) -> bool {
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        if GetForegroundWindow() == h {
            return true;
        }
        // Um evento de teclado nosso libera o "foreground lock".
        send(&[key_input(VK_MASK, false), key_input(VK_MASK, true)]);
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, 1) != 0;
        SetForegroundWindow(h);
        BringWindowToTop(h);
        if attached {
            AttachThreadInput(me, fg_tid, 0);
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(300) {
            if GetForegroundWindow() == h {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        GetForegroundWindow() == h
    }
}

pub fn maximize(h: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        if style & WS_MAXIMIZEBOX != 0 && IsZoomed(h) == 0 {
            ShowWindow(h, SW_MAXIMIZE);
        }
    }
}

pub fn minimize(h: HWND) {
    unsafe { ShowWindow(h, SW_MINIMIZE) };
}

pub fn close(h: HWND) {
    unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
}

pub fn toggle_topmost(h: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let after = if ex & WS_EX_TOPMOST != 0 { HWND_NOTOPMOST } else { HWND_TOPMOST };
        SetWindowPos(h, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
}

/// Executa um programa, arquivo, URL ou `shell:AppsFolder\...`.
pub fn shell_execute(file: &str, args: Option<&str>, dir: Option<&str>) -> Result<(), String> {
    let file = expand_env(file.trim());
    let args = args.map(expand_env);
    let dir = dir.map(expand_env).or_else(|| {
        let p = std::path::Path::new(&file);
        if p.is_absolute() && p.is_file() {
            p.parent().map(|d| d.to_string_lossy().into_owned())
        } else {
            None
        }
    });
    let f = wide(&file);
    let op = wide("open");
    let a = args.as_deref().map(wide);
    let d = dir.as_deref().map(wide);
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            f.as_ptr(),
            a.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            d.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            SW_SHOWNORMAL,
        )
    } as isize;
    if r <= 32 {
        Err(format!("não consegui executar \"{file}\" (código {r})"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Clipboard (backup completo dos formatos baseados em memória)

pub struct ClipBackup(Vec<(u32, Vec<u8>)>);

fn open_clipboard() -> bool {
    for _ in 0..20 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

pub fn clip_backup() -> ClipBackup {
    let mut out = vec![];
    if !open_clipboard() {
        return ClipBackup(out);
    }
    unsafe {
        let mut fmt = 0u32;
        loop {
            fmt = EnumClipboardFormats(fmt);
            if fmt == 0 {
                break;
            }
            // formatos baseados em handles GDI são sintetizados pelo Windows
            if matches!(fmt, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E) {
                continue;
            }
            let h = GetClipboardData(fmt);
            if h.is_null() {
                continue;
            }
            let size = GlobalSize(h as HGLOBAL);
            let p = GlobalLock(h as HGLOBAL) as *const u8;
            if p.is_null() || size == 0 {
                continue;
            }
            out.push((fmt, std::slice::from_raw_parts(p, size).to_vec()));
            GlobalUnlock(h as HGLOBAL);
        }
        CloseClipboard();
    }
    ClipBackup(out)
}

pub fn clip_restore(b: &ClipBackup) {
    if !open_clipboard() {
        return;
    }
    unsafe {
        EmptyClipboard();
        for (fmt, data) in &b.0 {
            let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
            if h.is_null() {
                continue;
            }
            let p = GlobalLock(h) as *mut u8;
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            GlobalUnlock(h);
            if SetClipboardData(*fmt, h as HANDLE).is_null() {
                GlobalFree(h);
            }
        }
        CloseClipboard();
    }
}

pub fn clip_seq() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

pub fn clip_text() -> String {
    if !open_clipboard() {
        return String::new();
    }
    let mut s = String::new();
    unsafe {
        let h = GetClipboardData(13); // CF_UNICODETEXT
        if !h.is_null() {
            let p = GlobalLock(h as HGLOBAL) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                GlobalUnlock(h as HGLOBAL);
            }
        }
        CloseClipboard();
    }
    s
}

pub fn clip_clear() {
    if open_clipboard() {
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

/// Processos rodando com nome do exe (minúsculo).
pub fn running_exes() -> Vec<String> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let mut out = vec![];
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                out.push(from_wide(&e.szExeFile).to_lowercase());
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

pub fn message_box(title: &str, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(title);
    let m = wide(text);
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), flags) }
}


// ---------------------------------------------------------------------------
// Ícones de programas (para o editor)

fn file_exists(p: &str) -> bool {
    !p.is_empty() && std::path::Path::new(p).is_file()
}

/// Caminho completo de um `.exe` pelo nome: App Paths do registro e depois o PATH.
pub fn find_exe_path(exe: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::SearchPathW;
    use windows_sys::Win32::System::Registry::*;
    let exe = exe.trim();
    if exe.is_empty() || exe.contains(['*', '?']) {
        return None;
    }
    let sub = wide(&format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}"));
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(root, sub.as_ptr(), std::ptr::null(), RRF_RT_ANY, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        if r == 0 {
            let p = expand_env(from_wide(&buf).trim().trim_matches('"'));
            if file_exists(&p) {
                return Some(p);
            }
        }
    }
    let name = wide(exe);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { SearchPathW(std::ptr::null(), name.as_ptr(), std::ptr::null(), buf.len() as u32, buf.as_mut_ptr(), std::ptr::null_mut()) };
    if n > 0 && (n as usize) < buf.len() {
        let p = String::from_utf16_lossy(&buf[..n as usize]);
        if file_exists(&p) {
            return Some(p);
        }
    }
    None
}

/// Primeiro ícone de um .exe/.dll/.ico como RGBA `size`×`size`.
pub fn file_icon_rgba(path: &str, size: i32) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::*;
    if !file_exists(path) {
        return None;
    }
    let p = wide(path);
    unsafe {
        let mut icon: HICON = std::ptr::null_mut();
        let n = PrivateExtractIconsW(p.as_ptr(), 0, size, size, &mut icon, std::ptr::null_mut(), 1, 0);
        if n == 0 || n == u32::MAX || icon.is_null() {
            return None;
        }
        let mut ii: ICONINFO = std::mem::zeroed();
        let mut out = None;
        if GetIconInfo(icon, &mut ii) != 0 && !ii.hbmColor.is_null() {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = size;
            bmi.bmiHeader.biHeight = -size; // de cima para baixo
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut px = vec![0u8; (size * size * 4) as usize];
            let dc = GetDC(std::ptr::null_mut());
            let lines = GetDIBits(dc, ii.hbmColor, 0, size as u32, px.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
            ReleaseDC(std::ptr::null_mut(), dc);
            if lines == size {
                // ícones antigos sem canal alfa: tudo opaco
                let no_alpha = px.chunks(4).all(|c| c[3] == 0);
                for c in px.chunks_mut(4) {
                    c.swap(0, 2);
                    if no_alpha {
                        c[3] = 255;
                    }
                }
                out = Some(px);
            }
        }
        if !ii.hbmColor.is_null() {
            DeleteObject(ii.hbmColor);
        }
        if !ii.hbmMask.is_null() {
            DeleteObject(ii.hbmMask);
        }
        DestroyIcon(icon);
        out
    }
}
