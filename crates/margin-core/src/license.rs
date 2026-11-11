//! Margin Pro licensing: offline-verified keys and a 14-day trial.
//!
//! A key is `MARGIN1.<payload>.<signature>` (base64url): the payload is a
//! small JSON document (name, email, "updates until" date) signed with
//! Ed25519. The app only *verifies* with the public key compiled into it, so
//! there's no licence server and nothing to phone home (works offline and on
//! locked-down corporate networks).
//!
//! "One-time purchase + updates": a key unlocks Pro forever in every Margin
//! version built on or before its `updates_until` date. A newer build needs a
//! renewed key.
//!
//! Release builds take the public key from `MARGIN_LICENSE_PUBKEY` (hex) at
//! compile time. Without it, a built-in DEVELOPMENT key is used whose private
//! seed is public (below) — fine for testing, never for shipping.

use crate::{Error, Result, bail, config};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_compact::{KeyPair, PublicKey, Seed, Signature};
use serde::{Deserialize, Serialize};

pub const TRIAL_DAYS: i64 = 14;
const PREFIX: &str = "MARGIN1";
/// Development signing seed — public on purpose, only for builds without
/// `MARGIN_LICENSE_PUBKEY`.
pub const DEV_SEED: [u8; 32] = *b"margin-development-licence-seed!";
/// Date this binary was built (YYYY-MM-DD), set by build.rs.
pub const BUILD_DATE: &str = env!("MARGIN_BUILD_DATE");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub v: u32,
    pub name: String,
    pub email: String,
    pub edition: String,
    /// YYYY-MM-DD
    pub issued: String,
    /// Versions built on or before this date (YYYY-MM-DD) are covered.
    pub updates_until: String,
    pub id: String,
}

