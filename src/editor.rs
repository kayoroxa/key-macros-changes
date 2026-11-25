//! Editor de macros (egui). Roda como processo separado e só existe enquanto
//! a janela está aberta; o daemon na bandeja continua leve.
//!
//! A fonte da verdade é o arquivo `.kbd`. O formulário edita apenas as
//! camadas `km-*`; a aba "Código" edita o arquivo inteiro.
//!
//! O visual (cores, fontes, botões, teclas, cartões) fica em `theme.rs`.

use crate::kbd::{self, Act, Binding, Layer};
use crate::theme::{self as th, icon, Ico, Look};
use crate::trigger::{self, Trigger};
use crate::{backup, daemon, features::{self, Features}, msg, paths, remap, scancode, update, win};
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

#[cfg(not(debug_assertions))]
const TITLE: &str = "KeyMacros — Editor";
/// build de teste: título diferente para poder abrir junto com o editor normal
#[cfg(debug_assertions)]
const TITLE: &str = "KeyMacros — Editor (dev)";

const SKY: Color32 = Color32::from_rgb(56, 189, 248);
const ORANGE: Color32 = Color32::from_rgb(245, 158, 11);
const VIOLET: Color32 = Color32::from_rgb(167, 139, 250);
const CYAN: Color32 = Color32::from_rgb(34, 211, 238);
/// Miniatura da tela: relógio embaixo à direita, seta para o canto de cima.
fn clock_illustration(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(230.0, 140.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect(r, 10, th::FIELD, Stroke::new(1.0, th::BORDER_HI), StrokeKind::Inside);
    // barra de tarefas
    let tb = Rect::from_min_max(pos2(r.left() + 1.0, r.bottom() - 16.0), pos2(r.right() - 1.0, r.bottom() - 1.0));
    p.rect_filled(tb, 0, th::CARD_HOVER);
    let (w, h) = (48.0, 16.0);
    let x = r.right() - w - 8.0;
    let top_min = pos2(x, r.top() + 8.0);
    let bot_min = pos2(x, tb.top() - h - 5.0);
    let chip = |min: egui::Pos2, fill: Color32, edge: Color32| {
        let cr = Rect::from_min_size(min, vec2(w, h));
        p.rect(cr, 4, fill, Stroke::new(1.0, edge), StrokeKind::Inside);
        p.text(cr.center(), Align2::CENTER_CENTER, "14:32", th::regular(10.0), th::TEXT);
    };
    chip(top_min, th::CARD, th::ACCENT);
    chip(bot_min, th::CARD, th::BORDER_HI);
    // seta tracejada subindo
    let cx = x + w / 2.0;
    let (y0, y1) = (bot_min.y - 2.0, top_min.y + h + 2.0);
    let mut y = y1;
    while y < y0 {
        p.line_segment([pos2(cx, y), pos2(cx, (y + 4.0).min(y0))], Stroke::new(1.5, th::ACCENT_HI));
        y += 7.0;
    }
    p.add(egui::Shape::convex_polygon(vec![pos2(cx, y1 - 1.0), pos2(cx - 4.0, y1 + 6.0), pos2(cx + 4.0, y1 + 6.0)], th::ACCENT_HI, Stroke::NONE));
    // cursor do mouse chegando perto
    let t = pos2(bot_min.x + 6.0, bot_min.y - 22.0);
    p.add(egui::Shape::convex_polygon(vec![t, t + vec2(0.0, 13.0), t + vec2(9.0, 9.0)], Color32::WHITE, Stroke::new(1.0, Color32::BLACK)));
}

/// largura da coluna "Ao tocar" / "Ao segurar por" (alinha os seletores)
const HEAD_W: f32 = 250.0;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Macros,
    Code,
    Remap,
    Features,
    Help,
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Grid,
    Keyboard,
    List,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum Kind {
    Nothing,
    App,
    Site,
    Run,
    Text,
    Keys,
    Raw,
}

const KINDS: &[(Kind, &str)] = &[
    (Kind::App, "Abrir/focar programa"),
    (Kind::Site, "Site no navegador"),
    (Kind::Run, "Executar comando"),
    (Kind::Text, "Digitar texto"),
    (Kind::Keys, "Enviar teclas/atalho"),
    (Kind::Raw, "Código kanata (avançado)"),
    (Kind::Nothing, "Nada (bloquear)"),
];

fn kind_ico(k: Kind) -> (&'static str, Color32) {
    match k {
        Kind::App => (icon::WINDOW, th::ACCENT_HI),
        Kind::Site => (icon::GLOBE, SKY),
        Kind::Run => (icon::CUBE, ORANGE),
        Kind::Text => (icon::FONT, VIOLET),
        Kind::Keys => (icon::KEYBOARD, th::WEAK),
        Kind::Raw => (icon::CODE, CYAN),
        Kind::Nothing => (icon::BLOCKED, th::WEAK),
    }
}

#[derive(Clone)]
struct ActForm {
    kind: Kind,
    exe: String,
    run: String,
    args: String,
    opts: String,
    site: String,
    search: String,
    url: String,
    cmd: String,
    cmd_args: String,
    text: String,
    keys: String,
    raw: String,
    /// popup "gravar atalho" aberto
    rec: bool,
    rec_buf: String,
}

impl Default for ActForm {
    fn default() -> Self {
        ActForm {
            kind: Kind::App,
            exe: String::new(),
            run: String::new(),
            args: String::new(),
            opts: String::new(),
            site: String::new(),
            search: String::new(),
            url: String::new(),
            cmd: String::new(),
            cmd_args: String::new(),
            text: String::new(),
            keys: String::new(),
            raw: String::new(),
            rec: false,
            rec_buf: String::new(),
        }
    }
}

impl ActForm {
    fn from_act(a: &Act) -> Self {
        let mut f = ActForm::default();
        match a {
            Act::Nothing => f.kind = Kind::Nothing,
            Act::App { exe, run, args, opts } => {
                f.kind = Kind::App;
                f.exe = exe.clone();
                f.run = run.clone();
                f.args = args.clone();
                f.opts = opts.clone();
            }
            Act::Site { matches, search, url } => {
                f.kind = Kind::Site;
                f.site = matches.clone();
                f.search = search.clone();
                f.url = url.clone();
            }
            Act::Run { cmd, args } => {
                f.kind = Kind::Run;
                f.cmd = cmd.clone();
                f.cmd_args = args.clone();
            }
            Act::Text(t) => {
                f.kind = Kind::Text;
                f.text = t.clone();
            }
            Act::Keys(k) => {
                f.kind = Kind::Keys;
                f.keys = k.clone();
            }
            Act::Raw(r) => {
                f.kind = Kind::Raw;
                f.raw = r.clone();
            }
        }
        f
    }

    fn to_act(&self) -> Act {
        match self.kind {
            Kind::Nothing => Act::Nothing,
            Kind::App => Act::App { exe: self.exe.clone(), run: self.run.clone(), args: self.args.clone(), opts: self.opts.clone() },
            Kind::Site => Act::Site { matches: self.site.clone(), search: self.search.clone(), url: self.url.clone() },
            Kind::Run => Act::Run { cmd: self.cmd.clone(), args: self.cmd_args.clone() },
            Kind::Text => Act::Text(self.text.clone()),
            Kind::Keys => Act::Keys(self.keys.clone()),
            Kind::Raw => Act::Raw(self.raw.clone()),
        }
    }
}

struct Form {
    key: String,
    tap: ActForm,
    hold_on: bool,
    hold_secs: f32,
    hold: ActForm,
    /// índice da tecla sendo editada na camada
    editing: Option<usize>,
    capturing: bool,
    /// tecla apertada no popup de captura (vale ao clicar em Aceitar)
    cap_key: String,
    /// nome opcional do atalho
    name: String,
    /// painel de edição visível (só depois de clicar num card / Novo atalho)
    open: bool,
}

impl Default for Form {
    fn default() -> Self {
        Form { key: String::new(), tap: ActForm::default(), hold_on: false, hold_secs: 0.3, hold: ActForm::default(), editing: None, capturing: false, cap_key: String::new(), name: String::new(), open: false }
    }
}

impl Form {
    fn from_binding(b: &Binding, editing: Option<usize>) -> Self {
        Form {
            key: b.key.clone(),
            tap: ActForm::from_act(&b.tap),
            hold_on: b.hold.is_some(),
            hold_secs: b.hold.as_ref().map(|h| h.0 as f32 / 1000.0).unwrap_or(0.3),
            hold: b.hold.as_ref().map(|h| ActForm::from_act(&h.1)).unwrap_or_default(),
            editing,
            capturing: false,
            cap_key: String::new(),
            name: b.name.clone(),
            open: false,
        }
    }

    fn binding(&self) -> Binding {
        Binding {
            key: self.key.trim().to_string(),
            tap: self.tap.to_act(),
            hold: self.hold_on.then(|| (((self.hold_secs * 1000.0).round() as u32).max(1), self.hold.to_act())),
            name: self.name.trim().to_string(),
        }
    }
}

// ------------------------------------------------------------------ nomes amigáveis

/// Nome curto da camada (a tecla líder aparece desenhada ao lado).
fn layer_title(name: &str) -> String {
    match name {
        "km-lider" => "Líder".into(),
        "km-alt" => "Alt".into(),
        "km-win" => "Win".into(),
        "km-sites" => "Sites".into(),
        other => other.trim_start_matches(kbd::MANAGED_PREFIX).to_string(),
    }
}

/// Teclas que abrem a camada, na ordem em que se aperta (`F13`, `B`).
fn trigger_caps(triggers: &HashMap<String, Trigger>, name: &str, depth: u8) -> Vec<String> {
    match triggers.get(name) {
        Some(Trigger::Key(k)) => vec![key_label(k)],
        Some(Trigger::Chord(ks)) => ks.iter().map(|k| key_label(k)).collect(),
        Some(Trigger::Inside { layer, key }) => {
            let mut v = if depth < 4 { trigger_caps(triggers, layer, depth + 1) } else { vec![] };
            v.push(key_label(key));
            v
        }
        _ => vec![],
    }
}

/// Explica como usar a tecla líder.
fn trigger_hint(triggers: &HashMap<String, Trigger>, name: &str) -> String {
    let caps = trigger_caps(triggers, name, 0).join(" + ");
    match triggers.get(name) {
        Some(Trigger::Key(_)) => format!("Segure {caps} e aperte a tecla do atalho. Tocar {caps} sozinha continua normal."),
        Some(Trigger::Chord(_)) => format!("Aperte {caps} juntas e depois a tecla do atalho (até {} s). Digitando uma depois da outra, sai normal.", trigger::ONE_SHOT_MS / 1000),
        Some(Trigger::Inside { layer, .. }) => format!("Aperte {caps} e depois a tecla do atalho (aberta pela camada {}).", layer_title(layer)),
        _ => "Esta camada ainda não tem tecla líder — defina uma para ela funcionar.".into(),
    }
}

fn cap_w(c: &str, h: f32) -> f32 {
    let n = c.chars().count();
    if n <= 2 {
        h
    } else {
        (h * 0.5 + n as f32 * h * 0.22).max(h * 1.3)
    }
}

/// Largura que `paint_caps` vai ocupar.
fn caps_width(caps: &[String], h: f32) -> f32 {
    caps.iter().map(|c| cap_w(c, h)).sum::<f32>() + 16.0 * caps.len().saturating_sub(1) as f32
}

/// Teclas desenhadas lado a lado ("R + T"), centradas em `y`, a partir de `x`.
/// Devolve o x final.
fn paint_caps(ui: &Ui, x: f32, y: f32, caps: &[String], h: f32) -> f32 {
    let mut x = x;
    let font = h * 0.44;
    for (i, c) in caps.iter().enumerate() {
        if i > 0 {
            ui.painter().text(pos2(x + 8.0, y), Align2::CENTER_CENTER, "+", th::semibold(h * 0.4), th::WEAK);
            x += 16.0;
        }
        let w = cap_w(c, h);
        let r = Rect::from_min_size(pos2(x, y - h / 2.0 - 1.5), vec2(w, h));
        // o keycap encolhe rótulos longos ("F13", "Alt"); compensa para ficarem legíveis
        let n = c.chars().count() as f32;
        let f = if n > 2.0 { h * 0.34 / (2.2 / n).clamp(0.42, 0.7) } else { font };
        th::keycap_paint(ui, r, c, f, false);
        x = r.max.x;
    }
    x
}

/// Como a tecla aparece desenhada (`c` → `C`, `spc` → `Espaço`, `___` → `(outras)`).
fn key_label(k: &str) -> String {
    let s = match k {
        "" => "?",
        "___" => "(outras)",
        "spc" => "Espaço",
        "ret" => "Enter",
        "tab" => "Tab",
        "esc" => "Esc",
        "bspc" => "Bksp",
        "del" => "Del",
        "ins" => "Ins",
        "home" => "Home",
        "end" => "End",
        "pgup" => "PgUp",
        "pgdn" => "PgDn",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "scln" => "Ç",
        "grv" => "'",
        "apo" => "~",
        "comma" => ",",
        "min" => "-",
        "eql" => "=",
        "lbrc" => "´",
        "rbrc" => "[",
        "bksl" => "]",
        "caps" => "Caps",
        "lsft" | "rsft" => "Shift",
        "lctl" | "rctl" => "Ctrl",
        "lalt" => "Alt",
        "ralt" => "AltGr",
        "lmet" | "rmet" => "Win",
        other => {
            if other.chars().count() == 1 || (other.starts_with('f') && other[1..].parse::<u8>().is_ok()) {
                return other.to_uppercase();
            }
            other
        }
    };
    s.to_string()
}

/// `C-S-a` → `Ctrl+Shift+A`.
fn pretty_keys(k: &str) -> String {
    let k = k.trim();
    if k.starts_with('(') {
        return if k.starts_with("(macro") { "Sequência de teclas".into() } else { "Teclas (kanata)".into() };
    }
    let mut rest = k;
    let mut parts: Vec<String> = vec![];
    'outer: loop {
        for (p, name) in [("RA-", "AltGr"), ("AG-", "AltGr"), ("C-", "Ctrl"), ("S-", "Shift"), ("A-", "Alt"), ("M-", "Win")] {
            if rest.len() > p.len() && rest.starts_with(p) {
                parts.push(name.into());
                rest = &rest[p.len()..];
                continue 'outer;
            }
        }
        break;
    }
    parts.push(key_label(rest));
    parts.join("+")
}

fn app_name(exe: &str) -> String {
    let file = exe.trim().rsplit(['\\', '/']).next().unwrap_or("");
    let stem = if file.to_lowercase().ends_with(".exe") { &file[..file.len() - 4] } else { file };
    let known = match stem.to_lowercase().as_str() {
        "code" => "VS Code",
        "msedge" => "Edge",
        "notepad" => "Bloco de notas",
        "wt" | "windowsterminal" => "Terminal",
        "explorer" => "Explorador",
        "obs64" | "obs32" | "obs" => "OBS",
        "wezterm-gui" | "wezterm" => "WezTerm",
        "cmd" => "Prompt de comando",
        "powershell" | "pwsh" => "PowerShell",
        "calc" => "Calculadora",
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Programa".into(),
    }
}

fn secs_label(ms: u32) -> String {
    format!("{:.2} s", ms as f32 / 1000.0).replace('.', ",")
}

fn key_name(k: egui::Key) -> Option<String> {
    use egui::Key::*;
    let s = match k {
        ArrowUp => "up",
        ArrowDown => "down",
        ArrowLeft => "left",
        ArrowRight => "right",
        Space => "spc",
        Enter => "ret",
        Tab => "tab",
        Escape => "esc",
        Backspace => "bspc",
        Delete => "del",
        Insert => "ins",
        Home => "home",
        End => "end",
        PageUp => "pgup",
        PageDown => "pgdn",
        Comma => "comma",
        Period => ".",
        Minus => "min",
        Semicolon => "scln",
        Slash => "/",
        _ => {
            let n = k.name();
            if n.len() == 1 || (n.starts_with('F') && n[1..].parse::<u8>().is_ok()) {
                return Some(n.to_lowercase());
            }
            return None;
        }
    };
    Some(s.to_string())
}

// ------------------------------------------------------------------ ícones dos apps

/// Ícones extraídos dos .exe (uma vez por nome/caminho).
#[derive(Default)]
struct IconCache {
    map: HashMap<String, Option<egui::TextureHandle>>,
}

