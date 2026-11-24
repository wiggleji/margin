//! Updating Margin in place from GitHub Releases, on macOS, Windows and Linux.
//!
//! * **Check:** the latest release (`/releases/latest`: no drafts or
//!   pre-releases) from the GitHub API, at most once a day by default.
//! * **Download:** this OS's update package, checked against its `.sha256`:
//!   * macOS: `Margin-<v>-macos.zip` (Margin.app zipped with `ditto`),
//!     unpacked next to the running app and checked again: its version must
//!     be the release's, its code signature must verify, and when the running
//!     app is Developer ID signed the new one must come from the same team.
//!   * Windows: `Margin-<v>-windows-x64-setup.exe`, the installer.
//!   * Linux: `margin-<v>-linux-x64.tar.gz`, unpacked, or, when Margin runs
//!     as an AppImage, `Margin-<v>-x86_64.AppImage`; the new
//!     `margin --version` must print the release's version.
//! * **Install:**
//!   * macOS: after Margin quits, a shell helper swaps the new app into place
//!     (keeping the old one until the swap succeeds).
//!   * Windows: after Margin quits, the installer runs silently over the
//!     existing install (same folder, same options).
//!   * Linux: the files (or the AppImage) are replaced right away (a running
//!     program keeps its old copy) and the new one starts after Margin quits.
//!
//!   "Restart to update" opens the new version once it's in place.
//!
//! In place means: macOS — Margin.app in a folder you can write to; Windows —
//! installed with the installer, for this user; Linux — the AppImage, or the
//! tar.gz installed in a folder you own (e.g. ~/.local). Elsewhere (a .deb owned by
//! apt, Homebrew on Linux, the portable Windows zip, a development build)
//! Margin says how to update instead (the release page, or `brew upgrade`).
//!
//! Files downloaded this way get no quarantine flag (macOS) or
//! Mark-of-the-Web (Windows), so an update opens without the first-launch
//! warnings even for unsigned builds.

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
    /// This OS's update package and its `.sha256`, when the release has them.
    pub package: Option<String>,
    pub package_sha256: Option<String>,
}

/// The update package's file name for this OS.
pub fn package_name(version: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("Margin-{version}-macos.zip")
    } else if cfg!(windows) {
        format!("Margin-{version}-windows-x64-setup.exe")
    } else if appimage().is_some() {
        format!("Margin-{version}-x86_64.AppImage")
    } else {
        format!("margin-{version}-linux-x64.tar.gz")
    }
}

