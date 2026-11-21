//! Backups automáticos. Toda mudança (config ou registro) grava antes uma cópia
//! do estado anterior em `backups\` ao lado da config. Nada aqui apaga arquivos:
//! cada backup tem data e hora no nome e, se já existir um igual, ganha `-2`, `-3`...

use std::path::{Path, PathBuf};

/// `registro` = Scancode Map (.reg que restaura o estado), `config` = keymacros.kbd.
pub const KIND_REG: &str = "registro";
pub const KIND_CFG: &str = "config";

pub fn dir(cfg: &Path) -> PathBuf {
    cfg.parent().map(Path::to_path_buf).unwrap_or_default().join("backups")
}

/// `20261120-174512` (hora local).
pub fn stamp() -> String {
    #[cfg(windows)]
    {
        let mut t = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }
    #[cfg(not(windows))]
    {
        format!("{}", now_unix())
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Grava um backup novo (nunca sobrescreve). `note` vai no nome (ex.: `antes-de-aplicar`).
pub fn save(cfg: &Path, kind: &str, note: &str, ext: &str, content: &[u8]) -> std::io::Result<PathBuf> {
    let d = dir(cfg);
    std::fs::create_dir_all(&d)?;
    let base = if note.is_empty() { format!("{kind}-{}", stamp()) } else { format!("{kind}-{}-{note}", stamp()) };
    let mut p = d.join(format!("{base}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = d.join(format!("{base}-{n}.{ext}"));
        n += 1;
    }
    std::fs::write(&p, content)?;
    Ok(p)
}

pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// "20/11/2026 17:45:12"
    pub when: String,
}

/// Backups de um tipo, do mais novo para o mais velho.
pub fn list(cfg: &Path, kind: &str) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir(cfg)) else { return vec![] };
    let prefix = format!("{kind}-");
    let mut v: Vec<Entry> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_string();
            let rest = name.strip_prefix(&prefix)?;
            Some(Entry { when: pretty_stamp(rest), name, path: p })
        })
        .collect();
    v.sort_by(|a, b| b.name.cmp(&a.name));
    v
}

fn pretty_stamp(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 15 && b[8] == b'-' && b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit) {
        format!("{}/{}/{} {}:{}:{}", &s[6..8], &s[4..6], &s[0..4], &s[9..11], &s[11..13], &s[13..15])
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_overwrites_and_lists_newest_first() {
        let d = std::env::temp_dir().join(format!("km-backup-test-{}", std::process::id()));
        let cfg = d.join("keymacros.kbd");
        let a = save(&cfg, KIND_CFG, "", "kbd", b"um").unwrap();
        let b = save(&cfg, KIND_CFG, "", "kbd", b"dois").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"um");
        assert_eq!(std::fs::read(&b).unwrap(), b"dois");
        let l = list(&cfg, KIND_CFG);
        assert_eq!(l.len(), 2);
        assert!(!l[0].when.is_empty());
        assert!(list(&cfg, KIND_REG).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pretty() {
        assert_eq!(pretty_stamp("20261120-174512-x.reg"), "20/11/2026 17:45:12");
    }
}