impl IconCache {
    /// Tenta cada candidato (nome do exe ou caminho) até achar um ícone.
    fn get(&mut self, ctx: &egui::Context, windows: &[win::WinInfo], cands: &[&str]) -> Option<egui::TextureHandle> {
        for c in cands {
            let c = c.trim();
            if c.is_empty() || c == "*" {
                continue;
            }
            let key = c.to_lowercase();
            if let Some(t) = self.map.get(&key) {
                if t.is_some() {
                    return t.clone();
                }
                continue;
            }
            let path = if c.contains(['\\', '/']) {
                win::expand_env(c)
            } else {
                windows.iter().find(|w| w.exe == key && !w.path.is_empty()).map(|w| w.path.clone()).or_else(|| win::find_exe_path(c)).unwrap_or_default()
            };
            let tex = if path.to_lowercase().ends_with(".exe") {
                win::file_icon_rgba(&path, 48).map(|px| ctx.load_texture(format!("ico-{key}"), egui::ColorImage::from_rgba_unmultiplied([48, 48], &px), egui::TextureOptions::LINEAR))
            } else {
                None
            };
            self.map.insert(key, tex.clone());
            if tex.is_some() {
                return tex;
            }
        }
        None
    }
}

fn quoted_exes(r: &str) -> Vec<&str> {
    r.split('"').skip(1).step_by(2).filter(|s| s.to_lowercase().ends_with(".exe")).collect()
}

/// Título + ícone de uma ação, para os cartões.
fn act_view(icons: &mut IconCache, ctx: &egui::Context, windows: &[win::WinInfo], a: &Act, other_keys: bool) -> (String, Ico) {
    let glyph = |k| {
        let (g, c) = kind_ico(k);
        Ico::Glyph(g, c)
    };
    match a {
        Act::Nothing => ((if other_keys { "Bloquear" } else { "Nada" }).into(), glyph(Kind::Nothing)),
        Act::App { exe, run, opts, .. } => {
            let title = if exe.trim() == "*" || exe.trim().is_empty() {
                kbd::split_opts(opts).iter().find_map(|o| o.strip_prefix("title=").map(str::to_string)).unwrap_or_else(|| "Programa".into())
            } else {
                app_name(exe)
            };
            let ico = icons.get(ctx, windows, &[exe, run]).map(Ico::Tex).unwrap_or_else(|| glyph(Kind::App));
            (title, ico)
        }
        Act::Site { matches, .. } => (matches.split('|').next().unwrap_or("").trim().trim_start_matches("www.").to_string(), glyph(Kind::Site)),
        Act::Run { cmd, .. } => {
            let lower = cmd.trim().to_lowercase();
            let title = if lower.ends_with(".exe") {
                app_name(cmd)
            } else if lower.starts_with("http") {
                "Abrir link".into()
            } else {
                "Executar".into()
            };
            let ico = icons.get(ctx, windows, &[cmd]).map(Ico::Tex).unwrap_or_else(|| match lower.trim_end_matches(".exe") {
                "wt" | "cmd" | "powershell" | "pwsh" | "windowsterminal" => Ico::Glyph(icon::CONSOLE, th::TEXT),
                _ => glyph(Kind::Run),
            });
            (title, ico)
        }
        Act::Text(t) => (format!("“{t}”"), glyph(Kind::Text)),
        Act::Keys(k) => (pretty_keys(k), glyph(Kind::Keys)),
        Act::Raw(r) => {
            let cmd = r.split("push-msg").nth(1).and_then(|s| s.split_whitespace().next()).map(|s| s.trim_matches(['(', ')', '"']).to_string()).unwrap_or_default();
            let named = match cmd.as_str() {
                "topmost" => Some(("Sempre no topo", icon::WINDOW)),
                "next-window" => Some(("Próxima janela", icon::SWAP)),
                "sleep-pc" => Some(("Suspender o PC", icon::POWER)),
                "shutdown" => Some(("Desligar o PC", icon::POWER)),
                "lock" => Some(("Bloquear o PC", icon::BLOCKED)),
                "maximize" => Some(("Maximizar", icon::WINDOW)),
                "minimize" => Some(("Minimizar", icon::WINDOW)),
                "close-window" => Some(("Fechar janela", icon::WINDOW)),
                "notify" => Some(("Aviso", icon::WARNING)),
                _ => None,
            };
            if let Some((t, g)) = named {
                return (t.into(), Ico::Glyph(g, th::ACCENT_HI));
            }
            if cmd == "browser" {
                let ico = icons.get(ctx, windows, &["msedge.exe"]).map(Ico::Tex).unwrap_or_else(|| glyph(Kind::Site));
                return ("Navegador".into(), ico);
            }
            let exes = quoted_exes(r);
            if let Some(first) = exes.first() {
                let ico = icons.get(ctx, windows, &exes).map(Ico::Tex).unwrap_or_else(|| glyph(Kind::App));
                return (app_name(first), ico);
            }
            ("Código kanata".into(), glyph(Kind::Raw))
        }
    }
}

// ------------------------------------------------------------------ cartões

struct Row {
    title: String,
    sub: String,
    ico: Ico,
}

struct Item {
    idx: usize,
    caps: Vec<String>,
    rows: Vec<Row>,
    /// caminho que não existe neste PC
    broken: Option<String>,
}

/// O texto parece um caminho de arquivo (e não um nome solto, URL ou shell:)?
fn looks_like_path(s: &str) -> bool {
    let s = s.trim();
    let low = s.to_lowercase();
    if s.is_empty() || low.contains("://") || low.starts_with("shell:") || low.starts_with("mailto:") {
        return false;
    }
    s.contains('\\') || s.contains('/') || s.contains('%')
}

/// Existe? (com cache de alguns segundos para não consultar o disco a cada quadro)
fn path_exists(s: &str) -> bool {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};
    thread_local! {
        static CACHE: RefCell<(Option<Instant>, HashMap<String, bool>)> = RefCell::new((None, HashMap::new()));
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.0.map_or(true, |t| t.elapsed() > Duration::from_secs(3)) {
            c.1.clear();
            c.0 = Some(Instant::now());
        }
        if let Some(v) = c.1.get(s) {
            return *v;
        }
        let e = win::expand_env(s.trim().trim_matches('"'));
        let v = std::path::Path::new(&e).exists();
        c.1.insert(s.to_string(), v);
        v
    })
}

/// Caminho quebrado numa ação (programa ou arquivo que não existe).
fn act_broken(a: &Act) -> Option<String> {
    let p = match a {
        Act::App { run, .. } => run,
        Act::Run { cmd, .. } => cmd,
        _ => return None,
    };
    (looks_like_path(p) && !path_exists(p)).then(|| format!("Não encontrado neste PC: {}", p.trim()))
}

fn binding_broken(b: &Binding) -> Option<String> {
    act_broken(&b.tap).or_else(|| b.hold.as_ref().and_then(|h| act_broken(&h.1)))
}

fn layer_broken(l: &Layer) -> bool {
    l.bindings.iter().any(|b| binding_broken(b).is_some())
}

enum Pick {
    Edit(usize),
    Delete(usize),
    /// (índice do atalho na camada atual, nome da camada de destino)
    Move(usize, String),
    New,
    NewKey(String),
}

/// Submenu "Mover para camada" do botão direito. `targets` = (nome, rótulo).
fn move_menu(ui: &mut Ui, targets: &[(String, String)], idx: usize) -> Option<Pick> {
    let mut pick = None;
    ui.menu_button("Mover para camada", |ui| {
        if targets.is_empty() {
            ui.label(RichText::new("Não há outra camada").color(th::WEAK));
        }
        for (name, label) in targets {
            if ui.button(label).clicked() {
                pick = Some(Pick::Move(idx, name.clone()));
                ui.close();
            }
        }
    });
    pick
}

/// Nome curto de um atalho para o aviso de conflito.
fn binding_short(b: &Binding) -> String {
    if !b.name.trim().is_empty() {
        return b.name.trim().to_string();
    }
    match &b.tap {
        Act::App { exe, .. } => exe.clone(),
        Act::Keys(k) => key_label(k),
        Act::Text(t) => format!("texto \"{}\"", t.chars().take(20).collect::<String>()),
        Act::Run { cmd, .. } => cmd.clone(),
        Act::Site { .. } => "site".into(),
        _ => "outra ação".into(),
    }
}

/// Pinta o conteúdo de um cartão: tecla(s) | ícone + título + "tocar"/"segurar".
fn paint_card(ui: &Ui, rect: Rect, caps: &[String], rows: &[Row]) {
    let inner = rect.shrink2(vec2(12.0, 8.0));
    let p = ui.painter();
    let mut x = inner.min.x;
    if caps.len() <= 1 {
        let s = 42.0;
        let r = Rect::from_min_size(pos2(x, rect.center().y - s / 2.0 - 1.5), vec2(s, s));
        th::keycap_paint(ui, r, caps.first().map(String::as_str).unwrap_or("?"), 18.0, false);
        x = r.max.x;
    } else {
        let s = 30.0;
        for (i, c) in caps.iter().enumerate() {
            if i > 0 {
                p.text(pos2(x + 6.0, rect.center().y), Align2::CENTER_CENTER, "+", th::semibold(12.0), th::WEAK);
                x += 12.0;
            }
            let r = Rect::from_min_size(pos2(x, rect.center().y - s / 2.0 - 1.5), vec2(s, s));
            th::keycap_paint(ui, r, c, 14.0, false);
            x = r.max.x;
        }
    }
    x += 10.0;
    p.line_segment([pos2(x, inner.min.y + 2.0), pos2(x, inner.max.y - 2.0)], Stroke::new(1.0, th::BORDER));
    x += 10.0;
    let n = rows.len().max(1);
    let slot = inner.height() / n as f32;
    for (i, row) in rows.iter().enumerate() {
        let cy = inner.min.y + slot * (i as f32 + 0.5);
        let isz = if n == 1 { 26.0 } else { 18.0 };
        let ir = Rect::from_center_size(pos2(x + isz / 2.0, cy), Vec2::splat(isz));
        th::paint_ico(p, ir, &row.ico);
        let tx = ir.max.x + 9.0;
        let tw = inner.max.x - tx;
        if n == 1 {
            // título em cima, "tocar" embaixo
            let (t_h, s_h) = (14.5 * 1.3, 12.0 * 1.3);
            let top = cy - (t_h + s_h) / 2.0;
            th::text_trunc(ui, pos2(tx, top), &row.title, th::semibold(14.5), th::TEXT, tw);
            th::text_trunc(ui, pos2(tx, top + t_h), &row.sub, th::regular(12.0), th::WEAK, tw);
        } else {
            // tocar + segurar: uma linha cada ("Claude  tocar")
            let mut j = egui::text::LayoutJob::default();
            j.append(&row.title, 0.0, egui::TextFormat::simple(th::semibold(13.5), th::TEXT));
            j.append(&row.sub, 7.0, egui::TextFormat::simple(th::regular(11.5), th::WEAK));
            j.wrap = egui::text::TextWrapping { max_width: tw.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let g = ui.ctx().fonts_mut(|f| f.layout_job(j));
            p.galley(pos2(tx, cy - g.size().y / 2.0), g, th::TEXT);
        }
        if i + 1 < n {
            let y = inner.min.y + slot * (i as f32 + 1.0);
            p.line_segment([pos2(x, y), pos2(inner.max.x, y)], Stroke::new(1.0, th::BORDER));
        }
    }
}

/// Grade de cartões + um cartão "novo" no fim.
fn cards_grid(ui: &mut Ui, items: &[Item], targets: &[(String, String)], selected: Option<usize>, new_active: bool, new_label: &str, hl: Option<usize>, hovered: &mut Option<usize>) -> Option<Pick> {
    let gap = 10.0;
    let avail = ui.available_width();
    let cols = ((avail + gap) / (205.0 + gap)).floor().clamp(1.0, 8.0) as usize;
    let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let h = 66.0;
    let mut pick = None;
    let total = items.len() + 1;
    ui.spacing_mut().item_spacing = vec2(gap, gap);
    for start in (0..total).step_by(cols) {
        ui.horizontal(|ui| {
            for idx in start..(start + cols).min(total) {
                if let Some(it) = items.get(idx) {
                    let resp = th::card(ui, vec2(w, h), selected == Some(it.idx));
                    if resp.hovered() {
                        *hovered = Some(it.idx);
                    }
                    if ui.is_rect_visible(resp.rect) {
                        if hl == Some(it.idx) && selected != Some(it.idx) {
                            ui.painter().rect(resp.rect, 12, th::CARD_HOVER, Stroke::new(1.5, th::ACCENT_HI), StrokeKind::Inside);
                        }
                        paint_card(ui, resp.rect, &it.caps, &it.rows);
                        if it.broken.is_some() {
                            let p = ui.painter();
                            p.rect(resp.rect, 12, Color32::from_rgba_unmultiplied(248, 113, 113, 18), Stroke::new(2.0, th::RED), StrokeKind::Inside);
                            p.circle_filled(pos2(resp.rect.max.x - 12.0, resp.rect.min.y + 12.0), 5.0, th::RED);
                        }
                    }
                    let resp = match &it.broken {
                        Some(e) => resp.on_hover_text(e),
                        None => resp,
                    };
                    if resp.clicked() {
                        pick = Some(Pick::Edit(it.idx));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Editar").clicked() {
                            pick = Some(Pick::Edit(it.idx));
                        }
                        if let Some(p) = move_menu(ui, targets, it.idx) {
                            pick = Some(p);
                        }
                        if ui.button("Excluir").clicked() {
                            pick = Some(Pick::Delete(it.idx));
                        }
                    });
                } else {
                    // sem atalho selecionado = o formulário de baixo é o "novo"
                    let resp = th::card(ui, vec2(w, h), new_active);
                    let col = if new_active { th::TEXT } else { th::WEAK };
                    let j = th::icon_job(icon::ADD, new_label, th::regular(14.5), col, col);
                    let g = ui.ctx().fonts_mut(|f| f.layout_job(j));
                    ui.painter().galley(resp.rect.center() - g.size() / 2.0, g, th::WEAK);
                    if resp.clicked() {
                        pick = Some(Pick::New);
                    }
                }
            }
        });
    }
    pick
}

/// Teclado ABNT2/US simplificado: (nome no kanata, largura em teclas).
fn kb_layout() -> Vec<Vec<(String, f32)>> {
    let s = |v: &str| v.to_string();
    let letters = |l: &str| l.chars().map(|c| (c.to_string(), 1.0)).collect::<Vec<_>>();
    let mut r0 = vec![(s("esc"), 1.0)];
    r0.extend((1..=12).map(|i| (format!("f{i}"), 1.0)));
    let mut r1 = vec![(s("grv"), 1.0)];
    r1.extend(letters("1234567890"));
    r1.extend([(s("min"), 1.0), (s("eql"), 1.0), (s("bspc"), 2.0)]);
    let mut r2 = vec![(s("tab"), 1.5)];
    r2.extend(letters("qwertyuiop"));
    r2.extend([(s("lbrc"), 1.0), (s("rbrc"), 1.0), (s("bksl"), 1.5)]);
    let mut r3 = vec![(s("caps"), 1.75)];
    r3.extend(letters("asdfghjkl"));
    r3.extend([(s("scln"), 1.0), (s("apo"), 1.0), (s("ret"), 2.25)]);
    let mut r4 = vec![(s("lsft"), 2.25)];
    r4.extend(letters("zxcvbnm"));
    r4.extend([(s("comma"), 1.0), (s("."), 1.0), (s("/"), 1.0), (s("rsft"), 2.75)]);
    let r5 = vec![(s("lctl"), 1.25), (s("lmet"), 1.25), (s("lalt"), 1.25), (s("spc"), 6.25), (s("ralt"), 1.25), (s("rmet"), 1.25), (s("rctl"), 1.25)];
    let r6 = ["ins", "home", "pgup", "del", "end", "pgdn", "", "left", "up", "down", "right"].iter().map(|k| (s(k), if k.is_empty() { 0.5 } else { 1.0 })).collect();
    vec![r0, r1, r2, r3, r4, r5, r6]
}

// ------------------------------------------------------------------ formulário de ação (reutilizado)