/// Linux: the AppImage file Margin runs from (the runtime sets $APPIMAGE).
pub fn appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let p = PathBuf::from(std::env::var_os("APPIMAGE").filter(|v| !v.is_empty())?);
    p.is_file().then_some(p)
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
    let name = package_name(&version);
    Some(Release {
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        package: asset(&name),
        package_sha256: asset(&format!("{name}.sha256")),
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

/// SHA-256 of a file, lowercase hex.
fn sha256_file(p: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download this OS's package for `rel` into `dir` and check its SHA-256.
fn download_checked(rel: &Release, dir: &Path) -> Result<PathBuf> {
    let (Some(url), Some(sha_url)) = (&rel.package, &rel.package_sha256) else {
        bail!("Margin {} has no update package for this system; download it from the release page", rel.version);
    };
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let file = dir.join(package_name(&rel.version));
    download(url, &file, 500 * 1024 * 1024)?;
    let want = fetch_text(sha_url)?.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if want.len() != 64 || want != sha256_file(&file)? {
        let _ = std::fs::remove_dir_all(dir);
        bail!("the download didn't match its checksum; nothing was changed");
    }
    Ok(file)
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<String> {
    let out = crate::platform::command(cmd).args(args).output().map_err(|e| Error(format!("{cmd}: {e}")))?;
    if !out.status.success() {
        bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Is `dir` writable by us? (Tried with a throwaway file.)
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".margin-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|e| crate::platform::canonical(&e))
}

/// The `.app` bundle the running binary is in (None for `cargo run` builds).
pub fn app_bundle() -> Option<PathBuf> {
    let exe = current_exe()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app").then(|| app.to_path_buf())
}

/// Can this copy of Margin update itself? Ok: what an update replaces (the
/// .app bundle on macOS, the install folder on Windows, the install prefix
/// on Linux). Err says why not.
pub fn can_install() -> std::result::Result<PathBuf, String> {
    let page = "download the new version from the release page";
    if cfg!(target_os = "macos") {
        let Some(app) = app_bundle() else {
            return Err("This is a development build (not inside Margin.app)".into());
        };
        let shown = app.display().to_string();
        if shown.contains("/AppTranslocation/") || shown.starts_with("/Volumes/") {
            return Err("Move Margin to the Applications folder to update it".into());
        }
        let parent = app.parent().ok_or("no folder above Margin.app")?;
        return if writable(parent) { Ok(app) } else { Err(format!("Margin can't write to {}; {page}", parent.display())) };
    }
    let exe = current_exe().ok_or("can't find Margin's program file")?;
    let dir = exe.parent().ok_or("no folder above Margin's program")?.to_path_buf();
    if cfg!(windows) {
        // the installer leaves its uninstaller next to margin.exe
        if !dir.join("unins000.exe").is_file() {
            return Err(format!("This copy wasn't installed with the installer (the portable zip, or a development build); {page}"));
        }
        return if writable(&dir) { Ok(dir) } else { Err(format!("Margin is installed for all users; {page} (or run the new installer)")) };
    }
    // Linux AppImage: the .AppImage file itself is replaced
    if let Some(image) = appimage() {
        let dir = image.parent().ok_or("no folder above the AppImage")?;
        return if writable(dir) { Ok(image) } else { Err(format!("Margin can't write to {}; {page}", dir.display())) };
    }
    // Linux: installed with Homebrew (its Cellar): brew updates it
    if is_homebrew(&exe) {
        return Err("Margin was installed with Homebrew; update it with `brew upgrade margin`".into());
    }
    // Linux: <prefix>/bin/margin from the tar.gz (in ~/.local, or unpacked)
    if exe.starts_with("/usr") {
        return Err("Margin was installed with a package manager; update it there, or install the new .deb from the release page".into());
    }
    let prefix = dir.parent().filter(|_| dir.file_name().is_some_and(|n| n == "bin")).ok_or_else(|| format!("This is a development build; {page}"))?.to_path_buf();
    if writable(&dir) { Ok(prefix) } else { Err(format!("Margin can't write to {}; {page}", dir.display())) }
}

/// Is `exe` inside a Homebrew Cellar (`…/Cellar/margin/<version>/bin/margin`)?
fn is_homebrew(exe: &Path) -> bool {
    let parts: Vec<_> = exe.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    parts.windows(2).any(|w| w[0] == "Cellar" && w[1] == "margin")
}

/// The Developer ID team that signed `app` (None: ad-hoc or unsigned).
fn team_id(app: &Path) -> Option<String> {
    let out = Command::new("codesign").arg("-dv").arg(app).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).map(str::trim).filter(|t| !t.is_empty() && *t != "not set").map(str::to_string)
}

/// Where downloads are unpacked before installing: next to what they
/// replace (same disk, so moving into place is instant), or the temporary
/// folder for the Windows installer.
fn stage_dir(root: &Path) -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir().join(format!("margin-update-{}", std::process::id()))
    } else {
        root.parent().unwrap_or(root).join(".margin-update")
    }
}

/// Download and check `rel`. Returns what [`install_after_exit`] installs:
/// the new Margin.app (macOS), the installer (Windows), the unpacked
/// package folder (Linux).
pub fn stage(rel: &Release, root: &Path) -> Result<PathBuf> {
    let dir = stage_dir(root);
    let file = download_checked(rel, &dir)?;
    let fail = |msg: String| -> Result<PathBuf> {
        let _ = std::fs::remove_dir_all(&dir);
        Err(Error(msg))
    };
    if cfg!(windows) {
        return Ok(file);
    }
    if cfg!(target_os = "macos") {
        run("ditto", &["-x".as_ref(), "-k".as_ref(), file.as_os_str(), dir.as_os_str()])?;
        let _ = std::fs::remove_file(&file);
        let new = dir.join("Margin.app");
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
        if let Some(team) = team_id(root)
            && team_id(&new).as_deref() != Some(team.as_str())
        {
            return fail("the update isn't signed by the same developer; nothing was changed".into());
        }
        return Ok(new);
    }
    // Linux AppImage: make it runnable and ask it its version (extracting
    // itself, so no FUSE is needed for the check)
    if appimage().is_some() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        }
        let out = crate::platform::command(&file).arg("--version").env("APPIMAGE_EXTRACT_AND_RUN", "1").env_remove("APPIMAGE").output();
        let v = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if v != format!("margin {}", rel.version) {
            return fail(format!("the update says {v:?} instead of margin {}", rel.version));
        }
        return Ok(file);
    }
    // Linux: unpack and ask the new program its version
    run("tar", &["-xzf".as_ref(), file.as_os_str(), "-C".as_ref(), dir.as_os_str()])?;
    let _ = std::fs::remove_file(&file);
    let new = dir.join(format!("margin-{}-linux-x64", rel.version));
    let v = run(&new.join("bin/margin").display().to_string(), &["--version".as_ref()]).unwrap_or_default();
    if v.trim() != format!("margin {}", rel.version) {
        return fail(format!("the update package says {:?} instead of margin {}", v.trim(), rel.version));
    }
    Ok(new)
}

