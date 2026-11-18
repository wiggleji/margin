#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branch_tree;
mod commands;
mod commit_list;
mod icons;
mod details;
mod diff_view;
mod emoji;
mod fonts;
mod jobs;
mod markdown_view;
mod menu_bar;
mod note_format;
mod notes_panel;
mod palette;
mod search;
mod path_elide;
mod pr_view;
mod sidebar;
mod tab_strip;
mod theme;
mod widgets;

use std::path::PathBuf;

/// Apps opened from Finder/Dock get a minimal PATH (/usr/bin:/bin:…), so
/// Homebrew's git (often newer than Apple's), gh and credential helpers would
/// be missed. Prepend the standard Homebrew locations, like a login shell.
fn extend_path_for_gui_launch() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = current.split(':').filter(|p| !p.is_empty()).map(String::from).collect();
    for dir in ["/usr/local/bin", "/opt/homebrew/bin"] {
        if !parts.iter().any(|p| p == dir) && std::path::Path::new(dir).is_dir() {
            parts.insert(0, dir.to_string());
        }
    }
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe { std::env::set_var("PATH", parts.join(":")) };
}

fn main() -> eframe::Result {
    extend_path_for_gui_launch();
    let mut repo: Option<PathBuf> = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--version" => {
                println!("margin {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                println!("usage: margin [path/to/repo]");
                return Ok(());
            }
            p => repo = Some(PathBuf::from(p)),
        }
    }
    // `margin` with no args inside a repo opens it, like `smerge .`
    let repo = repo.or_else(|| std::env::current_dir().ok());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Margin")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Margin",
        options,
        Box::new(move |cc| Ok(Box::new(app::MarginApp::new(&cc.egui_ctx, repo)))),
    )
}
