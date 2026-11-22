//! Recursos opcionais, ligados/desligados na aba "Recursos" do editor.
//! Ficam em `keymacros-recursos.json`, na mesma pasta da config. Tudo vem
//! desativado por padrão.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Features {
    pub clock: bool,
}

pub fn file(cfg: &Path) -> PathBuf {
    cfg.with_file_name("keymacros-recursos.json")
}

pub fn load(cfg: &Path) -> Features {
    let Ok(text) = std::fs::read_to_string(file(cfg)) else {
        return Features::default();
    };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Features {
        clock: v.get("relogio").and_then(Value::as_bool).unwrap_or(false),
    }
}

pub fn save(cfg: &Path, f: Features) -> std::io::Result<()> {
    let v = json!({ "relogio": f.clock });
    std::fs::write(file(cfg), serde_json::to_string_pretty(&v).unwrap_or_default())
}
