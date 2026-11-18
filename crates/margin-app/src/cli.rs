//! The `margin` command: `margin .` or `margin ~/src/app` from a terminal.
//!
//! The command is a small shell script (Margin menu → Install 'margin'
//! Command…) that starts this app's binary with the folder. A running Margin
//! listens on a local socket in its config folder; a second start hands its
//! folder over and exits, so the folder opens as a tab in the window that's
//! already open instead of in a second copy of the app.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

fn socket_path() -> PathBuf {
    margin_core::config::config_dir().join("margin.sock")
}

/// Ask a running Margin to open `dir`. False if none is running.
#[cfg(unix)]
pub fn hand_over(dir: &Path) -> bool {
    use std::io::Write;
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(socket_path()) else {
        return false;
    };
    let _ = s.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    writeln!(s, "{}", dir.display()).is_ok()
}

#[cfg(not(unix))]
pub fn hand_over(_dir: &Path) -> bool {
    false
}

/// Listen for folders handed over by later starts. None if another Margin
/// already listens (or the socket can't be made); the app works the same,
/// only `margin <dir>` then opens a second window.
#[cfg(unix)]
pub fn listen(ctx: &egui::Context) -> Option<Receiver<PathBuf>> {
    use std::io::{BufRead, Read};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        return None; // another Margin owns it
    }
    let _ = std::fs::create_dir_all(path.parent()?);
    let _ = std::fs::remove_file(&path); // left over from a crash
    let listener = UnixListener::bind(&path).ok()?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(stream.take(4096)).read_line(&mut line);
            let dir = line.trim_end_matches(['\r', '\n']);
            if dir.is_empty() {
                continue;
            }
            if tx.send(PathBuf::from(dir)).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    Some(rx)
}

#[cfg(not(unix))]
pub fn listen(_ctx: &egui::Context) -> Option<Receiver<PathBuf>> {
    None
}

/// Where the command goes: /usr/local/bin on macOS (on the default PATH),
/// ~/.local/bin elsewhere.
pub fn command_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/usr/local/bin/margin")
    } else {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        home.join(".local/bin/margin")
    }
}

/// `s` in double quotes for sh.
fn sh_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

/// The launcher script for the app binary at `exe`.
pub fn script(exe: &Path) -> String {
    let exe = sh_quote(&exe.display().to_string());
    format!(
        r#"#!/bin/sh
# Margin command-line launcher (installed from Margin → Install 'margin' Command…).
#   margin [folder]   open a repository in Margin (default: this folder)
case "$1" in -h|--help|--version) exec {exe} "$@" ;; esac
dir="${{1:-.}}"
abs=$(cd "$dir" 2>/dev/null && pwd -P) || {{ echo "margin: no such folder: $dir" >&2; exit 1; }}
nohup {exe} "$abs" >/dev/null 2>&1 &
"#
    )
}

/// Install the `margin` command for this app. Asks for an administrator
/// password on macOS when /usr/local/bin isn't writable. Returns a message
/// for the status line.
pub fn install() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find Margin's binary: {e}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let shown = exe.display().to_string();
    // macOS runs apps opened straight from a download or the DMG from a
    // temporary copy; a command pointing there would break.
    if shown.contains("/AppTranslocation/") || shown.starts_with("/Volumes/") {
        return Err("Move Margin to the Applications folder and open it from there first".into());
    }
    let dest = command_path();
    let text = script(&exe);
    let direct = || -> std::io::Result<()> {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&dest, &text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    };
    match direct() {
        Ok(()) => {}
        Err(e) if cfg!(target_os = "macos") && e.kind() == std::io::ErrorKind::PermissionDenied => install_as_admin(&dest, &text)?,
        Err(e) => return Err(format!("couldn't write {}: {e}", dest.display())),
    }
    Ok(format!("Installed the margin command ({}). Try `margin .` in a repository.", dest.display()))
}

/// Copy the script into place with administrator rights (macOS password prompt).
fn install_as_admin(dest: &Path, text: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("margin-cli-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    let dir = dest.parent().unwrap_or(Path::new("/usr/local/bin"));
    let sh = format!("mkdir -p {} && cp {} {} && chmod 755 {}", q(dir), q(&tmp), q(dest), q(dest));
    let apple = format!(
        "do shell script \"{}\" with administrator privileges with prompt \"Margin wants to install the 'margin' command in {}.\"",
        sh.replace('\\', "\\\\").replace('"', "\\\""),
        dir.display()
    );
    let out = std::process::Command::new("osascript").arg("-e").arg(apple).output().map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&tmp);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-128") {
            Err("Install cancelled".into())
        } else {
            Err(format!("couldn't install the margin command: {}", err.trim()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_quotes_the_binary_path() {
        let s = script(Path::new("/Apps/My \"Tools\"/Margin.app/Contents/MacOS/margin"));
        assert!(s.contains(r#"nohup "/Apps/My \"Tools\"/Margin.app/Contents/MacOS/margin" "$abs""#));
        assert!(s.starts_with("#!/bin/sh\n"));
    }

    #[cfg(unix)]
    #[test]
    fn launcher_runs_the_binary_with_an_absolute_folder() {
        let d = std::env::temp_dir().join(format!("margin-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("repo")).unwrap();
        // a fake "app" that records its argument
        let fake = d.join("fake-margin");
        std::fs::write(&fake, format!("#!/bin/sh\necho \"$1\" > {}\n", d.join("got").display())).unwrap();
        let launcher = d.join("margin");
        std::fs::write(&launcher, script(&fake)).unwrap();
        for f in [&fake, &launcher] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let ok = std::process::Command::new(&launcher).arg(".").current_dir(d.join("repo")).status().unwrap();
        assert!(ok.success());
        for _ in 0..50 {
            if d.join("got").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = std::fs::read_to_string(d.join("got")).unwrap();
        assert_eq!(got.trim(), d.join("repo").canonicalize().unwrap().display().to_string());
        let bad = std::process::Command::new(&launcher).arg("/no/such/dir").output().unwrap();
        assert!(!bad.status.success());
        let _ = std::fs::remove_dir_all(&d);
    }
}
