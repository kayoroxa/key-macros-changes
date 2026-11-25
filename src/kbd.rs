//! Leitura/escrita de arquivos `.kbd` do kanata.
//!
//! - Um lexer de s-expressions com as mesmas regras do kanata (`;;`, `#| |#`,
//!   `"..."` sem escapes, `r#"..."#`), guardando a posição de cada nó.
//! - O modelo das camadas "gerenciadas" pelo editor: todo `(deflayermap (km-...))`.
//!   O resto do arquivo nunca é reescrito pelo app.

use crate::msg;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Sx {
    Atom(String, Range<usize>),
    List(Vec<Sx>, Range<usize>),
}

impl Sx {
    pub fn span(&self) -> Range<usize> {
        match self {
            Sx::Atom(_, s) | Sx::List(_, s) => s.clone(),
        }
    }
    pub fn atom(&self) -> Option<&str> {
        match self {
            Sx::Atom(a, _) => Some(a),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[Sx]> {
        match self {
            Sx::List(l, _) => Some(l),
            _ => None,
        }
    }
    pub fn head(&self) -> Option<&str> {
        self.list().and_then(|l| l.first()).and_then(|h| h.atom())
    }
}

/// Remove as aspas de um átomo (`"x"` ou `r#"x"#`).
pub fn unquote(a: &str) -> &str {
    if let Some(r) = a.strip_prefix("r#\"") {
        return r.strip_suffix("\"#").unwrap_or(r);
    }
    a.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(a)
}

/// Põe aspas no formato aceito pelo kanata.
pub fn quote(s: &str) -> Result<String, String> {
    if s.contains('\n') {
        return Err("texto não pode ter quebra de linha".into());
    }
    if s.contains('"') {
        if s.contains("\"#") {
            return Err(format!("texto com \"# não é suportado: {s}"));
        }
        Ok(format!("r#\"{s}\"#"))
    } else {
        Ok(format!("\"{s}\""))
    }
}

pub fn line_of(src: &str, pos: usize) -> usize {
    src[..pos.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

fn is_start(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'"') || b.is_ascii_whitespace()
}

pub fn parse(src: &str) -> Result<Vec<Sx>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut stack: Vec<(Vec<Sx>, usize)> = vec![(vec![], 0)];
    let err = |pos: usize, m: &str| format!("linha {}: {m}", line_of(src, pos));
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match c {
            b'(' => {
                stack.push((vec![], i));
                i += 1;
            }
            b')' => {
                if stack.len() == 1 {
                    return Err(err(i, "')' sem '(' correspondente"));
                }
                let (items, start) = stack.pop().unwrap();
                stack.last_mut().unwrap().0.push(Sx::List(items, start..i + 1));
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                    i += 1;
                }
                if i >= b.len() || b[i] != b'"' {
                    return Err(err(start, "texto entre aspas não terminado"));
                }
                i += 1;
                stack.last_mut().unwrap().0.push(Sx::Atom(src[start..i].to_string(), start..i));
            }
            b';' if b.get(i + 1) == Some(&b';') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' if b.get(i + 1) == Some(&b'|') => {
                let start = i;
                match src[i + 2..].find("|#") {
                    Some(e) => i = i + 2 + e + 2,
                    None => return Err(err(start, "comentário #| sem |#")),
                }
            }
            b'r' if b.get(i + 1) == Some(&b'#') && b.get(i + 2) == Some(&b'"') => {
                let start = i;
                match src[i + 3..].find("\"#") {
                    Some(e) => i = i + 3 + e + 2,
                    None => return Err(err(start, "texto r#\" sem \"#")),
                }
                stack.last_mut().unwrap().0.push(Sx::Atom(src[start..i].to_string(), start..i));
            }
            _ => {
                let start = i;
                while i < b.len() && !is_start(b[i]) {
                    i += 1;
                }
                stack.last_mut().unwrap().0.push(Sx::Atom(src[start..i].to_string(), start..i));
            }
        }
    }
    if stack.len() > 1 {
        return Err(err(stack.last().unwrap().1, "'(' sem ')' correspondente"));
    }
    Ok(stack.pop().unwrap().0)
}

