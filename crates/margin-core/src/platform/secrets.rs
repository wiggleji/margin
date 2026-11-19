//! Tokens (GitHub, Jira) in the OS's own secure store:
//!
//! * macOS: the Keychain (`security`)
//! * Windows: Credential Manager (generic credentials named `margin:<service>/<account>`)
//! * Linux: the desktop keyring through the Secret Service (`secret-tool`,
//!   from libsecret), when one is running
//!
//! Without a secure store (a Linux machine with no keyring), tokens go in
//! `secrets.json` in the config folder, readable only by the user. Tokens
//! saved there by earlier versions are still read, and move to the secure
//! store the next time they're saved.

use crate::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where tokens are kept, for the settings text ("your macOS Keychain").
pub fn store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "your macOS Keychain"
    } else if cfg!(windows) {
        "Windows Credential Manager"
    } else if os::available() {
        "your desktop keyring (Secret Service)"
    } else {
        "secrets.json in Margin's config folder, readable only by you (no desktop keyring found)"
    }
}

/// Save a token.
pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
    if os::available() {
        os::store(service, account, value)?;
        // it now lives in the secure store: drop an older plain copy
        let _ = file::delete(service, account);
        return Ok(());
    }
    file::store(service, account, value)
}

/// Forget a token (from the secure store and the file).
pub fn delete(service: &str, account: &str) -> Result<()> {
    if os::available() {
        os::delete(service, account)?;
    }
    file::delete(service, account)
}

/// A saved token for `service` (and `account`, or any account when None).
pub fn lookup(service: &str, account: Option<&str>) -> Option<String> {
    let found = if os::available() { os::lookup(service, account) } else { None };
    found.or_else(|| file::lookup(service, account)).filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------- macOS
#[cfg(target_os = "macos")]
mod os {
    use crate::Result;
    use std::process::Command;

    pub fn available() -> bool {
        true
    }

    pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
        // -U: update an existing item
        let st = Command::new("/usr/bin/security").args(["add-generic-password", "-U", "-s", service, "-a", account, "-w", value]).status()?;
        if !st.success() {
            crate::bail!("couldn't save to the Keychain");
        }
        Ok(())
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let _ = Command::new("/usr/bin/security").args(["delete-generic-password", "-s", service, "-a", account]).output();
        Ok(())
    }

    pub fn lookup(service: &str, account: Option<&str>) -> Option<String> {
        let mut c = Command::new("/usr/bin/security");
        c.args(["find-generic-password", "-s", service, "-w"]);
        if let Some(a) = account {
            c.args(["-a", a]);
        }
        let out = c.output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

// -------------------------------------------------------------- Windows
#[cfg(windows)]
mod os {
    use crate::{Error, Result};
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn target(service: &str, account: &str) -> String {
        format!("margin:{service}/{account}")
    }

    pub fn available() -> bool {
        true
    }

    pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
        let mut name = wide(&target(service, account));
        let mut user = wide(account);
        let blob = value.as_bytes();
        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            Comment: std::ptr::null_mut(),
            LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            // this user, on this computer (not roaming)
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: user.as_mut_ptr(),
        };
        // SAFETY: every pointer in `cred` points into buffers that outlive the call.
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(Error(format!("couldn't save to Credential Manager: {}", std::io::Error::last_os_error())));
        }
        Ok(())
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let name = wide(&target(service, account));
        // SAFETY: a NUL-terminated name; a missing credential is fine
        unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) };
        Ok(())
    }

    /// The secret of a credential returned by CredReadW / CredEnumerateW.
    ///
    /// SAFETY: `c` must point to a valid CREDENTIALW.
    unsafe fn secret(c: *const CREDENTIALW) -> Option<String> {
        let c = unsafe { &*c };
        if c.CredentialBlob.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize) };
        String::from_utf8(bytes.to_vec()).ok()
    }

    pub fn lookup(service: &str, account: Option<&str>) -> Option<String> {
        // SAFETY: Windows allocates the results; each is freed with CredFree.
        unsafe {
            match account {
                Some(a) => {
                    let name = wide(&target(service, a));
                    let mut c: *mut CREDENTIALW = std::ptr::null_mut();
                    if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut c) == 0 {
                        return None;
                    }
                    let s = secret(c);
                    CredFree(c as *const _);
                    s
                }
                None => {
                    let filter = wide(&format!("margin:{service}/*"));
                    let mut n = 0u32;
                    let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
                    if CredEnumerateW(filter.as_ptr(), 0, &mut n, &mut list) == 0 {
                        return None;
                    }
                    let s = if n > 0 { secret(*list) } else { None };
                    CredFree(list as *const _);
                    s
                }
            }
        }
    }
}