/// Campo principal mostrado na linha (o resto fica no "…").
fn main_field(f: &mut ActForm) -> Option<(&'static str, &mut String, &'static str, bool)> {
    match f.kind {
        Kind::App => Some(("Programa:", &mut f.exe, "ex.: Telegram.exe", false)),
        Kind::Site => Some(("Site:", &mut f.site, "youtube.com  (use | para alternativas)", false)),
        Kind::Run => Some(("Comando:", &mut f.cmd, "calc.exe, C:\\pasta\\arquivo.pdf, https://...", false)),
        Kind::Text => Some(("Texto:", &mut f.text, "ex.: Bom dia! Segue o orçamento.", false)),
        Kind::Keys => Some(("Teclas:", &mut f.keys, "C-S-a", true)),
        Kind::Raw if !f.raw.contains('\n') => Some(("Código:", &mut f.raw, "(push-msg ...)", true)),
        _ => None,
    }
}

/// Opções ao escolher uma janela na lista. Se mais de uma janela tem o mesmo
/// .exe (ex.: vários apps/sites no Edge), grava `title=...` para o atalho
/// focar a janela certa e não a primeira do Edge. Remove um `title=` antigo.
fn window_opts(old: &str, w: &win::WinInfo, same_exe: usize) -> String {
    let mut parts: Vec<String> = kbd::split_opts(old)
        .into_iter()
        .filter(|o| !o.starts_with("title="))
        .map(|o| if o.contains(' ') { format!("\"{o}\"") } else { o })
        .collect();
    let t = w.title.trim();
    if same_exe > 1 && !t.is_empty() && !t.contains('"') {
        parts.push(format!("title=\"{t}\""));
    }
    parts.join(" ")
}

fn has_extras(f: &ActForm) -> bool {
    matches!(f.kind, Kind::App | Kind::Site | Kind::Run) || (f.kind == Kind::Raw && f.raw.contains('\n'))
}

fn field(ui: &mut Ui, text: &mut String, hint: &str, mono: bool, width: f32) -> egui::Response {
    let mut te = egui::TextEdit::singleline(text).hint_text(hint.to_string()).desired_width(width).margin(Margin::symmetric(10, 8));
    if mono {
        te = te.font(egui::TextStyle::Monospace);
    }
    ui.add(te)
}

fn kind_combo(ui: &mut Ui, id: &str, f: &mut ActForm) {
    let (g, c) = kind_ico(f.kind);
    let label = KINDS.iter().find(|k| k.0 == f.kind).map(|k| k.1).unwrap_or("?");
    egui::ComboBox::from_id_salt(format!("{id}-kind"))
        .width(270.0)
        .icon(th::chevron)
        .selected_text(th::icon_job(g, label, th::regular(15.0), th::TEXT, c))
        .show_ui(ui, |ui| {
            for (k, label) in KINDS {
                let (g, c) = kind_ico(*k);
                ui.selectable_value(&mut f.kind, *k, th::icon_job(g, label, th::regular(15.0), th::TEXT, c));
            }
        });
}

/// Campos extras ("…") — os mesmos do formulário antigo.
fn act_extras(ui: &mut Ui, id: &str, f: &mut ActForm) {
    let w = (ui.available_width() - 200.0).max(200.0);
    egui::Grid::new(format!("{id}-grid")).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| match f.kind {
        Kind::App => {
            ui.label("Abrir com (se fechado)");
            field(ui, &mut f.run, "caminho do .exe, %LocalAppData%\\..., shell:AppsFolder\\...", false, w);
            ui.end_row();
            ui.label("Argumentos");
            field(ui, &mut f.args, "", false, w);
            ui.end_row();
            ui.label("Opções").on_hover_text(
                "nomax = não maximizar\ntitle=TEXTO = só janelas cujo título contém\ntitle-ends=TEXTO\nclass=CLASSE\nprefer=TEXTO = essas janelas primeiro no ciclo\nwait=5 = segundos esperando abrir\nUse aspas se tiver espaço: \"title=Meu App\"",
            );
            field(ui, &mut f.opts, "ex.: nomax  (passe o mouse em Opções)", false, w);
            ui.end_row();
        }
        Kind::Site => {
            ui.label("Busca de aba");
            field(ui, &mut f.search, "opcional — padrão: o próprio endereço", false, w);
            ui.end_row();
            ui.label("URL se o Edge estiver fechado");
            field(ui, &mut f.url, "opcional — padrão: https://endereço", false, w);
            ui.end_row();
        }
        Kind::Run => {
            ui.label("Argumentos");
            field(ui, &mut f.cmd_args, "", false, w);
            ui.end_row();
        }
        Kind::Raw => {
            ui.label("Código");
            ui.add(egui::TextEdit::multiline(&mut f.raw).code_editor().desired_rows(3).desired_width(w));
            ui.end_row();
        }
        _ => {}
    });
}

/// Uma linha "Ao tocar"/"Ao segurar": cabeçalho | tipo | campo principal | … | janelas.
fn act_row(ui: &mut Ui, id: &str, enabled: bool, head: impl FnOnce(&mut Ui), f: &mut ActForm, more: &mut bool, windows: &mut Vec<win::WinInfo>) {
    let narrow = ui.available_width() < 900.0;
    let mut head = Some(head);
    if narrow {
        ui.horizontal(|ui| {
            ui.set_min_height(th::BTN_H);
            if let Some(h) = head.take() {
                h(ui);
            }
        });
    }
    ui.horizontal(|ui| {
        if !narrow {
            ui.allocate_ui_with_layout(vec2(HEAD_W, th::BTN_H), Layout::left_to_right(Align::Center), |ui| {
                if let Some(h) = head.take() {
                    h(ui);
                }
            });
        }
        ui.add_enabled_ui(enabled, |ui| {
            kind_combo(ui, id, f);
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if f.kind == Kind::App {
                    let r = th::icon_button(ui, false, icon::WINDOW, "Escolher uma janela aberta");
                    egui::Popup::menu(&r).show(|ui| {
                        ui.set_min_width(420.0);
                        if ui.button(format!("{}  Atualizar lista", icon::REFRESH)).clicked() {
                            *windows = win::list_windows();
                        }
                        ui.separator();
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for w in windows.iter() {
                                let label = format!("{} — {}", w.exe, w.title.chars().take(50).collect::<String>());
                                if ui.selectable_label(false, label).clicked() {
                                    f.exe = w.exe.clone();
                                    f.run = w.path.clone();
                                    let same_exe = windows.iter().filter(|x| x.exe == w.exe).count();
                                    f.opts = window_opts(&f.opts, w, same_exe);
                                    if same_exe > 1 {
                                        // mostra o filtro `title=` na hora, em vez de escondê-lo em "Mais opções"
                                        *more = true;
                                    }
                                    ui.close();
                                }
                            }
                        });
                    });
                }
                if has_extras(f) && th::icon_button(ui, *more, icon::MORE, "Mais opções").clicked() {
                    *more = !*more;
                }
                if f.kind == Kind::Keys && th::icon_button(ui, f.rec, icon::KEYBOARD, "Gravar atalho: clique e aperte a combinação (ex.: Ctrl+Shift+F6)").clicked() {
                    f.rec = true;
                    f.rec_buf.clear();
                }
                let w = (ui.available_width() - 95.0).max(80.0);
                match main_field(f) {
                    Some((label, text, hint, mono)) => {
                        field(ui, text, hint, mono, w);
                        ui.label(RichText::new(label).color(th::WEAK));
                    }
                    None => {
                        let msg = if f.kind == Kind::Nothing { "A tecla não faz nada (bloqueada)." } else { "Código com várias linhas — abaixo." };
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.label(RichText::new(msg).color(th::WEAK));
                        });
                    }
                }
            });
        });
    });
    if f.rec {
        record_popup(ui, id, f);
    }
    let force = f.kind == Kind::Raw && f.raw.contains('\n');
    if enabled && has_extras(f) && (*more || force) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if !narrow {
                ui.add_space(HEAD_W + 8.0);
            }
            ui.vertical(|ui| act_extras(ui, id, f));
        });
    }
}

/// Converte modificadores + tecla para o formato do kanata (ex.: C-S-f6).
fn combo_name(m: egui::Modifiers, k: &str) -> String {
    let mut s = String::new();
    if m.ctrl {
        s += "C-";
    }
    if m.shift {
        s += "S-";
    }
    if m.alt {
        s += "A-";
    }
    s + k
}

/// Rótulo legível de um atalho kanata (C-S-f6 → Ctrl + Shift + F6).
fn combo_label(c: &str) -> String {
    let mut parts = vec![];
    let mut rest = c;
    loop {
        let p = match rest.get(..2) {
            Some("C-") => "Ctrl",
            Some("S-") => "Shift",
            Some("A-") => "Alt",
            Some("M-") => "Win",
            _ => break,
        };
        if rest.len() <= 2 {
            break;
        }
        parts.push(p.to_string());
        rest = &rest[2..];
    }
    parts.push(key_label(rest));
    parts.join(" + ")
}

/// Popup que grava a combinação apertada e põe no campo Teclas.
fn record_popup(ui: &mut Ui, id: &str, f: &mut ActForm) {
    let got = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } if *key != egui::Key::Escape => key_name(*key).map(|n| combo_name(*modifiers, &n)),
            egui::Event::Copy => Some("C-c".to_string()),
            egui::Event::Cut => Some("C-x".to_string()),
            egui::Event::Paste(_) => Some("C-v".to_string()),
            _ => None,
        })
    });
    if let Some(c) = got {
        f.rec_buf = c;
    }
    let cur = f.rec_buf.clone();
    let mut done: Option<bool> = None;
    let m = egui::Modal::new(egui::Id::unique(("rec-combo", id))).frame(th::section()).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Aperte o atalho a ser enviado").font(th::semibold(17.0)));
            ui.add_space(4.0);
            ui.label(RichText::new("Ex.: Ctrl + Shift + F6. Se errar, é só apertar de novo.").color(th::WEAK));
            ui.add_space(14.0);
            let txt = if cur.is_empty() { "…".to_string() } else { combo_label(&cur) };
            ui.label(RichText::new(txt).font(th::semibold(26.0)).color(th::TEXT));
            if !cur.is_empty() {
                ui.label(RichText::new(&cur).monospace().color(th::WEAK));
            }
            ui.add_space(14.0);
        });
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if ui.add_enabled_ui(!cur.is_empty(), |ui| th::button(ui, Look::Primary, icon::CHECK, "Aceitar")).inner.clicked() {
                    done = Some(true);
                }
                if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                    done = Some(false);
                }
            });
        });
    });
    if m.should_close() && done.is_none() {
        done = Some(false);
    }
    if let Some(ok) = done {
        if ok {
            f.keys = std::mem::take(&mut f.rec_buf);
        }
        f.rec_buf.clear();
        f.rec = false;
    }
}

/// Círculo com ícone + rótulo em negrito (cabeçalho das linhas do formulário).
fn row_head(ui: &mut Ui, ic: &str, text: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 18.0, Color32::from_rgb(25, 45, 82));
    ui.painter().text(r.center() + vec2(0.0, 1.0), Align2::CENTER_CENTER, ic, th::regular(17.0), th::ACCENT_HI);
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(th::semibold(15.5)));
}

/// Caixa "</> Código kanata gerado ✓ …" que ocupa a largura dada.
fn code_box(ui: &mut Ui, w: f32, rendered: &Result<String, String>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(120.0), th::BTN_H), Sense::hover());
    let p = ui.painter();
    p.rect(rect, 8, th::FIELD, Stroke::new(1.0, th::BORDER), StrokeKind::Inside);
    let j = th::icon_job(icon::CODE, "Código kanata gerado", th::regular(14.5), th::WEAK, th::WEAK);
    let g = ui.ctx().fonts_mut(|f| f.layout_job(j));
    let mut x = rect.min.x + 14.0;
    let gs = g.size();
    p.galley(pos2(x, rect.center().y - gs.y / 2.0), g, th::WEAK);
    x += gs.x + 16.0;
    let (ok, ic, col) = match rendered {
        Ok(_) => (true, icon::CHECK, th::GREEN),
        Err(_) => (false, icon::WARNING, th::RED),
    };
    let c = pos2(x + 10.0, rect.center().y);
    if ok {
        p.circle_filled(c, 10.0, Color32::from_rgb(34, 197, 94));
        p.text(c + vec2(0.0, 1.0), Align2::CENTER_CENTER, ic, th::regular(11.0), Color32::from_rgb(8, 30, 16));
    } else {
        p.text(c, Align2::CENTER_CENTER, ic, th::regular(16.0), col);
    }
    x += 32.0;
    p.line_segment([pos2(x, rect.min.y + 8.0), pos2(x, rect.max.y - 8.0)], Stroke::new(1.0, th::BORDER));
    x += 12.0;
    let (txt, font, color) = match rendered {
        Ok(code) => (code.split_whitespace().collect::<Vec<_>>().join(" "), egui::FontId::monospace(13.5), th::TEXT),
        Err(e) => (e.clone(), th::regular(14.0), th::RED),
    };
    let h = font.size * 1.25;
    th::text_trunc(ui, pos2(x, rect.center().y - h / 2.0), &txt, font, color, rect.max.x - x - 12.0);
    resp.on_hover_text(match rendered {
        Ok(c) => c.clone(),
        Err(e) => e.clone(),
    });
}

/// Barra de baixo: código gerado + Excluir / Limpar / Aplicar. Devolve (aplicar, limpar, excluir).
fn action_bar(ui: &mut Ui, rendered: &Result<String, String>, editing: bool, add_label: &str) -> (bool, bool, bool) {
    let mut out = (false, false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (ic, label) = if editing { (icon::CHECK, "Aplicar alteração") } else { (icon::ADD, add_label) };
        out.0 = ui.add_enabled_ui(rendered.is_ok(), |ui| th::button(ui, Look::Primary, ic, label)).inner.clicked();
        out.1 = th::button(ui, Look::Normal, icon::REFRESH, "Limpar").clicked();
        if editing {
            out.2 = th::button(ui, Look::Normal, icon::DELETE, "Excluir").clicked();
        }
        let w = ui.available_width() - 12.0;
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| code_box(ui, w, rendered));
    });
    out
}

/// Botões "Simples | Código" do cabeçalho do formulário. Devolve true se mudou.
fn mode_toggle(ui: &mut Ui, code_mode: &mut bool) -> bool {
    let mut changed = false;
    ui.spacing_mut().item_spacing.x = 4.0;
    // direita → esquerda
    if th::button(ui, if *code_mode { Look::Primary } else { Look::Normal }, icon::CODE, "Código").clicked() && !*code_mode {
        *code_mode = true;
        changed = true;
    }
    if th::button(ui, if *code_mode { Look::Normal } else { Look::Primary }, "", "Simples").clicked() && *code_mode {
        *code_mode = false;
        changed = true;
    }
    changed
}

// ------------------------------------------------------------------ editor

struct Editor {
    /// atalho sob o mouse no quadro anterior (liga card e tecla)
    hl: Option<usize>,
    cfg: PathBuf,
    text: String,
    disk_text: String,
    layers: Vec<Layer>,
    parse_err: Option<String>,
    sel_layer: String,
    tab: Tab,
    view: View,
    search: String,
    form: Form,
    more_tap: bool,
    more_hold: bool,
    code_mode: bool,
    code_buf: String,
    code_err: Option<String>,
    status: Option<(bool, String)>,
    windows: Vec<win::WinInfo>,
    icons: IconCache,
    scancode: Result<Option<Vec<scancode::Mapping>>, String>,
    daemon_running: bool,
    autostart: bool,
    kb_active: bool,
    last_poll: Instant,
    new_layer: String,
    new_layer_keys: String,
    /// captura da tecla da nova camada (modal aberto pelo botão)
    new_layer_cap: bool,
    new_layer_held: Vec<String>,
    new_layer_got: Vec<String>,
    /// o que ativa cada camada km-*
    triggers: HashMap<String, Trigger>,
    /// mover atalho esperando confirmação: (índice, camada de destino)
    pending_move: Option<(usize, String)>,
    lead: LeadEdit,
    remap: RemapForm,
    /// recursos opcionais (aba Recursos)
    features: Features,
    feat_status: Option<String>,
    /// versões anteriores da config, para o Ctrl+Z (a última no fim)
    undo: Vec<String>,
    /// atualização do GitHub (pop-up)
    upd: UpdateState,
}

/// Edição da tecla líder da camada aberta.
#[derive(Default)]
struct LeadEdit {
    open: bool,
    /// primeiro clique em "Apagar camada" (o segundo confirma)
    confirm_delete: bool,
    keys: String,
    capturing: bool,
    /// teclas seguradas agora / todas as apertadas na captura
    held: Vec<String>,
    got: Vec<String>,
}

/// Estado da aba "Troca de teclas".
struct RemapForm {
    /// mapa do registro sendo editado (só vale depois de "Aplicar no registro")
    reg_edit: Vec<scancode::Mapping>,
    reg_pending: bool,
    add_from: u16,
    add_to: u16,
    add_reg: bool,
    leader_phys: u16,
    leader_reg: bool,
}