/// Install `new` over `root` (see the module docs), then open the new
/// version if `relaunch`. On macOS and Windows that happens after this
/// process exits; on Linux the files are replaced now.
pub fn install_after_exit(new: &Path, root: &Path, relaunch: bool) -> Result<()> {
    let pid = std::process::id();
    if cfg!(target_os = "linux") && root.is_file() {
        // the AppImage: renamed over the old one (same folder, so atomic; the
        // running Margin keeps its mounted copy)
        std::fs::rename(new, root).map_err(|e| Error(format!("{}: {e}", root.display())))?;
        let _ = std::fs::remove_dir_all(new.parent().unwrap_or(new));
        if relaunch {
            spawn_detached(relaunch_after(pid, root))?;
        }
        return Ok(());
    }
    if cfg!(target_os = "linux") {
        replace_files(new, root)?;
        let _ = std::fs::remove_dir_all(new.parent().unwrap_or(new));
        if relaunch {
            spawn_detached(relaunch_after(pid, &root.join("bin/margin")))?;
        }
        return Ok(());
    }
    let cmd = if cfg!(windows) { windows_helper(pid, new, root, relaunch) } else { mac_helper(pid, new, root, relaunch) };
    spawn_detached(cmd)
}

fn spawn_detached(mut cmd: Command) -> Result<()> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error(format!("couldn't start the installer: {e}")))?;
    Ok(())
}

/// Linux: copy the package's files over the install (each file written
/// beside its target and renamed into place, so a running program keeps
/// its old copy and nothing is ever half-written).
fn replace_files(new: &Path, prefix: &Path) -> Result<()> {
    const FILES: [&str; 5] = [
        "bin/margin",
        "bin/margin-mcp",
        "share/applications/margin.desktop",
        "share/icons/hicolor/256x256/apps/margin.png",
        "share/icons/hicolor/512x512/apps/margin.png",
    ];
    for f in FILES {
        let (src, dst) = (new.join(f), prefix.join(f));
        if !src.is_file() {
            continue;
        }
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = dst.with_extension(format!("margin-new-{}", std::process::id()));
        std::fs::copy(&src, &tmp).map_err(|e| Error(format!("{}: {e}", dst.display())))?;
        std::fs::rename(&tmp, &dst).map_err(|e| Error(format!("{}: {e}", dst.display())))?;
    }
    Ok(())
}

/// Start `program` once `pid` has exited (Linux: so the new Margin can take
/// over the single-window socket).
fn relaunch_after(pid: u32, program: &Path) -> Command {
    const SCRIPT: &str = r#"
i=0
while kill -0 "$1" 2>/dev/null && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
exec "$2"
"#;
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(SCRIPT).arg("margin-update").arg(pid.to_string()).arg(program);
    c
}