// ---------------------------------------------------------------- Linux
#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    use crate::{Error, Result};
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// `secret-tool` is installed and a keyring answers.
    pub fn available() -> bool {
        static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OK.get_or_init(|| {
            // a lookup that finds nothing exits 1 when the keyring works, and
            // fails with an error message when no Secret Service is running
            Command::new("secret-tool")
                .args(["lookup", "application", "margin", "probe", "1"])
                .stdin(Stdio::null())
                .output()
                .is_ok_and(|o| o.status.success() || o.stderr.is_empty())
        })
    }

    fn attrs<'a>(service: &'a str, account: Option<&'a str>) -> Vec<&'a str> {
        let mut a = vec!["application", "margin", "service", service];
        if let Some(acc) = account {
            a.extend(["account", acc]);
        }
        a
    }

    pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
        let mut child = Command::new("secret-tool")
            .args(["store", &format!("--label=Margin: {service} ({account})")])
            .args(attrs(service, Some(account)))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        // the secret goes in on stdin, never on the command line
        child.stdin.take().expect("piped").write_all(value.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(Error(format!("couldn't save to the keyring: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        Ok(())
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let _ = Command::new("secret-tool").arg("clear").args(attrs(service, Some(account))).output();
        Ok(())
    }

    pub fn lookup(service: &str, account: Option<&str>) -> Option<String> {
        let out = Command::new("secret-tool").arg("lookup").args(attrs(service, account)).stdin(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    use crate::Result;
    pub fn available() -> bool {
        false
    }
    pub fn store(_: &str, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    pub fn delete(_: &str, _: &str) -> Result<()> {
        Ok(())
    }
    pub fn lookup(_: &str, _: Option<&str>) -> Option<String> {
        None
    }
}

// ------------------------------------------------- secrets.json fallback
mod file {
    use super::*;

    fn path() -> PathBuf {
        crate::platform::config_dir().join("secrets.json")
    }

    fn read() -> BTreeMap<String, String> {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn write(m: &BTreeMap<String, String>) -> Result<()> {
        let p = path();
        if m.is_empty() {
            let _ = std::fs::remove_file(&p);
            return Ok(());
        }
        std::fs::create_dir_all(crate::platform::config_dir())?;
        std::fs::write(&p, serde_json::to_string_pretty(m)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
        let mut m = read();
        m.insert(format!("{service}/{account}"), value.to_string());
        write(&m)
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let mut m = read();
        if m.remove(&format!("{service}/{account}")).is_some() {
            write(&m)?;
        }
        Ok(())
    }

    pub fn lookup(service: &str, account: Option<&str>) -> Option<String> {
        let m = read();
        match account {
            Some(a) => m.get(&format!("{service}/{a}")).cloned(),
            None => m.iter().find(|(k, _)| k.starts_with(&format!("{service}/"))).map(|(_, v)| v.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Save, find (by account and by service), and forget a token in this
    /// OS's store (Credential Manager on the Windows CI runner).
    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("margin-secrets-test-{}", std::process::id()));
        // SAFETY: tests that read MARGIN_CONFIG_DIR don't run concurrently with this one's use
        unsafe { std::env::set_var("MARGIN_CONFIG_DIR", &dir) };
        let service = format!("margin-test-{}", std::process::id());
        if cfg!(target_os = "macos") {
            return; // CI Macs have no unlocked login keychain for the test user
        }
        store(&service, "me@example.com", "s3cret").unwrap();
        // with a secure store, nothing lands in the plain file
        assert_eq!(dir.join("secrets.json").exists(), !os::available());
        assert_eq!(lookup(&service, Some("me@example.com")).as_deref(), Some("s3cret"));
        assert_eq!(lookup(&service, None).as_deref(), Some("s3cret"));
        delete(&service, "me@example.com").unwrap();
        assert_eq!(lookup(&service, Some("me@example.com")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
