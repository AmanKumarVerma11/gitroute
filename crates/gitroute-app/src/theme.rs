//! The transit-line look. Every account is a colored line with a lettered
//! bullet, the way metro lines are; everything else stays neutral so the lines
//! carry the meaning. Text is Helvetica Neue, the lettering of transit
//! signage and installed on every Mac; paths and codes use SF Mono. Light or
//! dark follows the system.
//!
//! Direction contract (impeccable, seed 5f6ad431):
//! - THESIS: routing is a line you can see. The window refuses the generic
//!   settings-form look; every account is a transit line and every decision
//!   is drawn as a route.
//! - OWN-WORLD: neutral cool grounds, ink controls, Helvetica Neue, and
//!   classic line colors used only on bullets and active routes. Hollow
//!   bullets mean "via the default".
//! - STORY: see each account's line, route folders onto lines, and check
//!   why a path lands where it does.
//! - FIRST VIEWPORT: the sidebar's navigation is itself a line with stations
//!   and install status at its foot; the content shows account name plates.
//! - FORM: the transit diagram, 6th of 7 grounded candidates.
//! - FINISH: unreviewed and undocumented is unfinished; this build ends with
//!   the finish review, the verdict, DESIGN.md, and every shipping raster
//!   carrying its provenance.

use std::fs;

use eframe::egui::{
    self, Button, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Margin, Response, RichText, Sense, Shadow, Stroke, Style, TextStyle, Theme, ThemePreference,
    Ui, Vec2, Visuals, vec2,
};

/// Classic transit line colors, assigned to accounts in order.
const LINES: [Color32; 8] = [
    Color32::from_rgb(0, 147, 60),   // green
    Color32::from_rgb(0, 57, 166),   // blue
    Color32::from_rgb(232, 82, 0),   // orange, a shade deeper for 3:1 on white
    Color32::from_rgb(185, 51, 173), // purple
    Color32::from_rgb(238, 53, 46),  // red
    Color32::from_rgb(252, 204, 10), // yellow
    Color32::from_rgb(153, 102, 51), // brown
    Color32::from_rgb(0, 173, 208),  // teal
];

/// A line that no longer has an account (a rule pointing at a removed one).
pub const LINE_UNKNOWN: Color32 = Color32::from_rgb(128, 129, 131);

/// Light-card yellow and teal, and dark-card blue: the classic colors fall
/// under 3:1 against those cards, so they're deepened or lifted there.
const GOLD: Color32 = Color32::from_rgb(176, 132, 0);
const DEEP_TEAL: Color32 = Color32::from_rgb(0, 140, 170);
const LIGHT_BLUE: Color32 = Color32::from_rgb(70, 130, 255);

/// The line color for the account at `index`, tuned for the appearance so
/// every line holds at least 3:1 against the cards it sits on.
pub fn line_color(index: usize, dark: bool) -> Color32 {
    match (index % LINES.len(), dark) {
        (1, true) => LIGHT_BLUE,
        (5, false) => GOLD,
        (7, false) => DEEP_TEAL,
        (i, _) => LINES[i],
    }
}