impl License {
    pub fn covers_build(&self, build_date: &str) -> bool {
        build_date <= self.updates_until.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Valid key covering this build.
    Pro(License),
    /// No (usable) key, trial running.
    Trial { days_left: i64 },
    /// Valid key, but this build is newer than its updates window.
    NeedsRenewal(License),
    TrialExpired,
}

impl Status {
    /// Pro features unlocked (licensed or in trial).
    pub fn is_pro(&self) -> bool {
        matches!(self, Status::Pro(_) | Status::Trial { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Status::Pro(l) => format!("Margin Pro — licensed to {} <{}> · updates until {}", l.name, l.email, l.updates_until),
            Status::Trial { days_left } => format!("Margin Pro trial — {days_left} day{} left", if *days_left == 1 { "" } else { "s" }),
            Status::NeedsRenewal(l) => format!(
                "Your licence covers Margin versions built until {}; this version was built {BUILD_DATE}. Renew to unlock Pro here.",
                l.updates_until
            ),
            Status::TrialExpired => "Margin Pro trial has ended".into(),
        }
    }
}

pub fn public_key() -> PublicKey {
    if let Some(hex) = option_env!("MARGIN_LICENSE_PUBKEY") {
        if let Some(pk) = from_hex(hex).and_then(|b| PublicKey::from_slice(&b).ok()) {
            return pk;
        }
    }
    KeyPair::from_seed(Seed::new(DEV_SEED)).pk
}

/// True when this build accepts development keys (no release key compiled in).
pub fn is_dev_build() -> bool {
    option_env!("MARGIN_LICENSE_PUBKEY").is_none()
}

/// Sign a licence (used by the `margin-license` issuing tool).
pub fn sign(lic: &License, seed: &[u8; 32]) -> Result<String> {
    let kp = KeyPair::from_seed(Seed::new(*seed));
    let payload = serde_json::to_vec(lic)?;
    let sig = kp.sk.sign(&payload, None);
    Ok(format!("{PREFIX}.{}.{}", B64.encode(&payload), B64.encode(sig.as_ref())))
}

pub fn public_key_hex(seed: &[u8; 32]) -> String {
    to_hex(KeyPair::from_seed(Seed::new(*seed)).pk.as_ref())
}

/// Verify a key's signature and decode it (does not check the build date).
pub fn verify(key: &str, pk: &PublicKey) -> Result<License> {
    // Keys are often pasted with line breaks or spaces.
    let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parts = key.split('.');
    let (Some(PREFIX), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        bail!("not a Margin licence key");
    };
    let payload = B64.decode(p).map_err(|_| Error("licence key is damaged".into()))?;
    let sig = B64.decode(s).map_err(|_| Error("licence key is damaged".into()))?;
    let sig = Signature::from_slice(&sig).map_err(|_| Error("licence key is damaged".into()))?;
    pk.verify(&payload, &sig).map_err(|_| Error("licence key is not valid".into()))?;
    let lic: License = serde_json::from_slice(&payload)?;
    if lic.edition != "pro" {
        bail!("unknown licence edition '{}'", lic.edition);
    }
    Ok(lic)
}

fn key_path() -> std::path::PathBuf {
    config::config_dir().join("license.key")
}

/// Current status: installed key first, otherwise the trial (started on first call).
pub fn current(now_unix: i64) -> Status {
    let pk = public_key();
    let mut renewal = None;
    if let Ok(k) = std::fs::read_to_string(key_path()) {
        if let Ok(lic) = verify(&k, &pk) {
            if lic.covers_build(BUILD_DATE) {
                return Status::Pro(lic);
            }
            renewal = Some(lic);
        }
    }
    let mut st = config::load_state();
    let started = match st.trial_started {
        Some(t) => t,
        None => {
            st.trial_started = Some(now_unix);
            let _ = config::save_state(&st);
            now_unix
        }
    };
    let days_left = TRIAL_DAYS - (now_unix - started).max(0) / 86_400;
    match (days_left > 0, renewal) {
        (true, _) => Status::Trial { days_left },
        (false, Some(lic)) => Status::NeedsRenewal(lic),
        (false, None) => Status::TrialExpired,
    }
}

/// Verify and store a key. Refuses keys that don't cover this build.
pub fn install(key: &str) -> Result<License> {
    let lic = verify(key, &public_key())?;
    if !lic.covers_build(BUILD_DATE) {
        bail!("this key covers versions built until {}, but this version was built {BUILD_DATE}", lic.updates_until);
    }
    let dir = config::config_dir();
    std::fs::create_dir_all(&dir)?;
    let clean: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    std::fs::write(key_path(), clean)?;
    Ok(lic)
}

pub fn remove() -> Result<()> {
    match std::fs::remove_file(key_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    (s.len() % 2 == 0)
        .then(|| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>())
        .flatten()
}

/// Parse a 32-byte seed (hex), e.g. from the issuing tool's private key file.
pub fn seed_from_hex(s: &str) -> Result<[u8; 32]> {
    let b = from_hex(s).ok_or_else(|| Error("private key file is not hex".into()))?;
    b.try_into().map_err(|_| Error("private key must be 32 bytes".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lic(until: &str) -> License {
        License {
            v: 1,
            name: "Kim Dev".into(),
            email: "kim@example.com".into(),
            edition: "pro".into(),
            issued: "2026-11-11".into(),
            updates_until: until.into(),
            id: "t-1".into(),
        }
    }

    #[test]
    fn sign_verify_roundtrip_and_tamper() {
        let pk = KeyPair::from_seed(Seed::new(DEV_SEED)).pk;
        let key = sign(&lic("2027-11-11"), &DEV_SEED).unwrap();
        assert!(key.starts_with("MARGIN1."));
        assert_eq!(verify(&key, &pk).unwrap().name, "Kim Dev");
        // pasted with line breaks
        let wrapped: String = key.chars().collect::<Vec<_>>().chunks(20).map(|c| c.iter().collect::<String>() + "\n").collect();
        assert!(verify(&wrapped, &pk).is_ok());
        // tampered payload (change the date) → rejected
        let mut parts: Vec<String> = key.split('.').map(String::from).collect();
        let mut p: License = serde_json::from_slice(&B64.decode(&parts[1]).unwrap()).unwrap();
        p.updates_until = "2099-01-01".into();
        parts[1] = B64.encode(serde_json::to_vec(&p).unwrap());
        assert!(verify(&parts.join("."), &pk).is_err());
        // signed by another key → rejected
        let other = sign(&lic("2027-11-11"), &[9u8; 32]).unwrap();
        assert!(verify(&other, &pk).is_err());
        assert!(verify("hello", &pk).is_err());
    }

    #[test]
    fn updates_window() {
        assert!(lic("2027-11-11").covers_build("2026-11-11"));
        assert!(lic("2026-11-11").covers_build("2026-11-11"));
        assert!(!lic("2026-11-10").covers_build("2026-11-11"));
    }

    #[test]
    fn trial_and_install() {
        // isolated config dir for this test
        let d = std::env::temp_dir().join(format!("margin-lic-{}-{}", std::process::id(), crate::time::now_unix()));
        unsafe { std::env::set_var("MARGIN_CONFIG_DIR", &d) };
        let t0 = 1_800_000_000;
        assert_eq!(current(t0), Status::Trial { days_left: 14 });
        assert_eq!(current(t0 + 86_400 * 13 + 5), Status::Trial { days_left: 1 });
        assert_eq!(current(t0 + 86_400 * 14), Status::TrialExpired);
        assert!(!current(t0 + 86_400 * 30).is_pro());
        // an expired-updates key can't be installed on this build
        assert!(install(&sign(&lic("2000-01-01"), &DEV_SEED).unwrap()).is_err());
        install(&sign(&lic("2999-01-01"), &DEV_SEED).unwrap()).unwrap();
        assert!(matches!(current(t0 + 86_400 * 30), Status::Pro(_)));
        remove().unwrap();
        assert_eq!(current(t0 + 86_400 * 30), Status::TrialExpired);
        unsafe { std::env::remove_var("MARGIN_CONFIG_DIR") };
    }
}