/// macOS: the swap, as a shell command that first waits (up to a minute) for `pid` to exit.
fn mac_helper(pid: u32, new: &Path, app: &Path, relaunch: bool) -> Command {
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

/// Windows: PowerShell waits for `pid` to exit, runs the installer silently
/// over the existing install, cleans up, and opens the new Margin.
fn windows_helper(pid: u32, setup: &Path, dir: &Path, relaunch: bool) -> Command {
    // values go in as single-quoted PowerShell strings ('' escapes ')
    let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
    let script = format!(
        "Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue; \
         Start-Process -Wait -FilePath {setup} -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER'; \
         Remove-Item -Recurse -Force -ErrorAction SilentlyContinue {stage}; \
         if (${relaunch}) {{ Start-Process -FilePath {exe} }}",
        setup = q(setup),
        stage = q(setup.parent().unwrap_or(setup)),
        exe = q(&dir.join("margin.exe")),
        relaunch = if relaunch { "true" } else { "false" },
    );
    let mut c = crate::platform::command("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", &script]);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homebrew_installs_are_recognised() {
        assert!(is_homebrew(Path::new("/home/linuxbrew/.linuxbrew/Cellar/margin/0.8.1/bin/margin")));
        assert!(!is_homebrew(Path::new("/home/me/.local/bin/margin")));
        assert!(!is_homebrew(Path::new("/home/me/Cellar/other/bin/margin")));
    }
    use serde_json::json;

    /// Linux: files are replaced in place, missing ones skipped, extra
    /// files in the install left alone.
    #[test]
    fn replaces_linux_files() {
        let d = std::env::temp_dir().join(format!("margin-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (new, prefix) = (d.join("new"), d.join("prefix"));
        for (root, text) in [(&new, "new"), (&prefix, "old")] {
            std::fs::create_dir_all(root.join("bin")).unwrap();
            std::fs::write(root.join("bin/margin"), text).unwrap();
        }
        std::fs::write(prefix.join("bin/other-tool"), "keep").unwrap();
        replace_files(&new, &prefix).unwrap();
        assert_eq!(std::fs::read_to_string(prefix.join("bin/margin")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(prefix.join("bin/other-tool")).unwrap(), "keep");
        assert!(!prefix.join("bin/margin-mcp").exists(), "not in the package: not created");
        assert_eq!(std::fs::read_dir(prefix.join("bin")).unwrap().count(), 2, "no temporary files left");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn windows_installer_command() {
        let c = windows_helper(42, Path::new(r"C:\Temp\it's\Margin-0.3.0-windows-x64-setup.exe"), Path::new(r"C:\Apps\Margin"), true);
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let script = args.last().unwrap();
        assert!(script.contains("Wait-Process -Id 42"));
        assert!(script.contains(r"-FilePath 'C:\Temp\it''s\Margin-0.3.0-windows-x64-setup.exe'"), "{script}");
        assert!(script.contains("'/VERYSILENT'"));
        assert!(script.contains(r"Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'C:\Temp\it''s'"), "{script}");
        assert!(script.contains(r"if ($true) { Start-Process -FilePath 'C:\Apps\Margin\margin.exe' }"), "{script}");
    }

    #[test]
    fn sha256_of_a_file() {
        let p = std::env::temp_dir().join(format!("margin-sha-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&p);
    }

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
        if cfg!(target_os = "macos") {
            assert_eq!(r.package.as_deref(), Some("https://dl/zip"));
            assert_eq!(r.package_sha256.as_deref(), Some("https://dl/sha"));
        }
        // every OS finds its own package
        let all = json!({"tag_name": "v0.3.0", "assets": [
            {"name": "Margin-0.3.0-macos.zip", "browser_download_url": "mac"},
            {"name": "Margin-0.3.0-macos.zip.sha256", "browser_download_url": "mac.sha"},
            {"name": "Margin-0.3.0-windows-x64-setup.exe", "browser_download_url": "win"},
            {"name": "Margin-0.3.0-windows-x64-setup.exe.sha256", "browser_download_url": "win.sha"},
            {"name": "margin-0.3.0-linux-x64.tar.gz", "browser_download_url": "linux"},
            {"name": "margin-0.3.0-linux-x64.tar.gz.sha256", "browser_download_url": "linux.sha"}
        ]});
        let r = release_from_json(&all).unwrap();
        let want = if cfg!(target_os = "macos") { "mac" } else if cfg!(windows) { "win" } else { "linux" };
        assert_eq!(r.package.as_deref(), Some(want));
        assert_eq!(r.package_sha256, Some(format!("{want}.sha")));
        // the first release had only a DMG
        let old = json!({"tag_name": "v0.1.0", "assets": [{"name": "Margin-0.1.0.dmg", "browser_download_url": "x"}]});
        if !cfg!(target_os = "macos") {
            assert_eq!(release_from_json(&old).unwrap().package, None);
        }
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
        assert!(mac_helper(pid, &stage.join("Margin.app"), &app, false).status().unwrap().success());
        assert_eq!(std::fs::read_to_string(app.join("Contents/v")).unwrap(), "new");
        assert!(!stage.exists());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no Margin.app.old left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