/// The bullet letter: white, as on transit bullets, unless white falls under
/// 3:1 on that line (yellow, the classic teal), then black.
fn on_line(color: Color32) -> Color32 {
    if contrast(color, Color32::WHITE) >= 3.0 {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// WCAG relative luminance.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio.
fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Neutral roles for one appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    pub ground: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub field: Color32,
    pub hairline: Color32,
    pub ink: Color32,
    pub ink_soft: Color32,
    pub ink_faint: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

const LIGHT: Palette = Palette {
    ground: Color32::from_rgb(243, 244, 246),
    sidebar: Color32::from_rgb(232, 234, 237),
    card: Color32::from_rgb(255, 255, 255),
    field: Color32::from_rgb(247, 248, 250),
    hairline: Color32::from_rgb(218, 221, 226),
    ink: Color32::from_rgb(22, 24, 29),
    ink_soft: Color32::from_rgb(88, 95, 106),
    ink_faint: Color32::from_rgb(160, 166, 175),
    ok: Color32::from_rgb(24, 128, 56),
    warn: Color32::from_rgb(168, 86, 0),
    danger: Color32::from_rgb(190, 44, 38),
};

const DARK: Palette = Palette {
    ground: Color32::from_rgb(20, 21, 24),
    sidebar: Color32::from_rgb(27, 29, 33),
    card: Color32::from_rgb(31, 34, 38),
    field: Color32::from_rgb(23, 25, 28),
    hairline: Color32::from_rgb(48, 52, 58),
    ink: Color32::from_rgb(238, 240, 242),
    ink_soft: Color32::from_rgb(166, 173, 183),
    ink_faint: Color32::from_rgb(98, 105, 115),
    ok: Color32::from_rgb(76, 194, 108),
    warn: Color32::from_rgb(232, 168, 64),
    danger: Color32::from_rgb(242, 112, 102),
};

pub fn palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Text sizes beyond egui's built-in styles.
pub fn title() -> TextStyle {
    TextStyle::Name("title".into())
}

pub fn code() -> TextStyle {
    TextStyle::Name("code".into())
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(ThemePreference::System);
    ctx.set_style_of(Theme::Light, style(&LIGHT, false));
    ctx.set_style_of(Theme::Dark, style(&DARK, true));
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let helvetica = fs::read("/System/Library/Fonts/HelveticaNeue.ttc").ok();
    let faces = [
        ("Helvetica Neue", FontFamily::Proportional),
        ("Helvetica Neue Medium", medium()),
        ("Helvetica Neue Bold", bold()),
    ];
    for (name, family) in faces {
        let index = helvetica.as_deref().and_then(|data| face_index(data, name));
        let list = fonts.families.entry(family.clone()).or_default();
        if let (Some(data), Some(index)) = (&helvetica, index) {
            let mut font = FontData::from_owned(data.clone());
            font.index = index;
            fonts.font_data.insert(name.to_string(), font.into());
            list.insert(0, name.to_string());
        }
        // egui's own fonts stay behind as the fallback (arrows, symbols,
        // and Macs without the system face).
        if family != FontFamily::Proportional {
            let defaults = FontDefinitions::default().families[&FontFamily::Proportional].clone();
            list.extend(defaults);
        }
    }
    if let Ok(data) = fs::read("/System/Library/Fonts/SFNSMono.ttf") {
        fonts
            .font_data
            .insert("SF Mono".into(), FontData::from_owned(data).into());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "SF Mono".into());
    }
    fonts
}

/// The index of the face named `full_name` in a TrueType collection, read
/// from each face's `name` table (name id 4, the full name).
fn face_index(data: &[u8], full_name: &str) -> Option<u32> {
    let u16_at = |o: usize| Some(u16::from_be_bytes(data.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?));
    if data.get(0..4)? != b"ttcf" {
        return None;
    }
    for face in 0..u32_at(8)? {
        let offset = u32_at(12 + 4 * face as usize)? as usize;
        for table in 0..u16_at(offset + 4)? as usize {
            let record = offset + 12 + 16 * table;
            if data.get(record..record + 4)? != b"name" {
                continue;
            }
            let name = u32_at(record + 8)? as usize;
            let strings = name + u16_at(name + 4)? as usize;
            for i in 0..u16_at(name + 2)? as usize {
                let r = name + 6 + 12 * i;
                let (platform, name_id) = (u16_at(r)?, u16_at(r + 6)?);
                if name_id != 4 {
                    continue;
                }
                let (len, off) = (u16_at(r + 8)? as usize, u16_at(r + 10)? as usize);
                let raw = data.get(strings + off..strings + off + len)?;
                let text = if platform == 1 {
                    raw.iter().map(|&b| b as char).collect::<String>()
                } else {
                    let units: Vec<u16> = raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_be_bytes(*c))
                        .collect();
                    String::from_utf16_lossy(&units)
                };
                if text == full_name {
                    return Some(face);
                }
            }
        }
    }
    None
}

fn style(p: &Palette, dark: bool) -> Style {
    let mut style = Style {
        text_styles: [
            (TextStyle::Heading, FontId::new(26.0, medium())),
            (title(), FontId::new(17.0, medium())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, medium())),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (code(), FontId::new(34.0, FontFamily::Monospace)),
        ]
        .into(),
        ..Style::default()
    };

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 5.0);
    s.interact_size = vec2(40.0, 28.0);
    s.window_margin = Margin::same(24);
    s.menu_margin = Margin::same(6);
    s.combo_width = 260.0;
    s.text_edit_width = 320.0;

    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.ground;
    v.window_fill = p.card;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.field;
    v.code_bg_color = p.field;
    v.window_stroke = Stroke::new(1.0, p.hairline);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 36,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 46 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 36 }),
    };
    v.selection.bg_fill = p.ink.gamma_multiply(0.22);
    v.selection.stroke = Stroke::new(1.0, p.ink);
    v.text_cursor.stroke = Stroke::new(1.5, p.ink);
    v.hyperlink_color = p.ink;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.danger;

    let button = if dark {
        Color32::from_rgb(44, 48, 54)
    } else {
        Color32::from_rgb(229, 231, 235)
    };
    let hover = if dark {
        Color32::from_rgb(54, 59, 66)
    } else {
        Color32::from_rgb(218, 221, 226)
    };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
    for (state, fill) in [
        (&mut w.inactive, button),
        (&mut w.hovered, hover),
        (&mut w.active, hover),
        (&mut w.open, hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
        state.fg_stroke = Stroke::new(1.5, p.ink);
        state.corner_radius = CornerRadius::same(8);
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, p.hairline);
    w.active.bg_stroke = Stroke::new(1.0, p.ink_faint);
    style.visuals = v;
    style
}