impl Editor {
    fn new(cfg: PathBuf) -> Self {
        let _ = paths::ensure_config(&cfg);
        let text = std::fs::read_to_string(&cfg).unwrap_or_default();
        let features = features::load(&cfg);
        let mut e = Editor {
            features,
            feat_status: None,
            cfg,
            disk_text: text.clone(),
            text,
            layers: vec![],
            parse_err: None,
            sel_layer: "km-lider".into(),
            tab: Tab::Macros,
            view: View::Grid,
            search: String::new(),
            form: Form::default(),
            more_tap: false,
            more_hold: false,
            code_mode: false,
            code_buf: String::new(),
            code_err: None,
            hl: None,
            status: None,
            windows: win::list_windows(),
            icons: IconCache::default(),
            scancode: scancode::read_system(),
            daemon_running: false,
            autostart: daemon::autostart_enabled(),
            kb_active: false,
            last_poll: Instant::now(),
            new_layer: String::new(),
            new_layer_keys: String::new(),
            new_layer_cap: false,
            new_layer_held: vec![],
            new_layer_got: vec![],
            triggers: HashMap::new(),
            pending_move: None,
            lead: LeadEdit::default(),
            remap: RemapForm { reg_edit: vec![], reg_pending: false, add_from: 0x003A, add_to: 0x001D, add_reg: false, leader_phys: 0xE05B, leader_reg: false },
            undo: vec![],
            upd: UpdateState { check: Some(update::check_in_background()), ..Default::default() },
        };
        e.reload_registry();
        // primeiro uso: guarda o registro como estava antes de o app mexer em qualquer coisa
        if backup::list(&e.cfg, backup::KIND_REG).is_empty() {
            if let Ok(cur) = &e.scancode {
                let cur = cur.clone().unwrap_or_default();
                let _ = backup::save(&e.cfg, backup::KIND_REG, "original", "reg", &scancode::reg_bytes(&scancode::reg_for(&cur)));
            }
        }
        e.poll();
        e.reparse();
        if !e.layers.iter().any(|l| l.name == e.sel_layer) {
            if let Some(l) = e.layers.first() {
                e.sel_layer = l.name.clone();
            }
        }
        // abre já com o primeiro atalho selecionado
        if let Some(b) = e.layers.iter().find(|l| l.name == e.sel_layer).and_then(|l| l.bindings.first()).cloned() {
            let _ = b; // nada selecionado ao abrir: o popup só aparece ao clicar
        }
        e
    }

    fn poll(&mut self) {
        self.daemon_running = !daemon::find_daemon().is_null();
        self.kb_active = std::fs::read_to_string(self.cfg.with_file_name("keymacros-estado.txt")).map(|t| t.trim() == "ativo").unwrap_or(false);
        self.last_poll = Instant::now();
    }

    fn reparse(&mut self) {
        match kbd::managed_layers(&self.text) {
            Ok(l) => {
                let names: Vec<String> = l.iter().map(|x| x.name.clone()).collect();
                self.triggers = trigger::find_all(&self.text, &names);
                self.layers = l;
                self.parse_err = None;
            }
            Err(e) => self.parse_err = Some(e),
        }
    }

    fn dirty(&self) -> bool {
        self.text != self.disk_text
    }

    /// Valida e salva. Guarda a versão anterior para o Ctrl+Z.
    fn save(&mut self) -> bool {
        let old = self.disk_text.clone();
        let ok = self.write_config();
        if ok && old != self.disk_text {
            self.push_undo(old);
        }
        ok
    }

