//! Tecla líder de cada camada `km-*`: o que a ativa e como trocar.
//!
//! Uma camada pode ser ativada por:
//! - uma tecla da camada base, segurada (`f13 @lider`, `lalt @alt`);
//! - teclas apertadas juntas (`(r t) (one-shot ... (layer-while-held km-x))` no `defchordsv2`);
//! - uma tecla dentro de outra camada (`b ... (layer-while-held km-sites)` na `km-lider`).

use crate::kbd::{self, Act, Binding, Sx};
use crate::{chords, remap};
use std::collections::HashMap;

/// Depois das teclas juntas, quanto tempo a camada espera a tecla do atalho.
pub const ONE_SHOT_MS: u32 = 2000;
/// Segurar a tecla líder por mais que isso (sem apertar outra) manda a própria tecla.
pub const HOLD_MS: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// tecla da camada base (segurar)
    Key(String),
    /// teclas apertadas juntas
    Chord(Vec<String>),
    /// tecla dentro de outra camada
    Inside { layer: String, key: String },
    None,
}

/// Modificadores: segurando um deles, a tecla líder não abre a camada, manda a
/// própria tecla na hora (ex.: Alt+Tab precisa do Tab apertado já no "down").
pub const MODIFIERS: &[&str] = &["lsft", "rsft", "lctl", "rctl", "lalt", "ralt", "lmet", "rmet"];

/// `(lsft rsft ...)` sem as teclas da própria líder (um líder que é modificador não se bloqueia).
fn modifiers_except(keys: &[String]) -> String {
    format!("({})", MODIFIERS.iter().filter(|m| !keys.iter().any(|k| k == *m)).copied().collect::<Vec<_>>().join(" "))
}

/// Líder de uma tecla: segurar abre a camada, tocar manda a tecla. Com modificador
/// já segurado, manda a tecla imediatamente (a camada não abre).
pub fn key_leader_action(key: &str, layer: &str) -> String {
    format!("(fork (tap-hold-press 0 {HOLD_MS} {key} (layer-while-held {layer})) {key} {})", modifiers_except(&[key.to_string()]))
}

const LAYER_ACTIONS: &[&str] = &["layer-while-held", "layer-toggle", "layer-switch"];

type Aliases<'a> = HashMap<String, &'a Sx>;

fn aliases(top: &[Sx]) -> Aliases<'_> {
    let mut out = HashMap::new();
    for form in top.iter().filter(|f| f.head() == Some("defalias")) {
        for kv in form.list().unwrap()[1..].chunks(2) {
            if let (Some(name), Some(v)) = (kv[0].atom(), kv.get(1)) {
                out.insert(name.to_string(), v);
            }
        }
    }
    out
}

/// A ação liga a camada `layer` (direto ou por um alias `@x`)?
fn activates(sx: &Sx, layer: &str, al: &Aliases, depth: u8) -> bool {
    if depth > 6 {
        return false;
    }
    match sx {
        Sx::Atom(a, _) => a.strip_prefix('@').and_then(|n| al.get(n)).is_some_and(|s| activates(s, layer, al, depth + 1)),
        Sx::List(items, _) => {
            if sx.head().is_some_and(|h| LAYER_ACTIONS.contains(&h)) && items.get(1).and_then(Sx::atom) == Some(layer) {
                return true;
            }
            items.iter().any(|i| activates(i, layer, al, depth))
        }
    }
}

fn layer_name(form: &Sx) -> Option<&str> {
    if form.head() != Some("deflayermap") {
        return None;
    }
    form.list()?.get(1)?.list()?.first()?.atom()
}

/// A camada base é a primeira do arquivo (se não for uma `km-*`).
fn base_pairs(top: &[Sx]) -> &[Sx] {
    for f in top {
        match f.head() {
            Some("deflayer") => return &[],
            Some("deflayermap") => {
                if layer_name(f).is_some_and(|n| n.starts_with(kbd::MANAGED_PREFIX)) {
                    return &[];
                }
                return f.list().unwrap().get(2..).unwrap_or(&[]);
            }
            _ => {}
        }
    }
    &[]
}

fn chord_activates(c: &chords::Chord, layer: &str, al: &Aliases) -> bool {
    let text = match &c.action {
        Act::Raw(r) => r.clone(),
        Act::Keys(k) => k.clone(),
        _ => return false,
    };
    kbd::parse(&text).is_ok_and(|v| v.iter().any(|sx| activates(sx, layer, al, 0)))
}