/// A rounded surface for one group of related content.
pub fn card(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.hairline))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
}

/// The one filled button per area: ink on light, white on dark.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(
        Button::new(RichText::new(text).color(p.ground))
            .fill(p.ink)
            .corner_radius(CornerRadius::same(8)),
    )
}

/// Destructive actions stay quiet text until hovered, and sit apart from
/// everything else.
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(Button::new(RichText::new(text).color(p.danger)).frame_when_inactive(false))
}

/// A line bullet: a filled circle with the line's letter, like a metro badge.
/// `hollow` draws the ring only: a folder that gets this line through the
/// default rather than its own rule.
pub fn bullet(ui: &mut Ui, letter: char, color: Color32, diameter: f32, hollow: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    let card = palette(ui).card;
    paint_bullet(
        ui.painter(),
        rect.center(),
        diameter,
        letter,
        color,
        hollow,
        card,
    );
    response
}

pub fn paint_bullet(
    painter: &egui::Painter,
    center: egui::Pos2,
    diameter: f32,
    letter: char,
    color: Color32,
    hollow: bool,
    card: Color32,
) {
    let r = diameter / 2.0;
    let font = FontId::new(diameter * 0.52, bold());
    if hollow {
        painter.circle(center, r - 1.5, card, Stroke::new(2.5, color));
        painter.text(center, egui::Align2::CENTER_CENTER, letter, font, color);
    } else {
        painter.circle_filled(center, r, color);
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            letter,
            font,
            on_line(color),
        );
    }
}

/// gitroute's mark: the menu bar's branch glyph, in ink.
pub fn mark(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let ink = palette(ui).ink;
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * (size / 36.0);
    let painter = ui.painter();
    let stroke = Stroke::new(3.0 * size / 36.0 * 1.6, ink);
    painter.line_segment([at(12.0, 7.0), at(12.0, 29.0)], stroke);
    painter.line_segment([at(25.0, 11.0), at(14.0, 24.0)], stroke);
    for (x, y) in [(12.0, 7.0), (12.0, 29.0), (25.0, 11.0)] {
        painter.circle_filled(at(x, y), 4.5 * size / 36.0 * 1.4, ink);
    }
}

/// The open/close indicator for every dropdown: a small up/down chevron pair,
/// as on macOS pop-up buttons, instead of egui's filled triangle.
pub fn chevrons(ui: &Ui, rect: egui::Rect, _visuals: &egui::style::WidgetVisuals, _open: bool) {
    let p = palette(ui);
    let c = rect.center();
    let stroke = Stroke::new(1.6, p.ink_soft);
    let painter = ui.painter();
    for dir in [-1.0, 1.0] {
        let tip = c + vec2(0.0, 6.0 * dir);
        let base = c + vec2(0.0, 2.0 * dir);
        painter.line_segment([base + vec2(-3.5, 0.0), tip], stroke);
        painter.line_segment([tip, base + vec2(3.5, 0.0)], stroke);
    }
}

/// A small filled circle, for status next to its text (never alone).
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A rounded outline label, e.g. "Default".
pub fn chip(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    Frame::new()
        .stroke(Stroke::new(1.0, p.hairline))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(p.ink_soft))
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_helvetica_faces_by_name() {
        let Ok(data) = fs::read("/System/Library/Fonts/HelveticaNeue.ttc") else {
            return; // not on this machine
        };
        assert!(face_index(&data, "Helvetica Neue").is_some());
        assert!(face_index(&data, "Helvetica Neue Medium").is_some());
        assert!(face_index(&data, "Helvetica Neue Bold").is_some());
        assert_eq!(face_index(&data, "Comic Neue"), None);
        assert_eq!(face_index(b"not a font", "Helvetica Neue"), None);
    }

    #[test]
    fn every_line_holds_three_to_one_on_its_cards() {
        for (dark, card) in [(false, LIGHT.card), (true, DARK.card)] {
            for i in 0..LINES.len() {
                let line = line_color(i, dark);
                assert!(
                    contrast(line, card) >= 3.0,
                    "line {i} (dark: {dark}) is {:.2}:1",
                    contrast(line, card)
                );
            }
        }
    }

    #[test]
    fn line_colors_cycle_and_letters_stay_readable() {
        assert_eq!(line_color(0, false), line_color(8, false));
        for dark in [false, true] {
            for i in 0..LINES.len() {
                let line = line_color(i, dark);
                assert!(contrast(line, on_line(line)) >= 3.0, "letter on line {i}");
            }
        }
    }
}