    fn push_undo(&mut self, text: String) {
        self.undo.push(text);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z: volta à versão anterior da config. Na aba Macros já grava de novo.
    fn undo_last(&mut self) {
        let Some(prev) = self.undo.pop() else {
            self.status = Some((false, "Nada para desfazer.".into()));
            return;
        };
        let cur = std::mem::replace(&mut self.text, prev.clone());
        self.reparse();
        if self.tab == Tab::Macros && !self.write_config() {
            self.text = cur;
            self.reparse();
            self.undo.push(prev);
            return;
        }
        self.load_form(Form::default());
        self.lead = LeadEdit::default();
        self.status = Some(if self.tab == Tab::Macros {
            (true, "Desfeito.".into())
        } else {
            (true, "Desfeito. Clique em Salvar para gravar.".into())
        });
    }

    /// Valida e grava. Antes de sobrescrever, guarda o arquivo antigo em `backups\`.
    fn write_config(&mut self) -> bool {
        if let Err(e) = paths::check_text(&self.text, Some(&self.cfg)) {
            self.status = Some((false, format!("Não salvei — a config tem erro:\n{e}")));
            return false;
        }
        if let Ok(old) = std::fs::read(&self.cfg) {
            if old != self.text.as_bytes() {
                if let Err(e) = backup::save(&self.cfg, backup::KIND_CFG, "", "kbd", &old) {
                    self.status = Some((false, format!("Não salvei — falhou o backup da config anterior: {e}")));
                    return false;
                }
            }
        }
        match std::fs::write(&self.cfg, &self.text) {
            Ok(()) => {
                self.disk_text = self.text.clone();
                let extra = if self.daemon_running { " O KeyMacros recarrega sozinho." } else { "" };
                self.status = Some((true, format!("Salvo e validado (versão anterior guardada em backups).{extra}")));
                true
            }
            Err(e) => {
                self.status = Some((false, format!("Erro ao salvar: {e}")));
                false
            }
        }
    }

    fn reload_registry(&mut self) {
        self.scancode = scancode::read_system();
        self.remap.reg_edit = self.current_reg();
        self.remap.reg_pending = scancode::reboot_pending(&self.cfg);
    }

    fn current_reg(&self) -> Vec<scancode::Mapping> {
        match &self.scancode {
            Ok(Some(m)) => m.clone(),
            _ => vec![],
        }
    }

    /// Grava no registro um `.reg` (pede admin). Sempre faz antes o backup do estado atual.
    fn import_registry(&mut self, file: &std::path::Path, expected: Option<&[scancode::Mapping]>) {
        let cur = match &self.scancode {
            Ok(m) => m.clone().unwrap_or_default(),
            Err(e) => {
                self.status = Some((false, format!("Não mexi no registro: não consegui ler o estado atual para o backup ({e}).")));
                return;
            }
        };
        let before = match backup::save(&self.cfg, backup::KIND_REG, "antes", "reg", &scancode::reg_bytes(&scancode::reg_for(&cur))) {
            Ok(p) => p,
            Err(e) => {
                self.status = Some((false, format!("Não mexi no registro: falhou o backup ({e}).")));
                return;
            }
        };
        let bname = before.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match scancode::import_reg_elevated(file) {
            Err(e) => self.status = Some((false, format!("Registro não alterado: {e}"))),
            Ok(()) => {
                scancode::mark_applied(&self.cfg);
                self.reload_registry();
                let now = self.current_reg();
                if expected.is_some_and(|x| x != now.as_slice()) {
                    self.status = Some((false, "O reg.exe terminou, mas o registro não ficou como esperado. Confira a lista abaixo.".into()));
                } else {
                    self.status = Some((true, format!("Registro gravado. REINICIE o PC (Reiniciar, não Desligar) para valer. Estado anterior salvo em backups\\{bname}.")));
                }
            }
        }
    }

    fn apply_registry(&mut self, maps: Vec<scancode::Mapping>) {
        if let Err(e) = scancode::validate(&maps) {
            self.status = Some((false, e));
            return;
        }
        match backup::save(&self.cfg, backup::KIND_REG, "novo", "reg", &scancode::reg_bytes(&scancode::reg_for(&maps))) {
            Ok(file) => self.import_registry(&file, Some(&maps)),
            Err(e) => self.status = Some((false, format!("Não consegui gravar o .reg: {e}"))),
        }
    }

    /// Altera o texto da config e salva (com backup). Se não salvar, volta o texto.
    fn edit_config_and_save(&mut self, new: Result<String, String>) -> bool {
        match new {
            Err(e) => {
                self.status = Some((false, e));
                false
            }
            Ok(t) => {
                let old = std::mem::replace(&mut self.text, t);
                self.reparse();
                if self.save() {
                    true
                } else {
                    self.text = old;
                    self.reparse();
                    false
                }
            }
        }
    }

    /// Grava uma mudança feita pela UI (salva na hora; se a config ficar inválida, volta e mostra o erro).
    fn commit(&mut self, new: String) -> Result<(), String> {
        if self.edit_config_and_save(Ok(new)) {
            Ok(())
        } else {
            Err(self.status.as_ref().map(|s| s.1.clone()).unwrap_or_default())
        }
    }

    fn set_layer_bindings(&mut self, bindings: Vec<Binding>) -> Result<(), String> {
        let new = kbd::replace_layer(&self.text, &self.sel_layer, &bindings)?;
        self.commit(new)
    }

    /// Rótulo de uma camada: as teclas que a abrem (ex.: `F13 + C`) ou o nome.
    fn layer_label(&self, name: &str) -> String {
        let caps = trigger_caps(&self.triggers, name, 0);
        if caps.is_empty() {
            format!("{} (sem tecla)", layer_title(name))
        } else {
            caps.join(" + ")
        }
    }

    /// Camadas que podem receber o atalho: (nome, rótulo).
    fn move_targets(&self) -> Vec<(String, String)> {
        self.layers.iter().filter(|l| l.name != self.sel_layer).map(|l| (l.name.clone(), self.layer_label(&l.name))).collect()
    }

    /// Move o atalho `idx` da camada atual para `to`. Se a camada de destino já
    /// tem a mesma tecla e `replace` é false, não mexe em nada e devolve Ok(false)
    /// para a tela perguntar. Com `replace`, a tecla antiga do destino é trocada.
    fn move_binding(&mut self, idx: usize, to: &str, replace: bool) -> Result<bool, String> {
        let src_name = self.sel_layer.clone();
        let mut src = self.layers.iter().find(|l| l.name == src_name).map(|l| l.bindings.clone()).ok_or("camada não encontrada")?;
        let mut dst = self.layers.iter().find(|l| l.name == to).map(|l| l.bindings.clone()).ok_or("camada de destino não encontrada")?;
        if idx >= src.len() {
            return Err("atalho não encontrado".into());
        }
        let b = src[idx].clone();
        if let Some(j) = dst.iter().position(|x| x.key == b.key) {
            if !replace {
                return Ok(false);
            }
            dst.remove(j);
        }
        dst.push(b);
        src.remove(idx);
        // duas trocas no texto e um único salvamento
        let t1 = kbd::replace_layer(&self.text, to, &dst)?;
        let t2 = kbd::replace_layer(&t1, &src_name, &src)?;
        self.commit(t2)?;
        self.status = Some((true, format!("Atalho movido para {}. Salvo automaticamente.", self.layer_label(to))));
        Ok(true)
    }

    /// Aviso antes de mover quando a camada de destino já tem a mesma tecla.
    fn move_conflict_modal(&mut self, ctx: &egui::Context) {
        let Some((idx, to)) = self.pending_move.clone() else { return };
        let Some(src) = self.layers.iter().find(|l| l.name == self.sel_layer) else {
            self.pending_move = None;
            return;
        };
        let Some(b) = src.bindings.get(idx).cloned() else {
            self.pending_move = None;
            return;
        };
        let existing = self.layers.iter().find(|l| l.name == to).and_then(|l| l.bindings.iter().find(|x| x.key == b.key)).cloned();
        let dst_label = self.layer_label(&to);
        let mut choice: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("move-conflict")).frame(th::section()).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Tecla já existe na camada").font(th::semibold(17.0)));
            ui.add_space(6.0);
            let key = key_label(&b.key);
            let what = existing.as_ref().map(binding_short).unwrap_or_default();
            ui.label(RichText::new(format!("Já existe a tecla {key} em {dst_label} ({what}).")).color(th::TEXT));
            ui.label(RichText::new("Quer substituir pela tecla que você está movendo? A antiga será apagada.").color(th::WEAK));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if th::button(ui, Look::Primary, icon::CHECK, "Substituir").clicked() {
                        choice = Some(true);
                    }
                    if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                        choice = Some(false);
                    }
                });
            });
        });
        if m.should_close() && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                self.pending_move = None;
                if let Err(e) = self.move_binding(idx, &to, true) {
                    self.status = Some((false, e));
                }
            }
            Some(false) => self.pending_move = None,
            None => {}
        }
    }

    fn post_daemon(&self, msg: u32, wp: usize) {
        let h = daemon::find_daemon();
        if !h.is_null() {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(h, msg, wp, 0) };
        }
    }

    fn load_form(&mut self, f: Form) {
        self.form = f;
        self.more_tap = false;
        self.more_hold = false;
        self.code_buf = self.form.binding().render().unwrap_or_default();
        self.code_err = None;
    }

    /// Modo "Código": o texto digitado vira o formulário (mesmo parser do arquivo).
    fn sync_from_code(&mut self) {
        let src = format!("(deflayermap (km-x) {} )", self.code_buf);
        match kbd::managed_layers(&src) {
            Ok(l) if l.len() == 1 && l[0].bindings.len() == 1 => {
                let editing = self.form.editing;
                let name = std::mem::take(&mut self.form.name);
                let open = self.form.open;
                self.form = Form::from_binding(&l[0].bindings[0], editing);
                self.form.open = open;
                self.form.name = name;
                self.code_err = None;
            }
            Ok(_) => self.code_err = Some("escreva uma tecla e uma ação, ex.: c (push-msg app \"claude.exe\")".into()),
            Err(e) => self.code_err = Some(e),
        }
    }

    // ------------------------------------------------------------------ barra superior

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (t, ic, label) in [(Tab::Macros, icon::KEYBOARD, "Macros"), (Tab::Code, icon::CODE, "Código (.kbd)"), (Tab::Remap, icon::SWAP, "Troca de teclas"), (Tab::Features, icon::CUBE, "Recursos"), (Tab::Help, icon::HELP, "Ajuda")] {
                let look = if self.tab == t { Look::Primary } else { Look::Ghost };
                if th::button(ui, look, ic, label).clicked() {
                    self.tab = t;
                }
            }
            ui.add_space(8.0);
            th::vsep(ui, 28.0);
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // estado do salvamento (mais à direita). Macros salva sozinho; Código tem o botão Salvar.
                let dirty = self.dirty();
                match self.tab {
                    Tab::Macros => {
                        let j = th::icon_job(icon::CHECK, "Salvo automaticamente", th::semibold(14.5), th::GREEN, th::GREEN);
                        th::pill(ui, th::GREEN, th::GREEN_BG, j).on_hover_text("Cada mudança feita aqui já é gravada. Ctrl+Z desfaz a última.");
                    }
                    Tab::Code if dirty => {
                        let j = th::icon_job("●", "Não salvo", th::semibold(14.5), th::YELLOW, th::YELLOW);
                        th::pill(ui, th::YELLOW, th::YELLOW_BG, j).on_hover_text("Clique em Salvar (ou Ctrl+S) para gravar");
                    }
                    _ => {}
                }
                if self.daemon_running {
                    if self.kb_active {
                        if th::button(ui, Look::Normal, icon::PAUSE, "Pausar").on_hover_text("Para o kanata — teclado volta ao normal").clicked() {
                            self.post_daemon(daemon::WM_KM_SET_PAUSED, 1);
                        }
                    } else if th::button(ui, Look::Normal, icon::PLAY, "Ativar teclado").on_hover_text("Inicia o kanata (atalhos passam a valer)").clicked() {
                        self.post_daemon(daemon::WM_KM_SET_PAUSED, 0);
                    }
                } else if th::button(ui, Look::Normal, icon::PLAY, "Iniciar motor").on_hover_text("Abre o KeyMacros na bandeja sem mexer no teclado").clicked() {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("--paused").arg("--cfg").arg(&self.cfg).spawn();
                    }
                }
                let mut on = self.autostart;
                if th::checkbox(ui, &mut on).changed() {
                    match daemon::set_autostart(on, &self.cfg) {
                        Ok(()) => self.autostart = on,
                        Err(e) => self.status = Some((false, format!("Não consegui mudar o início com o Windows: {e}"))),
                    }
                }
                ui.label(RichText::new("Iniciar com o Windows").color(th::WEAK))
                    .on_hover_text("Abre o KeyMacros sozinho ao ligar o computador (recompila se o código mudou)");
                let (fg, bg, main, extra) = match (self.daemon_running, self.kb_active) {
                    (true, true) => (th::GREEN, th::GREEN_BG, "teclado: ATIVO", " (atalhos valendo)"),
                    (true, false) => (th::YELLOW, th::YELLOW_BG, "teclado: pausado", " (normal)"),
                    _ => (th::WEAK, th::CARD, "motor parado", ""),
                };
                let mut j = th::icon_job(icon::KEYBOARD, main, th::semibold(15.0), Color32::WHITE, fg);
                j.append(extra, 0.0, egui::TextFormat::simple(th::regular(14.5), fg));
                th::pill(ui, fg, bg, j);
            });
        });
        ui.add_space(6.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment([pos2(r.min.x - 16.0, r.min.y), pos2(r.max.x + 16.0, r.min.y)], Stroke::new(1.0, th::BORDER));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let j = th::icon_job(icon::WARNING, "Pânico: Ctrl (esq) + Espaço + Esc", th::regular(14.0), th::RED, th::RED);
                th::pill(ui, th::RED, th::RED_BG, j).on_hover_text("Segure os três juntos para fechar o kanata na hora — o teclado volta ao normal.");
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new(icon::FOLDER).size(17.0).color(th::WEAK));
                    ui.label(RichText::new(self.cfg.display().to_string()).color(th::WEAK));
                    ui.add_space(20.0);
                    let st = match &self.status {
                        Some((true, s)) => Some((icon::CHECK, th::GREEN, s.clone())),
                        Some((false, s)) => Some((icon::WARNING, th::RED, s.clone())),
                        None if self.dirty() => Some(("●", th::YELLOW, "Alterações não salvas — Ctrl+S para salvar".to_string())),
                        None => None,
                    };
                    if let Some((ic, col, s)) = st {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        if ic == icon::CHECK {
                            ui.painter().circle_filled(r.center(), 10.0, Color32::from_rgb(34, 197, 94));
                            ui.painter().text(r.center() + vec2(0.0, 1.0), Align2::CENTER_CENTER, ic, th::regular(11.0), Color32::from_rgb(8, 30, 16));
                        } else {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, ic, th::regular(15.0), col);
                        }
                        let first = s.lines().next().unwrap_or("").to_string();
                        ui.add(egui::Label::new(RichText::new(first).color(col)).truncate()).on_hover_text(s);
                    }
                });
            });
        });
    }

    // ------------------------------------------------------------------ aba Macros

    /// Uma linha do seletor de camadas: teclas da líder + "+ tecla" + nome.
    fn paint_layer_row(&self, ui: &Ui, rect: Rect, name: &str) {
        let caps = trigger_caps(&self.triggers, name, 0);
        let y = rect.center().y;
        let mut x = rect.min.x;
        if caps.is_empty() {
            let g = ui.painter().layout_no_wrap("sem tecla líder".into(), th::regular(14.5), th::YELLOW);
            let w = g.size().x;
            ui.painter().galley(pos2(x, y - g.size().y / 2.0), g, th::YELLOW);
            x += w;
        } else {
            x = paint_caps(ui, x, y, &caps, 28.0);
            let g = ui.painter().layout_no_wrap("+ tecla".into(), th::regular(14.5), th::WEAK);
            let w = g.size().x;
            ui.painter().galley(pos2(x + 8.0, y - g.size().y / 2.0), g, th::WEAK);
            x += 8.0 + w;
        }
        // nome gerado das teclas (km-r-t) não é mostrado: as teclas já dizem tudo
        let raw: Vec<String> = match self.triggers.get(name) {
            Some(Trigger::Key(k)) => vec![k.clone()],
            Some(Trigger::Chord(ks)) => ks.clone(),
            _ => vec![],
        };
        let short = name.trim_start_matches(kbd::MANAGED_PREFIX);
        let bad = self.layers.iter().find(|l| l.name == name).is_some_and(layer_broken);
        let room = if bad { 22.0 } else { 0.0 };
        if raw.is_empty() || (short != raw.join("-") && short != raw.join("")) {
            let col = if bad { th::RED } else { th::TEXT };
            th::text_trunc(ui, pos2(x + 14.0, y - 10.0), &layer_title(name), th::semibold(15.0), col, rect.max.x - x - 14.0 - room);
        }
        if bad {
            ui.painter().circle_filled(pos2(rect.max.x - 8.0, y), 5.0, th::RED);
        }
    }

    fn layer_picker(&mut self, ui: &mut Ui) {
        let w = 400.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, th::BTN_H + 8.0), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let stroke = if open { th::ACCENT_HI } else if resp.hovered() { th::BORDER_HI } else { th::BORDER };
        ui.painter().rect(rect, 8, if resp.hovered() { th::CARD_HOVER } else { th::CARD }, Stroke::new(1.0, stroke), StrokeKind::Inside);
        let sel = self.sel_layer.clone();
        self.paint_layer_row(ui, Rect::from_min_max(rect.min + vec2(12.0, 0.0), rect.max - vec2(36.0, 0.0)), &sel);
        let c = if open { th::ACCENT_HI } else { th::WEAK };
        ui.painter().text(pos2(rect.max.x - 20.0, rect.center().y), Align2::CENTER_CENTER, icon::CHEVRON, th::regular(13.0), c);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let mut pick = None;
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(w);
            ui.spacing_mut().item_spacing.y = 2.0;
            for l in &self.layers {
                let (r, rr) = ui.allocate_exact_size(vec2(w - 16.0, 46.0), Sense::click());
                if l.name == self.sel_layer {
                    ui.painter().rect_filled(r, 6, th::CARD_SEL);
                } else if rr.hovered() {
                    ui.painter().rect_filled(r, 6, th::CARD_HOVER);
                }
                self.paint_layer_row(ui, r.shrink2(vec2(10.0, 0.0)), &l.name);
                if rr.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick = Some(l.name.clone());
                    ui.close();
                }
            }
        });
        if let Some(n) = pick {
            self.sel_layer = n;
        }
    }

    fn new_layer_popup(&mut self, ui: &mut Ui) {
        let r = th::button(ui, Look::Normal, icon::ADD, "Camada");
        // com a janela de captura aberta, clicar nela não pode fechar este popup
        let close = if self.new_layer_cap { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClickOutside };
        egui::Popup::menu(&r).close_behavior(close).show(|ui| {
            ui.set_min_width(340.0);
            ui.label(RichText::new("Nova camada").font(th::semibold(15.0)));
            ui.add_space(4.0);
            ui.label(RichText::new("Tecla líder (uma tecla, ou várias apertadas juntas)").color(th::WEAK));
            let keys = trigger::parse_keys(&self.new_layer_keys);
            let (rr, _) = ui.allocate_exact_size(vec2(310.0, 46.0), Sense::hover());
            if keys.is_empty() {
                ui.painter().text(rr.left_center(), Align2::LEFT_CENTER, "nenhuma tecla ainda", th::regular(13.0), th::WEAK);
            } else {
                let caps: Vec<String> = keys.iter().map(|k| key_label(k)).collect();
                paint_caps(ui, rr.min.x, rr.center().y, &caps, 38.0);
            }
            if th::button(ui, Look::Normal, icon::KEYBOARD, "Capturar tecla…").on_hover_text("Abre uma janela: aperte a tecla (ou as teclas juntas)").clicked() {
                self.new_layer_cap = true;
                self.new_layer_held.clear();
                self.new_layer_got.clear();
            }
            ui.add_space(4.0);
            field(ui, &mut self.new_layer_keys, "ou digite: r t   ou   f14", true, 310.0);
            ui.label(RichText::new("Nome (opcional)").color(th::WEAK));
            field(ui, &mut self.new_layer, "ex.: premiere", false, 310.0);
            ui.add_space(4.0);
            if th::button(ui, Look::Primary, icon::ADD, "Criar camada").clicked() {
                let slug = |t: &str| t.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string();
                let base = if self.new_layer.trim().is_empty() {
                    keys.iter().map(|k| slug(k)).collect::<Vec<_>>().join("-")
                } else {
                    slug(self.new_layer.trim().trim_start_matches(kbd::MANAGED_PREFIX))
                };
                let name = format!("{}{}", kbd::MANAGED_PREFIX, base);
                match trigger::create_layer(&self.text, &name, &keys).and_then(|t| self.commit(t)) {
                    Ok(()) => {
                        self.sel_layer = name;
                        self.new_layer.clear();
                        self.new_layer_keys.clear();
                        self.load_form(Form::default());
                        self.lead = LeadEdit::default();
                        self.status = Some((true, "Camada criada e salva automaticamente — adicione os atalhos.".into()));
                    }
                    Err(e) => self.status = Some((false, e)),
                }
                ui.close();
            }
        });
    }

    /// Modal que captura a tecla da nova camada. Teclas apertadas juntas viram um acorde
    /// (ex.: R e T juntas = "r t"; Tab = "tab"). Esc cancela.
    fn new_layer_capture(&mut self, ctx: &egui::Context) {
        if !self.new_layer_cap {
            return;
        }
        let evs: Vec<(String, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed, repeat: false, .. } if *key != egui::Key::Escape => key_name(*key).map(|n| (n, *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (k, pressed) in evs {
            if pressed {
                // nenhuma tecla segurada: começou uma combinação nova, a anterior sai
                if self.new_layer_held.is_empty() {
                    self.new_layer_got.clear();
                }
                if !self.new_layer_held.contains(&k) {
                    self.new_layer_held.push(k.clone());
                }
                if !self.new_layer_got.contains(&k) {
                    self.new_layer_got.push(k);
                }
            } else {
                self.new_layer_held.retain(|x| *x != k);
            }
        }

        let cap: Vec<String> = self.new_layer_got.iter().map(|k| key_label(k)).collect();
        let mut done: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("new-layer-capture")).frame(th::section()).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Aperte a tecla da camada").font(th::semibold(17.0)));
                ui.add_space(4.0);
                ui.label(RichText::new("Teclas juntas valem como uma. Se errar, aperte outra.").color(th::WEAK));
                ui.add_space(12.0);
                if cap.is_empty() {
                    ui.label(RichText::new("…").size(34.0).color(th::WEAK));
                } else {
                    let (r, _) = ui.allocate_exact_size(vec2(caps_width(&cap, 56.0), 70.0), Sense::hover());
                    paint_caps(ui, r.min.x, r.center().y, &cap, 56.0);
                }
                ui.add_space(14.0);
            });
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if ui.add_enabled_ui(!cap.is_empty(), |ui| th::button(ui, Look::Primary, icon::CHECK, "Aceitar")).inner.clicked() {
                        done = Some(true);
                    }
                    if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                        done = Some(false);
                    }
                });
            });
        });
        if m.should_close() && done.is_none() {
            done = Some(false);
        }
        if let Some(ok) = done {
            if ok {
                self.new_layer_keys = self.new_layer_got.join(" ");
            }
            self.new_layer_cap = false;
            self.new_layer_held.clear();
            self.new_layer_got.clear();
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let before = self.sel_layer.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(RichText::new("Camada:").font(th::semibold(15.5)));
            self.layer_picker(ui);
            self.new_layer_popup(ui);
            self.new_layer_capture(ui.ctx());
            self.move_conflict_modal(ui.ctx());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(format!("{}   Buscar...", icon::SEARCH))
                        .desired_width(260.0)
                        .margin(Margin::symmetric(12, 8)),
                );
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                for (v, ic, tip) in [(View::List, icon::LIST, "Visualização em lista"), (View::Keyboard, icon::KEYBOARD, "Visualização no teclado"), (View::Grid, icon::GRID, "Visualização em grade")] {
                    if th::icon_button(ui, self.view == v, ic, tip).clicked() {
                        self.view = v;
                    }
                }
            });
        });
        if self.sel_layer != before {
            self.load_form(Form::default());
            self.lead = LeadEdit::default();
        }
    }

    /// Primeira linha da camada: a tecla líder (e como trocar).
    fn leader_row(&mut self, ui: &mut Ui) {
        let name = self.sel_layer.clone();
        let caps = trigger_caps(&self.triggers, &name, 0);
        let mut apply: Option<Vec<String>> = None;
        let mut delete = false;
        egui::Frame::new().fill(th::CARD).stroke(Stroke::new(1.0, th::BORDER)).corner_radius(12).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let shown: Vec<String> = if caps.is_empty() { vec!["?".into()] } else { caps.clone() };
                let (r, _) = ui.allocate_exact_size(vec2(caps_width(&shown, 38.0), 42.0), Sense::hover());
                paint_caps(ui, r.min.x, r.center().y, &shown, 38.0);
                ui.add_space(6.0);
                th::vsep(ui, 34.0);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new("Tecla líder").font(th::semibold(14.5)));
                    let col = if caps.is_empty() { th::YELLOW } else { th::WEAK };
                    ui.add(egui::Label::new(RichText::new(trigger_hint(&self.triggers, &name)).size(13.0).color(col)).truncate());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if self.lead.confirm_delete {
                        if th::button(ui, Look::Tint(th::RED, th::RED_BG), icon::DELETE, "Confirmar: apagar camada").clicked() {
                            delete = true;
                        }
                        if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                            self.lead.confirm_delete = false;
                        }
                    } else if th::icon_button(ui, false, icon::DELETE, "Apagar camada").clicked() {
                        self.lead.confirm_delete = true;
                    }
                    if !self.lead.open && !self.lead.confirm_delete && th::button(ui, Look::Normal, icon::KEYBOARD, "Trocar tecla líder").clicked() {
                        let cur = match self.triggers.get(&name) {
                            Some(Trigger::Key(k)) => k.clone(),
                            Some(Trigger::Chord(ks)) => ks.join(" "),
                            Some(Trigger::Inside { key, .. }) => key.clone(),
                            _ => String::new(),
                        };
                        self.lead = LeadEdit { open: true, keys: cur, ..LeadEdit::default() };
                    }
                });
            });
            if !self.lead.open {
                return;
            }
            ui.add_space(6.0);
            let r = ui.available_rect_before_wrap();
            ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, th::BORDER));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let keys = trigger::parse_keys(&self.lead.keys);
                let shown: Vec<String> = if self.lead.capturing {
                    if self.lead.got.is_empty() { vec!["…".into()] } else { self.lead.got.iter().map(|k| key_label(k)).collect() }
                } else if keys.is_empty() {
                    vec!["?".into()]
                } else {
                    keys.iter().map(|k| key_label(k)).collect()
                };
                let (r, resp) = ui.allocate_exact_size(vec2(caps_width(&shown, 38.0).max(38.0), 44.0), Sense::click());
                if self.lead.capturing {
                    ui.painter().rect_stroke(r.expand(4.0), 10, Stroke::new(1.5, th::ACCENT_HI), StrokeKind::Inside);
                }
                paint_caps(ui, r.min.x, r.center().y, &shown, 38.0);
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Clique e aperte a tecla (ou as teclas juntas)");
                if resp.clicked() {
                    self.lead.capturing = !self.lead.capturing;
                    self.lead.held.clear();
                    self.lead.got.clear();
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let hint = if self.lead.capturing { "aperte a tecla (ou as teclas juntas)…" } else { "ex.: r t   ou   f13" };
                    field(ui, &mut self.lead.keys, hint, true, 240.0);
                    let tip = match (keys.len(), self.triggers.get(&name)) {
                        (0, _) => "Clique nas teclas ao lado e aperte, ou digite os nomes.".to_string(),
                        (1, Some(Trigger::Inside { layer, .. })) => format!("Troca a tecla dentro da camada {}.", layer_title(layer)),
                        (1, _) => "Uma tecla: segure ela e aperte o atalho. Tocar sozinha continua normal.".to_string(),
                        _ => "Várias teclas: aperte juntas e depois o atalho. Uma depois da outra, sai normal.".to_string(),
                    };
                    ui.label(RichText::new(tip).small().color(th::WEAK));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if ui.add_enabled_ui(!keys.is_empty(), |ui| th::button(ui, Look::Primary, icon::CHECK, "Aplicar")).inner.clicked() {
                        apply = Some(keys.clone());
                    }
                    if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                        self.lead = LeadEdit::default();
                    }
                });
            });
            if self.lead.capturing {
                let evs: Vec<(String, bool)> = ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Key { key, pressed, repeat: false, .. } => key_name(*key).map(|n| (n, *pressed)),
                            _ => None,
                        })
                        .collect()
                });
                for (k, pressed) in evs {
                    if pressed {
                        if !self.lead.held.contains(&k) {
                            self.lead.held.push(k.clone());
                        }
                        if !self.lead.got.contains(&k) {
                            self.lead.got.push(k);
                        }
                    } else {
                        self.lead.held.retain(|x| *x != k);
                        if self.lead.held.is_empty() && !self.lead.got.is_empty() {
                            self.lead.keys = self.lead.got.join(" ");
                            self.lead.capturing = false;
                        }
                    }
                }
            }
        });
        if delete {
            match trigger::delete_layer(&self.text, &name).and_then(|t| self.commit(t)) {
                Ok(()) => {
                    self.sel_layer = self.layers.first().map(|l| l.name.clone()).unwrap_or_default();
                    self.lead = LeadEdit::default();
                    self.load_form(Form::default());
                    self.status = Some((true, "Camada apagada e salva automaticamente.".into()));
                }
                Err(e) => {
                    self.lead.confirm_delete = false;
                    self.status = Some((false, e));
                }
            }
        }
        if let Some(keys) = apply {
            match trigger::set(&self.text, &name, &keys).and_then(|t| self.commit(t)) {
                Ok(()) => {
                    self.lead = LeadEdit::default();
                    self.status = Some((true, "Tecla líder trocada e salva automaticamente.".into()));
                }
                Err(e) => self.status = Some((false, e)),
            }
        }
    }

    fn binding_items(&mut self, ctx: &egui::Context, layer: &Layer) -> Vec<Item> {
        let q = self.search.trim().to_lowercase();
        let mut out = vec![];
        for (i, b) in layer.bindings.iter().enumerate() {
            // "outras teclas = nada" fica no arquivo, mas não aparece
            if b.key == "___" {
                continue;
            }
            let other = false;
            let mut rows = vec![];
            let hold_only = matches!(b.tap, Act::Nothing) && b.hold.is_some();
            if !hold_only {
                let (title, ico) = act_view(&mut self.icons, ctx, &self.windows, &b.tap, other);
                rows.push(Row { title, sub: "tocar".into(), ico });
            }
            if let Some((ms, a)) = &b.hold {
                let (title, ico) = act_view(&mut self.icons, ctx, &self.windows, a, other);
                rows.push(Row { title, sub: format!("segurar ({})", secs_label(*ms)), ico });
            }
            if !b.name.is_empty() {
                if let Some(r) = rows.first_mut() {
                    r.sub = format!("{} · {}", r.sub, r.title);
                    r.title = b.name.clone();
                }
            }
            let find = format!("{} {} {} {} {}", b.name, b.key, key_label(&b.key), rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>().join(" "), b.render().unwrap_or_default()).to_lowercase();
            if q.is_empty() || find.contains(&q) {
                out.push(Item { idx: i, caps: vec![key_label(&b.key)], rows, broken: binding_broken(b) });
            }
        }
        out
    }

    fn keyboard_view(&mut self, ui: &mut Ui, layer: &Layer, avail_h: f32, compact: bool, hovered: &mut Option<usize>) -> Option<Pick> {
        let targets = self.move_targets();
        let items = self.binding_items(ui.ctx(), layer);
        let by_key: HashMap<&str, &Item> = items.iter().map(|it| (layer.bindings[it.idx].key.as_str(), it)).collect();
        let rows = kb_layout();
        if compact {
            ui.spacing_mut().item_spacing.y = 0.0;
        }
        // cabe inteiro: largura (15 unidades) e altura (linhas + espaços)
        let by_w = (ui.available_width() - 4.0) / 15.0;
        let by_h = (avail_h - 24.0) / rows.len() as f32;
        let unit = by_w.min(by_h).clamp(if compact { 18.0 } else { 24.0 }, 90.0);
        let gap = 6.0;
        let mut pick = None;
        let mut shown: Vec<&str> = vec![];
        for (ri, row) in rows.iter().enumerate() {
            if ri == 1 || ri == rows.len() - 1 {
                ui.add_space(8.0);
            }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), unit), Sense::hover());
            let mut x = rect.min.x;
            for (k, w) in row {
                let kr = Rect::from_min_size(pos2(x, rect.min.y), vec2(unit * w - gap, unit - gap));
                x += unit * w;
                if k.is_empty() {
                    continue;
                }
                let it = by_key.get(k.as_str()).copied();
                if it.is_some() {
                    shown.push(k);
                }
                let resp = ui.interact(kr, ui.scope_id().with(("kb", k)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.hovered() {
                    if let Some(it) = it {
                        *hovered = Some(it.idx);
                    }
                }
                let linked = it.is_some_and(|it| self.hl == Some(it.idx));
                let selected = it.is_some_and(|it| self.form.editing == Some(it.idx)) || linked;
                let (fill, stroke) = match (it.is_some(), selected, resp.hovered()) {
                    (_, true, _) => (th::CARD_SEL, Stroke::new(2.0, th::ACCENT_HI)),
                    (true, _, h) => (if h { th::CARD_HOVER } else { Color32::from_rgb(24, 38, 62) }, Stroke::new(1.0, if h { th::ACCENT_HI } else { th::ACCENT })),
                    (false, _, h) => (if h { th::CARD_HOVER } else { th::CARD }, Stroke::new(1.0, if h { th::BORDER_HI } else { th::BORDER })),
                };
                let p = ui.painter();
                p.rect_filled(kr.translate(vec2(0.0, 2.0)), 8, Color32::from_rgb(5, 8, 13));
                p.rect(kr, 8, fill, stroke, StrokeKind::Inside);
                let col = if it.is_some() { th::TEXT } else { th::WEAK };
                let fs = (unit * 0.24).clamp(9.0, 15.0);
                p.text(kr.min + vec2(unit * 0.12, unit * 0.08), Align2::LEFT_TOP, key_label(k), th::semibold(fs), col);
                if let Some(it) = it {
                    let s = (unit * 0.42).min(kr.height() * 0.5);
                    let ir = Rect::from_center_size(pos2(kr.center().x, kr.max.y - s / 2.0 - 6.0), Vec2::splat(s));
                    if let Some(row) = it.rows.first() {
                        th::paint_ico(p, ir, &row.ico);
                    }
                    let tip = it.rows.iter().map(|r| format!("{} — {}", r.title, r.sub)).collect::<Vec<_>>().join("\n");
                    let idx = it.idx;
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        pick = Some(Pick::Edit(idx));
                    }
                    resp.context_menu(|ui| {
                        if let Some(p) = move_menu(ui, &targets, idx) {
                            pick = Some(p);
                        }
                        if ui.button("Excluir").clicked() {
                            pick = Some(Pick::Delete(idx));
                        }
                    });
                } else if resp.clicked() {
                    pick = Some(Pick::NewKey(k.clone()));
                }
            }
        }
        let rest: Vec<&Item> = items.iter().filter(|it| !shown.contains(&layer.bindings[it.idx].key.as_str())).collect();
        if !rest.is_empty() && !compact {
            ui.add_space(12.0);
            ui.label(RichText::new("Outras teclas desta camada").color(th::WEAK));
            ui.horizontal_wrapped(|ui| {
                for it in rest {
                    let t = format!("{}  →  {}", it.caps.join(" + "), it.rows.first().map(|r| r.title.as_str()).unwrap_or(""));
                    let look = if self.form.editing == Some(it.idx) { Look::Primary } else { Look::Normal };
                    if th::button(ui, look, "", &t).clicked() {
                        pick = Some(Pick::Edit(it.idx));
                    }
                }
            });
        }
        pick
    }

    fn list_view(&mut self, ui: &mut Ui, layer: &Layer) -> Option<Pick> {
        let items = self.binding_items(ui.ctx(), layer);
        let mut pick = None;
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 6.0;
        for it in &items {
            let b = &layer.bindings[it.idx];
            let tip = match &b.hold {
                Some((ms, a)) => format!("Tocar: {} — {}\nSegurar {}: {} — {}", b.tap.kind_label(), b.tap.summary(), secs_label(*ms), a.kind_label(), a.summary()),
                None => format!("Tocar: {} — {}", b.tap.kind_label(), b.tap.summary()),
            };
            let resp = th::card(ui, vec2(w, 46.0), self.form.editing == Some(it.idx)).on_hover_text(tip);
            let rect = resp.rect;
            if ui.is_rect_visible(rect) {
                let cap = Rect::from_min_size(pos2(rect.min.x + 14.0, rect.center().y - 16.0), vec2(48.0, 30.0));
                th::keycap_paint(ui, cap, &it.caps.join("+"), 14.0, false);
                let x0 = cap.max.x + 20.0;
                let col_w = (rect.max.x - 60.0 - x0) / 2.0;
                let b = &layer.bindings[it.idx];
                let hold_only = matches!(b.tap, Act::Nothing) && b.hold.is_some();
                // coluna 1 = tocar, coluna 2 = segurar
                let mut cols: Vec<Option<&Row>> = vec![None, None];
                let mut rows = it.rows.iter();
                if !hold_only {
                    cols[0] = rows.next();
                }
                if b.hold.is_some() {
                    cols[1] = rows.next();
                }
                for (ci, row) in cols.iter().enumerate() {
                    let x = x0 + col_w * ci as f32;
                    match row {
                        Some(row) => {
                            let ir = Rect::from_center_size(pos2(x + 13.0, rect.center().y), Vec2::splat(20.0));
                            th::paint_ico(ui.painter(), ir, &row.ico);
                            let tx = ir.max.x + 10.0;
                            th::text_trunc(ui, pos2(tx, rect.center().y - 16.0), &row.title, th::semibold(13.5), th::TEXT, col_w - 50.0);
                            th::text_trunc(ui, pos2(tx, rect.center().y + 1.0), &row.sub, th::regular(11.5), th::WEAK, col_w - 50.0);
                        }
                        None => {
                            let t = if ci == 0 { "tocar: nada" } else { "segurar: —" };
                            th::text_trunc(ui, pos2(x, rect.center().y - 9.0), t, th::regular(14.0), th::WEAK, col_w - 10.0);
                        }
                    }
                }
            }
            // lixeira à direita
            let del = Rect::from_center_size(pos2(rect.max.x - 32.0, rect.center().y), Vec2::splat(36.0));
            let dr = ui.interact(del, ui.scope_id().with(("del", it.idx)), Sense::click()).on_hover_text("Excluir");
            let col = if dr.hovered() { th::RED } else { th::WEAK };
            ui.painter().text(del.center(), Align2::CENTER_CENTER, icon::DELETE, th::regular(16.0), col);
            if dr.clicked() {
                pick = Some(Pick::Delete(it.idx));
            } else if resp.clicked() {
                pick = Some(Pick::Edit(it.idx));
            }
        }
        let resp = th::card(ui, vec2(w, 40.0), self.form.editing.is_none());
        let j = th::icon_job(icon::ADD, "Novo atalho", th::regular(15.0), th::WEAK, th::WEAK);
        let g = ui.ctx().fonts_mut(|f| f.layout_job(j));
        ui.painter().galley(resp.rect.center() - g.size() / 2.0, g, th::WEAK);
        if resp.clicked() {
            pick = Some(Pick::New);
        }
        pick
    }

    fn macros_tab(&mut self, ui: &mut Ui) {
        if let Some(e) = &self.parse_err {
            th::section().show(ui, |ui| {
                ui.colored_label(th::RED, format!("O arquivo tem erro de sintaxe, corrija na aba Código:\n{e}"));
            });
            return;
        }
        self.toolbar(ui);
        ui.add_space(8.0);

        let Some(layer) = self.layers.iter().find(|l| l.name == self.sel_layer).cloned() else {
            ui.label("Nenhuma camada km-* encontrada. Crie uma acima.");
            return;
        };

        self.capture_popup(ui);
        if self.form.open {
            // editar/criar atalho num popup no meio da tela
            let sw = ui.ctx().content_rect().width();
            let w = (sw * 0.85).clamp(700.0, 1150.0);
            let m = egui::Modal::new(egui::Id::unique("edit-modal")).frame(egui::Frame::NONE).show(ui.ctx(), |ui| {
                ui.set_width(w);
                self.edit_panel(ui, &layer);
            });
            // clicar fora / Esc fecha (mas não enquanto um popup de captura está aberto)
            
            if (ui.input(|i| i.key_pressed(egui::Key::Escape)) || m.backdrop_response.clicked()) && !self.form.capturing && !self.form.tap.rec && !self.form.hold.rec {
                self.load_form(Form::default());
            }
        }

        self.leader_row(ui);
        ui.add_space(8.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, th::BORDER_HI));
        ui.add_space(4.0);
        ui.label(RichText::new("Atalhos desta camada").size(12.5).color(th::WEAK));

        let kb_h = ui.available_height();
        let mut hovered: Option<usize> = None;
        let pick = if self.view == View::Grid {
            let full = ui.available_rect_before_wrap();
            let wide = full.width() >= 1300.0;
            let mut top_pick = None;
            let (cards_rect, kb_rect) = if wide {
                // tela larga: cards à esquerda, teclado à direita (40%)
                let kw = (full.width() * 0.4).floor();
                let kr = Rect::from_min_max(pos2(full.max.x - kw, full.min.y), full.max);
                (Rect::from_min_max(full.min, pos2(kr.min.x - 20.0, full.max.y)), kr)
            } else {
                // tela estreita / vertical: teclado em cima
                let kh = (full.height() * 0.42).floor();
                let kr = Rect::from_min_max(full.min + vec2(0.0, 4.0), pos2(full.max.x, full.min.y + kh));
                (Rect::from_min_max(pos2(full.min.x, kr.max.y + 10.0), full.max), kr)
            };
            ui.scope_builder(egui::UiBuilder::new().max_rect(kb_rect), |ui| {
                if wide {
                    ui.add_space(4.0);
                }
                top_pick = self.keyboard_view(ui, &layer, kb_rect.height(), true, &mut hovered);
            });
            let hl = hovered.or(self.hl);
            let targets = self.move_targets();
            let p = ui
                .scope_builder(egui::UiBuilder::new().max_rect(cards_rect), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("cards")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let items = self.binding_items(ui.ctx(), &layer);
                            cards_grid(ui, &items, &targets, self.form.editing, self.form.editing.is_none() && (self.form.open || self.form.capturing), "Novo atalho", hl, &mut hovered)
                        })
                        .inner
                })
                .inner;
            ui.allocate_rect(full, Sense::hover());
            p.or(top_pick)
        } else {
            egui::ScrollArea::vertical()
                .id_salt("cards")
                .auto_shrink([false, false])
                .show(ui, |ui| match self.view {
                    View::Keyboard => self.keyboard_view(ui, &layer, kb_h, false, &mut hovered),
                    _ => self.list_view(ui, &layer),
                })
                .inner
        };
        if hovered != self.hl {
            self.hl = hovered;
            ui.ctx().request_repaint();
        }
        match pick {
            Some(Pick::Edit(i)) => self.load_form(Form { open: true, ..Form::from_binding(&layer.bindings[i], Some(i)) }),
            Some(Pick::Delete(i)) => {
                let mut v = layer.bindings.clone();
                v.remove(i);
                match self.set_layer_bindings(v) {
                    Ok(()) => self.status = Some((true, "Atalho excluído e salvo automaticamente.".into())),
                    Err(e) => self.status = Some((false, e)),
                }
                self.load_form(Form::default());
            }
            Some(Pick::Move(i, to)) => match self.move_binding(i, &to, false) {
                Ok(true) => {}
                Ok(false) => self.pending_move = Some((i, to)),
                Err(e) => self.status = Some((false, e)),
            },
            Some(Pick::New) => self.load_form(Form { capturing: true, ..Form::default() }),
            Some(Pick::NewKey(k)) => self.load_form(Form { key: k, open: true, ..Form::default() }),
            None => {}
        }
    }

    /// Popup "Aperte a tecla do atalho" (aparece mesmo com o painel fechado).
    fn capture_popup(&mut self, ui: &mut Ui) {
        if self.form.capturing {
            let k = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } if *key != egui::Key::Escape => Some(*key),
                    _ => None,
                })
            });
            if let Some(n) = k.and_then(key_name) {
                self.form.cap_key = n;
            }
            // popup: deixa claro que está esperando a tecla
            let mut done: Option<bool> = None;
            let cap = self.form.cap_key.clone();
            let m = egui::Modal::new(egui::Id::unique("capture-key")).frame(th::section()).show(ui.ctx(), |ui| {
                ui.set_width(340.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Aperte a tecla do atalho").font(th::semibold(17.0)));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Se errar, é só apertar outra.").color(th::WEAK));
                    ui.add_space(12.0);
                    let label = if cap.is_empty() { "…".to_string() } else { key_label(&cap) };
                    th::keycap(ui, vec2(96.0, 80.0), &label, 34.0, Sense::hover());
                    ui.add_space(14.0);
                });
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if ui.add_enabled_ui(!cap.is_empty(), |ui| th::button(ui, Look::Primary, icon::CHECK, "Aceitar")).inner.clicked() {
                            done = Some(true);
                        }
                        if th::button(ui, Look::Normal, "", "Cancelar").clicked() {
                            done = Some(false);
                        }
                    });
                });
            });
            if m.should_close() && done.is_none() {
                done = Some(false);
            }
            if let Some(ok) = done {
                if ok {
                    self.form.key = std::mem::take(&mut self.form.cap_key);
                    self.form.open = true;
                }
                self.form.cap_key.clear();
                self.form.capturing = false;
            }
        }
    }

    fn edit_panel(&mut self, ui: &mut Ui, layer: &Layer) {
        th::section().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::KEYBOARD).size(20.0).color(th::TEXT));
                ui.label(RichText::new(if self.form.editing.is_some() { "Editar atalho" } else { "Novo atalho" }).font(th::semibold(19.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if th::button(ui, Look::Normal, "", "Fechar").on_hover_text("Fecha o painel sem aplicar").clicked() {
                        self.load_form(Form::default());
                    }
                    ui.add_space(6.0);
                    if mode_toggle(ui, &mut self.code_mode) && self.code_mode {
                        self.code_buf = self.form.binding().render().unwrap_or_default();
                        self.code_err = None;
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(84.0);
                    let label = if self.form.capturing { "…".to_string() } else { key_label(self.form.key.trim()) };
                    let r = th::keycap(ui, vec2(84.0, 64.0), &label, 30.0, Sense::click()).on_hover_text("Clique e aperte a tecla para capturar");
                    if r.clicked() {
                        self.form.capturing = !self.form.capturing;
                    }
                    ui.add_space(2.0);
                    let hint = if self.form.capturing { "aperte uma tecla…" } else { "tecla: c, f5, up…" };
                    field(ui, &mut self.form.key, hint, true, 84.0);
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    if self.code_mode {
                        ui.label(RichText::new("Código kanata deste atalho (tecla + ação). Ex.: c (tap-hold 0 300 (push-msg app \"claude.exe\") (push-msg run \"wt.exe\"))").color(th::WEAK));
                        let r = ui.add(egui::TextEdit::multiline(&mut self.code_buf).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                        if r.changed() {
                            self.sync_from_code();
                        }
                        if let Some(e) = &self.code_err {
                            ui.colored_label(th::RED, e);
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Nome").font(th::semibold(15.0)));
                            let w = (ui.available_width() - 8.0).max(120.0);
                            field(ui, &mut self.form.name, "opcional — ex.: Gravar tela", false, w);
                        });
                        ui.add_space(4.0);
                        let windows = &mut self.windows;
                        th::row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            act_row(ui, "tap", true, |ui| row_head(ui, icon::TOUCH, "Ao tocar"), &mut self.form.tap, &mut self.more_tap, windows);
                        });
                        ui.add_space(2.0);
                        let enabled = self.form.hold_on;
                        let (hold_on, hold_secs) = (&mut self.form.hold_on, &mut self.form.hold_secs);
                        th::row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            act_row(
                                ui,
                                "hold",
                                enabled,
                                |ui| {
                                    ui.add_space(6.0);
                                    th::checkbox(ui, hold_on);
                                    ui.label(RichText::new("Ao segurar por").font(th::semibold(15.5)));
                                    ui.add_enabled(
                                        *hold_on,
                                        egui::DragValue::new(hold_secs)
                                            .range(0.05..=60.0)
                                            .speed(0.01)
                                            .custom_formatter(|v, _| format!("{v:.2} s").replace('.', ","))
                                            .custom_parser(|s| s.trim().trim_end_matches('s').trim().replace(',', ".").parse().ok()),
                                    )
                                    .on_hover_text("Arraste ou clique para digitar");
                                },
                                &mut self.form.hold,
                                &mut self.more_hold,
                                windows,
                            );
                        });
                    }
                });
            });
            ui.add_space(12.0);
            let b = self.form.binding();
            let rendered = if self.code_mode && self.code_err.is_some() {
                Err(self.code_err.clone().unwrap_or_default())
            } else if b.key.is_empty() {
                Err("escolha a tecla: clique no quadrado ao lado e aperte a tecla".into())
            } else {
                b.render()
            };
            let (apply, clear, del) = action_bar(ui, &rendered, self.form.editing.is_some(), "Adicionar atalho");
            if clear {
                self.load_form(Form { open: true, ..Form::default() });
            }
            if del {
                if let Some(i) = self.form.editing {
                    let mut v = layer.bindings.clone();
                    v.remove(i);
                    match self.set_layer_bindings(v) {
                        Ok(()) => self.status = Some((true, "Atalho excluído e salvo automaticamente.".into())),
                        Err(e) => self.status = Some((false, e)),
                    }
                    self.load_form(Form::default());
                }
            }
            if apply {
                let mut v = layer.bindings.clone();
                let dup = v.iter().position(|x| x.key == b.key);
                match (self.form.editing, dup) {
                    (Some(i), Some(d)) if d != i => {
                        self.status = Some((false, format!("A tecla {} já existe nesta camada.", b.key)));
                        return;
                    }
                    (Some(i), _) => v[i] = b.clone(),
                    (None, Some(d)) => v[d] = b.clone(),
                    (None, None) => {
                        // mantém o "___" (outras teclas) sempre no fim
                        let pos = v.iter().position(|x| x.key == "___").unwrap_or(v.len());
                        v.insert(pos, b.clone());
                    }
                }
                match self.set_layer_bindings(v) {
                    Ok(()) => {
                        self.status = Some((true, "Atalho aplicado e salvo automaticamente.".into()));
                        // fecha o painel; reabre só em "Novo atalho" ou num card
                        self.load_form(Form::default());
                    }
                    Err(e) => self.status = Some((false, e)),
                }
            }
        });
    }

    // ------------------------------------------------------------------ outras abas

    fn code_tab(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dirty = self.dirty();
            if th::button(ui, if dirty { Look::Primary } else { Look::Normal }, icon::SAVE, "Salvar").on_hover_text("Valida e salva (Ctrl+S)").clicked() && dirty {
                let _ = self.save();
            }
            if th::button(ui, Look::Normal, icon::CHECK, "Validar").clicked() {
                self.status = Some(match paths::check_text(&self.text, Some(&self.cfg)) {
                    Ok(()) => (true, "Config válida.".into()),
                    Err(e) => (false, e),
                });
            }
            if th::button(ui, Look::Normal, icon::REFRESH, "Descartar alterações").clicked() {
                if dirty {
                    let cur = std::mem::replace(&mut self.text, self.disk_text.clone());
                    self.push_undo(cur);
                }
                self.reparse();
            }
            ui.label(RichText::new("Linguagem do kanata. Comentários: ;;   Camadas km-* aparecem na aba Macros.").color(th::WEAK));
        });
        ui.add_space(8.0);
        th::section().inner_margin(Margin::same(4)).show(ui, |ui| {
            egui::ScrollArea::both().id_salt("code").auto_shrink([false, false]).show(ui, |ui| {
                let r = ui.add(egui::TextEdit::multiline(&mut self.text).code_editor().frame(egui::Frame::NONE).desired_rows(40).desired_width(f32::INFINITY).lock_focus(true));
                if r.changed() {
                    self.reparse();
                }
            });
        });
    }

    fn key_combo(ui: &mut Ui, id: &str, val: &mut u16, allow_off: bool) {
        egui::ComboBox::from_id_salt(id)
            .width(220.0)
            .icon(th::chevron)
            .selected_text(scancode::name(*val).0)
            .show_ui(ui, |ui| {
                for (sc, label, _) in scancode::KEYS {
                    if *sc == 0 && !allow_off {
                        continue;
                    }
                    ui.selectable_value(val, *sc, *label);
                }
            });
    }

    fn mode_toggle(ui: &mut Ui, reg: &mut bool) {
        for (v, ic, label, tip) in [
            (false, icon::KEYBOARD, "Pelo kanata", "Vale na hora ao salvar. Não precisa de admin nem reiniciar. Só funciona com o teclado ATIVO."),
            (true, icon::WINDOW, "Pelo registro", "Scancode Map do Windows (igual ao SharpKeys). Vale sempre, até com o kanata desligado e na tela de login. Pede admin e precisa reiniciar."),
        ] {
            let look = if *reg == v { Look::Primary } else { Look::Normal };
            if th::button(ui, look, ic, label).on_hover_text(tip).clicked() {
                *reg = v;
            }
        }
    }

    fn remap_tab(&mut self, ui: &mut Ui) {
        enum Do {
            Leader,
            Add,
            RemoveKanata(String),
            RemoveReg(u16),
            ApplyReg,
            ResetRegEdit,
            ClearReg,
            RestoreReg(PathBuf),
            LoadCfg(PathBuf),
            Reload,
        }
        let mut todo: Option<Do> = None;
        let cur_reg = self.current_reg();
        let kanata_swaps = remap::swaps(&self.text);
        let leader = remap::leader_key(&self.text);
        let base_pairs = remap::pairs(&self.text).unwrap_or_default();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some((ok, m)) = &self.status {
                let (fg, bg) = if *ok { (th::GREEN, th::GREEN_BG) } else { (th::RED, th::RED_BG) };
                egui::Frame::new().fill(bg).stroke(Stroke::new(1.0, fg)).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.colored_label(fg, m);
                });
                ui.add_space(10.0);
            }
            if self.remap.reg_pending {
                egui::Frame::new().fill(th::YELLOW_BG).stroke(Stroke::new(1.0, th::YELLOW)).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.colored_label(th::YELLOW, format!("{}  O registro foi alterado. As trocas do registro só valem depois de REINICIAR o PC (Reiniciar, não Desligar).", icon::WARNING));
                });
                ui.add_space(10.0);
            }

            // ---------------------------------------------------------- tecla líder
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("Tecla líder");
                ui.add_space(4.0);
                match &leader {
                    Some(k) => {
                        let sc = scancode::by_kanata(k);
                        let phys: Vec<&str> = scancode::KEYS.iter().filter(|x| x.0 != 0 && Some(scancode::received(&cur_reg, x.0)) == sc).map(|x| x.1).collect();
                        let via = match (sc, phys.first()) {
                            (Some(s), Some(p)) if scancode::name(s).0 != *p => format!("  (tecla física: {p} — o registro troca para {})", scancode::name(s).0),
                            _ => String::new(),
                        };
                        ui.label(RichText::new(format!("Hoje: {} — o kanata recebe `{k}`{via}", scancode::label_kanata(k))).font(th::semibold(15.0)));
                    }
                    None => {
                        ui.colored_label(th::YELLOW, "Nenhuma tecla da camada base está ligada a @lider.");
                    }
                }
                if !remap::has_leader_alias(&self.text) {
                    ui.colored_label(th::RED, "Não achei o alias `lider` (tap-hold ...) no defalias — a troca da líder fica desligada.");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Nova tecla líder:");
                    Self::key_combo(ui, "leader-phys", &mut self.remap.leader_phys, false);
                    ui.add_space(8.0);
                    Self::mode_toggle(ui, &mut self.remap.leader_reg);
                });
                let target = if self.remap.leader_reg {
                    None
                } else {
                    Some(scancode::name(scancode::received(&cur_reg, self.remap.leader_phys)).1)
                };
                ui.add_space(4.0);
                if self.remap.leader_reg {
                    ui.label(RichText::new("O registro troca essa tecla por uma F13–F24 livre e a config passa a usar essa F-tecla como líder. Pede admin e só vale depois de reiniciar.").color(th::WEAK));
                } else if let Some(t) = target.filter(|t| leader.as_deref() == Some(*t)) {
                    ui.label(RichText::new(format!("Essa tecla já é a líder (o kanata recebe `{t}`).")).color(th::WEAK));
                } else if let Some(t) = target {
                    ui.label(RichText::new(format!("O kanata passa a usar `{t}` como líder. Vale assim que salvar. Tocar a líder sozinha continua mandando a tecla normal.")).color(th::WEAK));
                    if let Some(p) = base_pairs.iter().find(|p| p.key == t && p.action != remap::LEADER_ALIAS) {
                        ui.colored_label(th::YELLOW, format!("Atenção: essa tecla hoje faz `{}` na camada base; isso será substituído.", p.action));
                    }
                }
                ui.add_space(6.0);
                ui.add_enabled_ui(remap::has_leader_alias(&self.text), |ui| {
                    if th::button(ui, Look::Primary, icon::CHECK, "Usar como tecla líder").clicked() {
                        todo = Some(Do::Leader);
                    }
                });
            });
            ui.add_space(12.0);

            // ---------------------------------------------------------- trocas
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("Trocar uma tecla por outra");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Apertar");
                    Self::key_combo(ui, "add-from", &mut self.remap.add_from, false);
                    ui.label("vira");
                    Self::key_combo(ui, "add-to", &mut self.remap.add_to, true);
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    Self::mode_toggle(ui, &mut self.remap.add_reg);
                    ui.add_space(8.0);
                    if th::button(ui, Look::Primary, icon::ADD, "Adicionar troca").clicked() {
                        todo = Some(Do::Add);
                    }
                });
                ui.label(RichText::new("No kanata, a tecla de origem é a que ele recebe — já trocada pelo registro, se houver troca lá também.").color(th::WEAK));
                ui.add_space(12.0);

                ui.label(RichText::new(format!("{}  Pelo kanata (config)", icon::KEYBOARD)).font(th::semibold(15.0)));
                match &kanata_swaps {
                    Err(e) => {
                        ui.colored_label(th::RED, e);
                    }
                    Ok(v) if v.is_empty() => {
                        ui.label(RichText::new("  nenhuma").color(th::WEAK));
                    }
                    Ok(v) => {
                        for (k, a) in v {
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(vec2(520.0, 24.0), Layout::left_to_right(Align::Center), |ui| ui.label(RichText::new(format!("  {} ({k})  →  {} ({a})", scancode::label_kanata(k), scancode::label_kanata(a))).monospace()));
                                if th::button(ui, Look::Ghost, icon::DELETE, "Remover").clicked() {
                                    todo = Some(Do::RemoveKanata(k.clone()));
                                }
                            });
                        }
                    }
                }
                ui.add_space(12.0);

                ui.label(RichText::new(format!("{}  Pelo registro (Scancode Map)", icon::WINDOW)).font(th::semibold(15.0)));
                if let Err(e) = &self.scancode {
                    ui.colored_label(th::RED, e);
                }
                if self.remap.reg_edit.is_empty() {
                    ui.label(RichText::new("  nenhuma (teclado normal do Windows)").color(th::WEAK));
                }
                for m in &self.remap.reg_edit {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(vec2(520.0, 24.0), Layout::left_to_right(Align::Center), |ui| ui.label(RichText::new(format!("  {}", scancode::describe(m))).monospace()));
                        if !cur_reg.contains(m) {
                            ui.colored_label(th::YELLOW, "(não aplicada)");
                        }
                        if th::button(ui, Look::Ghost, icon::DELETE, "Remover").clicked() {
                            todo = Some(Do::RemoveReg(m.from));
                        }
                    });
                }
                for m in cur_reg.iter().filter(|m| !self.remap.reg_edit.iter().any(|x| x.from == m.from)) {
                    ui.colored_label(th::YELLOW, format!("  {}  (será removida ao aplicar)", scancode::describe(m)));
                }
                ui.add_space(6.0);
                let changed = self.remap.reg_edit != cur_reg;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(changed, |ui| {
                        if th::button(ui, Look::Primary, icon::SAVE, "Aplicar no registro").on_hover_text("Faz backup do estado atual, pede admin (UAC) e grava. Vale depois de reiniciar.").clicked() {
                            todo = Some(Do::ApplyReg);
                        }
                        if th::button(ui, Look::Normal, icon::REFRESH, "Descartar mudanças").clicked() {
                            todo = Some(Do::ResetRegEdit);
                        }
                    });
                    ui.add_enabled_ui(!cur_reg.is_empty(), |ui| {
                        if th::button(ui, Look::Tint(th::RED, th::RED_BG), icon::BLOCKED, "Voltar ao normal (tirar tudo do registro)").on_hover_text("Remove o Scancode Map inteiro (com backup antes). Vale depois de reiniciar.").clicked() {
                            todo = Some(Do::ClearReg);
                        }
                    });
                    if th::button(ui, Look::Ghost, icon::REFRESH, "Reler registro").clicked() {
                        todo = Some(Do::Reload);
                    }
                });
            });
            ui.add_space(12.0);

            // ---------------------------------------------------------- backups
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.heading("Backups");
                    ui.add_space(8.0);
                    if th::button(ui, Look::Normal, icon::FOLDER, "Abrir pasta").clicked() {
                        let d = backup::dir(&self.cfg);
                        let _ = std::fs::create_dir_all(&d);
                        let _ = std::process::Command::new("explorer.exe").arg(d).spawn();
                    }
                });
                ui.label(RichText::new("Toda mudança guarda antes uma cópia do estado anterior. Nenhum backup é apagado pelo app. Os .reg também podem ser aplicados com duplo clique.").color(th::WEAK));
                ui.add_space(6.0);
                let regs = backup::list(&self.cfg, backup::KIND_REG);
                let cfgs = backup::list(&self.cfg, backup::KIND_CFG);
                egui::CollapsingHeader::new(format!("Registro ({})", regs.len())).id_salt("bk-reg").show(ui, |ui| {
                    for b in regs.iter().take(30) {
                        ui.horizontal(|ui| {
                            ui.monospace(&b.when);
                            ui.label(&b.name);
                            if th::button(ui, Look::Ghost, icon::REFRESH, "Restaurar").on_hover_text("Aplica este .reg no registro (backup do atual antes, pede admin, reiniciar).").clicked() {
                                todo = Some(Do::RestoreReg(b.path.clone()));
                            }
                        });
                    }
                });
                egui::CollapsingHeader::new(format!("Config .kbd ({})", cfgs.len())).id_salt("bk-cfg").show(ui, |ui| {
                    for b in cfgs.iter().take(30) {
                        ui.horizontal(|ui| {
                            ui.monospace(&b.when);
                            ui.label(&b.name);
                            if th::button(ui, Look::Ghost, icon::REFRESH, "Carregar").on_hover_text("Coloca este backup no editor. Clique em Salvar para valer (o atual vira backup).").clicked() {
                                todo = Some(Do::LoadCfg(b.path.clone()));
                            }
                        });
                    }
                });
            });
        });

        let Some(todo) = todo else { return };
        match todo {
            Do::Leader => {
                let phys = self.remap.leader_phys;
                if self.remap.leader_reg {
                    let mut maps = self.remap.reg_edit.clone();
                    maps.retain(|m| m.from != phys);
                    let free = (0x0064u16..=0x006E).chain([0x0076]).find(|f| !maps.iter().any(|m| m.to == *f || m.from == *f));
                    let Some(f) = free else {
                        self.status = Some((false, "Não sobrou nenhuma F13–F24 livre no registro.".into()));
                        return;
                    };
                    maps.push(scancode::Mapping { from: phys, to: f });
                    let new = remap::set_leader(&self.text, scancode::name(f).1);
                    if self.edit_config_and_save(new) {
                        self.apply_registry(maps);
                    }
                } else {
                    let t = scancode::name(scancode::received(&cur_reg, phys)).1;
                    if t == "XX" || t == "?" {
                        self.status = Some((false, "Essa tecla está desligada no registro; o kanata não a recebe.".into()));
                        return;
                    }
                    let new = remap::set_leader(&self.text, t);
                    if self.edit_config_and_save(new) {
                        self.status = Some((true, format!("Tecla líder agora é `{t}` (backup da config anterior guardado).")));
                    }
                }
            }
            Do::Add => {
                let (from, to) = (self.remap.add_from, self.remap.add_to);
                if from == to {
                    self.status = Some((false, "Escolha teclas diferentes.".into()));
                    return;
                }
                if self.remap.add_reg {
                    self.remap.reg_edit.retain(|m| m.from != from);
                    self.remap.reg_edit.push(scancode::Mapping { from, to });
                    self.status = Some((true, "Troca adicionada à lista do registro. Clique em \"Aplicar no registro\" para gravar.".into()));
                } else {
                    let k = scancode::name(from).1;
                    if leader.as_deref() == Some(k) {
                        self.status = Some((false, format!("`{k}` é a tecla líder. Troque a líder antes.")));
                        return;
                    }
                    let new = remap::set(&self.text, k, scancode::name(to).1);
                    if self.edit_config_and_save(new) {
                        self.status = Some((true, format!("Troca {} → {} salva no kanata (backup da config anterior guardado).", scancode::name(from).0, scancode::name(to).0)));
                    }
                }
            }
            Do::RemoveKanata(k) => {
                let new = remap::remove(&self.text, &k);
                if self.edit_config_and_save(new) {
                    self.status = Some((true, format!("Troca de `{k}` removida do kanata (backup guardado).")));
                }
            }
            Do::RemoveReg(from) => self.remap.reg_edit.retain(|m| m.from != from),
            Do::ApplyReg => {
                let maps = self.remap.reg_edit.clone();
                self.apply_registry(maps);
            }
            Do::ResetRegEdit => self.remap.reg_edit = cur_reg,
            Do::ClearReg => self.apply_registry(vec![]),
            Do::RestoreReg(p) => self.import_registry(&p, None),
            Do::LoadCfg(p) => match std::fs::read_to_string(&p) {
                Ok(t) => {
                    let nome = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.status = Some(match self.commit(t) {
                        Ok(()) => (true, format!("Backup {nome} carregado e salvo (a config atual foi guardada em backups).")),
                        Err(e) => (false, e),
                    });
                }
                Err(e) => self.status = Some((false, e.to_string())),
            },
            Do::Reload => self.reload_registry(),
        }
    }

    fn features_tab(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("Recursos");
                ui.label(RichText::new("Funções extras do KeyMacros. Todas vêm desativadas: ligue só as que quiser.").color(th::WEAK));
            });
            ui.add_space(12.0);
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.heading("Relógio na tela");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut on = self.features.clock;
                        if th::toggle(ui, &mut on).changed() {
                            self.features.clock = on;
                            self.save_features();
                        }
                        let (txt, c) = if on { ("Ligado", th::GREEN) } else { ("Desligado", th::WEAK) };
                        ui.label(RichText::new(txt).color(c));
                    });
                });
                ui.add_space(10.0);
                let mut follow = self.features.clock_follow;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Posição:").color(th::WEAK));
                    let a = ui.radio_value(&mut follow, false, "Fixo (foge do mouse)").changed();
                    let b = ui.radio_value(&mut follow, true, "Segue o mouse").changed();
                    if a || b {
                        self.features.clock_follow = follow;
                        self.save_features();
                    }
                });
                ui.add_space(10.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(400.0);
                        for line in [
                            "Mostra a hora (horas:minutos) pequena na tela, igual ao relógio do Windows.",
                            "Fica com 50% de opacidade, para você ver o que está por baixo.",
                            "Fixo: fica no canto. Se o mouse chegar perto dele, ele sobe para o canto superior direito na hora, sem animação. Quando o mouse sai, ele volta para baixo.",
                            "Segue o mouse: a hora fica sempre logo abaixo do cursor.",
                            "Passa o clique por ele, então não atrapalha o mouse.",
                            "Não aparece em gravações nem em compartilhamento de tela (exige Windows 10 versão 2004 ou superior).",
                        ] {
                            ui.label(RichText::new(format!("•  {line}")).color(th::TEXT));
                            ui.add_space(4.0);
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("A escolha fica salva. O relógio aparece enquanto o KeyMacros estiver rodando.").color(th::WEAK).small());
                    });
                    ui.add_space(24.0);
                    clock_illustration(ui);
                });
            });
            if let Some(m) = &self.feat_status {
                ui.add_space(8.0);
                ui.label(RichText::new(m).color(th::WEAK));
            }
        });
    }

    /// Grava `keymacros-recursos.json` e avisa o daemon para aplicar na hora.
    fn save_features(&mut self) {
        let msg = match features::save(&self.cfg, self.features) {
            Ok(()) => {
                let h = daemon::find_daemon();
                if h.is_null() {
                    "Salvo. O relógio aparece quando o KeyMacros estiver rodando.".to_string()
                } else {
                    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(h, daemon::WM_KM_FEATURES, 0, 0) };
                    "Salvo e aplicado.".to_string()
                }
            }
            Err(e) => format!("Não consegui salvar: {e}"),
        };
        self.feat_status = Some(msg);
    }

    fn help_tab(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("Ações do KeyMacros — use dentro de (push-msg ...)");
                ui.add_space(4.0);
                egui::Grid::new("cmds").striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    for (name, desc) in msg::COMMANDS {
                        ui.monospace(*name);
                        ui.label(*desc);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(12.0);
            th::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("kanata em 1 minuto");
                ui.add_space(4.0);
                for line in [
                    "(deflayermap (nome) tecla ação tecla ação ...)        — define uma camada",
                    "(layer-while-held km-x)                                 — camada enquanto a tecla está segurada",
                    "(tap-hold 0 300 TOQUE SEGURAR)                          — tocar vs segurar 300 ms",
                    "(tap-hold-press 0 200 TOQUE SEGURAR)                    — vira SEGURAR assim que outra tecla é apertada",
                    "C-c  S-a  A-tab  M-e                                    — Ctrl / Shift / Alt / Win + tecla",
                    "(macro C-l 100 C-c)                                      — sequência de teclas com espera (ms)",
                    "(multi a b)                                              — várias ações juntas",
                    "(fork NORMAL ALTERNATIVA (lctl rctl))                   — muda se Ctrl estiver apertado",
                    "(one-shot 2000 (layer-while-held km-x))                 — camada só para a próxima tecla (2 s)",
                    "XX = nada   _ = transparente   ___ XX = bloqueia todas as outras teclas da camada",
                    "Strings: \"texto\" (sem escapes). Com aspas dentro: r#\"texto \"com\" aspas\"#",
                ] {
                    ui.monospace(line);
                }
                ui.add_space(8.0);
                ui.hyperlink_to("Guia completo do kanata", "https://github.com/jtroo/kanata/blob/main/docs/config.adoc");
            });
        });
    }
}

