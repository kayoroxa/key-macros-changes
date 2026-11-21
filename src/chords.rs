//! Combinações de teclas apertadas juntas (`defchordsv2` do kanata).
//! Ex.: `r` + `t` ao mesmo tempo = `f14`. Uma depois da outra = normal.

use crate::kbd::{self, Act, Sx};
use std::ops::Range;

pub const DEFAULT_TIMEOUT: u32 = 40;
/// Depois de qualquer tecla "normal", combinações ficam desligadas por este
/// tempo (ms). Evita disparar sem querer no meio de uma palavra.
pub const DEFAULT_MIN_IDLE: u32 = 150;

#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
    pub keys: Vec<String>,
    pub action: Act,
    /// ms entre a primeira e a última tecla para contar como "juntas"
    pub timeout: u32,
    /// `first-release` ou `all-released`
    pub release: String,
    /// lista de camadas onde a combinação fica desligada, ex.: `()` ou `(km-lider)`
    pub disabled: String,
}

impl Chord {
    pub fn new(keys: Vec<String>, action: Act) -> Self {
        Chord { keys, action, timeout: DEFAULT_TIMEOUT, release: "first-release".into(), disabled: "()".into() }
    }

    pub fn render(&self) -> Result<String, String> {
        if self.keys.len() < 2 {
            return Err("a combinação precisa de pelo menos 2 teclas".into());
        }
        for (i, k) in self.keys.iter().enumerate() {
            if k.is_empty() || k.contains(['(', ')', '"']) || k.contains(char::is_whitespace) {
                return Err(format!("tecla inválida: '{k}'"));
            }
            if self.keys[..i].contains(k) {
                return Err(format!("tecla repetida: {k}"));
            }
        }
        if self.timeout == 0 || self.timeout > 1000 {
            return Err("o tempo deve ser entre 1 e 1000 ms".into());
        }
        if !["first-release", "all-released"].contains(&self.release.as_str()) {
            return Err("soltura deve ser first-release ou all-released".into());
        }
        let disabled = self.disabled.trim();
        let d = kbd::parse(disabled).map_err(|e| format!("camadas desligadas: {e}"))?;
        if d.len() != 1 || d[0].list().is_none() {
            return Err("camadas desligadas deve ser uma lista, ex.: () ou (km-lider)".into());
        }
        Ok(format!("({}) {} {} {} {}", self.keys.join(" "), self.action.render()?, self.timeout, self.release, disabled))
    }

    pub fn same_keys(&self, other: &[String]) -> bool {
        let mut a = self.keys.clone();
        let mut b = other.to_vec();
        a.sort();
        b.sort();
        a == b
    }
}