/// O que ativa a camada. Ordem: tecla da base, teclas juntas, tecla em outra camada.
pub fn find(src: &str, layer: &str) -> Trigger {
    let Ok(top) = kbd::parse(src) else { return Trigger::None };
    let al = aliases(&top);
    for kv in base_pairs(&top).chunks(2) {
        if kv.len() == 2 && activates(&kv[1], layer, &al, 0) {
            if let Some(k) = kv[0].atom() {
                return Trigger::Key(k.to_string());
            }
        }
    }
    if let Ok((_, list)) = chords::read(src) {
        if let Some(c) = list.iter().find(|c| chord_activates(c, layer, &al)) {
            return Trigger::Chord(c.keys.clone());
        }
    }
    for f in &top {
        let Some(name) = layer_name(f) else { continue };
        if name == layer || !name.starts_with(kbd::MANAGED_PREFIX) {
            continue;
        }
        for kv in f.list().unwrap()[2..].chunks(2) {
            if kv.len() == 2 && activates(&kv[1], layer, &al, 0) {
                if let Some(k) = kv[0].atom() {
                    return Trigger::Inside { layer: name.to_string(), key: k.to_string() };
                }
            }
        }
    }
    Trigger::None
}

/// `find` para todas as camadas de uma vez.
pub fn find_all(src: &str, layers: &[String]) -> HashMap<String, Trigger> {
    layers.iter().map(|l| (l.clone(), find(src, l))).collect()
}

/// Separa o que o usuário digitou: `r t`, `r+t`, `f13`.
/// Nomes de tecla do kanata com mais de uma letra.
fn is_key_name(k: &str) -> bool {
    const NAMES: &[&str] = &[
        "esc", "tab", "spc", "ret", "bspc", "del", "ins", "home", "end", "pgup", "pgdn", "up", "down", "left", "right", "caps", "lsft", "rsft", "lctl",
        "rctl", "lalt", "ralt", "lmet", "rmet", "menu", "grv", "apo", "min", "eql", "scln", "comma", "lbrc", "rbrc", "bksl", "slck", "pause", "prtsc", "nlck",
    ];
    NAMES.contains(&k) || (k.starts_with('f') && k[1..].parse::<u8>().is_ok()) || k.starts_with("kp")
}