// ------------------------------------------------------------------ atualização (GitHub)

#[derive(Clone)]
enum UpdPhase {
    /// commits novos no GitHub (mensagens, mais recentes primeiro)
    Available(Vec<String>),
    Working(String),
    Failed(String),
}

#[derive(Default)]
struct UpdateState {
    check: Option<std::sync::mpsc::Receiver<Result<Option<Vec<String>>, String>>>,
    work: Option<std::sync::mpsc::Receiver<update::Passo>>,
    phase: Option<UpdPhase>,
    /// aberto pelo início do Windows só para mostrar a atualização: "Depois" fecha a janela
    so_atualizacao: bool,
}

impl Editor {
    /// Recebe o resultado da checagem e o andamento da atualização (sem travar a janela).
    fn poll_update(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;
        if let Some(res) = self.upd.check.as_ref().map(|rx| rx.try_recv()) {
            match res {
                Err(TryRecvError::Empty) => {}
                Ok(Ok(Some(commits))) => {
                    self.upd.check = None;
                    self.upd.phase = Some(UpdPhase::Available(commits));
                }
                // já está em dia, ou sem rede / sem git: não incomoda
                _ => self.upd.check = None,
            }
        }
        let mut fim: Option<Result<(), String>> = None;
        if let Some(rx) = &self.upd.work {
            loop {
                match rx.try_recv() {
                    Ok(update::Passo::Texto(t)) => self.upd.phase = Some(UpdPhase::Working(t)),
                    Ok(update::Passo::Pronto(r)) => {
                        fim = Some(r);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        fim = Some(Err("a atualização parou inesperadamente".into()));
                        break;
                    }
                }
            }
        }
        match fim {
            None => {}
            // o app novo é aberto pelo processo que foi disparado; esta janela só fecha
            Some(Ok(())) => {
                self.upd.work = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Err(e)) => {
                self.upd.work = None;
                self.upd.phase = Some(UpdPhase::Failed(e));
            }
        }
        if self.upd.work.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn start_update(&mut self) {
        self.upd.work = Some(update::atualizar_in_background(self.cfg.clone()));
        self.upd.phase = Some(UpdPhase::Working("Baixando a nova versão do GitHub…".into()));
    }

    /// Pop-up: "Atualizar agora" / "Depois" (ou "Fechar", se a atualização falhou).
    fn update_modal(&mut self, ctx: &egui::Context) {
        let Some(phase) = self.upd.phase.clone() else { return };
        let mut atualizar = false;
        let mut depois = false;
        egui::Modal::new(egui::Id::unique("update-modal")).frame(th::section()).show(ctx, |ui| {
            ui.set_width(460.0);
            match &phase {
                UpdPhase::Available(commits) => {
                    ui.label(RichText::new("Atualização disponível").font(th::semibold(17.0)));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Tem mudanças novas no GitHub. Ao atualizar, o KeyMacros baixa, compila e reinicia sozinho.").color(th::WEAK));
                    ui.add_space(8.0);
                    for c in commits.iter().take(6) {
                        ui.label(format!("•  {c}"));
                    }
                    if commits.len() > 6 {
                        ui.label(RichText::new(format!("… e mais {}", commits.len() - 6)).color(th::WEAK));
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if th::button(ui, Look::Primary, icon::REFRESH, "Atualizar agora").clicked() {
                            atualizar = true;
                        }
                        if th::button(ui, Look::Ghost, "", "Depois").clicked() {
                            depois = true;
                        }
                    });
                }
                UpdPhase::Working(texto) => {
                    ui.label(RichText::new("Atualizando o KeyMacros").font(th::semibold(17.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(texto.as_str());
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new("Não feche esta janela. Ao terminar, o app reinicia sozinho.").color(th::WEAK));
                }
                UpdPhase::Failed(e) => {
                    ui.label(RichText::new("Não consegui atualizar").font(th::semibold(17.0)).color(th::RED));
                    ui.add_space(8.0);
                    ui.label(RichText::new(e.as_str()).color(th::WEAK));
                    ui.add_space(14.0);
                    if th::button(ui, Look::Ghost, "", "Fechar").clicked() {
                        depois = true;
                    }
                }
            }
        });
        if atualizar {
            self.start_update();
        }
        if depois {
            self.upd.phase = None;
            if self.upd.so_atualizacao {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_poll.elapsed().as_secs() >= 2 {
            self.poll();
        }
        self.poll_update(ui.ctx());
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) && self.dirty() {
            let _ = self.save();
        }
        // Ctrl+Z só quando nenhum campo de texto está em edição (o campo tem o seu próprio desfazer)
        if !ui.ctx().egui_wants_keyboard_input() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            self.undo_last();
        }
        egui::Panel::top("top")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(th::BAR).inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 10 }))
            .show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default_margins().frame(egui::Frame::new().fill(th::BG).inner_margin(Margin::same(18))).show(ui, |ui| match self.tab {
            Tab::Macros => self.macros_tab(ui),
            Tab::Code => self.code_tab(ui),
            Tab::Remap => self.remap_tab(ui),
            Tab::Features => self.features_tab(ui),
            Tab::Help => self.help_tab(ui),
        });
        self.update_modal(ui.ctx());
    }
}

