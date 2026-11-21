//! Visual do editor: cores, fontes (Segoe UI + Segoe Fluent Icons do próprio
//! Windows) e componentes reutilizáveis — botões, "teclas", cartões, pílulas.

use eframe::egui::{self, pos2, text::LayoutJob, vec2, Color32, CornerRadius, FontData, FontFamily, FontId, Margin, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::sync::Arc;

// ------------------------------------------------------------------ cores

pub const BG: Color32 = Color32::from_rgb(11, 17, 27);
pub const BAR: Color32 = Color32::from_rgb(14, 21, 33);
pub const PANEL: Color32 = Color32::from_rgb(16, 24, 37);
pub const CARD: Color32 = Color32::from_rgb(21, 30, 45);
pub const CARD_HOVER: Color32 = Color32::from_rgb(27, 38, 56);
pub const CARD_SEL: Color32 = Color32::from_rgb(19, 37, 66);
pub const FIELD: Color32 = Color32::from_rgb(12, 19, 30);
pub const BORDER: Color32 = Color32::from_rgb(40, 52, 72);
pub const BORDER_HI: Color32 = Color32::from_rgb(70, 86, 112);
pub const TEXT: Color32 = Color32::from_rgb(232, 238, 246);
pub const WEAK: Color32 = Color32::from_rgb(150, 164, 186);
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
pub const ACCENT_HI: Color32 = Color32::from_rgb(59, 130, 246);
pub const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
pub const GREEN_BG: Color32 = Color32::from_rgb(14, 42, 28);
pub const YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const YELLOW_BG: Color32 = Color32::from_rgb(45, 37, 10);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);
pub const RED_BG: Color32 = Color32::from_rgb(48, 16, 20);

// ------------------------------------------------------------------ ícones (Segoe Fluent Icons / MDL2)

pub mod icon {
    pub const ADD: &str = "\u{E710}";
    pub const MORE: &str = "\u{E712}";
    pub const SEARCH: &str = "\u{E721}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const CHECK: &str = "\u{E73E}";
    pub const DELETE: &str = "\u{E74D}";
    pub const SAVE: &str = "\u{E74E}";
    pub const KEYBOARD: &str = "\u{E765}";
    pub const PLAY: &str = "\u{E768}";
    pub const PAUSE: &str = "\u{E769}";
    pub const GLOBE: &str = "\u{E774}";
    pub const CONSOLE: &str = "\u{E756}";
    pub const FOLDER: &str = "\u{E8B7}";
    pub const HELP: &str = "\u{E897}";
    pub const CODE: &str = "</>";
    pub const SWAP: &str = "\u{E8AB}";
    pub const GRID: &str = "\u{F0E2}";
    pub const LIST: &str = "\u{E8FD}";
    pub const BLOCKED: &str = "\u{E733}";
    pub const TOUCH: &str = "\u{E815}";
    pub const WARNING: &str = "\u{E7BA}";
    pub const WINDOW: &str = "\u{E737}";
    pub const CUBE: &str = "\u{F158}";
    pub const FONT: &str = "\u{E8D2}";
    pub const POWER: &str = "\u{E7E8}";
    pub const CHEVRON: &str = "\u{E70D}";
}

// ------------------------------------------------------------------ fontes e estilo

const SEMIBOLD: &str = "semibold";

