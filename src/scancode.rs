//! Leitura do "Scancode Map" do Windows (o que o SharpKeys grava).
//! Só altera o registro quando o usuário clica em "Aplicar" (com aviso do UAC),
//! e sempre grava antes um backup `.reg` do estado anterior.

#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    pub from: u16,
    pub to: u16,
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Mapping>, String> {
    if bytes.len() < 16 {
        return Err("Scancode Map muito curto".into());
    }
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if count == 0 || bytes.len() < 12 + count * 4 {
        return Err("Scancode Map com tamanho inválido".into());
    }
    let mut out = vec![];
    for i in 0..count - 1 {
        let o = 12 + i * 4;
        let to = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let from = u16::from_le_bytes([bytes[o + 2], bytes[o + 3]]);
        out.push(Mapping { from, to });
    }
    Ok(out)
}

/// Teclas que o app sabe trocar: (scancode, nome legível, nome no kanata).
pub const KEYS: &[(u16, &str, &str)] = &[
    (0x0000, "(desligada)", "XX"),
    (0xE05B, "Windows esquerdo", "lmet"),
    (0xE05C, "Windows direito", "rmet"),
    (0x003A, "Caps Lock", "caps"),
    (0x001D, "Ctrl esquerdo", "lctl"),
    (0xE01D, "Ctrl direito", "rctl"),
    (0x0038, "Alt esquerdo", "lalt"),
    (0xE038, "Alt direito / AltGr", "ralt"),
    (0x002A, "Shift esquerdo", "lsft"),
    (0x0036, "Shift direito", "rsft"),
    (0xE05D, "Menu de contexto", "menu"),
    (0x0001, "Esc", "esc"),
    (0x000F, "Tab", "tab"),
    (0x001C, "Enter", "ret"),
    (0x000E, "Backspace", "bspc"),
    (0x0039, "Espaço", "spc"),
    (0xE052, "Insert", "ins"),
    (0xE053, "Delete", "del"),
    (0xE047, "Home", "home"),
    (0xE04F, "End", "end"),
    (0xE049, "Page Up", "pgup"),
    (0xE051, "Page Down", "pgdn"),
    (0xE037, "Print Screen", "prnt"),
    (0x0046, "Scroll Lock", "slck"),
    (0x0045, "Num Lock", "nlck"),
    (0x003B, "F1", "f1"),
    (0x003C, "F2", "f2"),
    (0x003D, "F3", "f3"),
    (0x003E, "F4", "f4"),
    (0x003F, "F5", "f5"),
    (0x0040, "F6", "f6"),
    (0x0041, "F7", "f7"),
    (0x0042, "F8", "f8"),
    (0x0043, "F9", "f9"),
    (0x0044, "F10", "f10"),
    (0x0057, "F11", "f11"),
    (0x0058, "F12", "f12"),
    (0x0064, "F13", "f13"),
    (0x0065, "F14", "f14"),
    (0x0066, "F15", "f15"),
    (0x0067, "F16", "f16"),
    (0x0068, "F17", "f17"),
    (0x0069, "F18", "f18"),
    (0x006A, "F19", "f19"),
    (0x006B, "F20", "f20"),
    (0x006C, "F21", "f21"),
    (0x006D, "F22", "f22"),
    (0x006E, "F23", "f23"),
    (0x0076, "F24", "f24"),
];

/// Nome legível de um scancode + nome no kanata.
pub fn name(sc: u16) -> (&'static str, &'static str) {
    KEYS.iter().find(|k| k.0 == sc).map(|k| (k.1, k.2)).unwrap_or(("?", "?"))
}

/// Scancode a partir do nome no kanata.
pub fn by_kanata(n: &str) -> Option<u16> {
    KEYS.iter().find(|k| k.2 == n).map(|k| k.0)
}

/// Nome legível a partir do nome no kanata (ou o próprio nome se não for conhecido).
pub fn label_kanata(n: &str) -> String {
    KEYS.iter().find(|k| k.2 == n).map(|k| k.1.to_string()).unwrap_or_else(|| n.to_string())
}

/// O que o kanata recebe quando a tecla física `sc` é apertada, dado o mapa do registro.
pub fn received(maps: &[Mapping], sc: u16) -> u16 {
    maps.iter().find(|m| m.from == sc).map(|m| m.to).unwrap_or(sc)
}

pub fn describe(m: &Mapping) -> String {
    format!("{} (0x{:04X})  →  {} (0x{:04X})", name(m.from).0, m.from, name(m.to).0, m.to)
}

#[cfg(windows)]
pub fn read_system() -> Result<Option<Vec<Mapping>>, String> {
    use windows_sys::Win32::System::Registry::*;
    let key: Vec<u16> = "SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout\0".encode_utf16().collect();
    let val: Vec<u16> = "Scancode Map\0".encode_utf16().collect();
    let mut buf = vec![0u8; 1024];
    let mut len = buf.len() as u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), val.as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
    };
    if r == 2 {
        return Ok(None); // ERROR_FILE_NOT_FOUND
    }
    if r != 0 {
        return Err(format!("erro {r} ao ler o registro"));
    }
    decode(&buf[..len as usize]).map(Some)
}

