//! What the window shows: the main panels and the sidebar's sections, each
//! shown or hidden from the View menu (macOS menu bar; the toolbar's View
//! button elsewhere), ⌘P, or ⌘⌥1…4. Remembered in state.json.

use margin_core::config::State;

/// A main panel: id, menu label, and its ⌘⌥ digit.
pub struct Pane {
    pub id: &'static str,
    pub label: &'static str,
    pub digit: u8,
}

pub const PANES: [Pane; 4] = [
    Pane { id: "sidebar", label: "Sidebar", digit: 1 },
    Pane { id: "history", label: "History", digit: 2 },
    Pane { id: "details", label: "Details and diff", digit: 3 },
    Pane { id: "notes", label: "Notes", digit: 4 },
];

/// Sidebar sections that can be hidden: (section id, label). The view id is
/// `section:<id>`.
pub const SECTIONS: [(&str, &str); 7] = [
    ("branches", "Branches"),
    ("pulls", "Pull requests"),
    ("worktrees", "Worktrees"),
    ("remotes", "Remotes"),
    ("tags", "Tags"),
    ("stashes", "Stashes"),
    ("mytasks", "My tasks"),
];

pub fn section_id(section: &str) -> String {
    format!("section:{section}")
}

pub fn shown(state: &State, id: &str) -> bool {
    !state.hidden_views.iter().any(|h| h == id)
}

/// Show `id` if it's hidden, hide it if it's shown. The history and the
/// details never both disappear: hiding one brings the other back.
pub fn toggle(state: &mut State, id: &str) {
    if shown(state, id) {
        state.hidden_views.push(id.to_string());
        let other = match id {
            "history" => Some("details"),
            "details" => Some("history"),
            _ => None,
        };
        if let Some(o) = other {
            state.hidden_views.retain(|h| h != o);
        }
    } else {
        state.hidden_views.retain(|h| h != id);
    }
}

/// The ⌘⌥ shortcut label for a panel ("⌥⌘1" / "Ctrl+Alt+1").
pub fn shortcut(digit: u8) -> String {
    if cfg!(target_os = "macos") { format!("⌥⌘{digit}") } else { format!("Ctrl+Alt+{digit}") }
}

/// The View menu's items for an egui menu (the toolbar's View button).
pub fn menu(ui: &mut eframe::egui::Ui, state: &State, focus: bool, actions: &mut Vec<crate::app::Action>) {
    let mut f = focus;
    let key = if cfg!(target_os = "macos") { "⇧⌘↩" } else { "Ctrl+Shift+Enter" };
    if ui.checkbox(&mut f, "Focus on diff").on_hover_text(format!("The diff takes the whole window ({key})")).clicked() {
        actions.push(crate::app::Action::ToggleFocusDiff);
    }
    ui.separator();
    for p in &PANES {
        let mut on = shown(state, p.id);
        // (hidden panels stay hidden in focus mode; the boxes still say what returns)
        if ui.add(eframe::egui::Checkbox::new(&mut on, p.label)).on_hover_text(shortcut(p.digit)).clicked() {
            actions.push(crate::app::Action::ToggleView(p.id.to_string()));
        }
    }
    ui.separator();
    ui.weak("Sidebar sections");
    for (s, label) in SECTIONS {
        let id = section_id(s);
        let mut on = shown(state, &id);
        if ui.checkbox(&mut on, label).clicked() {
            actions.push(crate::app::Action::ToggleView(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_and_details_never_both_hidden() {
        let mut s = State::default();
        toggle(&mut s, "sidebar");
        assert!(!shown(&s, "sidebar"));
        toggle(&mut s, "history");
        assert!(!shown(&s, "history") && shown(&s, "details"));
        toggle(&mut s, "details");
        assert!(!shown(&s, "details") && shown(&s, "history"), "hiding details brings the history back");
        toggle(&mut s, "sidebar");
        assert!(shown(&s, "sidebar"));
    }
}
