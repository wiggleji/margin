//! Small reusable controls with a clear "this is clickable" look:
//! a segmented control (tabs / modes) and painted icon buttons.

use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

/// One segment: optional icon, text, and optional trailing marker
/// (an icon + optional text in a colour, e.g. a note icon or a PR number).
pub struct Seg {
    pub icon: Option<Icon>,
    /// A painted glyph instead of an octicon (e.g. the diff layouts).
    pub glyph: Option<Glyph>,
    pub text: String,
    pub marks: Vec<(Icon, Option<String>, Color32)>,
    pub tip: String,
}

impl Seg {
    pub fn new(icon: Option<Icon>, text: impl Into<String>) -> Self {
        Seg { icon, glyph: None, text: text.into(), marks: vec![], tip: String::new() }
    }
    /// An icon-only segment drawn with `glyph`.
    pub fn glyph(glyph: Glyph) -> Self {
        Seg { icon: None, glyph: Some(glyph), text: String::new(), marks: vec![], tip: String::new() }
    }
    pub fn mark(mut self, icon: Icon, text: Option<String>, color: Color32) -> Self {
        self.marks.push((icon, text, color));
        self
    }
    pub fn tip(mut self, tip: impl Into<String>) -> Self {
        self.tip = tip.into();
        self
    }
}

/// A row of segments in a rounded track; the selected one is filled.
/// Returns the index clicked this frame.
pub fn segmented(ui: &mut Ui, segs: &[Seg], selected: Option<usize>) -> Option<usize> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let isz = (font.size + 1.0).round();
    let h = ui.spacing().interact_size.y + 2.0;
    let pad = 9.0;
    let gap = 5.0;
    // measure
    let widths: Vec<f32> = segs
        .iter()
        .map(|s| {
            let tw = if s.text.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(s.text.clone(), font.clone(), theme::TEXT()).size().x };
            let mut w = pad * 2.0 + tw;
            if s.icon.is_some() || s.glyph.is_some() {
                w += isz + if tw > 0.0 { gap } else { 0.0 };
            }
            for (_, t, _) in &s.marks {
                w += gap + isz;
                if let Some(t) = t {
                    w += 2.0 + ui.painter().layout_no_wrap(t.clone(), font.clone(), theme::TEXT()).size().x;
                }
            }
            w
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (track, _) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(track, 7.0, theme::BG_DARK());
    p.rect_stroke(track, 7.0, Stroke::new(1.0, theme::HOVER()), egui::StrokeKind::Inside);
    let mut x = track.left() + 2.0;
    let mut clicked = None;
    for (i, (s, w)) in segs.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, track.top() + 2.0), vec2(*w, h - 4.0));
        let resp = ui.interact(r, ui.scope_id().with(("seg", i, &s.text)), Sense::click());
        let on = selected == Some(i);
        let p = ui.painter();
        if on {
            p.rect_filled(r, 5.0, theme::SELECTED());
        } else if resp.hovered() {
            p.rect_filled(r, 5.0, theme::HOVER());
        }
        let color = if on || resp.hovered() { theme::TEXT() } else { theme::TEXT_DIM() };
        let mut cx = r.left() + pad;
        let cy = r.center().y;
        if let Some(ic) = s.icon {
            icons::paint(p, ic, Rect::from_center_size(pos2(cx + isz / 2.0, cy), vec2(isz, isz)), color);
            cx += isz + gap;
        } else if let Some(g) = s.glyph {
            paint_glyph(p, g, Rect::from_center_size(pos2(cx + isz / 2.0, cy), vec2(isz, isz)), color);
            cx += isz + gap;
        }
        let g = p.layout_no_wrap(s.text.clone(), font.clone(), color);
        let gw = g.size().x;
        p.galley(pos2(cx, cy - g.size().y / 2.0), g, color);
        cx += gw;
        for (ic, t, c) in &s.marks {
            cx += gap;
            icons::paint(p, *ic, Rect::from_center_size(pos2(cx + isz / 2.0, cy), vec2(isz, isz)), *c);
            cx += isz;
            if let Some(t) = t {
                cx += 2.0;
                let g = p.layout_no_wrap(t.clone(), font.clone(), *c);
                let tw = g.size().x;
                p.galley(pos2(cx, cy - g.size().y / 2.0), g, *c);
                cx += tw;
            }
        }
        let resp = if s.tip.is_empty() { resp } else { resp.on_hover_text(&s.tip) };
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

/// Shapes for `icon_button`.
#[derive(Clone, Copy)]
pub enum Glyph {
    /// ▾ / ▸
    Chevron { open: bool },
    /// A panel moving to the left / right.
    Dock { to_right: bool },
    Trash,
    /// Diff layouts: three full lines / two columns of lines.
    Inline,
    SideBySide,
    /// Focus mode: corner brackets out (off) / in (on).
    Focus { on: bool },
    ArrowUp,
    ArrowDown,
}

/// A square icon button (full button height, centred), with a hover background.
pub fn icon_button(ui: &mut Ui, glyph: Glyph, tip: &str) -> egui::Response {
    let s = ui.spacing().interact_size.y + 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click());
    let hot = resp.hovered();
    let p = ui.painter();
    if hot {
        p.rect_filled(rect, 5.0, theme::HOVER());
    }
    let c = match glyph {
        Glyph::Trash if hot => theme::ERROR(),
        _ if hot => theme::TEXT(),
        _ => theme::TEXT_DIM(),
    };
    paint_glyph(p, glyph, rect, c);
    resp.on_hover_text(tip)
}

/// A filled accent button for the main action in a spot ("Write note").
pub fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let fg = if theme::cur().dark { theme::BG_DARK() } else { Color32::WHITE };
    ui.add(egui::Button::new(egui::RichText::new(text).color(fg).strong()).fill(theme::ACCENT()).corner_radius(5.0))
}

