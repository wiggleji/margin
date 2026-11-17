//! Colours and text styles. The palette is switchable at runtime (Settings →
//! Appearance); drawing code asks for colours by role (`theme::TEXT()`), so a
//! theme change applies on the next frame.
//!
//! Themes: Margin (the original), the Material Theme family (Material,
//! Darker, Palenight, Ocean, Lighter) and Solarized Dark / Light. Each is
//! defined by a few base colours; panel shades, diff backgrounds, selection
//! and graph lanes are mixed from them.

use eframe::egui::{self, Color32, FontId, TextStyle};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Everything the UI paints with.
#[derive(Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub bg: Color32,
    pub bg_dark: Color32,
    pub bg_row_alt: Color32,
    pub selected: Color32,
    pub badge: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub added: Color32,
    pub removed: Color32,
    pub added_word_bg: Color32,
    pub removed_word_bg: Color32,
    pub hunk_bg: Color32,
    pub error: Color32,
    pub link: Color32,
    /// Text fields.
    pub field: Color32,
    pub lanes: [Color32; 8],
}

const fn hex(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

const fn c(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// `a` → `b` by `t`/1000.
const fn mix(a: [u8; 3], b: [u8; 3], t: u32) -> Color32 {
    Color32::from_rgb(mix1(a[0], b[0], t), mix1(a[1], b[1], t), mix1(a[2], b[2], t))
}

const fn mix1(x: u8, y: u8, t: u32) -> u8 {
    ((x as u32 * (1000 - t) + y as u32 * t + 500) / 1000) as u8
}

/// The base colours a theme is made from.
struct Base {
    id: &'static str,
    name: &'static str,
    dark: bool,
    bg: u32,
    /// Darker (or, for light themes, slightly deeper) panel shade.
    bg_dark: u32,
    text: u32,
    text_dim: u32,
    accent: u32,
    red: u32,
    green: u32,
    yellow: u32,
    blue: u32,
    purple: u32,
    cyan: u32,
    orange: u32,
}

const fn make(b: Base) -> Palette {
    let (bg, text) = (hex(b.bg), hex(b.text));
    let (red, green, blue) = (hex(b.red), hex(b.green), hex(b.blue));
    let d = b.dark;
    Palette {
        id: b.id,
        name: b.name,
        dark: d,
        bg: c(bg),
        bg_dark: c(hex(b.bg_dark)),
        bg_row_alt: mix(bg, text, if d { 25 } else { 30 }),
        selected: mix(bg, blue, if d { 330 } else { 230 }),
        badge: mix(bg, text, if d { 160 } else { 120 }),
        hover: mix(bg, text, if d { 70 } else { 60 }),
        text: c(text),
        text_dim: c(hex(b.text_dim)),
        accent: c(hex(b.accent)),
        added_bg: mix(bg, green, if d { 170 } else { 200 }),
        removed_bg: mix(bg, red, if d { 170 } else { 150 }),
        added: c(green),
        removed: c(red),
        added_word_bg: mix(bg, green, if d { 380 } else { 420 }),
        removed_word_bg: mix(bg, red, if d { 380 } else { 330 }),
        hunk_bg: mix(bg, blue, if d { 90 } else { 80 }),
        error: c(red),
        link: c(blue),
        field: if d { c(hex(b.bg_dark)) } else { mix(bg, hex(0xffffff), 700) },
        lanes: [
            c(blue),
            c(hex(b.yellow)),
            c(green),
            c(hex(b.purple)),
            c(hex(b.orange)),
            c(hex(b.cyan)),
            c(red),
            mix(blue, hex(b.purple), 500),
        ],
    }
}

/// The original Margin look (kept exactly).
const MARGIN: Palette = Palette {
    id: "margin",
    name: "Margin",
    dark: true,
    bg: Color32::from_rgb(0x22, 0x25, 0x2c),
    bg_dark: Color32::from_rgb(0x1a, 0x1c, 0x21),
    bg_row_alt: Color32::from_rgb(0x25, 0x28, 0x30),
    selected: Color32::from_rgb(0x2c, 0x4a, 0x78),
    badge: Color32::from_rgb(0x3a, 0x3f, 0x4b),
    hover: Color32::from_rgb(0x2d, 0x31, 0x3b),
    text: Color32::from_rgb(0xd4, 0xd7, 0xdd),
    text_dim: Color32::from_rgb(0x8a, 0x90, 0x9c),
    accent: Color32::from_rgb(0xe5, 0xb5, 0x67),
    added_bg: Color32::from_rgb(0x23, 0x3d, 0x2b),
    removed_bg: Color32::from_rgb(0x46, 0x26, 0x2a),
    added: Color32::from_rgb(0x8f, 0xd1, 0x8f),
    removed: Color32::from_rgb(0xf0, 0x8c, 0x8c),
    added_word_bg: Color32::from_rgb(0x2f, 0x6a, 0x3e),
    removed_word_bg: Color32::from_rgb(0x7a, 0x34, 0x3b),
    hunk_bg: Color32::from_rgb(0x2a, 0x30, 0x3c),
    error: Color32::from_rgb(0xff, 0x7a, 0x7a),
    link: Color32::from_rgb(0x7a, 0xb8, 0xff),
    field: Color32::from_rgb(0x1a, 0x1c, 0x21),
    lanes: [
        Color32::from_rgb(0x5a, 0xb0, 0xf0),
        Color32::from_rgb(0xe5, 0xb5, 0x67),
        Color32::from_rgb(0x9a, 0xd0, 0x6c),
        Color32::from_rgb(0xd0, 0x7c, 0xe0),
        Color32::from_rgb(0xf0, 0x80, 0x60),
        Color32::from_rgb(0x50, 0xd0, 0xc0),
        Color32::from_rgb(0xe0, 0x70, 0x9a),
        Color32::from_rgb(0xb0, 0xb0, 0xf0),
    ],
};

// Material Theme syntax colours (dark variants share them).
const M_RED: u32 = 0xff5370;
const M_GREEN: u32 = 0xc3e88d;
const M_YELLOW: u32 = 0xffcb6b;
const M_BLUE: u32 = 0x82aaff;
const M_PURPLE: u32 = 0xc792ea;
const M_CYAN: u32 = 0x89ddff;
const M_ORANGE: u32 = 0xf78c6c;

pub const THEMES: [Palette; 8] = [
    MARGIN,
    make(Base {
        id: "material",
        name: "Material",
        dark: true,
        bg: 0x263238,
        bg_dark: 0x1e272c,
        text: 0xeeffff,
        text_dim: 0x8a9ba6,
        accent: 0x80cbc4,
        red: M_RED,
        green: M_GREEN,
        yellow: M_YELLOW,
        blue: M_BLUE,
        purple: M_PURPLE,
        cyan: M_CYAN,
        orange: M_ORANGE,
    }),
    make(Base {
        id: "material-darker",
        name: "Material Darker",
        dark: true,
        bg: 0x212121,
        bg_dark: 0x1a1a1a,
        text: 0xeeffff,
        text_dim: 0x8c8c8c,
        accent: 0xff9800,
        red: M_RED,
        green: M_GREEN,
        yellow: M_YELLOW,
        blue: M_BLUE,
        purple: M_PURPLE,
        cyan: M_CYAN,
        orange: M_ORANGE,
    }),
    make(Base {
        id: "material-palenight",
        name: "Material Palenight",
        dark: true,
        bg: 0x292d3e,
        bg_dark: 0x202331,
        text: 0xbfc7d5,
        text_dim: 0x7f87ad,
        accent: 0xc792ea,
        red: M_RED,
        green: M_GREEN,
        yellow: M_YELLOW,
        blue: M_BLUE,
        purple: M_PURPLE,
        cyan: M_CYAN,
        orange: M_ORANGE,
    }),
    make(Base {
        id: "material-ocean",
        name: "Material Ocean",
        dark: true,
        bg: 0x0f111a,
        bg_dark: 0x090b10,
        text: 0xa6accd,
        text_dim: 0x5f6582,
        accent: 0x84ffff,
        red: M_RED,
        green: M_GREEN,
        yellow: M_YELLOW,
        blue: M_BLUE,
        purple: M_PURPLE,
        cyan: M_CYAN,
        orange: M_ORANGE,
    }),
    make(Base {
        id: "material-lighter",
        name: "Material Lighter",
        dark: false,
        bg: 0xfafafa,
        bg_dark: 0xeeeeee,
        text: 0x37474f,
        text_dim: 0x7b8f99,
        accent: 0x0097a7,
        red: 0xe53935,
        green: 0x6a9a35,
        yellow: 0xd68a00,
        blue: 0x4a6fb0,
        purple: 0x7c4dff,
        cyan: 0x2a9ca5,
        orange: 0xe5602f,
    }),
    make(Base {
        id: "solarized-dark",
        name: "Solarized Dark",
        dark: true,
        bg: 0x002b36,
        bg_dark: 0x00212b,
        text: 0x93a1a1,
        text_dim: 0x657b83,
        accent: 0xb58900,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        purple: 0x6c71c4,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
    }),
    make(Base {
        id: "solarized-light",
        name: "Solarized Light",
        dark: false,
        bg: 0xfdf6e3,
        bg_dark: 0xeee8d5,
        text: 0x586e75,
        text_dim: 0x93a1a1,
        accent: 0xb58900,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        purple: 0x6c71c4,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
    }),
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active palette.
#[inline]
pub fn cur() -> &'static Palette {
    &THEMES[CURRENT.load(Ordering::Relaxed).min(THEMES.len() - 1)]
}

/// Switch to the theme with this id (unknown ids fall back to Margin).
pub fn set(id: Option<&str>) {
    let i = id.and_then(|id| THEMES.iter().position(|t| t.id == id)).unwrap_or(0);
    CURRENT.store(i, Ordering::Relaxed);
}

macro_rules! roles {
    ($($name:ident => $field:ident),* $(,)?) => {
        $(
            #[allow(non_snake_case)]
            #[inline]
            pub fn $name() -> Color32 {
                cur().$field
            }
        )*
    };
}

roles! {
    BG => bg,
    BG_DARK => bg_dark,
    BG_ROW_ALT => bg_row_alt,
    SELECTED => selected,
    // Ahead/behind pill background.
    BADGE => badge,
    HOVER => hover,
    TEXT => text,
    TEXT_DIM => text_dim,
    ACCENT => accent,
    ADDED_BG => added_bg,
    REMOVED_BG => removed_bg,
    ADDED => added,
    REMOVED => removed,
    // Word-level changes inside a changed line (brighter than the line).
    ADDED_WORD_BG => added_word_bg,
    REMOVED_WORD_BG => removed_word_bg,
    HUNK_BG => hunk_bg,
    ERROR => error,
}

pub fn lane(i: u16) -> Color32 {
    let l = &cur().lanes;
    l[i as usize % l.len()]
}

/// Code and notes text size (f32 bits), set by `apply`.
static CONTENT: AtomicU32 = AtomicU32::new(0x4150_0000); // 13.0

/// Text size for code, notes and commit messages.
pub fn content_size() -> f32 {
    f32::from_bits(CONTENT.load(Ordering::Relaxed))
}

/// Make `ui` use the code / notes text size (for notes and commit messages;
/// diffs get it through the monospace style).
pub fn content_style(ui: &mut egui::Ui) {
    let f = content_size();
    let s = ui.style_mut();
    s.text_styles.insert(TextStyle::Body, FontId::proportional(f));
    s.text_styles.insert(TextStyle::Small, FontId::proportional(f - 2.0));
    s.text_styles.insert(TextStyle::Heading, FontId::proportional(f + 3.0));
    s.text_styles.insert(TextStyle::Monospace, FontId::monospace(f - 0.5));
}

/// Apply the current palette and text sizes to egui: `font_size` for the
/// interface, `content` for code (monospace) and, via `content_style`, notes.
pub fn apply(ctx: &egui::Context, font_size: f32, content: f32) {
    CONTENT.store(content.to_bits(), Ordering::Relaxed);
    let p = cur();
    let ec = |c: Color32| [c.r(), c.g(), c.b()];
    let shade = |t: u32| mix(ec(p.bg), ec(p.text), t);
    let theme = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(if p.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.bg_row_alt;
    v.code_bg_color = p.bg_dark;
    v.selection.bg_fill = p.selected;
    v.selection.stroke.color = p.text;
    v.hyperlink_color = p.link;
    v.override_text_color = Some(p.text);
    v.warn_fg_color = p.accent;
    v.error_fg_color = p.error;
    v.window_stroke.color = shade(160);
    // Buttons and other widgets: shades of the theme instead of egui's greys.
    v.widgets.noninteractive.bg_fill = p.bg;
    v.widgets.noninteractive.weak_bg_fill = p.bg;
    v.widgets.noninteractive.bg_stroke.color = shade(120);
    for (w, t) in [(&mut v.widgets.inactive, 100), (&mut v.widgets.hovered, 160), (&mut v.widgets.active, 220), (&mut v.widgets.open, 140)] {
        w.bg_fill = shade(t);
        w.weak_bg_fill = shade(t);
        w.fg_stroke.color = p.text;
    }
    v.widgets.inactive.bg_stroke.color = Color32::TRANSPARENT;
    v.widgets.hovered.bg_stroke.color = shade(260);
    v.widgets.active.bg_stroke.color = p.accent;
    ctx.set_visuals_of(theme, v);
    ctx.global_style_mut(|s| {
        let f = font_size;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(f));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(f));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(content - 0.5));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(f - 2.0));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(f + 3.0));
        s.spacing.item_spacing = egui::vec2(6.0, 3.0);
        s.spacing.button_padding = egui::vec2(7.0, 2.0);
        s.spacing.interact_size.y = f + 7.0;
    });
}