fn read_font(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    std::fs::read(format!("{dir}\\Fonts\\{name}")).ok()
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |key: &str, files: &[&str], tweak: Option<egui::FontTweak>| -> bool {
        for f in files {
            if let Some(data) = read_font(f) {
                let mut fd = FontData::from_owned(data);
                if let Some(t) = tweak.clone() {
                    fd = fd.tweak(t);
                }
                fonts.font_data.insert(key.into(), Arc::new(fd));
                return true;
            }
        }
        false
    };
    let ui_font = add("segoe", &["segoeui.ttf"], None);
    let sb_font = add("segoe-sb", &["seguisb.ttf", "segoeuib.ttf"], None);
    let mono_font = add("mono", &["CascadiaMono.ttf", "consola.ttf"], None);
    let icons = add(
        "icons",
        &["SegoeIcons.ttf", "segmdl2.ttf"],
        Some(egui::FontTweak { scale: 0.92, y_offset_factor: 0.12, ..Default::default() }),
    );
    let defaults_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let defaults_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let chain = |first: &[(&str, bool)], rest: &[String]| -> Vec<String> {
        let mut v: Vec<String> = first.iter().filter(|(_, ok)| *ok).map(|(n, _)| n.to_string()).collect();
        v.extend(rest.iter().cloned());
        v
    };
    let prop = chain(&[("segoe", ui_font), ("icons", icons)], &defaults_prop);
    let semi = chain(&[("segoe-sb", sb_font), ("segoe", ui_font), ("icons", icons)], &defaults_prop);
    let mono = chain(&[("mono", mono_font)], &defaults_mono);
    let mono = {
        let mut m = mono;
        if icons {
            m.push("icons".into());
        }
        m
    };
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semi);
    fonts.families.insert(FontFamily::Monospace, mono);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(15.0));
        s.text_styles.insert(Button, FontId::proportional(15.0));
        s.text_styles.insert(Small, FontId::proportional(12.5));
        s.text_styles.insert(Monospace, FontId::monospace(13.5));
        s.text_styles.insert(Heading, FontId::new(20.0, FontFamily::Name(SEMIBOLD.into())));
        s.interaction.selectable_labels = false;
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(10.0, 6.0);
        s.spacing.interact_size = vec2(36.0, 30.0);
        s.spacing.combo_height = 400.0;
        s.spacing.menu_margin = Margin::same(8);

        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = FIELD;
        v.text_edit_bg_color = Some(FIELD);
        v.faint_bg_color = Color32::from_rgb(18, 27, 41);
        v.code_bg_color = FIELD;
        v.hyperlink_color = ACCENT_HI;
        v.error_fg_color = RED;
        v.warn_fg_color = YELLOW;
        v.selection.bg_fill = Color32::from_rgb(29, 78, 216);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        let r = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        for (st, fill, stroke) in [
            (&mut w.inactive, CARD, BORDER),
            (&mut w.hovered, CARD_HOVER, BORDER_HI),
            (&mut w.active, CARD_SEL, ACCENT_HI),
            (&mut w.open, CARD_HOVER, ACCENT_HI),
        ] {
            st.bg_fill = fill;
            st.weak_bg_fill = fill;
            st.bg_stroke = Stroke::new(1.0, stroke);
            st.corner_radius = r;
            st.fg_stroke = Stroke::new(1.5, TEXT);
            st.expansion = 0.0;
        }
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    });
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// "ícone  texto" com o ícone em `icon_color`.
pub fn icon_job(ic: &str, text: &str, font: FontId, color: Color32, icon_color: Color32) -> LayoutJob {
    let mut j = LayoutJob::default();
    if !ic.is_empty() {
        j.append(ic, 0.0, egui::TextFormat::simple(FontId::proportional(font.size + 2.0), icon_color));
    }
    if !text.is_empty() {
        j.append(text, if ic.is_empty() { 0.0 } else { 10.0 }, egui::TextFormat::simple(font, color));
    }
    j
}

// ------------------------------------------------------------------ botões

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Look {
    Primary,
    Normal,
    Ghost,
    /// cor do texto/borda e do fundo
    Tint(Color32, Color32),
}

pub const BTN_H: f32 = 38.0;

/// Botão desenhado à mão (para controlar fundo/borda/hover como no mockup).
pub fn button_job(ui: &mut Ui, look: Look, job: LayoutJob, min_w: f32, sense: Sense) -> Response {
    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
    let w = (galley.size().x + 28.0).max(min_w);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BTN_H), sense);
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered() && sense != Sense::hover();
        let down = resp.is_pointer_button_down_on();
        let (fill, stroke) = match look {
            Look::Primary => (if down { Color32::from_rgb(29, 78, 216) } else if hot { ACCENT_HI } else { ACCENT }, Stroke::new(1.0, ACCENT_HI)),
            Look::Normal => (if hot { CARD_HOVER } else { CARD }, Stroke::new(1.0, if hot { BORDER_HI } else { BORDER })),
            Look::Ghost => (if hot { CARD } else { Color32::TRANSPARENT }, Stroke::NONE),
            Look::Tint(fg, bg) => (bg, Stroke::new(1.0, fg.gamma_multiply(if hot { 1.0 } else { 0.8 }))),
        };
        let (fill, stroke) = if ui.is_enabled() { (fill, stroke) } else { (fill.gamma_multiply(0.5), Stroke::new(stroke.width, stroke.color.gamma_multiply(0.5))) };
        ui.painter().rect(rect, 8, fill, stroke, StrokeKind::Inside);
        let pos = rect.center() - galley.size() / 2.0;
        let color = if ui.is_enabled() { TEXT } else { WEAK.gamma_multiply(0.6) };
        ui.painter().galley(pos, galley, color);
    }
    if sense != Sense::hover() {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

pub fn button(ui: &mut Ui, look: Look, ic: &str, text: &str) -> Response {
    let fg = match look {
        Look::Primary => Color32::WHITE,
        Look::Tint(c, _) => c,
        _ => TEXT,
    };
    let ic_color = if look == Look::Primary { Color32::WHITE } else { fg };
    button_job(ui, look, icon_job(ic, text, regular(15.0), fg, ic_color), 0.0, Sense::click())
}

/// Botão quadrado só com ícone.
pub fn icon_button(ui: &mut Ui, selected: bool, ic: &str, tip: &str) -> Response {
    let look = if selected { Look::Primary } else { Look::Normal };
    let color = if selected { Color32::WHITE } else { TEXT };
    button_job(ui, look, icon_job(ic, "", regular(15.0), color, color), 46.0, Sense::click()).on_hover_text(tip)
}

/// Rótulo com borda colorida (não clicável).
pub fn pill(ui: &mut Ui, fg: Color32, bg: Color32, job: LayoutJob) -> Response {
    button_job(ui, Look::Tint(fg, bg), job, 0.0, Sense::hover())
}

// ------------------------------------------------------------------ "tecla" (keycap)

pub fn keycap_paint(ui: &Ui, rect: Rect, label: &str, font_size: f32, hot: bool) {
    let p = ui.painter();
    let shadow = rect.translate(vec2(0.0, 3.0));
    p.rect_filled(shadow, 10, Color32::from_rgb(5, 8, 13));
    let edge = if hot { ACCENT_HI } else { Color32::from_rgb(62, 74, 94) };
    p.rect(rect, 10, Color32::from_rgb(24, 32, 45), Stroke::new(1.2, edge), StrokeKind::Inside);
    let face = Rect::from_min_max(rect.min + vec2(4.0, 3.0), rect.max - vec2(4.0, 6.0));
    p.rect(face, 7, Color32::from_rgb(35, 45, 61), Stroke::new(1.0, Color32::from_rgb(52, 64, 83)), StrokeKind::Inside);
    let mut size = font_size;
    let n = label.chars().count();
    if n > 2 {
        size = (font_size * 2.2 / n as f32).max(font_size * 0.42).min(font_size * 0.7);
    }
    p.text(face.center(), egui::Align2::CENTER_CENTER, label, semibold(size), TEXT);
}

pub fn keycap(ui: &mut Ui, size: Vec2, label: &str, font_size: f32, sense: Sense) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size + vec2(0.0, 3.0), sense);
    let rect = Rect::from_min_size(rect.min, size);
    keycap_paint(ui, rect, label, font_size, resp.hovered() && sense != Sense::hover());
    resp
}