pub fn parse_keys(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for k in s.split(|c: char| c.is_whitespace() || c == '+' || c == ',').filter(|k| !k.is_empty()) {
        let k = k.to_lowercase();
        // "rt" sem espaço = r + t (mas "f13", "spc", "lalt" são nomes de tecla)
        let parts: Vec<String> = if k.len() > 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) && !is_key_name(&k) { k.chars().map(String::from).collect() } else { vec![k] };
        for k in parts {
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// Combinação: abre a camada. Com modificador já segurado, as teclas saem normais.
fn chord_action(layer: &str, keys: &[String]) -> String {
    format!("(fork (one-shot {ONE_SHOT_MS} (layer-while-held {layer})) (multi {}) {})", keys.join(" "), modifiers_except(keys))
}

fn remove_chords_of(src: &str, layer: &str) -> Result<String, String> {
    let top = kbd::parse(src)?;
    let al = aliases(&top);
    let (_, mut list) = chords::read(src)?;
    let n = list.len();
    list.retain(|c| !chord_activates(c, layer, &al));
    if list.len() == n {
        return Ok(src.to_string());
    }
    chords::write(src, &list)
}

/// Define a tecla líder da camada: uma tecla (segurar) ou várias (apertar juntas).
pub fn set(src: &str, layer: &str, keys: &[String]) -> Result<String, String> {
    if keys.is_empty() {
        return Err("escolha a tecla líder".into());
    }
    for k in keys {
        if k.contains(|c: char| c.is_whitespace() || "()\";@$".contains(c)) {
            return Err(format!("tecla inválida: '{k}'"));
        }
    }
    let cur = find(src, layer);
    let out = if keys.len() == 1 {
        let k = &keys[0];
        match &cur {
            Trigger::Key(old) => remap::move_key(src, old, k)?,
            Trigger::Inside { layer: parent, key: old } => {
                if old == k {
                    return Ok(src.to_string());
                }
                let layers = kbd::managed_layers(src)?;
                let p = layers.iter().find(|l| &l.name == parent).ok_or(format!("camada {parent} não encontrada"))?;
                if p.bindings.iter().any(|b| &b.key == k) {
                    return Err(format!("a tecla {k} já tem um atalho na camada de onde esta é aberta"));
                }
                let v: Vec<Binding> = p.bindings.iter().map(|b| if &b.key == old { Binding { key: k.clone(), ..b.clone() } } else { b.clone() }).collect();
                kbd::replace_layer(src, parent, &v)?
            }
            Trigger::Chord(_) | Trigger::None => {
                if let Some(p) = remap::pairs(src)?.iter().find(|p| &p.key == k) {
                    return Err(format!("a tecla {k} já faz `{}` na camada base — escolha outra ou mude pela aba Código", p.action));
                }
                let s = remove_chords_of(src, layer)?;
                remap::set_action(&s, k, &key_leader_action(k, layer))?
            }
        }
    } else {
        if let Trigger::Chord(old) = &cur {
            let mut a = old.clone();
            let mut b = keys.to_vec();
            a.sort();
            b.sort();
            if a == b {
                return Ok(src.to_string());
            }
        }
        let mut s = src.to_string();
        if let Trigger::Key(old) = &cur {
            s = remap::remove(&s, old)?;
        }
        let top = kbd::parse(&s)?;
        let al = aliases(&top);
        let (_, mut list) = chords::read(&s)?;
        // as mesmas teclas faziam outra coisa (ex.: r+t = f14): passam a abrir esta camada
        let mine = list.iter().position(|c| chord_activates(c, layer, &al));
        let mut i = 0;
        list.retain(|c| {
            let keep = Some(i) == mine || !c.same_keys(keys);
            i += 1;
            keep
        });
        match list.iter_mut().find(|c| chord_activates(c, layer, &al)) {
            Some(c) => {
                c.keys = keys.to_vec();
                c.action = Act::Raw(chord_action(layer, keys));
            }
            None => list.push(chords::Chord::new(keys.to_vec(), Act::Raw(chord_action(layer, keys)))),
        }
        chords::write(&s, &list)?
    };
    kbd::managed_layers(&out)?;
    Ok(out)
}

/// Cria uma camada nova (com "outras teclas = nada", escondido no editor) e a tecla líder.
pub fn create_layer(src: &str, name: &str, keys: &[String]) -> Result<String, String> {
    let s = kbd::append_layer(src, name)?;
    let s = kbd::replace_layer(&s, name, &[Binding { key: "___".into(), tap: Act::Nothing, hold: None, name: String::new() }])?;
    if keys.is_empty() {
        return Ok(s);
    }
    set(&s, name, keys)
}

/// Apaga a camada e o que a ativa (tecla da base ou teclas juntas).
pub fn delete_layer(src: &str, layer: &str) -> Result<String, String> {
    let mut s = src.to_string();
    match find(src, layer) {
        Trigger::Key(k) => s = remap::remove(&s, &k)?,
        Trigger::Chord(_) => s = remove_chords_of(&s, layer)?,
        Trigger::Inside { layer: parent, key } => {
            return Err(format!("esta camada é aberta pela tecla {key} da camada {parent} — apague esse atalho lá primeiro"));
        }
        Trigger::None => {}
    }
    let l = kbd::managed_layers(&s)?.into_iter().find(|l| l.name == layer).ok_or(format!("camada {layer} não encontrada"))?;
    let after = s[l.span.end..].trim_start_matches([' ', '\t']);
    let after = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n')).unwrap_or(after);
    let out = format!("{}{}", s[..l.span.start].trim_end_matches([' ', '\t']), after);
    // algum alias ou outra camada ainda usa o nome?
    fn uses(sx: &Sx, name: &str) -> bool {
        match sx {
            Sx::Atom(a, _) => a == name,
            Sx::List(items, _) => items.iter().any(|i| uses(i, name)),
        }
    }
    if kbd::parse(&out)?.iter().any(|f| uses(f, layer)) {
        return Err(format!("a camada {layer} ainda é usada em outro lugar do arquivo (ex.: num alias) — apague pela aba Código"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = include_str!("../config/keymacros.kbd");

    #[test]
    fn finds_user_triggers() {
        assert_eq!(find(USER, "km-lider"), Trigger::Key("f13".into()));
        assert_eq!(find(USER, "km-alt"), Trigger::Key("lalt".into()));
        assert_eq!(find(USER, "km-win"), Trigger::Key("lmet".into()));
        assert_eq!(find(USER, "km-sites"), Trigger::Inside { layer: "km-lider".into(), key: "b".into() });
        assert_eq!(find(USER, "km-nada"), Trigger::None);
    }

    #[test]
    fn chord_layer_replaces_plain_chord() {
        let s = create_layer(USER, "km-rt", &["r".into(), "t".into()]).unwrap();
        assert_eq!(find(&s, "km-rt"), Trigger::Chord(vec!["r".into(), "t".into()]));
        assert!(s.contains("(r t) (fork (one-shot 2000 (layer-while-held km-rt)) (multi r t) (lsft rsft lctl rctl lalt ralt lmet rmet)) 40 first-release ()"));
        assert!(!s.contains("(r t) f14"));
        assert!(s.contains("(deflayermap (km-rt)\n  ___ XX\n)"));
        // troca as teclas da combinação
        let s2 = set(&s, "km-rt", &["r".into(), "y".into()]).unwrap();
        assert_eq!(find(&s2, "km-rt"), Trigger::Chord(vec!["r".into(), "y".into()]));
        // vira uma tecla só (segurar)
        let s3 = set(&s2, "km-rt", &["f14".into()]).unwrap();
        assert_eq!(find(&s3, "km-rt"), Trigger::Key("f14".into()));
        assert!(!s3.contains("km-rt))) 40"));
        assert!(s3.contains("f14 (fork (tap-hold-press 0 200 f14 (layer-while-held km-rt)) f14 (lsft rsft lctl rctl lalt ralt lmet rmet))"));
        // e volta a ser combinação: some da base
        let s4 = set(&s3, "km-rt", &["g".into(), "h".into()]).unwrap();
        assert_eq!(find(&s4, "km-rt"), Trigger::Chord(vec!["g".into(), "h".into()]));
        assert!(!s4.contains("f14 (fork"));
        // a líder que é modificador não bloqueia a si mesma
        assert_eq!(key_leader_action("lctl", "km-x"), "(fork (tap-hold-press 0 200 lctl (layer-while-held km-x)) lctl (lsft rsft rctl lalt ralt lmet rmet))");
        assert_eq!(chord_action("km-x", &["lctl".into(), "tab".into()]), "(fork (one-shot 2000 (layer-while-held km-x)) (multi lctl tab) (lsft rsft rctl lalt ralt lmet rmet))");
    }

    #[test]
    fn moves_key_and_inside_triggers() {
        let s = set(USER, "km-lider", &["f14".into()]).unwrap();
        assert_eq!(find(&s, "km-lider"), Trigger::Key("f14".into()));
        assert!(s.contains("(tap-hold-press 0 200 f14 (layer-while-held km-lider))"));
        assert!(set(USER, "km-lider", &["caps".into()]).is_err());
        let s = set(USER, "km-sites", &["w".into()]).unwrap();
        assert_eq!(find(&s, "km-sites"), Trigger::Inside { layer: "km-lider".into(), key: "w".into() });
        assert!(set(USER, "km-sites", &["c".into()]).is_err());
    }

    #[test]
    fn parses_typed_keys() {
        assert_eq!(parse_keys(" R + t "), vec!["r", "t"]);
        assert_eq!(parse_keys("f13"), vec!["f13"]);
        assert_eq!(parse_keys("a a"), vec!["a"]);
        assert_eq!(parse_keys("rt"), vec!["r", "t"]);
        assert_eq!(parse_keys("f13 lalt"), vec!["f13", "lalt"]);
    }
    #[test]
    fn deletes_layer() {
        let s = create_layer(USER, "km-rt", &["r".into(), "t".into()]).unwrap();
        let d = delete_layer(&s, "km-rt").unwrap();
        assert!(!d.contains("km-rt"));
        assert!(delete_layer(USER, "km-sites").is_err());
        assert!(delete_layer(USER, "km-lider").is_err());
        let k = create_layer(USER, "km-x", &["f15".into()]).unwrap();
        let d = delete_layer(&k, "km-x").unwrap();
        assert!(!d.contains("km-x") && !d.contains("f15 "));
    }
}
