use eframe::egui::{self, Color32, FontId, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x22, 0x25, 0x2c);
pub const BG_DARK: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x21);
pub const BG_ROW_ALT: Color32 = Color32::from_rgb(0x25, 0x28, 0x30);
pub const SELECTED: Color32 = Color32::from_rgb(0x2c, 0x4a, 0x78);
/// Ahead/behind pill background.
pub const BADGE: Color32 = Color32::from_rgb(0x3a, 0x3f, 0x4b);
pub const HOVER: Color32 = Color32::from_rgb(0x2d, 0x31, 0x3b);
pub const TEXT: Color32 = Color32::from_rgb(0xd4, 0xd7, 0xdd);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x90, 0x9c);
pub const ACCENT: Color32 = Color32::from_rgb(0xe5, 0xb5, 0x67);
pub const ADDED_BG: Color32 = Color32::from_rgb(0x23, 0x3d, 0x2b);
pub const REMOVED_BG: Color32 = Color32::from_rgb(0x46, 0x26, 0x2a);
pub const ADDED: Color32 = Color32::from_rgb(0x8f, 0xd1, 0x8f);
pub const REMOVED: Color32 = Color32::from_rgb(0xf0, 0x8c, 0x8c);
/// Word-level changes inside a changed line (brighter than the line).
pub const ADDED_WORD_BG: Color32 = Color32::from_rgb(0x2f, 0x6a, 0x3e);
pub const REMOVED_WORD_BG: Color32 = Color32::from_rgb(0x7a, 0x34, 0x3b);
pub const HUNK_BG: Color32 = Color32::from_rgb(0x2a, 0x30, 0x3c);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x7a, 0x7a);

const LANES: [Color32; 8] = [
    Color32::from_rgb(0x5a, 0xb0, 0xf0),
    Color32::from_rgb(0xe5, 0xb5, 0x67),
    Color32::from_rgb(0x9a, 0xd0, 0x6c),
    Color32::from_rgb(0xd0, 0x7c, 0xe0),
    Color32::from_rgb(0xf0, 0x80, 0x60),
    Color32::from_rgb(0x50, 0xd0, 0xc0),
    Color32::from_rgb(0xe0, 0x70, 0x9a),
    Color32::from_rgb(0xb0, 0xb0, 0xf0),
];

pub fn lane(i: u16) -> Color32 {
    LANES[i as usize % LANES.len()]
}


pub fn apply(ctx: &egui::Context, font_size: f32) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = BG_DARK;
    v.faint_bg_color = BG_ROW_ALT;
    v.selection.bg_fill = SELECTED;
    v.hyperlink_color = Color32::from_rgb(0x7a, 0xb8, 0xff);
    v.override_text_color = Some(TEXT);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        let f = font_size;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(f));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(f));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(f - 0.5));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(f - 2.0));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(f + 3.0));
        s.spacing.item_spacing = egui::vec2(6.0, 3.0);
        s.spacing.button_padding = egui::vec2(7.0, 2.0);
        s.spacing.interact_size.y = f + 7.0;
    });
}
