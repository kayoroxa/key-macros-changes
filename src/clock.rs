//! Relógio pequeno (HH:MM) sempre visível, feito só com Win32.
//!
//! - Fica no canto inferior direito, acima da barra de tarefas.
//! - Se o mouse chegar perto, troca na hora para o canto superior direito
//!   (sem animação). Quando o mouse sai da região, volta para baixo.
//! - Opacidade de 50 % e clique através: não atrapalha o mouse.
//! - `WDA_EXCLUDEFROMCAPTURE`: o Windows não mostra a janela em gravações
//!   nem em compartilhamento de tela feitos pelas APIs de captura do sistema.
//!   Exige Windows 10 versão 2004 ou superior.

use crate::win;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU16, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "KeyMacrosClock";
const W: i32 = 56;
const H: i32 = 22;
const MARGIN: i32 = 6;
/// Distância (px) em volta do relógio que já conta como "mouse perto".
const ZONE: i32 = 40;
const TIMER: usize = 7;
const TICK_MS: u32 = 150;
/// 128 de 255 = 50 % de opacidade.
const ALPHA: u8 = 128;

static HWND_CLOCK: AtomicIsize = AtomicIsize::new(0);
/// true = está no canto superior
static AT_TOP: AtomicBool = AtomicBool::new(false);
/// hora*60+minuto do último desenho (evita repintar à toa)
static LAST_KEY: AtomicU16 = AtomicU16::new(u16::MAX);

fn work_area() -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, std::ptr::addr_of_mut!(r).cast(), 0);
    }
    r
}

/// Canto do relógio: direito; em cima ou embaixo, dentro da área de trabalho.
fn spot(top: bool) -> (i32, i32) {
    let wa = work_area();
    let x = wa.right - W - MARGIN;
    let y = if top { wa.top + MARGIN } else { wa.bottom - H - MARGIN };
    (x, y)
}

fn near(px: i32, py: i32, top: bool) -> bool {
    let (x, y) = spot(top);
    px >= x - ZONE && px < x + W + ZONE && py >= y - ZONE && py < y + H + ZONE
}

fn tick(hwnd: HWND) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    // sobe enquanto o mouse estiver perto de qualquer um dos dois cantos
    let want_top = near(p.x, p.y, true) || near(p.x, p.y, false);
    if want_top != AT_TOP.load(Ordering::Relaxed) {
        AT_TOP.store(want_top, Ordering::Relaxed);
        let (x, y) = spot(want_top);
        unsafe { SetWindowPos(hwnd, HWND_TOPMOST, x, y, W, H, SWP_NOACTIVATE | SWP_NOSIZE) };
    }
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let key = st.wHour * 60 + st.wMinute;
    if LAST_KEY.swap(key, Ordering::Relaxed) != key {
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
    }
}

fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };

    // fundo escuro (vira 50 % por causa do LWA_ALPHA)
    let bg = unsafe { CreateSolidBrush(rgb(17, 24, 39)) };
    unsafe {
        FillRect(hdc, &rc, bg);
        DeleteObject(bg as _);
    }

    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let text = win::wide(&format!("{:02}:{:02}", st.wHour, st.wMinute));
    let face = win::wide("Segoe UI");
    unsafe {
        let font = CreateFontW(-14, 0, 0, 0, 600, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        let old = SelectObject(hdc, font as _);
        SetBkMode(hdc, TRANSPARENT as _);
        SetTextColor(hdc, rgb(232, 238, 246));
        DrawTextW(hdc, text.as_ptr(), -1, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(hdc, old);
        DeleteObject(font as _);
    }
    unsafe { EndPaint(hwnd, &ps) };
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            SetLayeredWindowAttributes(hwnd, 0, ALPHA, LWA_ALPHA);
            SetTimer(hwnd, TIMER, TICK_MS, None);
            0
        }
        WM_TIMER if wp == TIMER => {
            tick(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_DESTROY => {
            KillTimer(hwnd, TIMER);
            HWND_CLOCK.store(0, Ordering::SeqCst);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Liga (cria a janela) ou desliga (destrói). Idempotente.
/// Erro = não deu para ocultar da captura (Windows antigo) ou criar a janela.
pub fn set_enabled(on: bool) -> Result<(), String> {
    let cur = HWND_CLOCK.load(Ordering::SeqCst) as HWND;
    if !on {
        if !cur.is_null() {
            unsafe { DestroyWindow(cur) };
        }
        return Ok(());
    }
    if !cur.is_null() {
        return Ok(());
    }
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = win::wide(CLASS);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
        };
        // se a classe já existir, o RegisterClassW falha e tudo bem
        RegisterClassW(&wc);
        let (x, y) = spot(false);
        let title = win::wide("Relógio");
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            y,
            W,
            H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("não consegui criar a janela do relógio".into());
        }
        if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0 {
            DestroyWindow(hwnd);
            return Err("o relógio invisível para gravação precisa do Windows 10 versão 2004 ou superior".into());
        }
        HWND_CLOCK.store(hwnd as isize, Ordering::SeqCst);
        AT_TOP.store(false, Ordering::Relaxed);
        LAST_KEY.store(u16::MAX, Ordering::Relaxed);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}