/// `.reg` que remove o Scancode Map (desfaz o SharpKeys). Requer reiniciar.
pub const REG_REMOVE: &str = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout]\r\n\"Scancode Map\"=-\r\n";

/// Bytes do valor "Scancode Map" (formato do Windows).
pub fn encode(maps: &[Mapping]) -> Vec<u8> {
    let mut bytes = vec![0u8; 8];
    bytes.extend_from_slice(&((maps.len() as u32 + 1).to_le_bytes()));
    for m in maps {
        bytes.extend_from_slice(&m.to.to_le_bytes());
        bytes.extend_from_slice(&m.from.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
}

/// `.reg` que deixa o registro exatamente com este mapa (vazio = remove o Scancode Map).
pub fn reg_for(maps: &[Mapping]) -> String {
    if maps.is_empty() {
        return REG_REMOVE.to_string();
    }
    let hex: Vec<String> = encode(maps).iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}\"Scancode Map\"=hex:{}\r\n",
        REG_REMOVE.strip_suffix("\"Scancode Map\"=-\r\n").unwrap(),
        hex.join(",")
    )
}

/// `.reg` em UTF-16 com BOM (o formato que o regedit/reg.exe esperam).
pub fn reg_bytes(reg: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in reg.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// Valida um mapa antes de gravar.
pub fn validate(maps: &[Mapping]) -> Result<(), String> {
    for (i, m) in maps.iter().enumerate() {
        if m.from == 0 {
            return Err("a tecla de origem não pode ser \"(desligada)\"".into());
        }
        if m.from == m.to {
            return Err(format!("{} trocada por ela mesma", name(m.from).0));
        }
        if maps[..i].iter().any(|o| o.from == m.from) {
            return Err(format!("{} aparece duas vezes como origem", name(m.from).0));
        }
    }
    Ok(())
}

/// Importa um `.reg` com `reg.exe` como administrador (o Windows mostra o aviso do UAC).
/// Bloqueia até terminar. Só é chamado por clique explícito do usuário.
#[cfg(windows)]
pub fn import_reg_elevated(file: &std::path::Path) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let verb = crate::win::wide("runas");
    let exe = crate::win::wide("reg.exe");
    let params = crate::win::wide(&format!("import \"{}\"", file.display()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: exe.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: 0,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = std::io::Error::last_os_error();
        return Err(if e.raw_os_error() == Some(1223) { "cancelado no aviso de administrador — nada foi alterado".into() } else { format!("não consegui pedir permissão de administrador: {e}") });
    }
    if info.hProcess.is_null() {
        return Err("reg.exe não iniciou".into());
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        windows_sys::Win32::Foundation::CloseHandle(info.hProcess);
    }
    if code != 0 {
        return Err(format!("reg.exe falhou (código {code})"));
    }
    Ok(())
}

/// Momento (unix, segundos) em que o Windows ligou pela última vez.
#[cfg(windows)]
pub fn boot_unix() -> u64 {
    let up = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() } / 1000;
    crate::backup::now_unix().saturating_sub(up)
}

fn pending_file(cfg: &std::path::Path) -> std::path::PathBuf {
    cfg.with_file_name("keymacros-registro-aplicado.txt")
}

/// Anota que o registro foi alterado agora (só vale depois de reiniciar).
pub fn mark_applied(cfg: &std::path::Path) {
    let _ = std::fs::write(pending_file(cfg), crate::backup::now_unix().to_string());
}

/// O registro foi alterado depois do último boot? (então a troca ainda não vale)
#[cfg(windows)]
pub fn reboot_pending(cfg: &std::path::Path) -> bool {
    std::fs::read_to_string(pending_file(cfg)).ok().and_then(|t| t.trim().parse::<u64>().ok()).is_some_and(|t| t > boot_unix() + 5)
}

/// Se o SharpKeys trocou LWin -> F13 o kanata recebe `f13`; senão recebe `lmet`.
pub fn sharpkeys_leader(maps: &[Mapping]) -> Option<&'static str> {
    maps.iter().find(|m| m.from == 0xE05B).map(|m| name(m.to).1).filter(|n| *n != "?")
}

#[cfg(test)]
mod tests {
    use super::*;

    // bytes reais lidos desta máquina
    const USER_MAP: [u8; 24] = [0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0x64, 0, 0x5B, 0xE0, 0x5B, 0xE0, 0x1D, 0xE0, 0, 0, 0, 0];

    #[test]
    fn decodes_sharpkeys_map() {
        let m = decode(&USER_MAP).unwrap();
        assert_eq!(m, vec![Mapping { from: 0xE05B, to: 0x0064 }, Mapping { from: 0xE01D, to: 0xE05B }]);
        assert_eq!(describe(&m[0]), "Windows esquerdo (0xE05B)  →  F13 (0x0064)");
        assert_eq!(sharpkeys_leader(&m), Some("f13"));
    }

    #[test]
    fn reg_roundtrip() {
        let m = decode(&USER_MAP).unwrap();
        let reg = reg_for(&m);
        assert!(reg.contains("hex:00,00,00,00,00,00,00,00,03,00,00,00,64,00,5b,e0,5b,e0,1d,e0,00,00,00,00"));
    }
}
