//! Updating Margin in place from GitHub Releases (macOS).
//!
//! * **Check:** the latest release (`/releases/latest`: no drafts or
//!   pre-releases) from the GitHub API, at most once a day by default.
//! * **Download:** the release's `Margin-<version>-macos.zip` (Margin.app,
//!   zipped with `ditto`), checked against its `.sha256`, unpacked next to
//!   the running app (`<Applications>/.margin-update/`), and checked again:
//!   its version must be the release's, its code signature must verify, and
//!   when the running app is Developer ID signed, the new one must be signed
//!   by the same team.
//! * **Install:** after Margin quits, a small shell helper swaps the new app
//!   into place (keeping the old one until the swap succeeds) and, for
//!   "Restart to update", opens it again.
//!
//! Files downloaded this way get no quarantine flag, so an update opens
//! without the first-launch Gatekeeper prompt even for unsigned builds.

use crate::{Error, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Where releases come from.
pub const REPO: &str = "wiggleji/margin";
/// How often the automatic check runs.
pub const CHECK_EVERY_SECS: i64 = 24 * 3600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// "0.2.0" (the tag without its "v").
    pub version: String,
    /// The release page on GitHub.
    pub page: String,
    /// Release notes (Markdown).
    pub notes: String,
    /// `Margin-<version>-macos.zip` and its `.sha256`, when the release has them.
    pub zip: Option<String>,
    pub zip_sha256: Option<String>,
}

/// "v1.2.3" / "1.2.3" → (1, 2, 3). Pre-releases ("1.3.0-beta.1") → None.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    if s.contains('-') {
        return None;
    }
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next().unwrap_or(Some(0))?, it.next().unwrap_or(Some(0))?);
    Some(v)
}

/// Is `latest` a newer version than `current`?
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

fn release_from_json(v: &Value) -> Option<Release> {
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = v["tag_name"].as_str()?;
    parse_version(tag)?;
    let version = tag.trim_start_matches('v').to_string();
    let asset = |name: &str| {
        v["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name)).and_then(|a| a["browser_download_url"].as_str()).map(str::to_string)
    };
    let zip_name = format!("Margin-{version}-macos.zip");
    Some(Release {
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        zip: asset(&zip_name),
        zip_sha256: asset(&format!("{zip_name}.sha256")),
        version,
    })
}

/// The latest published release (None if there is none yet).
pub fn latest() -> Result<Option<Release>> {
    // MARGIN_UPDATE_API: another API base, for testing against a local server
    let base = std::env::var("MARGIN_UPDATE_API").unwrap_or_else(|_| "https://api.github.com".into());
    let url = format!("{}/repos/{REPO}/releases/latest", base.trim_end_matches('/'));
    let mut resp = match crate::http::agent().get(&url).header("Accept", "application/vnd.github+json").call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(ureq::Error::StatusCode(403 | 429)) => bail!("GitHub is rate limiting update checks; try again later"),
        Err(e) => bail!("couldn't check for updates: {e}"),
    };
    let v: Value = resp.body_mut().read_json().map_err(|e| Error(format!("update check: bad response: {e}")))?;
    Ok(release_from_json(&v))
}

/// Download `url` to `dest` (at most `max` bytes).
fn download(url: &str, dest: &Path, max: u64) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15 * 60)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .user_agent("margin-git-client")
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| Error(format!("download failed: {e}")))?;
    let mut file = std::fs::File::create(dest).map_err(|e| Error(format!("{}: {e}", dest.display())))?;
    let mut body = resp.body_mut().with_config().limit(max).reader();
    std::io::copy(&mut body, &mut file).map_err(|e| Error(format!("download failed: {e}")))?;
    Ok(())
}

fn fetch_text(url: &str) -> Result<String> {
    let mut resp = crate::http::agent().get(url).call().map_err(|e| Error(format!("download failed: {e}")))?;
    resp.body_mut().with_config().limit(64 * 1024).read_to_string().map_err(|e| Error(format!("download failed: {e}")))
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| Error(format!("{cmd}: {e}")))?;
    if !out.status.success() {
        bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `.app` bundle the running binary is in (None for `cargo run` builds).
pub fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app").then(|| app.to_path_buf())
}

/// Can this copy of Margin update itself? Err says why not.
pub fn can_install() -> std::result::Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("Updating in place works on macOS; download the new version from the release page".into());
    }
    let Some(app) = app_bundle() else {
        return Err("This is a development build (not inside Margin.app)".into());
    };
    let shown = app.display().to_string();
    if shown.contains("/AppTranslocation/") || shown.starts_with("/Volumes/") {
        return Err("Move Margin to the Applications folder to update it".into());
    }
    let parent = app.parent().ok_or("no folder above Margin.app")?;
    let probe = parent.join(format!(".margin-write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(app)
        }
        Err(_) => Err(format!("Margin can't write to {}; download the new version from the release page", parent.display())),
    }
}

/// The Developer ID team that signed `app` (None: ad-hoc or unsigned).
fn team_id(app: &Path) -> Option<String> {
    let out = Command::new("codesign").arg("-dv").arg(app).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).map(str::trim).filter(|t| !t.is_empty() && *t != "not set").map(str::to_string)
}