/// `tab`: aba inicial (`troca`, `codigo`, `ajuda`); padrão = Macros.
/// `so_atualizacao`: aberto pelo início do Windows só para mostrar a atualização.
pub fn run(cfg: PathBuf, tab: Option<&str>, so_atualizacao: bool) -> i32 {
    let tab = match tab {
        Some("troca") => Tab::Remap,
        Some("recursos") => Tab::Features,
        Some("codigo") => Tab::Code,
        Some("ajuda") => Tab::Help,
        _ => Tab::Macros,
    };
    // já aberto? traz para frente
    let t = win::wide(TITLE);
    let existing = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(std::ptr::null(), t.as_ptr()) };
    if !existing.is_null() {
        win::activate(existing);
        return 0;
    }
    let icon = egui::IconData { rgba: daemon::icon_rgba([37, 99, 235]), width: 32, height: 32 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([1200.0, 880.0]).with_min_inner_size([900.0, 640.0]).with_maximized(true).with_icon(icon),
        ..Default::default()
    };
    match eframe::run_native(
        TITLE,
        options,
        Box::new(|cc| {
            th::install(&cc.egui_ctx);
            let mut e = Editor::new(cfg);
            e.tab = tab;
            e.upd.so_atualizacao = so_atualizacao;
            Ok(Box::new(e))
        }),
    ) {
        Ok(()) => 0,
        Err(e) => {
            win::message_box("KeyMacros", &format!("Erro ao abrir o editor: {e}"), windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
            1
        }
    }
}
