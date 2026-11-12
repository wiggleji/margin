//! Native menu bar (macOS): Margin, File, Edit, Repository, Navigate, Window, Help.
//!
//! Items map to `Cmd`s that `MarginApp::menu_cmd` turns into the same actions
//! as buttons, shortcuts and the ⌘P palette. A menu item with a shortcut
//! receives that key before the window does, so on macOS the shortcuts below
//! reach the app only through the menu. The Edit items therefore re-inject
//! the matching egui events (copy, paste, undo…) for the focused text field.
//!
//! Other platforms have no menu bar; `MenuBar` is a no-op there.

/// What a menu item asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Cmd {
    // Margin
    Settings,
    // File
    OpenFolder,
    OpenRecent(String),
    SaveNote,
    CloseTab,
    Reveal,
    Terminal,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    FindCommits,
    // Repository
    Fetch,
    Pull,
    Push,
    Commit,
    Stash,
    NewBranch,
    /// Start a palette command by key (`Group/Title`).
    Palette(&'static str),
    Refresh,
    // Navigate
    CommandPalette,
    Back,
    Forward,
    WorkingDirectory,
    // Window
    NextTab,
    PrevTab,
    // Help
    Help,
    ReportIssue,
}

#[cfg(target_os = "macos")]
pub use mac::MenuBar;

#[cfg(not(target_os = "macos"))]
pub struct MenuBar;

#[cfg(not(target_os = "macos"))]
impl MenuBar {
    pub fn new(_ctx: &eframe::egui::Context) -> Self {
        MenuBar
    }
    pub fn update(&mut self, _recent: &[String], _has_repo: bool) {}
    pub fn poll(&mut self) -> Vec<Cmd> {
        vec![]
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Cmd;
    use eframe::egui;
    use muda::{AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator};
    use std::collections::HashMap;
    use std::sync::mpsc::{Receiver, channel};

    pub struct MenuBar {
        _menu: Menu,
        ids: HashMap<MenuId, Cmd>,
        rx: Receiver<MenuId>,
        recent: Submenu,
        recent_items: Vec<MenuItem>,
        /// Items that need an open repository.
        repo_items: Vec<MenuItem>,
        repo_menus: Vec<Submenu>,
        shown: Option<(Vec<String>, bool)>,
    }

    struct Builder {
        ids: HashMap<MenuId, Cmd>,
        repo_items: Vec<MenuItem>,
    }

    impl Builder {
        /// A menu item; `key` like "cmd+KeyR" (muda accelerator syntax).
        fn item(&mut self, title: &str, key: Option<&str>, cmd: Cmd) -> MenuItem {
            let accel = key.map(|k| k.parse::<Accelerator>().expect("valid accelerator"));
            let it = MenuItem::new(title, true, accel);
            self.ids.insert(it.id().clone(), cmd);
            it
        }
        /// Same, disabled while no repository is open.
        fn repo(&mut self, title: &str, key: Option<&str>, cmd: Cmd) -> MenuItem {
            let it = self.item(title, key, cmd);
            self.repo_items.push(it.clone());
            it
        }
    }

    fn sep() -> PredefinedMenuItem {
        PredefinedMenuItem::separator()
    }

    impl MenuBar {
        /// Install the menu bar. Must run on the main thread once the app has
        /// launched (eframe's app-creation callback qualifies).
        pub fn new(ctx: &egui::Context) -> Self {
            let mut b = Builder { ids: HashMap::new(), repo_items: vec![] };
            let menu = Menu::new();

            // --- Margin (application menu; macOS titles it with the app name)
            let about = AboutMetadata { name: Some("Margin".into()), version: Some(env!("CARGO_PKG_VERSION").into()), ..Default::default() };
            let app = Submenu::with_items(
                "Margin",
                true,
                &[
                    &PredefinedMenuItem::about(Some("About Margin"), Some(about)),
                    &sep(),
                    &b.item("Settings…", Some("cmd+Comma"), Cmd::Settings),
                    &sep(),
                    &PredefinedMenuItem::services(None),
                    &sep(),
                    &PredefinedMenuItem::hide(Some("Hide Margin")),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    &PredefinedMenuItem::quit(Some("Quit Margin")),
                ],
            )
            .expect("app menu");

            // --- File
            let recent = Submenu::new("Open Recent", true);
            let file = Submenu::with_items(
                "File",
                true,
                &[
                    &b.item("Open Folder…", Some("cmd+KeyO"), Cmd::OpenFolder),
                    &recent,
                    &sep(),
                    &b.repo("Save Note", Some("cmd+KeyS"), Cmd::SaveNote),
                    &sep(),
                    &b.repo("Reveal in Finder", None, Cmd::Reveal),
                    &b.repo("Open in Terminal", None, Cmd::Terminal),
                    &sep(),
                    &b.repo("Close Tab", Some("cmd+KeyW"), Cmd::CloseTab),
                ],
            )
            .expect("file menu");

            // --- Edit
            let edit = Submenu::with_items(
                "Edit",
                true,
                &[
                    &b.item("Undo", Some("cmd+KeyZ"), Cmd::Undo),
                    &b.item("Redo", Some("shift+cmd+KeyZ"), Cmd::Redo),
                    &sep(),
                    &b.item("Cut", Some("cmd+KeyX"), Cmd::Cut),
                    &b.item("Copy", Some("cmd+KeyC"), Cmd::Copy),
                    &b.item("Paste", Some("cmd+KeyV"), Cmd::Paste),
                    &b.item("Select All", Some("cmd+KeyA"), Cmd::SelectAll),
                    &sep(),
                    &b.repo("Find Commits…", Some("cmd+KeyF"), Cmd::FindCommits),
                ],
            )
            .expect("edit menu");

            // --- Repository
            let repository = Submenu::with_items(
                "Repository",
                true,
                &[
                    &b.repo("Fetch", None, Cmd::Fetch),
                    &b.repo("Pull", None, Cmd::Pull),
                    &b.repo("Push", None, Cmd::Push),
                    &sep(),
                    &b.repo("Commit…", None, Cmd::Commit),
                    &b.repo("Stash Changes", None, Cmd::Stash),
                    &sep(),
                    &b.repo("New Branch…", Some("shift+cmd+KeyN"), Cmd::NewBranch),
                    &b.repo("Checkout Branch…", None, Cmd::Palette("Branch/Checkout branch…")),
                    &b.repo("Merge Branch…", None, Cmd::Palette("Branch/Merge branch into current…")),
                    &b.repo("Rebase onto…", None, Cmd::Palette("Branch/Rebase current branch onto…")),
                    &b.repo("Create Tag…", None, Cmd::Palette("Tag/Create tag…")),
                    &sep(),
                    &b.repo("Add Worktree…", None, Cmd::Palette("Worktree/Add worktree for branch…")),
                    &b.repo("Git Flow…", None, Cmd::Palette("git flow")),
                    &sep(),
                    &b.repo("Refresh", Some("cmd+KeyR"), Cmd::Refresh),
                ],
            )
            .expect("repository menu");

            // --- Navigate
            let navigate = Submenu::with_items(
                "Navigate",
                true,
                &[
                    &b.item("Command Palette…", Some("cmd+KeyP"), Cmd::CommandPalette),
                    &sep(),
                    &b.repo("Back", Some("cmd+BracketLeft"), Cmd::Back),
                    &b.repo("Forward", Some("cmd+BracketRight"), Cmd::Forward),
                    &sep(),
                    &b.repo("Working Directory", None, Cmd::WorkingDirectory),
                    &b.repo("Go to Branch…", None, Cmd::Palette("Go to/Go to branch…")),
                    &b.repo("Go to Tag…", None, Cmd::Palette("Go to/Go to tag…")),
                    &b.repo("Go to Commit…", None, Cmd::Palette("Go to/Go to commit…")),
                ],
            )
            .expect("navigate menu");

            // --- Window
            let window = Submenu::with_items(
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                    &PredefinedMenuItem::fullscreen(None),
                    &sep(),
                    &b.item("Show Next Tab", Some("ctrl+Tab"), Cmd::NextTab),
                    &b.item("Show Previous Tab", Some("ctrl+shift+Tab"), Cmd::PrevTab),
                    &sep(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ],
            )
            .expect("window menu");

            // --- Help
            let help = Submenu::with_items(
                "Help",
                true,
                &[&b.item("Margin Help", None, Cmd::Help), &b.item("Report an Issue…", None, Cmd::ReportIssue)],
            )
            .expect("help menu");

            menu.append_items(&[&app, &file, &edit, &repository, &navigate, &window, &help]).expect("menu bar");
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help.set_as_help_menu_for_nsapp();

            // Menu clicks arrive outside egui's event loop: forward and wake it.
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let _ = tx.send(e.id);
                ctx.request_repaint();
            }));

