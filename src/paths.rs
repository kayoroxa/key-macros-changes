//! Onde ficam o kanata, a config e os logs.

use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 37210;
pub const TEMPLATE: &str = include_str!("../config/exemplo.kbd");

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

fn appdata_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(exe_dir);
    base.join("KeyMacros")
}

/// Ordem: `--cfg`, `keymacros.kbd` ao lado do .exe (modo portátil),
/// `%APPDATA%\KeyMacros\keymacros.kbd` (criado a partir do modelo).
pub fn config_path(arg: Option<&str>) -> PathBuf {
    if let Some(a) = arg {
        return PathBuf::from(a);
    }
    let portable = exe_dir().join("keymacros.kbd");
    if portable.exists() {
        return portable;
    }
    appdata_dir().join("keymacros.kbd")
}

/// Cria a config a partir do modelo se ela não existir.
/// Se o SharpKeys já trocou o Win por F13, o modelo usa `f13` como líder.
pub fn ensure_config(p: &Path) -> std::io::Result<bool> {
    if p.exists() {
        return Ok(false);
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let leader = crate::scancode::read_system()
        .ok()
        .flatten()
        .and_then(|m| crate::scancode::sharpkeys_leader(&m))
        .unwrap_or("lmet");
    std::fs::write(p, template_for(leader))?;
    Ok(true)
}

pub fn template_for(leader: &str) -> String {
    if leader == "lmet" {
        return TEMPLATE.to_string();
    }
    // Com SharpKeys: Win físico já chega como F13 e Ctrl direito já chega como Win.
    TEMPLATE
        .replace("  lmet @lider\n  rctl lmet\n", &format!("  {leader} @lider\n  ;; rctl -> lmet já é feito pelo SharpKeys\n"))
        .replace("(tap-hold-press 0 200 lmet ", &format!("(tap-hold-press 0 200 {leader} "))
}

pub fn kanata_exe() -> Option<PathBuf> {
    let d = exe_dir();
    let candidates = [
        d.join("kanata.exe"),
        d.join("tools").join("kanata").join("bin").join("kanata.exe"),
        d.join("..").join("..").join("tools").join("kanata").join("bin").join("kanata.exe"),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|p| p.join("kanata.exe")).find(|p| p.is_file()))
}

pub fn log_path(cfg: &Path) -> PathBuf {
    cfg.with_file_name("keymacros-kanata.log")
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for c2 in it.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Roda `kanata --check` + a validação dos `push-msg`. Ok(()) se estiver tudo certo.
pub fn check_config(cfg: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(cfg).map_err(|e| format!("não consegui ler {}: {e}", cfg.display()))?;
    check_text(&text, Some(cfg))
}

pub fn check_text(text: &str, cfg: Option<&Path>) -> Result<(), String> {
    let errs = crate::kbd::check_push_msgs(text)?;
    if !errs.is_empty() {
        return Err(errs.iter().map(|(l, e)| format!("linha {l}: {e}")).collect::<Vec<_>>().join("\n"));
    }
    let Some(k) = kanata_exe() else {
        return Err("kanata.exe não encontrado (coloque-o ao lado do keymacros.exe)".into());
    };
    // valida uma cópia temporária ao lado da original (para `include` relativo funcionar)
    let dir = cfg.and_then(|c| c.parent().map(Path::to_path_buf)).unwrap_or_else(std::env::temp_dir);
    let tmp = dir.join(format!(".keymacros-check-{}.kbd", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new(k)
        .arg("--cfg")
        .arg(&tmp)
        .arg("--check")
        .env("NO_COLOR", "1")
        .creation_flags(0x0800_0000)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| format!("falha ao rodar kanata: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut msg = strip_ansi(&String::from_utf8_lossy(&out.stderr));
    msg.push_str(&strip_ansi(&String::from_utf8_lossy(&out.stdout)));
    let tmpname = tmp.file_name().unwrap().to_string_lossy().to_string();
    let msg: Vec<&str> = msg.lines().filter(|l| !l.contains("[INFO]") && !l.trim().is_empty()).collect();
    Err(msg.join("\n").replace(&tmpname, "config"))
}