/// Draw `glyph` centred in `rect` in colour `c`.
pub fn paint_glyph(p: &egui::Painter, glyph: Glyph, rect: Rect, c: Color32) {
    let st = Stroke::new(1.5, c);
    let m = rect.center();
    match glyph {
        Glyph::Chevron { open } => {
            let pts = if open {
                vec![m + vec2(-4.5, -2.0), m + vec2(0.0, 2.5), m + vec2(4.5, -2.0)]
            } else {
                vec![m + vec2(-2.0, -4.5), m + vec2(2.5, 0.0), m + vec2(-2.0, 4.5)]
            };
            p.add(egui::Shape::line(pts, st));
        }
        Glyph::Dock { to_right } => {
            // a window with a bar on one side, and an arrow towards the other
            let b = Rect::from_center_size(m, vec2(14.0, 11.0));
            p.rect_stroke(b, 2.0, Stroke::new(1.2, c), egui::StrokeKind::Inside);
            let bar_x = if to_right { b.right() - 4.0 } else { b.left() + 4.0 };
            p.line_segment([pos2(bar_x, b.top()), pos2(bar_x, b.bottom())], Stroke::new(1.2, c));
            let (from, to) = if to_right { (b.left() + 2.5, bar_x - 2.0) } else { (b.right() - 2.5, bar_x + 2.0) };
            let y = m.y;
            p.line_segment([pos2(from, y), pos2(to, y)], Stroke::new(1.2, c));
            let d = if to_right { -2.5 } else { 2.5 };
            p.line_segment([pos2(to, y), pos2(to + d, y - 2.5)], Stroke::new(1.2, c));
            p.line_segment([pos2(to, y), pos2(to + d, y + 2.5)], Stroke::new(1.2, c));
        }
        Glyph::Inline | Glyph::SideBySide => {
            let b = Rect::from_center_size(m, vec2(14.0, 11.0));
            let thin = Stroke::new(1.0, c);
            p.rect_stroke(b, 1.5, Stroke::new(1.2, c), egui::StrokeKind::Inside);
            let ys = [b.top() + 3.5, b.center().y, b.bottom() - 3.5];
            if matches!(glyph, Glyph::SideBySide) {
                p.line_segment([pos2(b.center().x, b.top()), pos2(b.center().x, b.bottom())], Stroke::new(1.2, c));
                for y in ys {
                    p.line_segment([pos2(b.left() + 2.5, y), pos2(b.center().x - 2.0, y)], thin);
                    p.line_segment([pos2(b.center().x + 2.0, y), pos2(b.right() - 2.5, y)], thin);
                }
            } else {
                for y in ys {
                    p.line_segment([pos2(b.left() + 2.5, y), pos2(b.right() - 2.5, y)], thin);
                }
            }
        }
        Glyph::Focus { on } => {
            let b = Rect::from_center_size(m, vec2(12.0, 10.0));
            let l = 3.5;
            for (pt, dx, dy) in [(b.left_top(), 1.0, 1.0), (b.right_top(), -1.0, 1.0), (b.left_bottom(), 1.0, -1.0), (b.right_bottom(), -1.0, -1.0)] {
                let pt = if on { pt + vec2(dx * 3.0, dy * 2.5) } else { pt };
                let (hx, vy) = if on { (-dx, -dy) } else { (dx, dy) };
                p.line_segment([pt, pt + vec2(hx * l, 0.0)], st);
                p.line_segment([pt, pt + vec2(0.0, vy * l)], st);
            }
        }
        Glyph::ArrowUp | Glyph::ArrowDown => {
            let d = if matches!(glyph, Glyph::ArrowUp) { -1.0 } else { 1.0 };
            p.line_segment([m + vec2(0.0, -5.0 * d), m + vec2(0.0, 5.0 * d)], st);
            p.add(egui::Shape::line(vec![m + vec2(-4.0, 1.0 * d), m + vec2(0.0, 5.0 * d), m + vec2(4.0, 1.0 * d)], st));
        }
        Glyph::Trash => {
            p.line_segment([m + vec2(-5.0, -3.5), m + vec2(5.0, -3.5)], st);
            p.line_segment([m + vec2(-1.5, -5.0), m + vec2(1.5, -5.0)], st);
            let can = vec![m + vec2(-3.8, -3.5), m + vec2(-3.0, 5.0), m + vec2(3.0, 5.0), m + vec2(3.8, -3.5)];
            p.add(egui::Shape::line(can, st));
            p.line_segment([m + vec2(-1.0, -1.5), m + vec2(-1.0, 3.0)], Stroke::new(1.0, c));
            p.line_segment([m + vec2(1.0, -1.5), m + vec2(1.0, 3.0)], Stroke::new(1.0, c));
        }
    }
}

/// An on/off chip in the segmented style (filled when on).
pub fn toggle_chip(ui: &mut Ui, on: bool, text: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let g = ui.painter().layout_no_wrap(text.to_string(), font, theme::TEXT());
    let h = ui.spacing().interact_size.y + 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, h), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 7.0, theme::BG_DARK());
    p.rect_stroke(rect, 7.0, Stroke::new(1.0, theme::HOVER()), egui::StrokeKind::Inside);
    let inner = rect.shrink(2.0);
    if on {
        p.rect_filled(inner, 5.0, theme::SELECTED());
    } else if resp.hovered() {
        p.rect_filled(inner, 5.0, theme::HOVER());
    }
    let color = if on || resp.hovered() { theme::TEXT() } else { theme::TEXT_DIM() };
    let g = p.layout_no_wrap(text.to_string(), egui::TextStyle::Button.resolve(ui.style()), color);
    p.galley(pos2(rect.center().x - g.size().x / 2.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}
