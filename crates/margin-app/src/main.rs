#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branch_tree;
mod cli;
mod commands;
mod commit_list;
mod conflicts;
mod icons;
mod insights;
mod details;
mod diff_view;
mod emoji;
mod file_view;
mod fonts;
mod jobs;
mod markdown_view;
mod mention;
mod menu_bar;
mod note_format;
mod notes_panel;
mod palette;
mod search;
mod path_elide;
mod platform;
mod pr_view;
mod rebase;
mod review;
mod sidebar;
mod syntax;
mod tab_strip;
mod view;
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
    // `margin <dir>` while Margin runs: open it there as a tab, and exit.
    if let Some(dir) = &repo {
        let dir = margin_core::platform::canonical(dir);
        if cli::hand_over(&dir) {
            return Ok(());
        }
    }
    // `margin` with no args inside a repo opens it, like `smerge .`
    let repo = repo.or_else(|| std::env::current_dir().ok());

    // Without an icon here eframe puts its own logo on the window and, on
    // macOS, over the app bundle's Dock icon.
    let mut viewport = eframe::egui::ViewportBuilder::default();
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: viewport
            .with_title("Margin")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    // Windows: OpenGL by default (fast to start); Direct3D when OpenGL can't
    // start (see `retry_with_direct3d`), or with MARGIN_RENDERER=wgpu.
    #[cfg(windows)]
    if renderer_override().as_deref() == Some("wgpu") {
        options.renderer = eframe::Renderer::Wgpu;
    }
    let result = eframe::run_native(
        "Margin",
        options,
        Box::new(move |cc| Ok(Box::new(app::MarginApp::new(&cc.egui_ctx, repo)))),
    );
    #[cfg(windows)]
    if let Err(e) = &result {
        return windows_fallback(e);
    }
    result
}

/// MARGIN_RENDERER: `glow` (OpenGL) or `wgpu` (Direct3D on Windows).
#[cfg(windows)]
fn renderer_override() -> Option<String> {
    std::env::var("MARGIN_RENDERER").ok().map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty())
}

/// The window couldn't start. Without OpenGL 2 or later (a virtual machine,
/// some Remote Desktop sessions, no graphics driver) OpenGL fails; run again
/// on Direct3D, which falls back to Windows' software renderer. (A second
/// try has to be a new process: the window system starts once per process.)
/// If that fails too, say so in a message box: a windowed app has no
/// console to print to.
#[cfg(windows)]
fn windows_fallback(e: &eframe::Error) -> eframe::Result {
    if renderer_override().is_none()
        && let Ok(exe) = std::env::current_exe()
        && let Ok(status) = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env("MARGIN_RENDERER", "wgpu").status()
    {
        std::process::exit(status.code().unwrap_or(1));
    }
    message_box(&format!(
        "Margin couldn't open its window: {e}\n\nIt needs OpenGL 2 or Direct3D 12. Updating the graphics driver usually fixes this."
    ));
    std::process::exit(1)
}

#[cfg(windows)]
fn message_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Margin"));
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
}