// ---------------------------------------------------------------------------
// Modelo das camadas gerenciadas

pub const MANAGED_PREFIX: &str = "km-";

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Não faz nada (`XX`).
    Nothing,
    /// Abre/foca um app. `opts` são opções extras (ex.: `nomax title-ends=Edge`).
    App { exe: String, run: String, args: String, opts: String },
    /// Vai para um site/aba do navegador.
    Site { matches: String, search: String, url: String },
    /// Executa comando/arquivo/URL.
    Run { cmd: String, args: String },
    /// Digita texto.
    Text(String),
    /// Tecla ou atalho no formato do kanata (ex.: `C-S-a`, `ret`).
    Keys(String),
    /// Qualquer código kanata.
    Raw(String),
}

impl Act {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Act::Nothing => "Nada",
            Act::App { .. } => "Abrir/focar app",
            Act::Site { .. } => "Site no navegador",
            Act::Run { .. } => "Executar",
            Act::Text(_) => "Digitar texto",
            Act::Keys(_) => "Enviar teclas",
            Act::Raw(_) => "Código kanata",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Act::Nothing => "—".into(),
            Act::App { exe, .. } => format!("app {exe}"),
            Act::Site { matches, .. } => format!("site {matches}"),
            Act::Run { cmd, args } => format!("run {cmd} {args}").trim().to_string(),
            Act::Text(t) => format!("digitar \"{t}\""),
            Act::Keys(k) => format!("teclas {k}"),
            Act::Raw(r) => {
                let one: String = r.split_whitespace().collect::<Vec<_>>().join(" ");
                if one.chars().count() > 70 {
                    format!("{}…", one.chars().take(70).collect::<String>())
                } else {
                    one
                }
            }
        }
    }

    pub fn render(&self) -> Result<String, String> {
        let q = quote;
        Ok(match self {
            Act::Nothing => "XX".into(),
            Act::App { exe, run, args, opts } => {
                if exe.trim().is_empty() {
                    return Err("informe o EXE do app".into());
                }
                let mut s = format!("(push-msg app {}", q(exe.trim())?);
                let run = run.trim();
                let args = args.trim();
                if !run.is_empty() || !args.is_empty() {
                    s += &format!(" {}", q(if run.is_empty() { exe.trim() } else { run })?);
                }
                if !args.is_empty() {
                    s += &format!(" {}", q(args)?);
                }
                for o in split_opts(opts) {
                    if o.contains(['(', ')', '"', ' ']) {
                        s += &format!(" {}", q(&o)?);
                    } else {
                        s += &format!(" {o}");
                    }
                }
                s + ")"
            }
            Act::Site { matches, search, url } => {
                if matches.trim().is_empty() {
                    return Err("informe o site (ex.: youtube.com)".into());
                }
                let mut s = format!("(push-msg browser-tab {}", q(matches.trim())?);
                if !search.trim().is_empty() || !url.trim().is_empty() {
                    let se = if search.trim().is_empty() { matches.split('|').next().unwrap_or("").trim() } else { search.trim() };
                    s += &format!(" {}", q(se)?);
                }
                if !url.trim().is_empty() {
                    s += &format!(" {}", q(url.trim())?);
                }
                s + ")"
            }
            Act::Run { cmd, args } => {
                if cmd.trim().is_empty() {
                    return Err("informe o que executar".into());
                }
                let mut s = format!("(push-msg run {}", q(cmd.trim())?);
                if !args.trim().is_empty() {
                    s += &format!(" {}", q(args.trim())?);
                }
                s + ")"
            }
            Act::Text(t) => {
                if t.is_empty() {
                    return Err("informe o texto".into());
                }
                format!("(push-msg type {})", q(t)?)
            }
            Act::Keys(k) => {
                let k = k.trim();
                if k.is_empty() || k.contains(char::is_whitespace) && !k.starts_with('(') {
                    return Err("teclas: use o formato do kanata, ex.: C-S-a ou (macro C-l 100 C-c)".into());
                }
                k.to_string()
            }
            Act::Raw(r) => {
                let r = r.trim();
                if r.is_empty() {
                    return Err("código vazio".into());
                }
                parse(r).map_err(|e| format!("código kanata: {e}"))?;
                r.to_string()
            }
        })
    }
}

