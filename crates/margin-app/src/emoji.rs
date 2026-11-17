//! GitHub emoji shortcodes: `:tada:` → 🎉 when showing comments, and a
//! dropdown while typing `:ta…` in a text field.
//!
//! Use the dropdown around a `TextEdit`: [`keys`] before adding it (it takes
//! ↑/↓/↵/Tab/Esc while the dropdown is open), [`dropdown`] after.

use crate::theme;
use eframe::egui::{self, Id, Key, Modifiers, Rect, Ui};
use std::borrow::Cow;

/// Replace known `:shortcode:`s with their emoji.
pub fn render(s: &str) -> Cow<'_, str> {
    if !s.contains(':') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find(':') {
        let after = &rest[a + 1..];
        let Some(b) = after.find(':') else { break };
        let code = &after[..b];
        match emojis::get_by_shortcode(code).filter(|_| !code.is_empty() && code.chars().all(is_code_char)) {
            Some(e) => {
                out.push_str(&rest[..a]);
                out.push_str(e.as_str());
                rest = &after[b + 1..];
            }
            None => {
                // not an emoji: keep this ':' and look again from the next one
                out.push_str(&rest[..a + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn is_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// The `:partial` shortcode just before char index `cursor`: (char index of
/// the ':', the partial text). The ':' must start the text or follow a space
/// (so `12:30` and URLs don't open the dropdown).
pub fn token_at(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().take(cursor).collect();
    let mut i = chars.len();
    while i > 0 && is_code_char(chars[i - 1]) {
        i -= 1;
    }
    let partial: String = chars[i..].iter().collect();
    if partial.is_empty() || i == 0 || chars[i - 1] != ':' {
        return None;
    }
    let colon = i - 1;
    if colon > 0 && !chars[colon - 1].is_whitespace() && !"([{\"'".contains(chars[colon - 1]) {
        return None;
    }
    Some((colon, partial.to_lowercase()))
}

/// Up to `n` emoji whose shortcode starts with `q` (then ones containing it):
/// (emoji, shortcode).
pub fn search(q: &str, n: usize) -> Vec<(&'static str, &'static str)> {
    let mut starts = vec![];
    let mut contains = vec![];
    for e in emojis::iter() {
        for code in e.shortcodes() {
            if code.starts_with(q) {
                starts.push((e.as_str(), code));
            } else if code.contains(q) {
                contains.push((e.as_str(), code));
            }
        }
    }
    // shortest first: ":smile" → smile before smile_cat
    starts.sort_by_key(|(_, c)| c.len());
    starts.extend(contains);
    starts.truncate(n);
    starts
}

#[derive(Clone, Default)]
struct State {
    sel: usize,
    /// Esc pressed for the token starting here (until it changes).
    hidden_at: Option<usize>,
}

/// What [`keys`] found: the token and its matches, and a pick made by key.
pub struct Open {
    colon: usize,
    cursor: usize,
    list: Vec<(&'static str, &'static str)>,
    sel: usize,
    accept: Option<usize>,
}

/// Before adding the `TextEdit` with this `id`: if the dropdown is open,
/// take the keys it uses. Returns the open dropdown (if any) for [`dropdown`].
pub fn keys(ui: &mut Ui, id: Id, text: &str) -> Option<Open> {
    if !ui.memory(|m| m.has_focus(id)) {
        return None;
    }
    let cursor = egui::text_edit::TextEditState::load(ui.ctx(), id)?.cursor.char_range()?;
    if cursor.primary != cursor.secondary {
        return None;
    }
    let cursor: usize = cursor.primary.index.into();
    let (colon, q) = token_at(text, cursor)?;
    let sid = id.with("emoji");
    let mut st: State = ui.data(|d| d.get_temp(sid)).unwrap_or_default();
    if st.hidden_at == Some(colon) {
        return None;
    }
    st.hidden_at = None;
    let list = search(&q, 8);
    if list.is_empty() {
        return None;
    }
    let n = list.len();
    st.sel = st.sel.min(n - 1);
    let mut accept = None;
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            st.sel = (st.sel + 1) % n;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            st.sel = (st.sel + n - 1) % n;
        }
        if i.consume_key(Modifiers::NONE, Key::Tab) || i.consume_key(Modifiers::NONE, Key::Enter) {
            accept = Some(st.sel);
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            st.hidden_at = Some(colon);
        }
    });
    let hidden = st.hidden_at.is_some();
    let sel = st.sel;
    ui.data_mut(|d| d.insert_temp(sid, st));
    if hidden {
        return None;
    }
    Some(Open { colon, cursor, list, sel, accept })
}

/// After adding the `TextEdit` (`rect` is where it is): show the dropdown
/// under it and insert the chosen emoji. Returns true if `text` changed.
pub fn dropdown(ui: &mut Ui, id: Id, rect: Rect, text: &mut String, open: Option<Open>) -> bool {
    let Some(open) = open else {
        return false;
    };
    let sid = id.with("emoji");
    let mut accept = open.accept;
    let mut hover = None;
    egui::Area::new(id.with("emoji-dropdown"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(240.0_f32.min(rect.width()));
                ui.style_mut().interaction.selectable_labels = false;
                for (k, (e, code)) in open.list.iter().enumerate() {
                    let r = ui.add(
                        egui::Button::selectable(
                            k == open.sel,
                            egui::Atoms::new((
                                egui::RichText::new(*e).size(16.0),
                                egui::RichText::new(format!(":{code}:")).color(theme::TEXT()),
                                egui::Atom::grow(),
                            )),
                        )
                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                    );
                    if r.hovered() {
                        hover = Some(k);
                    }
                    if r.clicked() {
                        accept = Some(k);
                    }
                }
            });
        });
    if let Some(k) = hover {
        ui.data_mut(|d| d.get_temp_mut_or_default::<State>(sid).sel = k);
    }
    let Some(k) = accept else {
        return false;
    };
    // replace ":partial" with the emoji, caret after it
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (a, b) = (byte(open.colon), byte(open.cursor));
    let e = open.list[k].0;
    text.replace_range(a..b, e);
    let caret = open.colon + e.chars().count();
    if let Some(mut st) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(caret))));
        st.store(ui.ctx(), id);
    }
    ui.data_mut(|d| d.insert_temp(sid, State::default()));
    ui.memory_mut(|m| m.request_focus(id));
    true
}

#[cfg(test)]
mod tests {
    use super::{render, search, token_at};

    #[test]
    fn renders_shortcodes() {
        assert_eq!(render("ship it :rocket: :+1:"), "ship it 🚀 👍");
        // unknown codes, times and lone colons stay
        assert_eq!(render("at 12:30: see :nope: ok"), "at 12:30: see :nope: ok");
        assert_eq!(render("a :x :tada:"), "a :x 🎉");
        assert_eq!(render("plain"), "plain");
    }

    #[test]
    fn tokens() {
        assert_eq!(token_at(":ta", 3), Some((0, "ta".into())));
        assert_eq!(token_at("nice :Sm", 8), Some((5, "sm".into())));
        assert_eq!(token_at("at 12:3", 7), None); // not after a space
        assert_eq!(token_at("x :", 3), None); // nothing typed yet
        assert_eq!(token_at("a :tada: b", 10), None);
        // cursor in the middle of the text
        assert_eq!(token_at("hi :ro and", 6), Some((3, "ro".into())));
    }

    #[test]
    fn searches() {
        let r = search("tad", 8);
        assert_eq!(r[0], ("🎉", "tada"));
        assert!(search("smile", 8).iter().any(|(_, c)| *c == "smile"));
        assert!(search("zzzz", 8).is_empty());
    }
}
