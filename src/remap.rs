//! Edição da camada base do `.kbd`: trocas simples de tecla (`rctl lmet`) e a
//! tecla líder (`f13 @lider`). Mexe só nos trechos necessários; comentários e o
//! resto do arquivo ficam como estão.

use crate::kbd::{self, Sx};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub key: String,
    pub action: String,
    key_span: Range<usize>,
    act_span: Range<usize>,
}

impl Pair {
    /// `rctl lmet`, `caps XX` — troca simples (não é alias nem lista).
    pub fn is_swap(&self) -> bool {
        self.action == "XX" || (!self.action.is_empty() && self.action.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
    }
}

pub const LEADER_ALIAS: &str = "@lider";

struct Base {
    span: Range<usize>,
    pairs: Vec<Pair>,
}

/// A primeira camada do arquivo (é a que o kanata usa como base).
fn base(src: &str) -> Result<Option<Base>, String> {
    let top = kbd::parse(src)?;
    for form in &top {
        match form.head() {
            Some("deflayer") => return Err("a camada base usa (deflayer ...); mude-a pela aba Código".into()),
            Some("deflayermap") => {
                let items = form.list().unwrap();
                let name = items.get(1).and_then(|n| n.list()).and_then(|l| l.first()).and_then(|a| a.atom()).unwrap_or("");
                if name.starts_with(kbd::MANAGED_PREFIX) {
                    // camada de macros vindo primeiro: não há base própria ainda
                    return Ok(None);
                }
                let pairs = items.get(2..).unwrap_or(&[]);
                if pairs.len() % 2 != 0 {
                    return Err("a camada base tem uma tecla sem ação".into());
                }
                let pairs = pairs
                    .chunks(2)
                    .map(|kv| Pair { key: src[kv[0].span()].to_string(), action: src[kv[1].span()].to_string(), key_span: kv[0].span(), act_span: kv[1].span() })
                    .collect();
                return Ok(Some(Base { span: form.span(), pairs }));
            }
            _ => {}
        }
    }
    Ok(None)
}

pub fn pairs(src: &str) -> Result<Vec<Pair>, String> {
    Ok(base(src)?.map(|b| b.pairs).unwrap_or_default())
}

/// Trocas simples da camada base.
pub fn swaps(src: &str) -> Result<Vec<(String, String)>, String> {
    Ok(pairs(src)?.into_iter().filter(Pair::is_swap).map(|p| (p.key, p.action)).collect())
}

fn check_key(k: &str) -> Result<(), String> {
    if k.is_empty() || k.contains(|c: char| c.is_whitespace() || "()\";".contains(c)) {
        return Err(format!("nome de tecla inválido: '{k}'"));
    }
    Ok(())
}

/// Define `key action` na camada base (substitui se a tecla já existir).
pub fn set(src: &str, key: &str, action: &str) -> Result<String, String> {
    check_key(action)?;
    set_action(src, key, action)
}

/// Como `set`, mas a ação pode ser qualquer código kanata (ex.: `(tap-hold ...)`).
pub fn set_action(src: &str, key: &str, action: &str) -> Result<String, String> {
    check_key(key)?;
    kbd::parse(action).map_err(|e| format!("ação inválida: {e}"))?;
    let Some(b) = base(src)? else {
        // sem camada base: cria logo depois do (defsrc)
        let top = kbd::parse(src)?;
        let defsrc = top.iter().find(|f| f.head() == Some("defsrc")).ok_or("não achei o (defsrc) na config")?;
        let at = defsrc.span().end;
        return Ok(format!("{}\n\n(deflayermap (base)\n  {key} {action}\n){}", &src[..at], &src[at..]));
    };
    if let Some(p) = b.pairs.iter().find(|p| p.key == key) {
        return Ok(format!("{}{action}{}", &src[..p.act_span.start], &src[p.act_span.end..]));
    }
    let close = b.span.end - 1;
    let line_start = src[..close].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if src[line_start..close].trim().is_empty() {
        Ok(format!("{}  {key} {action}\n{}", &src[..line_start], &src[line_start..]))
    } else {
        Ok(format!("{}\n  {key} {action}\n{}", &src[..close], &src[close..]))
    }
}

/// Tira a tecla da camada base (some a linha inteira se ela só tinha isso).
pub fn remove(src: &str, key: &str) -> Result<String, String> {
    let Some(b) = base(src)? else { return Ok(src.to_string()) };
    let Some(p) = b.pairs.iter().find(|p| p.key == key) else { return Ok(src.to_string()) };
    let (s, e) = (p.key_span.start, p.act_span.end);
    let ls = src[..s].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let le = src[e..].find('\n').map(|i| e + i).unwrap_or(src.len());
    if src[ls..s].trim().is_empty() && src[e..le].trim().is_empty() {
        let end = (le + 1).min(src.len());
        return Ok(format!("{}{}", &src[..ls], &src[end..]));
    }
    let mut end = e;
    if src[end..].starts_with(' ') {
        end += 1;
    }
    Ok(format!("{}{}", &src[..s], &src[end..]))
}

/// Trechos (posições) onde a própria tecla aparece dentro da ação: o "tocar" de
/// um `tap-hold*` ou as teclas soltas de um `multi`. Ex.: `(multi lalt (...))`.
fn own_key_spans(sx: &Sx, key: &str) -> Vec<Range<usize>> {
    let Some(items) = sx.list() else { return vec![] };
    let atom_span = |i: &Sx| match i {
        Sx::Atom(a, sp) if a == key => Some(sp.clone()),
        _ => None,
    };
    match sx.head() {
        Some("tap-hold" | "tap-hold-press" | "tap-hold-release") => items.get(3).and_then(atom_span).into_iter().collect(),
        Some("multi") => items[1..].iter().filter_map(atom_span).collect(),
        // `(fork (tap-hold-press 0 200 tab ...) tab (mods))`: líder que não abre a camada com modificador
        Some("fork") => {
            let mut v = items.get(1).map(|l| own_key_spans(l, key)).unwrap_or_default();
            if !v.is_empty() {
                v.extend(items.get(2).and_then(atom_span));
            }
            v
        }
        _ => vec![],
    }
}

fn replace_spans(src: &str, mut spans: Vec<Range<usize>>, with: &str) -> String {
    spans.sort_by_key(|s| std::cmp::Reverse(s.start));
    let mut s = src.to_string();
    for sp in spans {
        s.replace_range(sp, with);
    }
    s
}

/// Passa a função da tecla `old` da camada base para `new` (a antiga volta a ser
/// normal). Se a ação manda a própria tecla ao tocar (`tap-hold-press 0 200 f13 ...`,
/// direto ou num alias `@x`), passa a mandar a nova.
pub fn move_key(src: &str, old: &str, new: &str) -> Result<String, String> {
    check_key(new)?;
    if old == new {
        return Ok(src.to_string());
    }
    let ps = pairs(src)?;
    let p = ps.iter().find(|p| p.key == old).ok_or(format!("a tecla {old} não está na camada base"))?;
    if let Some(o) = ps.iter().find(|p| p.key == new) {
        return Err(format!("a tecla {new} já faz `{}` na camada base — escolha outra ou mude pela aba Código", o.action));
    }
    let action = match kbd::parse(&p.action)?.first() {
        Some(sx) => replace_spans(&p.action, own_key_spans(sx, old), new),
        None => p.action.clone(),
    };
    let mut s = remove(src, old)?;
    s = set_action(&s, new, &action)?;
    if let Some(alias) = p.action.strip_prefix('@') {
        let top = kbd::parse(&s)?;
        let mut spans = vec![];
        for form in top.iter().filter(|f| f.head() == Some("defalias")) {
            for kv in form.list().unwrap()[1..].chunks(2) {
                if kv.len() == 2 && kv[0].atom() == Some(alias) {
                    spans.extend(own_key_spans(&kv[1], old));
                }
            }
        }
        s = replace_spans(&s, spans, new);
    }
    kbd::parse(&s)?;
    Ok(s)
}

/// Tecla (nome no kanata) que hoje ativa a líder.
pub fn leader_key(src: &str) -> Option<String> {
    pairs(src).ok()?.into_iter().find(|p| p.action == LEADER_ALIAS).map(|p| p.key)
}

/// O que a líder manda quando é só tocada: (texto, posição) do alias `lider`.
fn leader_tap(src: &str) -> Option<(String, Vec<Range<usize>>)> {
    let top = kbd::parse(src).ok()?;
    for form in top.iter().filter(|f| f.head() == Some("defalias")) {
        let items = form.list()?;
        for kv in items[1..].chunks(2) {
            if kv.len() == 2 && kv[0].atom() == Some("lider") {
                let l = kv[1].list()?;
                // pode vir embrulhado num `(fork (tap-hold-press ...) tecla (modificadores))`
                let (th, il, outer) = match (kv[1].head(), l.get(1)) {
                    (Some("fork"), Some(inner)) => (inner, inner.list()?, Some(l)),
                    _ => (&kv[1], l, None),
                };
                if matches!(th.head(), Some("tap-hold" | "tap-hold-press" | "tap-hold-release")) && il.len() >= 5 {
                    if let Sx::Atom(a, span) = &il[3] {
                        let mut spans = vec![span.clone()];
                        // a tecla "normal" do fork (com modificador) é a mesma da líder
                        if let Some(Sx::Atom(b, sp)) = outer.and_then(|o| o.get(2)) {
                            if b == a {
                                spans.push(sp.clone());
                            }
                        }
                        return Some((a.clone(), spans));
                    }
                }
            }
        }
    }
    None
}

pub fn has_leader_alias(src: &str) -> bool {
    leader_tap(src).is_some()
}

/// Passa a tecla líder para `new_key`. A tecla antiga volta a ser normal.
/// Se a líder "tocada sozinha" mandava a própria tecla antiga, passa a mandar a nova.
pub fn set_leader(src: &str, new_key: &str) -> Result<String, String> {
    check_key(new_key)?;
    let (tap, _) = leader_tap(src).ok_or("não achei o alias `lider` (tap-hold ...) no defalias")?;
    let old = leader_key(src);
    if old.as_deref() == Some(new_key) {
        return Ok(src.to_string());
    }
    let mut s = src.to_string();
    if let Some(o) = &old {
        s = remove(&s, o)?;
    }
    s = set(&s, new_key, LEADER_ALIAS)?;
    if old.as_deref() == Some(tap.as_str()) {
        let (_, spans) = leader_tap(&s).unwrap();
        s = replace_spans(&s, spans, new_key);
    }
    kbd::parse(&s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "(defcfg x y)\n(defsrc)\n(defalias\n  lider (tap-hold-press 0 200 f13 (layer-while-held km-lider))\n)\n\n(deflayermap (base)\n  f13 @lider\n  lalt @alt\n  ;; comentário\n  caps (fork ret caps (lsft rsft))\n)\n\n#| (deflayermap (base) lmet @lider) |#\n(deflayermap (km-lider) a b)\n";

    #[test]
    fn reads_leader_and_swaps() {
        assert_eq!(leader_key(CFG).as_deref(), Some("f13"));
        assert!(swaps(CFG).unwrap().is_empty());
        let s = set(CFG, "rctl", "lmet").unwrap();
        assert_eq!(swaps(&s).unwrap(), vec![("rctl".to_string(), "lmet".to_string())]);
        assert!(s.contains("  caps (fork ret caps (lsft rsft))\n  rctl lmet\n)"));
        let s2 = set(&s, "rctl", "XX").unwrap();
        assert_eq!(swaps(&s2).unwrap(), vec![("rctl".to_string(), "XX".to_string())]);
        let back = remove(&s2, "rctl").unwrap();
        assert_eq!(back, CFG);
    }

    #[test]
    fn moves_leader() {
        let s = set_leader(CFG, "caps").unwrap();
        assert_eq!(leader_key(&s).as_deref(), Some("caps"));
        assert!(!s.contains("f13 @lider"));
        assert!(s.contains("caps @lider"));
        assert!(!s.contains("(fork ret caps"));
        // tocar sozinha mandava f13 (a própria tecla antiga) -> agora manda caps
        assert!(s.contains("(tap-hold-press 0 200 caps "));
        assert!(s.contains(";; comentário"));
        assert_eq!(set_leader(&s, "caps").unwrap(), s);
    }

    #[test]
    fn leader_with_modifier_guard() {
        let cfg = CFG.replace(
            "(tap-hold-press 0 200 f13 (layer-while-held km-lider))",
            "(fork (tap-hold-press 0 200 f13 (layer-while-held km-lider)) f13 (lsft rsft lctl rctl lalt ralt lmet rmet))",
        );
        assert!(has_leader_alias(&cfg));
        let s = set_leader(&cfg, "caps").unwrap();
        assert!(s.contains("(fork (tap-hold-press 0 200 caps (layer-while-held km-lider)) caps (lsft"));
        assert!(!s.contains("f13 (lsft"));
        // líder numa tecla da base: `tab (fork (tap-hold-press 0 200 tab ...) tab (...))`
        let t = set_action(&cfg, "tab", "(fork (tap-hold-press 0 200 tab (layer-while-held km-t)) tab (lsft rsft))").unwrap();
        let m = move_key(&t, "tab", "f15").unwrap();
        assert!(m.contains("f15 (fork (tap-hold-press 0 200 f15 (layer-while-held km-t)) f15 (lsft rsft))"));
        // fork que não é líder (caps) não muda de comportamento ao mover a tecla
        let c = move_key(&cfg, "caps", "f16").unwrap();
        assert!(c.contains("f16 (fork ret caps (lsft rsft))"));
    }

    #[test]
    fn moves_any_layer_key() {
        let cfg = CFG.replace("(defalias
", "(defalias
  alt (multi lalt (layer-while-held km-alt))
");
        let s = move_key(&cfg, "lalt", "ralt").unwrap();
        assert!(s.contains("  ralt @alt
"));
        assert!(s.contains("alt (multi ralt (layer-while-held km-alt))"));
        let s = move_key(&cfg, "f13", "f14").unwrap();
        assert!(s.contains("(tap-hold-press 0 200 f14 (layer-while-held km-lider))"));
        assert!(move_key(&cfg, "f13", "caps").unwrap_err().contains("caps"));
        let inline = set_action(&cfg, "f15", "(tap-hold-press 0 200 f15 (layer-while-held km-x))").unwrap();
        let s = move_key(&inline, "f15", "f16").unwrap();
        assert!(s.contains("f16 (tap-hold-press 0 200 f16 (layer-while-held km-x))"));
    }

    #[test]
    fn creates_base_when_missing() {
        let s = set("(defsrc)\n(deflayermap (km-x) a b)", "rctl", "lmet").unwrap();
        assert_eq!(swaps(&s).unwrap(), vec![("rctl".to_string(), "lmet".to_string())]);
        assert!(s.find("(base)").unwrap() < s.find("km-x").unwrap());
    }
}

#[cfg(test)]
mod real_config_tests {
    /// Gera variações da config real para validar com `kanata --check` (KM_DUMP=pasta).
    #[test]
    fn dump_variants_of_real_config() {
        let Ok(dir) = std::env::var("KM_DUMP") else { return };
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/dist/keymacros.kbd")).unwrap();
        let a = super::set_leader(&src, "caps").unwrap();
        let b = super::set(&a, "rctl", "lmet").unwrap();
        let c = super::set(&b, "f14", "XX").unwrap();
        let d = super::remove(&c, "rctl").unwrap();
        for (n, t) in [("a", &a), ("b", &b), ("c", &c), ("d", &d)] {
            std::fs::write(format!("{dir}/v-{n}.kbd"), t).unwrap();
        }
    }
}