/// Separa opções digitadas pelo usuário: `nomax "title=Foo Bar"`.
pub fn split_opts(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut inq = false;
    for c in s.chars() {
        match c {
            '"' => inq = !inq,
            c if c.is_whitespace() && !inq => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn join_opts(v: &[&str]) -> String {
    v.iter().map(|o| if o.contains(char::is_whitespace) { format!("\"{o}\"") } else { o.to_string() }).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub key: String,
    pub tap: Act,
    /// (milissegundos segurando, ação)
    pub hold: Option<(u32, Act)>,
    /// nome opcional (comentário `;; nome` no fim da linha)
    pub name: String,
}

impl Binding {
    pub fn render(&self) -> Result<String, String> {
        let key = self.key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) || key.contains(['(', ')', '"']) {
            return Err(format!("tecla inválida: '{key}'"));
        }
        let tap = self.tap.render()?;
        Ok(match &self.hold {
            None => format!("{key} {tap}"),
            Some((ms, act)) => {
                if *ms == 0 || *ms > 60000 {
                    return Err("tempo de segurar deve ser entre 1ms e 60s".into());
                }
                // tap-hold-press (igual às teclas líder): evita o toque disparar também a ação de segurar
                format!("{key} (tap-hold-press 0 {ms} {tap} {})", act.render()?)
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub span: Range<usize>,
    pub bindings: Vec<Binding>,
}

pub fn act_from(sx: &Sx, src: &str) -> Act {
    let raw = || Act::Raw(src[sx.span()].to_string());
    match sx {
        Sx::Atom(a, _) => {
            if a == "XX" {
                Act::Nothing
            } else if a.starts_with('@') || a.starts_with('$') {
                raw()
            } else {
                Act::Keys(a.clone())
            }
        }
        Sx::List(items, _) => {
            if sx.head() != Some("push-msg") || items.len() < 2 {
                return raw();
            }
            let atoms: Option<Vec<&str>> = items[1..].iter().map(|i| i.atom()).collect();
            let Some(atoms) = atoms else { return raw() };
            if atoms.iter().any(|a| a.starts_with('$')) {
                return raw();
            }
            let cmd = unquote(atoms[0]);
            let args: Vec<&str> = atoms[1..].iter().map(|a| unquote(a)).collect();
            let is_opt = |a: &&str| *a == "nomax" || a.split_once('=').is_some_and(|(k, _)| ["title", "title-ends", "class", "prefer", "dir", "wait"].contains(&k));
            let pos: Vec<&str> = args.iter().copied().filter(|a| !is_opt(a)).collect();
            let opts: Vec<&str> = args.iter().copied().filter(is_opt).collect();
            let g = |i: usize| pos.get(i).map(|s| s.to_string()).unwrap_or_default();
            match cmd {
                "app" if pos.len() <= 3 && !pos.is_empty() => Act::App { exe: g(0), run: g(1), args: g(2), opts: join_opts(&opts) },
                "browser-tab" if opts.is_empty() && (1..=3).contains(&pos.len()) => {
                    Act::Site { matches: g(0), search: g(1), url: g(2) }
                }
                "run" if opts.is_empty() && (1..=2).contains(&pos.len()) => Act::Run { cmd: g(0), args: g(1) },
                "type" if opts.is_empty() && pos.len() == 1 => Act::Text(g(0)),
                _ => raw(),
            }
        }
    }
}

fn binding_from(key: &Sx, action: &Sx, src: &str) -> Binding {
    let rest = src[action.span().end..].lines().next().unwrap_or("").trim();
    let name = rest.strip_prefix(";;").map(|n| n.trim().to_string()).unwrap_or_default();
    let mut b = binding_raw(key, action, src);
    b.name = name;
    b
}

fn binding_raw(key: &Sx, action: &Sx, src: &str) -> Binding {
    let key = src[key.span()].to_string();
    if let Some(items) = action.list() {
        if matches!(action.head(), Some("tap-hold" | "tap-hold-press")) && items.len() == 5 {
            if let (Some(_), Some(ms)) = (items[1].atom(), items[2].atom().and_then(|m| m.parse::<u32>().ok())) {
                return Binding { key, tap: act_from(&items[3], src), hold: Some((ms, act_from(&items[4], src))), name: String::new() };
            }
        }
    }
    Binding { key, tap: act_from(action, src), hold: None, name: String::new() }
}

/// Todas as `(deflayermap (km-...))` do arquivo.
pub fn managed_layers(src: &str) -> Result<Vec<Layer>, String> {
    let top = parse(src)?;
    let mut out = vec![];
    for form in &top {
        let Some(items) = form.list() else { continue };
        if form.head() != Some("deflayermap") || items.len() < 2 {
            continue;
        }
        let Some(name) = items[1].list().and_then(|l| l.first()).and_then(|a| a.atom()) else { continue };
        if !name.starts_with(MANAGED_PREFIX) {
            continue;
        }
        let pairs = &items[2..];
        if pairs.len() % 2 != 0 {
            return Err(format!("linha {}: camada {name} tem uma tecla sem ação", line_of(src, form.span().start)));
        }
        let bindings = pairs.chunks(2).map(|kv| binding_from(&kv[0], &kv[1], src)).collect();
        out.push(Layer { name: name.to_string(), span: form.span(), bindings });
    }
    Ok(out)
}

pub fn render_layer(name: &str, bindings: &[Binding]) -> Result<String, String> {
    let mut s = format!("(deflayermap ({name})\n");
    for b in bindings {
        s += "  ";
        s += &b.render().map_err(|e| format!("tecla {}: {e}", b.key))?;
        let n = b.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !n.is_empty() {
            s += &format!("  ;; {n}");
        }
        s += "\n";
    }
    s += ")";
    Ok(s)
}

/// Substitui a camada `name` no texto, mantendo o resto do arquivo intacto.
pub fn replace_layer(src: &str, name: &str, bindings: &[Binding]) -> Result<String, String> {
    let layers = managed_layers(src)?;
    let layer = layers.iter().find(|l| l.name == name).ok_or(format!("camada {name} não encontrada"))?;
    let new = render_layer(name, bindings)?;
    let out = format!("{}{}{}", &src[..layer.span.start], new, &src[layer.span.end..]);
    // garante que o resultado continua legível pelo nosso parser
    managed_layers(&out)?;
    Ok(out)
}

/// Acrescenta uma nova camada gerenciada no final do arquivo.
pub fn append_layer(src: &str, name: &str) -> Result<String, String> {
    if !name.starts_with(MANAGED_PREFIX) || name.len() <= MANAGED_PREFIX.len() || name.contains(char::is_whitespace) {
        return Err(format!("o nome precisa começar com {MANAGED_PREFIX} e não ter espaços"));
    }
    if managed_layers(src)?.iter().any(|l| l.name == name) {
        return Err("já existe".into());
    }
    let mut out = src.trim_end().to_string();
    out += &format!("\n\n{}\n", render_layer(name, &[])?);
    Ok(out)
}

/// Valida todos os `(push-msg ...)` do arquivo contra o vocabulário do app.
/// Retorna (linha, erro).
pub fn check_push_msgs(src: &str) -> Result<Vec<(usize, String)>, String> {
    fn to_item(sx: &Sx) -> Option<msg::Item> {
        match sx {
            Sx::Atom(a, _) if a.starts_with('$') => None,
            Sx::Atom(a, _) => Some(msg::Item::Atom(unquote(a).to_string())),
            Sx::List(l, _) => l.iter().map(to_item).collect::<Option<Vec<_>>>().map(msg::Item::List),
        }
    }
    fn walk(sx: &Sx, src: &str, out: &mut Vec<(usize, String)>) {
        if let Sx::List(items, span) = sx {
            if sx.head() == Some("push-msg") {
                if let Some(items) = items[1..].iter().map(to_item).collect::<Option<Vec<_>>>() {
                    if let Err(e) = msg::parse_items(&items) {
                        out.push((line_of(src, span.start), e));
                    }
                }
                return;
            }
            for i in items {
                walk(i, src, out);
            }
        }
    }
    let mut out = vec![];
    for f in parse(src)? {
        walk(&f, src, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_name_roundtrip() {
        let b = Binding { key: "a".into(), tap: Act::Keys("C-S-f6".into()), hold: None, name: "Gravar tela".into() };
        let src = format!("{}\n", render_layer("km-x", &[b.clone()]).unwrap());
        assert!(src.contains("C-S-f6  ;; Gravar tela"));
        let l = managed_layers(&src).unwrap();
        assert_eq!(l[0].bindings[0], b);
    }

    const SAMPLE: &str = r##"
;; comentário (com parênteses)
(defsrc)
#| bloco
 (deflayermap (km-falso) a b) |#
(deflayermap (base) f13 @lider)
(deflayermap (km-lider)
  c (tap-hold 0 300 (push-msg app "claude.exe" "shell:AppsFolder\Claude_x!Claude") (push-msg first (focus "wezterm-gui.exe") (app "wezterm-gui.exe")))
  d (push-msg app "Discord.exe" r#"%LocalAppData%\Discord\Update.exe"# "--processStart Discord.exe")
  s (push-msg app "obs64.exe" "%ProgramFiles%\obs-studio\bin\64bit\obs64.exe" nomax)
  y (push-msg browser-tab "youtube.com")
  k C-S-a
  x XX
)
"##;

    #[test]
    fn parses_managed_layer() {
        let layers = managed_layers(SAMPLE).unwrap();
        assert_eq!(layers.len(), 1);
        let l = &layers[0];
        assert_eq!(l.name, "km-lider");
        assert_eq!(l.bindings.len(), 6);
        let c = &l.bindings[0];
        assert_eq!(c.key, "c");
        assert_eq!(c.tap, Act::App { exe: "claude.exe".into(), run: r"shell:AppsFolder\Claude_x!Claude".into(), args: "".into(), opts: "".into() });
        let (ms, hold) = c.hold.as_ref().unwrap();
        assert_eq!(*ms, 300);
        assert!(matches!(hold, Act::Raw(r) if r.starts_with("(push-msg first")));
        assert_eq!(l.bindings[1].tap, Act::App { exe: "Discord.exe".into(), run: r"%LocalAppData%\Discord\Update.exe".into(), args: "--processStart Discord.exe".into(), opts: "".into() });
        assert!(matches!(&l.bindings[2].tap, Act::App { opts, .. } if opts == "nomax"));
        let tg = Act::App { exe: "Telegram.exe".into(), run: "".into(), args: "".into(), opts: "prefer=( \"title=Foo Bar\"".into() };
        assert_eq!(tg.render().unwrap(), r#"(push-msg app "Telegram.exe" "prefer=(" "title=Foo Bar")"#);
        assert_eq!(l.bindings[3].tap, Act::Site { matches: "youtube.com".into(), search: "".into(), url: "".into() });
        assert_eq!(l.bindings[4].tap, Act::Keys("C-S-a".into()));
        assert_eq!(l.bindings[5].tap, Act::Nothing);
    }

    #[test]
    fn roundtrip_is_stable() {
        let layers = managed_layers(SAMPLE).unwrap();
        let once = replace_layer(SAMPLE, "km-lider", &layers[0].bindings).unwrap();
        let again = managed_layers(&once).unwrap();
        assert_eq!(again[0].bindings, layers[0].bindings);
        let twice = replace_layer(&once, "km-lider", &again[0].bindings).unwrap();
        assert_eq!(once, twice);
        // o resto do arquivo é preservado
        assert!(once.contains(";; comentário (com parênteses)"));
        assert!(once.contains("(deflayermap (base) f13 @lider)"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(r"C:\a b").unwrap(), r#""C:\a b""#);
        assert_eq!(quote(r#""x" y"#).unwrap(), "r#\"\"x\" y\"#");
        assert_eq!(unquote("r#\"\"x\" y\"#"), r#""x" y"#);
    }

    #[test]
    fn render_hold_and_errors() {
        let b = Binding { key: "t".into(), tap: Act::Text("olá".into()), hold: Some((5000, Act::Run { cmd: "calc.exe".into(), args: "".into() })), name: String::new() };
        assert_eq!(b.render().unwrap(), r#"t (tap-hold-press 0 5000 (push-msg type "olá") (push-msg run "calc.exe"))"#);
        let bad = Binding { key: "a b".into(), tap: Act::Nothing, hold: None, name: String::new() };
        assert!(bad.render().is_err());
        assert!(Act::Raw("(oops".into()).render().is_err());
    }

    #[test]
    fn push_msg_validation() {
        let errs = check_push_msgs("(deflayermap (km-x) a (push-msg voar) b (push-msg app \"a.exe\"))").unwrap();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].1.contains("desconhecido"));
    }

    #[test]
    fn parse_errors_have_lines() {
        assert!(parse("(a\n(b)").unwrap_err().contains("linha 1"));
        assert!(parse("a)\n").unwrap_err().contains("linha 1"));
        assert!(parse("\n\"abc\n\"").unwrap_err().contains("linha 2"));
    }

    #[test]
    fn append_new_layer() {
        let out = append_layer("(defsrc)", "km-premiere").unwrap();
        assert_eq!(managed_layers(&out).unwrap()[0].name, "km-premiere");
        assert!(append_layer(&out, "km-premiere").is_err());
        assert!(append_layer(&out, "premiere").is_err());
    }
}

#[cfg(test)]
mod real_config_tests {
    use super::*;

    const USER: &str = include_str!("../config/keymacros.kbd");

    #[test]
    fn user_config_push_msgs_are_valid() {
        assert_eq!(check_push_msgs(USER).unwrap(), vec![]);
        assert_eq!(check_push_msgs(crate::paths::TEMPLATE).unwrap(), vec![]);
    }

    #[test]
    fn user_config_roundtrips_through_the_form() {
        let layers = managed_layers(USER).unwrap();
        assert_eq!(layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["km-lider", "km-sites", "km-alt", "km-win"]);
        let mut text = USER.to_string();
        for l in &layers {
            text = replace_layer(&text, &l.name, &l.bindings).unwrap();
        }
        assert_eq!(managed_layers(&text).unwrap(), managed_layers(&text).unwrap());
        for (a, b) in managed_layers(&text).unwrap().iter().zip(&layers) {
            assert_eq!(a.bindings, b.bindings, "camada {}", a.name);
        }
        assert_eq!(check_push_msgs(&text).unwrap(), vec![]);
    }

    #[test]
    fn template_uses_sharpkeys_leader() {
        let t = crate::paths::template_for("f13");
        assert!(t.contains("  f13 @lider\n"));
        assert!(t.contains("(tap-hold-press 0 200 f13 "));
        assert!(!t.contains("rctl lmet"));
        assert_eq!(crate::paths::template_for("lmet"), crate::paths::TEMPLATE);
    }
}
