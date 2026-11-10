#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commit_list;
mod icons;
mod details;
mod diff_view;
mod jobs;
mod markdown_view;
mod notes_panel;
mod pr_view;
mod sidebar;
mod theme;

use std::path::PathBuf;

fn main() -> eframe::Result {
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
