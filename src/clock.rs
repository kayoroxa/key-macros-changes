//! Relógio pequeno (HH:MM) sempre visível, feito só com Win32.
//!
//! - Modo fixo (padrão): fica no canto inferior direito, acima da barra de tarefas.
//!   Se o mouse chegar perto, troca na hora para o canto superior direito
//!   (sem animação). Quando o mouse sai da região, volta para baixo.
//! - Modo "segue o mouse": o relógio fica sempre logo abaixo do cursor.
//! - Opacidade de 50 % e clique através: não atrapalha o mouse.
//! - `WDA_EXCLUDEFROMCAPTURE`: o Windows não mostra a janela em gravações
//!   nem em compartilhamento de tela feitos pelas APIs de captura do sistema.
//!   Exige Windows 10 versão 2004 ou superior.

use crate::win;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU16, Ordering};
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
/// Intervalo do timer: 16 ms ≈ 60 quadros/s (antes 150 ms ≈ 6-7/s, o que deixava o relógio atrasado).
const TICK_MS: u32 = 16;
/// 128 de 255 = 50 % de opacidade.
const ALPHA: u8 = 128;

static HWND_CLOCK: AtomicIsize = AtomicIsize::new(0);
/// true = o relógio segue o cursor; false = fica no canto (sobe perto do mouse)
static FOLLOW: AtomicBool = AtomicBool::new(false);
/// última posição aplicada na janela (evita mover a janela à toa)
static LAST_X: AtomicI32 = AtomicI32::new(i32::MIN);
static LAST_Y: AtomicI32 = AtomicI32::new(i32::MIN);
/// Distância (px) entre a âncora do cursor e o relógio no modo "segue o mouse".
/// Definida em `set_enabled`: altura da seta + folga. Com 24 px o relógio cobria a ponta da seta.
static GAP: AtomicI32 = AtomicI32::new(52);
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

/// Área de trabalho do monitor onde está o ponto (cai no primário se não achar).
fn work_area_at(p: POINT) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            return mi.rcWork;
        }
    }
    work_area()
}

/// Modo "segue o mouse". Função pura (só conta, sem Win32), para dar para testar.
/// Centralizada logo abaixo do cursor (`gap` px abaixo de `cy`); se não couber embaixo,
/// fica acima. Nunca sai da área de trabalho `wa`.
pub fn follow_pos(cx: i32, cy: i32, wa: RECT, w: i32, h: i32, gap: i32) -> (i32, i32) {
    let mut y = cy + gap;
    if y + h > wa.bottom {
        y = cy - gap - h;
    }
    let x = (cx - w / 2).clamp(wa.left, wa.right - w);
    let y = y.clamp(wa.top, wa.bottom - h);
    (x, y)
}