/// Download and check `rel` next to the running `app`. Returns the new
/// Margin.app, ready for [`install_after_exit`].
pub fn stage(rel: &Release, app: &Path) -> Result<PathBuf> {
    let (Some(zip_url), Some(sha_url)) = (&rel.zip, &rel.zip_sha256) else {
        bail!("Margin {} has no update package; download it from the release page", rel.version);
    };
    let dir = app.parent().ok_or_else(|| Error("no folder above Margin.app".into()))?.join(".margin-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let zip = dir.join("Margin.zip");
    download(zip_url, &zip, 500 * 1024 * 1024)?;
    // checksum
    let want = fetch_text(sha_url)?.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let got = run("shasum", &["-a".as_ref(), "256".as_ref(), zip.as_os_str()])?.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if want.len() != 64 || want != got {
        let _ = std::fs::remove_dir_all(&dir);
        bail!("the download didn't match its checksum; nothing was changed");
    }
    run("ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), dir.as_os_str()])?;
    let _ = std::fs::remove_file(&zip);
    let new = dir.join("Margin.app");
    let fail = |msg: String| -> Result<PathBuf> {
        let _ = std::fs::remove_dir_all(&dir);
        Err(Error(msg))
    };
    if !new.is_dir() {
        return fail("the update package has no Margin.app".into());
    }
    // the right version …
    let plist = new.join("Contents/Info.plist");
    let v = run("plutil", &["-extract".as_ref(), "CFBundleShortVersionString".as_ref(), "raw".as_ref(), "-o".as_ref(), "-".as_ref(), plist.as_os_str()]).unwrap_or_default();
    if v.trim() != rel.version {
        return fail(format!("the update package is version {} instead of {}", v.trim(), rel.version));
    }
    // … intact …
    if let Err(e) = run("codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), new.as_os_str()]) {
        return fail(format!("the update's signature doesn't verify: {e}"));
    }
    // … and from the same developer as this copy, when this one is signed
    if let Some(team) = team_id(app)
        && team_id(&new).as_deref() != Some(team.as_str())
    {
        return fail("the update isn't signed by the same developer; nothing was changed".into());
    }
    Ok(new)
}

/// After this process exits, replace `app` with `new` (then open it if
/// `relaunch`). The old app is kept until the swap succeeds.
pub fn install_after_exit(new: &Path, app: &Path, relaunch: bool) -> Result<()> {
    helper(std::process::id(), new, app, relaunch)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error(format!("couldn't start the installer: {e}")))?;
    Ok(())
}

/// The swap, as a shell command that first waits (up to a minute) for `pid` to exit.
fn helper(pid: u32, new: &Path, app: &Path, relaunch: bool) -> Command {
    const SCRIPT: &str = r#"
pid="$1"; new="$2"; app="$3"; relaunch="$4"; dir="$5"
i=0
while kill -0 "$pid" 2>/dev/null && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
old="$app.old-$$"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then rm -rf "$old"; else mv "$old" "$app"; fi
fi
rm -rf "$dir"
if [ "$relaunch" = 1 ]; then open "$app"; fi
exit 0
"#;
    let dir = new.parent().unwrap_or(new);
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(SCRIPT).arg("margin-update").arg(pid.to_string()).arg(new).arg(app).arg(if relaunch { "1" } else { "0" }).arg(dir);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("1.3.0-beta.1"), None);
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn release_assets() {
        let v = json!({
            "tag_name": "v0.2.0", "html_url": "https://github.com/x/y/releases/tag/v0.2.0", "body": "notes",
            "draft": false, "prerelease": false,
            "assets": [
                {"name": "Margin-0.2.0.dmg", "browser_download_url": "https://dl/dmg"},
                {"name": "Margin-0.2.0-macos.zip", "browser_download_url": "https://dl/zip"},
                {"name": "Margin-0.2.0-macos.zip.sha256", "browser_download_url": "https://dl/sha"}
            ]
        });
        let r = release_from_json(&v).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.zip.as_deref(), Some("https://dl/zip"));
        assert_eq!(r.zip_sha256.as_deref(), Some("https://dl/sha"));
        // the first release had only a DMG
        let old = json!({"tag_name": "v0.1.0", "assets": [{"name": "Margin-0.1.0.dmg", "browser_download_url": "x"}]});
        assert_eq!(release_from_json(&old).unwrap().zip, None);
        assert!(release_from_json(&json!({"tag_name": "v0.3.0", "prerelease": true})).is_none());
    }

    /// The helper swaps the app once the given process is gone, and cleans up.
    #[cfg(unix)]
    #[test]
    fn helper_swaps_the_app() {
        let d = std::env::temp_dir().join(format!("margin-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let app = d.join("Margin.app");
        let stage = d.join(".margin-update");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/v"), "old").unwrap();
        std::fs::create_dir_all(stage.join("Margin.app/Contents")).unwrap();
        std::fs::write(stage.join("Margin.app/Contents/v"), "new").unwrap();
        // a process that has already exited stands in for Margin
        let mut gone = Command::new("true").spawn().unwrap();
        let pid = gone.id();
        gone.wait().unwrap();
        assert!(helper(pid, &stage.join("Margin.app"), &app, false).status().unwrap().success());
        assert_eq!(std::fs::read_to_string(app.join("Contents/v")).unwrap(), "new");
        assert!(!stage.exists());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no Margin.app.old left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