            MenuBar {
                _menu: menu,
                ids: b.ids,
                rx,
                recent,
                recent_items: vec![],
                repo_items: b.repo_items,
                repo_menus: vec![repository],
                shown: None,
            }
        }

        /// Keep "Open Recent" and the enabled state in sync (cheap when unchanged).
        pub fn update(&mut self, recent: &[String], has_repo: bool) {
            let now = (recent.to_vec(), has_repo);
            if self.shown.as_ref() == Some(&now) {
                return;
            }
            for it in self.recent_items.drain(..) {
                self.ids.remove(it.id());
                let _ = self.recent.remove(&it);
            }
            for p in recent {
                let it = MenuItem::new(crate::app::tilde(std::path::Path::new(p)), true, None);
                self.ids.insert(it.id().clone(), Cmd::OpenRecent(p.clone()));
                let _ = self.recent.append(&it);
                self.recent_items.push(it);
            }
            self.recent.set_enabled(!recent.is_empty());
            for it in &self.repo_items {
                it.set_enabled(has_repo);
            }
            for m in &self.repo_menus {
                m.set_enabled(has_repo);
            }
            self.shown = Some(now);
        }

        /// Menu commands chosen since the last frame.
        pub fn poll(&mut self) -> Vec<Cmd> {
            self.rx.try_iter().filter_map(|id| self.ids.get(&id).cloned()).collect()
        }
    }
}
