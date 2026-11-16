//! Minimal Markdown renderer for egui (no images, no HTML — matching the
//! note policy). ~200 lines instead of a rendering dependency.
//! Task-list checkboxes are clickable and toggle the source text.

use crate::theme;
use eframe::egui::{self, Label, RichText, Sense, Ui};
use margin_core::markdown;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

pub enum MdAction {
    Link(String),
    /// Byte offset of the char inside `[ ]` / `[x]` to flip.
    ToggleTask { offset: usize, checked: bool },
}

#[derive(Default, Clone)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

enum Prefix {
    Bullet,
    Number(u64),
    Task { checked: bool, offset: usize },
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
}

struct Ctx<'a> {
    out: &'a mut Option<MdAction>,
    base: f32,
}

pub fn render(ui: &mut Ui, src: &str) -> Option<MdAction> {
    let mut out = None;
    let base = ui.style().text_styles[&egui::TextStyle::Body].size;
    let mut cx = Ctx { out: &mut out, base };

    let mut runs: Vec<Run> = Vec::new();
    let mut st = Run::default();
    let (mut bold, mut italic, mut strike) = (0, 0, 0);
    let mut heading: Option<u8> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut prefix: Option<Prefix> = None;
    let mut quote = 0usize;
    let mut code: Option<String> = None;
    let mut table: Option<Table> = None;

    for (ev, range) in Parser::new_ext(src, markdown::options()).into_offset_iter() {
        let indent = lists.len().saturating_sub(1) as f32 * 16.0 + quote as f32 * 12.0;
        match ev {
            Event::Start(Tag::Heading { level, .. }) => heading = Some(level as u8),
            Event::End(TagEnd::Heading(_)) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                ui.add_space(4.0);
            }
            Event::Start(Tag::BlockQuote(_)) => quote += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    egui::Frame::new()
                        .fill(theme::BG_DARK())
                        .inner_margin(6.0)
                        .corner_radius(3.0)
                        .show(ui, |ui| {
                            ui.add(Label::new(RichText::new(c.trim_end()).monospace()).wrap_mode(egui::TextWrapMode::Extend));
                        });
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::List(start)) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                lists.pop();
                if lists.is_empty() {
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::Item) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                prefix = Some(match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        Prefix::Number(*n - 1)
                    }
                    _ => Prefix::Bullet,
                });
            }
            Event::End(TagEnd::Item) => flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote),
            Event::TaskListMarker(checked) => prefix = Some(Prefix::Task { checked, offset: range.start + 1 }),
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold -= 1,
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic -= 1,
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike -= 1,
            Event::Start(Tag::Link { dest_url, .. }) => st.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => st.link = None,
            Event::Start(Tag::Image { .. }) => runs.push(Run { text: "[image not allowed]".into(), italic: true, ..Default::default() }),
            Event::Start(Tag::Table(_)) => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                table = Some(Table::default());
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = table.as_mut() {
                    t.row.push(std::mem::take(&mut t.cell));
                }
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.rows.push(std::mem::take(&mut t.row));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    draw_table(ui, &t);
                }
            }
            Event::Text(t) | Event::Code(t) if code.is_some() || table.is_some() => {
                if let Some(c) = code.as_mut() {
                    c.push_str(&t);
                } else if let Some(tb) = table.as_mut() {
                    tb.cell.push_str(&t);
                }
            }
            Event::Text(t) => runs.push(Run {
                text: t.to_string(),
                bold: bold > 0,
                italic: italic > 0,
                strike: strike > 0,
                link: st.link.clone(),
                ..Default::default()
            }),
            Event::Code(t) => runs.push(Run { text: t.to_string(), code: true, link: st.link.clone(), ..Default::default() }),
            Event::SoftBreak => runs.push(Run { text: " ".into(), ..Default::default() }),
            Event::HardBreak => flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote),
            Event::Rule => {
                flush(ui, &mut cx, &mut runs, prefix.take(), indent, heading, quote);
                ui.separator();
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                runs.push(Run { text: h.to_string(), code: true, ..Default::default() })
            }
            _ => {}
        }
    }
    flush(ui, &mut cx, &mut runs, prefix.take(), 0.0, None, 0);
    out
}

fn flush(ui: &mut Ui, cx: &mut Ctx, runs: &mut Vec<Run>, prefix: Option<Prefix>, indent: f32, heading: Option<u8>, quote: usize) {
    if runs.is_empty() && prefix.is_none() {
        return;
    }
    let size = heading.map(|h| cx.base + match h {
        1 => 7.0,
        2 => 4.0,
        3 => 2.0,
        _ => 1.0,
    });
    if heading.is_some() {
        ui.add_space(4.0);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent);
        if quote > 0 {
            ui.label(RichText::new("│ ").color(theme::TEXT_DIM()));
        }
        match prefix {
            Some(Prefix::Bullet) => {
                ui.label("•  ");
            }
            Some(Prefix::Number(n)) => {
                ui.label(format!("{n}. "));
            }
            Some(Prefix::Task { checked, offset }) => {
                let mut c = checked;
                if ui.checkbox(&mut c, "").changed() {
                    *cx.out = Some(MdAction::ToggleTask { offset, checked });
                }
            }
            None => {}
        }
        for r in runs.drain(..) {
            let mut rt = RichText::new(r.text);
            if let Some(s) = size {
                rt = rt.size(s).strong();
            }
            if r.bold {
                rt = rt.strong();
            }
            if r.italic {
                rt = rt.italics();
            }
            if r.strike {
                rt = rt.strikethrough();
            }
            if r.code {
                rt = rt.code();
            }
            if quote > 0 {
                rt = rt.color(theme::TEXT_DIM());
            }
            match r.link {
                Some(url) => {
                    let ok = markdown::link_allowed(&url);
                    let color = if ok { ui.visuals().hyperlink_color } else { theme::ERROR() };
                    let resp = ui
                        .add(Label::new(rt.color(color).underline()).sense(Sense::click()))
                        .on_hover_text(&url);
                    if ok && resp.clicked() {
                        *cx.out = Some(MdAction::Link(url));
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                None => {
                    ui.label(rt);
                }
            }
        }
    });
}

fn draw_table(ui: &mut Ui, t: &Table) {
    egui::Grid::new(ui.next_auto_id()).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
        for (i, row) in t.rows.iter().enumerate() {
            for cell in row {
                if i == 0 {
                    ui.label(RichText::new(cell).strong());
                } else {
                    ui.label(cell);
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
}

/// Flip a task checkbox in the source.
pub fn toggle_task(src: &mut String, offset: usize, checked: bool) {
    if offset < src.len() && src.is_char_boundary(offset) && src.is_char_boundary(offset + 1) {
        src.replace_range(offset..offset + 1, if checked { " " } else { "x" });
    }
}