// ------------------------------------------------------------------ cartões e seções

/// Moldura de seção (fundo levemente mais claro, borda e cantos arredondados).
pub fn section() -> egui::Frame {
    egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(12).inner_margin(Margin::same(16))
}

/// Linha interna do formulário (como "Ao tocar").
pub fn row_frame() -> egui::Frame {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10).inner_margin(Margin::symmetric(14, 8))
}

/// Cartão clicável de tamanho fixo; o conteúdo é pintado por quem chama (em `resp.rect`).
pub fn card(ui: &mut Ui, size: Vec2, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered();
        let (fill, stroke) = if selected {
            (CARD_SEL, Stroke::new(2.0, ACCENT_HI))
        } else if hot {
            (CARD_HOVER, Stroke::new(1.0, BORDER_HI))
        } else {
            (CARD, Stroke::new(1.0, BORDER))
        };
        ui.painter().rect(rect, 12, fill, stroke, StrokeKind::Inside);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Texto numa linha só, cortado com "…" se passar de `max_w`. Devolve o tamanho.
pub fn text_trunc(ui: &Ui, pos: egui::Pos2, text: &str, font: FontId, color: Color32, max_w: f32) -> Vec2 {
    let mut j = LayoutJob::simple_singleline(text.to_string(), font, color);
    j.wrap = egui::text::TextWrapping { max_width: max_w.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = ui.ctx().fonts_mut(|f| f.layout_job(j));
    let size = g.size();
    ui.painter().galley(pos, g, color);
    size
}

/// Linha vertical fina entre a tecla e as ações de um cartão.
pub fn vsep(ui: &mut Ui, height: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, BORDER));
}

/// Ícone desenhado numa área quadrada: textura (ícone do app) ou glifo.
#[derive(Clone)]
pub enum Ico {
    Tex(egui::TextureHandle),
    Glyph(&'static str, Color32),
}

pub fn paint_ico(p: &egui::Painter, r: Rect, ico: &Ico) {
    match ico {
        Ico::Tex(t) => {
            p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        Ico::Glyph(g, c) => {
            p.text(r.center() + vec2(0.0, r.height() * 0.04), egui::Align2::CENTER_CENTER, *g, FontId::proportional(r.height() * 0.86), *c);
        }
    }
}

/// Seta "v" no lugar do triângulo dos ComboBox.
pub fn chevron(ui: &Ui, rect: Rect, _v: &egui::style::WidgetVisuals, open: bool) {
    let c = if open { ACCENT_HI } else { WEAK };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::CHEVRON, FontId::proportional(13.0), c);
}

/// Caixa de seleção azul (como no Windows 11).
pub fn checkbox(ui: &mut Ui, on: &mut bool) -> Response {
    let (r, mut resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hot = resp.hovered();
    let p = ui.painter();
    if *on {
        p.rect_filled(r, 5, if hot { ACCENT_HI } else { ACCENT });
        p.text(r.center() + vec2(0.0, 1.0), egui::Align2::CENTER_CENTER, icon::CHECK, FontId::proportional(13.0), Color32::WHITE);
    } else {
        p.rect(r, 5, FIELD, Stroke::new(1.5, if hot { ACCENT_HI } else { BORDER_HI }), StrokeKind::Inside);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