fn tick(hwnd: HWND) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    let (x, y) = if FOLLOW.load(Ordering::Relaxed) {
        follow_pos(p.x, p.y, work_area_at(p), W, H, GAP.load(Ordering::Relaxed))
    } else {
        // sobe só enquanto o mouse estiver perto do relógio no canto inferior.
        // O canto superior não é gatilho: senão, clicar nos botões de fechar das janelas faria o relógio subir.
        spot(near(p.x, p.y, false))
    };
    if x != LAST_X.load(Ordering::Relaxed) || y != LAST_Y.load(Ordering::Relaxed) {
        LAST_X.store(x, Ordering::Relaxed);
        LAST_Y.store(y, Ordering::Relaxed);
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

/// Escolhe o modo: `true` = segue o mouse, `false` = fixo no canto.
/// Vale na hora, sem recriar a janela.
pub fn set_follow(on: bool) {
    FOLLOW.store(on, Ordering::Relaxed);
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
        LAST_X.store(i32::MIN, Ordering::Relaxed);
        LAST_Y.store(i32::MIN, Ordering::Relaxed);
        // folga = altura da seta do sistema + 20 px (lido uma vez, não a cada tick)
        GAP.store(GetSystemMetrics(SM_CYCURSOR) + 20, Ordering::Relaxed);
        LAST_KEY.store(u16::MAX, Ordering::Relaxed);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::time::Instant;

    /// Seta do cursor em 100 % de escala.
    const CURSOR: i32 = 32;
    /// Área de trabalho fixa, para o resultado não depender da tela.
    const AREA: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    /// Antes da mudança: 150 ms e folga de 24 px. Depois: TICK_MS e folga = seta + 20.
    /// Varredura (folga 36/44/52/60): só com 52 ou mais não há sobreposição em movimento rápido.
    const OLD_TICK_MS: u32 = 150;
    const OLD_GAP: i32 = 24;
    const NEW_GAP: i32 = CURSOR + 20;

    fn hit(ax: i32, ay: i32, aw: i32, ah: i32, bx: i32, by: i32, bw: i32, bh: i32) -> bool {
        ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
    }

    struct Run {
        /// maior distância (px) entre o cursor e a âncora usada pela janela
        lag_max: i32,
        /// ms em que o relógio cobriu a seta
        overlap_ms: u32,
        /// vezes que a janela realmente mudou de posição
        updates: u32,
    }

    /// Simulação determinística com tempo virtual: cursor em linha reta (vx, vy em px/s),
    /// janela recalculada só nos ticks, como o WM_TIMER faz. Um passo = 1 ms.
    fn simulate(tick_ms: u32, gap: i32, vx: i32, vy: i32, dur_ms: u32) -> Run {
        let mut r = Run { lag_max: 0, overlap_ms: 0, updates: 0 };
        let (mut wx, mut wy) = (i32::MIN, i32::MIN);
        let (mut ax, mut ay) = (0, 0);
        for t in 0..dur_ms {
            let cx = 960 + vx * t as i32 / 1000;
            let cy = 300 + vy * t as i32 / 1000;
            if t % tick_ms == 0 {
                ax = cx;
                ay = cy;
                let (x, y) = follow_pos(cx, cy, AREA, W, H, gap);
                if (x, y) != (wx, wy) {
                    wx = x;
                    wy = y;
                    r.updates += 1;
                }
            }
            r.lag_max = r.lag_max.max((cx - ax).abs().max((cy - ay).abs()));
            if hit(wx, wy, W, H, cx, cy, CURSOR, CURSOR) {
                r.overlap_ms += 1;
            }
        }
        r
    }

    #[test]
    fn parado_nunca_cobre_a_seta() {
        // grade de posições do cursor, incluindo bordas e cantos
        for cx in (AREA.left..AREA.right).step_by(13) {
            for cy in (AREA.top..AREA.bottom).step_by(11) {
                let (x, y) = follow_pos(cx, cy, AREA, W, H, NEW_GAP);
                assert!(!hit(x, y, W, H, cx, cy, CURSOR, CURSOR), "cobre a seta em ({cx},{cy}) -> ({x},{y})");
                assert!(x >= AREA.left && x + W <= AREA.right, "fora da área horizontal: x={x}");
                assert!(y >= AREA.top && y + H <= AREA.bottom, "fora da área vertical: y={y}");
            }
        }
    }

    #[test]
    fn controle_folga_antiga_cobre_a_seta() {
        // prova que o teste pega o bug: com a folga antiga (24 px) o relógio cobre a seta parada
        let old = simulate(OLD_TICK_MS, OLD_GAP, 0, 0, 1000);
        assert!(old.overlap_ms > 0, "o controle deveria detectar a sobreposição antiga");
        let new = simulate(TICK_MS, NEW_GAP, 0, 0, 1000);
        assert_eq!(new.overlap_ms, 0, "com a folga nova não pode cobrir a seta");
    }

    #[test]
    fn comparativo_atraso_quadros_e_sobreposicao() {
        let cenarios = [("parado", 0, 0), ("devagar", 300, 0), ("rapido", 900, 600)];
        for (nome, vx, vy) in cenarios {
            let old = simulate(OLD_TICK_MS, OLD_GAP, vx, vy, 1000);
            let new = simulate(TICK_MS, NEW_GAP, vx, vy, 1000);
            eprintln!(
                "{nome:>8}: atraso {OLD_TICK_MS}->{TICK_MS} ms | quadros/s {}->{} | lag {}->{} px | cobre a seta {}->{} ms",
                1000 / OLD_TICK_MS,
                1000 / TICK_MS,
                old.lag_max,
                new.lag_max,
                old.overlap_ms,
                new.overlap_ms
            );
            assert!(new.lag_max <= old.lag_max, "{nome}: lag não pode piorar");
            assert!(new.overlap_ms <= old.overlap_ms, "{nome}: sobreposição não pode piorar");
            if vx != 0 || vy != 0 {
                assert!(new.updates > old.updates, "{nome}: precisa atualizar mais vezes");
            }
        }
    }

    /// Só informativo (tempo varia de máquina para máquina, por isso não há assert).
    #[test]
    fn custo_do_calculo_por_posicao() {
        const N: u32 = 1_000_000;
        let t0 = Instant::now();
        let mut acc = 0i64;
        for i in 0..N {
            let (x, y) = follow_pos(black_box(i as i32 % 1900), black_box(500), AREA, W, H, NEW_GAP);
            acc += (x + y) as i64;
        }
        let dt = t0.elapsed();
        black_box(acc);
        eprintln!("follow_pos: {:.1} ns por chamada ({N} chamadas em {dt:?})", dt.as_nanos() as f64 / N as f64);
    }
}