/// Lê o `(defchordsv2 ...)`. `span` é None se ainda não existir.
pub fn read(src: &str) -> Result<(Option<Range<usize>>, Vec<Chord>), String> {
    let top = kbd::parse(src)?;
    let mut found: Option<&Sx> = None;
    for f in &top {
        if f.head() == Some("defchordsv2") {
            if found.is_some() {
                return Err(format!("linha {}: só pode existir um defchordsv2", kbd::line_of(src, f.span().start)));
            }
            found = Some(f);
        }
    }
    let Some(form) = found else { return Ok((None, vec![])) };
    let items = &form.list().unwrap()[1..];
    if items.len() % 5 != 0 {
        return Err(format!(
            "linha {}: cada combinação precisa de 5 itens: (teclas) ação tempo soltura (camadas)",
            kbd::line_of(src, form.span().start)
        ));
    }
    let mut out = vec![];
    for c in items.chunks(5) {
        let keys = c[0]
            .list()
            .ok_or(format!("linha {}: as teclas da combinação devem estar entre parênteses", kbd::line_of(src, c[0].span().start)))?
            .iter()
            .map(|k| k.atom().map(str::to_string).ok_or("tecla inválida na combinação".to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let timeout = c[2].atom().and_then(|t| t.parse().ok()).ok_or(format!("linha {}: tempo inválido", kbd::line_of(src, c[2].span().start)))?;
        out.push(Chord {
            keys,
            action: kbd::act_from(&c[1], src),
            timeout,
            release: c[3].atom().unwrap_or("first-release").to_string(),
            disabled: src[c[4].span()].to_string(),
        });
    }
    Ok((Some(form.span()), out))
}

pub fn render_block(chords: &[Chord]) -> Result<String, String> {
    let mut s = String::from("(defchordsv2\n");
    for c in chords {
        s += "  ";
        s += &c.render().map_err(|e| format!("combinação {}: {e}", c.keys.join("+")))?;
        s += "\n";
    }
    s += ")";
    Ok(s)
}

/// Garante `chave valor` dentro do `(defcfg ...)`. Se `force` for false e a
/// chave já existir, mantém o valor do usuário.
pub fn ensure_defcfg(src: &str, key: &str, value: &str, force: bool) -> Result<String, String> {
    let top = kbd::parse(src)?;
    let Some(cfg) = top.iter().find(|f| f.head() == Some("defcfg")) else {
        return Ok(format!("(defcfg\n  {key} {value}\n)\n\n{src}"));
    };
    let items = cfg.list().unwrap();
    let mut i = 1;
    while i < items.len() {
        if items[i].atom() == Some(key) {
            let Some(v) = items.get(i + 1) else { break };
            if !force || v.atom() == Some(value) {
                return Ok(src.to_string());
            }
            let sp = v.span();
            return Ok(format!("{}{}{}", &src[..sp.start], value, &src[sp.end..]));
        }
        i += 2;
    }
    let end = cfg.span().end - 1; // posição do ')'
    let before = src[..end].trim_end();
    Ok(format!("{before}\n  {key} {value}\n{}", &src[end..]))
}

/// Grava a lista de combinações no texto (cria o bloco se precisar).
/// Com combinações, o kanata exige `concurrent-tap-hold yes`.
pub fn write(src: &str, chords: &[Chord]) -> Result<String, String> {
    let (span, _) = read(src)?;
    let mut out = match (span, chords.is_empty()) {
        (Some(sp), true) => {
            // remove o bloco e a linha em branco que sobrar
            let after = src[sp.end..].strip_prefix('\n').unwrap_or(&src[sp.end..]);
            format!("{}{}", src[..sp.start].trim_end_matches([' ', '\t']), after)
        }
        (Some(sp), false) => format!("{}{}{}", &src[..sp.start], render_block(chords)?, &src[sp.end..]),
        (None, true) => src.to_string(),
        (None, false) => format!(
            "{}\n\n;; Combinações: teclas apertadas JUNTAS viram outra ação.\n;; (teclas) ação tempo-ms soltura (camadas-onde-desliga)\n{}\n",
            src.trim_end(),
            render_block(chords)?
        ),
    };
    if !chords.is_empty() {
        out = ensure_defcfg(&out, "concurrent-tap-hold", "yes", true)?;
        out = ensure_defcfg(&out, "chords-v2-min-idle", &DEFAULT_MIN_IDLE.to_string(), false)?;
    }
    read(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "(defcfg\n  process-unmapped-keys yes\n)\n(defsrc)\n(deflayermap (base) caps ret)\n";

    fn rt() -> Chord {
        Chord::new(vec!["r".into(), "t".into()], Act::Keys("f14".into()))
    }

    #[test]
    fn creates_block_and_defcfg_options() {
        let out = write(BASE, &[rt()]).unwrap();
        assert!(out.contains("(r t) f14 40 first-release ()"));
        assert!(out.contains("  concurrent-tap-hold yes\n"));
        assert!(out.contains("  chords-v2-min-idle 150\n"));
        assert!(out.contains("process-unmapped-keys yes"));
        let (_, chords) = read(&out).unwrap();
        assert_eq!(chords, vec![rt()]);
    }

    #[test]
    fn keeps_user_values_and_forces_concurrent() {
        let src = "(defcfg concurrent-tap-hold no chords-v2-min-idle 80)\n(defsrc)\n";
        let out = write(src, &[rt()]).unwrap();
        assert!(out.contains("concurrent-tap-hold yes"));
        assert!(out.contains("chords-v2-min-idle 80"));
        assert!(!out.contains("chords-v2-min-idle 150"));
    }

    #[test]
    fn edit_and_remove() {
        let one = write(BASE, &[rt()]).unwrap();
        let mut c = rt();
        c.action = Act::App { exe: "calc.exe".into(), run: "".into(), args: "".into(), opts: "".into() };
        c.timeout = 60;
        let two = write(&one, &[c.clone()]).unwrap();
        assert_eq!(read(&two).unwrap().1, vec![c]);
        let none = write(&two, &[]).unwrap();
        assert_eq!(read(&none).unwrap(), (None, vec![]));
        assert!(none.contains("(deflayermap (base) caps ret)"));
    }

    #[test]
    fn validation() {
        assert!(Chord::new(vec!["r".into()], Act::Keys("f14".into())).render().is_err());
        assert!(Chord::new(vec!["r".into(), "r".into()], Act::Keys("f14".into())).render().is_err());
        let mut c = rt();
        c.timeout = 0;
        assert!(c.render().is_err());
        assert!(read("(defchordsv2 (r t) f14 40)").is_err());
        assert!(rt().same_keys(&["t".into(), "r".into()]));
    }
}
