//! Desktop integration that differs between macOS, Windows and Linux:
//! showing a folder, opening a terminal or a link, picking a folder.
//! (OS facts such as folders and time zones: `margin_core::platform`.)

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Show `path` in Finder / File Explorer / the file manager.
pub fn reveal(path: &Path) -> io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(program).arg(path).spawn().map(drop)
}

/// Open a link in the default browser (or mail app).
pub fn open_url(url: &str) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("/usr/bin/open").arg(url).spawn().map(drop)
    } else if cfg!(windows) {
        // the URL handler, without a console window or `start` quoting rules
        margin_core::platform::command("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn().map(drop)
    } else {
        Command::new("xdg-open").arg(url).spawn().map(drop)
    }
}

/// Open a terminal in `dir`: Terminal on macOS, Windows Terminal (else a
/// Command Prompt) on Windows, and on Linux $TERMINAL or the first common
/// terminal found.
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        return Command::new("/usr/bin/open").args(["-a", "Terminal"]).arg(dir).spawn().map(drop);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if Command::new("wt").arg("-d").arg(dir).spawn().is_ok() {
            return Ok(());
        }
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        return Command::new("cmd").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE).spawn().map(drop);
    }
    #[allow(unreachable_code)]
    {
        let user = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty());
        // (program, flag that sets the folder; empty: it starts in the current folder)
        let known: [(&str, &str); 10] = [
            ("x-terminal-emulator", ""),
            ("gnome-terminal", "--working-directory="),
            ("konsole", "--workdir="),
            ("xfce4-terminal", "--working-directory="),
            ("kgx", "--working-directory="),
            ("tilix", "--working-directory="),
            ("alacritty", "--working-directory="),
            ("kitty", "--directory="),
            ("foot", "--working-directory="),
            ("xterm", ""),
        ];
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no terminal found (set $TERMINAL)");
        for (program, flag) in user.as_deref().map(|t| (t, "")).into_iter().chain(known) {
            let mut c = Command::new(program);
            c.current_dir(dir);
            if !flag.is_empty() {
                c.arg(format!("{flag}{}", dir.display()));
            }
            match c.spawn() {
                Ok(_) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

/// Can [`pick_folder`] show a folder dialog here? (Linux needs zenity or kdialog.)
pub fn can_pick_folder() -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        return true;
    }
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| linux_picker().is_some())
}

/// Ask for a folder (None: cancelled or no dialog available).
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(any(target_os = "macos", windows))]
    {
        return rfd::FileDialog::new().set_title(title).pick_folder();
    }
    #[allow(unreachable_code)]
    {
        let (program, args) = linux_picker()?;
        let home = margin_core::platform::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut c = Command::new(program);
        match program {
            "zenity" => c.args(args).arg(format!("--title={title}")).arg(format!("--filename={}/", home.display())),
            _ => c.args(args).arg(&home).arg("--title").arg(title),
        };
        let out = c.output().ok()?;
        let p = String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_string();
        (out.status.success() && !p.is_empty()).then(|| PathBuf::from(p))
    }
}

/// A folder dialog program on Linux: GNOME's zenity or KDE's kdialog.
fn linux_picker() -> Option<(&'static str, &'static [&'static str])> {
    let has = |p: &str| std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()));
    if has("zenity") {
        Some(("zenity", &["--file-selection", "--directory"]))
    } else if has("kdialog") {
        Some(("kdialog", &["--getexistingdirectory"]))
    } else {
        None
    }
}
