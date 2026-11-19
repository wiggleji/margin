//! The `margin` command: `margin .` or `margin ~/src/app` from a terminal.
//!
//! The command is a small script (Margin menu → Install 'margin' Command…,
//! or the Windows installer's PATH option) that starts this app's binary with
//! the folder. A running Margin listens on a local socket (a named pipe on
//! Windows) tied to its config folder; a second start hands its folder over
//! and exits, so the folder opens as a tab in the window that's already open
//! instead of in a second copy of the app.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

#[cfg(unix)]
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

/// The pipe for this config folder: `\\.\pipe\margin-<hash>`.
#[cfg(windows)]
fn pipe_name() -> String {
    let dir = margin_core::config::config_dir().display().to_string().to_lowercase();
    let h = dir.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!(r"\\.\pipe\margin-{h:016x}")
}

#[cfg(windows)]
pub fn hand_over(dir: &Path) -> bool {
    use std::io::Write;
    // the running Margin serves one client at a time: retry briefly when busy
    for _ in 0..20 {
        match std::fs::OpenOptions::new().write(true).open(pipe_name()) {
            // sync_all (FlushFileBuffers) waits until Margin has read the line,
            // so the pipe isn't closed before it's been delivered
            Ok(mut f) => return writeln!(f, "{}", dir.display()).is_ok() && f.sync_all().is_ok(),
            Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(std::time::Duration::from_millis(50)), // ERROR_PIPE_BUSY
            Err(_) => return false,
        }
    }
    false
}

#[cfg(not(any(unix, windows)))]
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

/// Windows: the same over a named pipe (local clients only; the default
/// security lets only this user, administrators and the system write to it).
#[cfg(windows)]
pub fn listen(ctx: &egui::Context) -> Option<Receiver<PathBuf>> {
    use windows_sys::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, ReadFile};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    let name: Vec<u16> = pipe_name().encode_utf16().chain(std::iter::once(0)).collect();
    // FIRST_PIPE_INSTANCE: fails when another Margin already owns the pipe
    // SAFETY: a NUL-terminated name and no security attributes (the defaults)
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            4096,
            0,
            std::ptr::null(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let h = h as usize; // (a HANDLE isn't Send; the thread owns it from here)
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let h = h as windows_sys::Win32::Foundation::HANDLE;
        loop {
            // SAFETY: `h` is our pipe; buffers outlive each call
            // A client that connected (PIPE_CONNECTED), or already wrote and
            // closed (NO_DATA), before we got here still left its line to read.
            let connected = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) != 0 || matches!(GetLastError(), ERROR_PIPE_CONNECTED | ERROR_NO_DATA) };
            if connected {
                let mut data = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    let mut n = 0u32;
                    let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) } != 0;
                    if !ok || n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n as usize]);
                    if data.contains(&b'\n') || data.len() > 4096 {
                        break;
                    }
                }
                let line = String::from_utf8_lossy(&data);
                let dir = line.lines().next().unwrap_or("").trim_end_matches('\r');
                if !dir.is_empty() {
                    if tx.send(PathBuf::from(dir)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
            unsafe { DisconnectNamedPipe(h) };
        }
    });
    Some(rx)
}

#[cfg(not(any(unix, windows)))]
pub fn listen(_ctx: &egui::Context) -> Option<Receiver<PathBuf>> {
    None
}

/// Where the command goes: /usr/local/bin on macOS (on the default PATH),
/// on Windows the user's WindowsApps folder (on PATH since Windows 10),
/// ~/.local/bin on Linux.
pub fn command_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/usr/local/bin/margin")
    } else if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        local.join(r"Microsoft\WindowsApps\margin.cmd")
    } else {
        let home = margin_core::platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
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
    if cfg!(windows) {
        // margin.exe is a windowed app: `start` returns to the prompt at once
        // (the app resolves "." against the folder it's started in)
        return format!("@echo off\r\nrem Margin command-line launcher (Margin → Install 'margin' Command…).\r\nstart \"\" \"{}\" %*\r\n", exe.display());
    }
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
    let exe = margin_core::platform::canonical(&exe);
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

    /// A running Margin receives the folder a second start hands over, and a
    /// second Margin doesn't take the socket / pipe away from it.
    #[cfg(any(unix, windows))]
    #[test]
    fn second_start_hands_its_folder_over() {
        let dir = std::env::temp_dir().join(format!("mcli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the only test in this crate that reads the config folder
        unsafe { std::env::set_var("MARGIN_CONFIG_DIR", &dir) };
        let ctx = egui::Context::default();
        let rx = listen(&ctx).expect("listening");
        assert!(listen(&ctx).is_none(), "a second Margin must not take over");
        let repo = dir.join("some repo");
        for _ in 0..2 {
            assert!(hand_over(&repo));
            assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), repo);
        }
        if cfg!(windows) {
            assert!(script(Path::new(r"C:\Apps\Margin\margin.exe")).contains(r#"start "" "C:\Apps\Margin\margin.exe" %*"#));
        }
    }

    #[cfg(unix)]
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
