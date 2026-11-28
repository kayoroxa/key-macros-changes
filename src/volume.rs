//! Volume com Shift + roda do mouse.
//!
//! - Gancho de mouse de baixo nível (WH_MOUSE_LL) na thread do daemon: com Shift
//!   segurado, a rodada do mouse é engolida e vira volume (2 % por "clique" da roda).
//! - O volume é lido/gravado no dispositivo de som padrão (Core Audio,
//!   IAudioEndpointVolume), então vale para o Windows todo.
//! - Uma janelinha (barra vertical + "55%") aparece perto do cursor e some
//!   sozinha depois de um tempo sem mexer.

use crate::win;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "KeyMacrosVolume";
const W: i32 = 32;
const H: i32 = 92;
/// Quanto o volume muda por "clique" da roda (delta 120).
const STEP: f32 = 0.02;
/// Tempo (ms) sem mexer até a janelinha sumir.
const HIDE_MS: u32 = 1500;
const TIMER_HIDE: usize = 1;
/// Mensagem que o gancho posta para a janelinha (wParam = delta da roda).
const WM_VOL_WHEEL: u32 = WM_APP + 41;

static HWND_VOL: AtomicIsize = AtomicIsize::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);
/// volume mostrado, 0..=100
static SHOWN: AtomicI32 = AtomicI32::new(0);
/// sobra de rodadas pequenas (touchpad manda deltas menores que 120)
static ACC: AtomicI32 = AtomicI32::new(0);

// ---------- Core Audio (COM na mão: só os métodos usados) ----------

const CLSID_MM_DEVICE_ENUMERATOR: GUID = GUID::from_u128(0xBCDE0395_E52F_467C_8E3D_C4579291692E);
const IID_IMM_DEVICE_ENUMERATOR: GUID = GUID::from_u128(0xA95664D2_9614_4F35_A746_DE8DB63617E6);
const IID_IAUDIO_ENDPOINT_VOLUME: GUID = GUID::from_u128(0x5CDF2C82_841E_4546_9722_0CF74078229A);

type Raw = *mut std::ffi::c_void;

/// Ponteiro para o método `i` da vtable do objeto COM `obj`.
unsafe fn method(obj: Raw, i: usize) -> *const std::ffi::c_void {
    let vtbl = *(obj as *const *const *const std::ffi::c_void);
    *vtbl.add(i)
}

unsafe fn release(obj: Raw) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(Raw) -> u32 = std::mem::transmute(method(obj, 2));
        f(obj);
    }
}

/// IAudioEndpointVolume do dispositivo de saída padrão (pega de novo a cada uso,
/// assim trocar de fone/caixa no Windows já vale).
unsafe fn endpoint() -> Option<Raw> {
    let mut en: Raw = std::ptr::null_mut();
    if CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, std::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut en) < 0 || en.is_null() {
        return None;
    }
    // IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender = 0, eConsole = 0, &dev)
    let get_default: unsafe extern "system" fn(Raw, i32, i32, *mut Raw) -> i32 = std::mem::transmute(method(en, 4));
    let mut dev: Raw = std::ptr::null_mut();
    let hr = get_default(en, 0, 0, &mut dev);
    release(en);
    if hr < 0 || dev.is_null() {
        return None;
    }
    // IMMDevice::Activate(iid, CLSCTX_ALL, null, &vol)
    let activate: unsafe extern "system" fn(Raw, *const GUID, u32, Raw, *mut Raw) -> i32 = std::mem::transmute(method(dev, 3));
    let mut vol: Raw = std::ptr::null_mut();
    let hr = activate(dev, &IID_IAUDIO_ENDPOINT_VOLUME, CLSCTX_ALL, std::ptr::null_mut(), &mut vol);
    release(dev);
    if hr < 0 || vol.is_null() {
        return None;
    }
    Some(vol)
}

/// Soma `delta` (-1.0..1.0) ao volume. Tira o mudo. Devolve o novo volume (0..=100).
fn change(delta: f32) -> Option<i32> {
    unsafe {
        let vol = endpoint()?;
        // IAudioEndpointVolume: 7 SetMasterVolumeLevelScalar, 9 GetMasterVolumeLevelScalar, 14 SetMute
        let get: unsafe extern "system" fn(Raw, *mut f32) -> i32 = std::mem::transmute(method(vol, 9));
        let set: unsafe extern "system" fn(Raw, f32, *const GUID) -> i32 = std::mem::transmute(method(vol, 7));
        let mute: unsafe extern "system" fn(Raw, i32, *const GUID) -> i32 = std::mem::transmute(method(vol, 14));
        let mut cur = 0f32;
        let ok = get(vol, &mut cur) >= 0;
        let new = if ok { next_level(cur, delta) } else { 0.0 };
        if ok {
            set(vol, new, std::ptr::null());
            mute(vol, 0, std::ptr::null());
        }
        release(vol);
        ok.then(|| (new * 100.0).round() as i32)
    }
}

/// Próximo nível, preso em 0..=1 e arredondado no passo (evita 53,9 %).
pub fn next_level(cur: f32, delta: f32) -> f32 {
    let steps = ((cur + delta) / STEP).round();
    (steps * STEP).clamp(0.0, 1.0)
}

// ---------- gancho do mouse ----------

unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wp as u32 == WM_MOUSEWHEEL && GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000 != 0 {
        let hwnd = HWND_VOL.load(Ordering::Relaxed) as HWND;
        if !hwnd.is_null() {
            let info = &*(lp as *const MSLLHOOKSTRUCT);
            let delta = (info.mouseData >> 16) as u16 as i16;
            // não chama o áudio aqui: o gancho precisa voltar rápido
            PostMessageW(hwnd, WM_VOL_WHEEL, delta as isize as usize, 0);
            return 1; // engole: o app embaixo não rola
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
}

// ---------- janelinha ----------

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

fn on_wheel(hwnd: HWND, delta: i32) {
    let total = ACC.load(Ordering::Relaxed) + delta;
    let notches = total / 120;
    ACC.store(total % 120, Ordering::Relaxed);
    if notches != 0 {
        if let Some(v) = change(notches as f32 * STEP) {
            SHOWN.store(v, Ordering::Relaxed);
        }
    } else if unsafe { IsWindowVisible(hwnd) } != 0 {
        return;
    } else if let Some(v) = change(0.0) {
        SHOWN.store(v, Ordering::Relaxed);
    }
    show(hwnd);
}

/// Mostra perto do cursor (à direita; à esquerda se não couber) e reinicia o tempo para sumir.
fn show(hwnd: HWND) {
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        GetCursorPos(&mut p);
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let mut x = p.x + 20;
        if x + W > wa.right {
            x = p.x - 20 - W;
        }
        let x = x.clamp(wa.left, wa.right - W);
        let y = (p.y - H / 2).clamp(wa.top, wa.bottom - H);
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, W, H, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(hwnd, std::ptr::null(), 0);
        SetTimer(hwnd, TIMER_HIDE, HIDE_MS, None);
    }
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let v = SHOWN.load(Ordering::Relaxed).clamp(0, 100);

        let fill = |r: RECT, c: u32| {
            let b = CreateSolidBrush(c);
            FillRect(hdc, &r, b);
            DeleteObject(b as _);
        };
        // fundo branco com borda cinza
        fill(rc, rgb(160, 160, 160));
        fill(RECT { left: 1, top: 1, right: rc.right - 1, bottom: rc.bottom - 1 }, rgb(255, 255, 255));

        // "55%"
        let face = win::wide("Segoe UI");
        let font = CreateFontW(-10, 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        let old = SelectObject(hdc, font as _);
        SetBkMode(hdc, TRANSPARENT as _);
        SetTextColor(hdc, rgb(0, 0, 0));
        let text = win::wide(&format!("{v}%"));
        let mut tr = RECT { left: 0, top: 3, right: rc.right, bottom: 16 };
        DrawTextW(hdc, text.as_ptr(), -1, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(hdc, old);
        DeleteObject(font as _);

        // trilho vertical: parte cheia em azul de baixo até o nível
        let (top, bot) = (21, rc.bottom - 8);
        let cx = rc.right / 2;
        let track = RECT { left: cx - 1, top, right: cx + 2, bottom: bot };
        fill(track, rgb(220, 220, 220));
        let level = bot - (bot - top) * v / 100;
        fill(RECT { left: cx - 1, top: level, right: cx + 2, bottom: bot }, rgb(0, 120, 215));
        // marcador (o "botão" da barra)
        let ky = level.clamp(top + 3, bot - 3);
        fill(RECT { left: cx - 6, top: ky - 3, right: cx + 6, bottom: ky + 3 }, rgb(0, 120, 215));

        EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_VOL_WHEEL => {
            on_wheel(hwnd, wp as isize as i16 as i32);
            0
        }
        WM_TIMER if wp == TIMER_HIDE => {
            KillTimer(hwnd, TIMER_HIDE);
            ShowWindow(hwnd, SW_HIDE);
            ACC.store(0, Ordering::Relaxed);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_HIDE);
            HWND_VOL.store(0, Ordering::SeqCst);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Liga (janela + gancho) ou desliga. Idempotente. Chamar na thread do daemon
/// (que tem o loop de mensagens; o gancho precisa dele).
pub fn set_enabled(on: bool) -> Result<(), String> {
    let cur = HWND_VOL.load(Ordering::SeqCst) as HWND;
    unsafe {
        if !on {
            let h = HOOK.swap(0, Ordering::SeqCst);
            if h != 0 {
                UnhookWindowsHookEx(h as HHOOK);
            }
            if !cur.is_null() {
                DestroyWindow(cur);
            }
            return Ok(());
        }
        if !cur.is_null() {
            return Ok(());
        }
        // COM para o Core Audio (se já estiver iniciado, devolve S_FALSE: tudo bem)
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
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
        RegisterClassW(&wc);
        let title = win::wide("Volume");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            W,
            H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("não consegui criar a janela do volume".into());
        }
        HWND_VOL.store(hwnd as isize, Ordering::SeqCst);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), hinst, 0);
        if hook.is_null() {
            DestroyWindow(hwnd);
            return Err("não consegui ouvir a roda do mouse".into());
        }
        HOOK.store(hook as isize, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::next_level;

    #[test]
    fn passo_e_limites() {
        assert!((next_level(0.54, 0.02) - 0.56).abs() < 1e-4);
        assert!((next_level(0.539, 0.02) - 0.56).abs() < 1e-4);
        assert_eq!(next_level(0.99, 0.04), 1.0);
        assert_eq!(next_level(0.01, -0.02), 0.0);
    }
}
