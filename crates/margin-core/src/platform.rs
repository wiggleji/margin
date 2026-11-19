//! What differs between macOS, Windows and Linux, in one place. The rest of
//! the code calls these and never asks which OS it runs on.
//!
//! (The app has its own `platform` module for desktop integration: opening
//! folders, terminals, links and folder pickers.)

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

/// The user's home folder: `HOME`, or on Windows `USERPROFILE`.
pub fn home_dir() -> Option<PathBuf> {
    let non_empty = |v: std::ffi::OsString| (!v.is_empty()).then_some(v);
    let v = std::env::var_os("HOME").and_then(non_empty).or_else(|| if cfg!(windows) { std::env::var_os("USERPROFILE").and_then(non_empty) } else { None })?;
    Some(PathBuf::from(v))
}

/// Margin's settings folder (`MARGIN_CONFIG_DIR` overrides it):
/// * macOS: `~/Library/Application Support/Margin`
/// * Windows: `%APPDATA%\Margin`
/// * Linux: `$XDG_CONFIG_HOME/margin`, else `~/.config/margin`
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MARGIN_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/Margin");
    }
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"));
        return appdata.join("Margin");
    }
    std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("margin")
}

/// A command for a console program (git, gh) that opens no window. On
/// Windows a GUI app otherwise flashes a console window for every call.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// The machine's offset from UTC at `secs` (seconds, daylight saving
/// included) and the zone's short name when the system has one ("KST").
#[cfg(unix)]
pub fn local_offset(secs: i64) -> Option<(i64, Option<String>)> {
    let t = secs as libc::time_t;
    // SAFETY: localtime_r only writes into the zeroed `tm` we pass; tm_zone
    // points to static zone data owned by libc.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        let name = (!tm.tm_zone.is_null())
            .then(|| std::ffi::CStr::from_ptr(tm.tm_zone).to_string_lossy().into_owned())
            .filter(|n| n.chars().all(|c| c.is_ascii_alphabetic()) && !n.is_empty());
        Some((tm.tm_gmtoff as i64, name))
    }
}

/// Windows: the time zone's rules applied to `secs` (zone names there are
/// long, "Korea Standard Time", so the offset is shown instead).
#[cfg(windows)]
pub fn local_offset(secs: i64) -> Option<(i64, Option<String>)> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME: 100 ns ticks since 1601-01-01
    const EPOCH_DIFF: i64 = 11_644_473_600;
    let ticks = (secs.checked_add(EPOCH_DIFF)?).checked_mul(10_000_000)? as u64;
    let utc_ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: plain out-parameters, all initialised before use.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        let mut local_ft: FILETIME = std::mem::zeroed();
        if FileTimeToSystemTime(&utc_ft, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
            || SystemTimeToFileTime(&local, &mut local_ft) == 0
        {
            return None;
        }
        let local_ticks = ((local_ft.dwHighDateTime as u64) << 32 | local_ft.dwLowDateTime as u64) as i64;
        Some(((local_ticks - ticks as i64) / 10_000_000, None))
    }
}

#[cfg(not(any(unix, windows)))]
pub fn local_offset(_secs: i64) -> Option<(i64, Option<String>)> {
    None
}

/// Folders cloud services usually sync on this computer, whether or not
/// they exist: (service, folder).
pub fn cloud_folders() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = home_dir() else { return vec![] };
    let mut v: Vec<(&'static str, PathBuf)> = vec![];
    if cfg!(target_os = "macos") {
        v.push(("iCloud Drive", home.join("Library/Mobile Documents/com~apple~CloudDocs")));
        v.push(("Dropbox", home.join("Library/CloudStorage/Dropbox")));
        // newer clients live under ~/Library/CloudStorage/<Service>-<account>
        if let Ok(rd) = std::fs::read_dir(home.join("Library/CloudStorage")) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("GoogleDrive-") {
                    v.push(("Google Drive", e.path().join("My Drive")));
                } else if n.starts_with("OneDrive") {
                    v.push(("OneDrive", e.path()));
                }
            }
        }
    }
    if cfg!(windows) {
        v.push(("iCloud Drive", home.join("iCloudDrive")));
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(p) = std::env::var_os(var).filter(|p| !p.is_empty()) {
                v.push(("OneDrive", PathBuf::from(p)));
            }
        }
        v.push(("OneDrive", home.join("OneDrive")));
        // Google Drive for desktop mounts a drive (G: by default)
        v.push(("Google Drive", PathBuf::from(r"G:\My Drive")));
    }
    // the classic places, on every OS
    v.push(("Dropbox", home.join("Dropbox")));
    v.push(("Google Drive", home.join("Google Drive")));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_commands() {
        assert!(config_dir().is_absolute() || std::env::var_os("MARGIN_CONFIG_DIR").is_some() || home_dir().is_none());
        assert!(cloud_folders().iter().all(|(_, p)| p.is_absolute() || home_dir().is_none()));
        // a console program runs (and, on Windows, without a window)
        let out = command("git").arg("--version").output().expect("git runs");
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("git version"));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn offset_is_sane() {
        let (off, _) = local_offset(1_700_000_000).expect("local time zone");
        assert!(off.abs() <= 14 * 3600, "{off}");
    }
}
