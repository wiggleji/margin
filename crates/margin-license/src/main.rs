//! margin-license — issue Margin Pro licence keys.
//!
//!   margin-license keygen <dir>
//!       Creates <dir>/margin-license.private (KEEP SECRET, never commit) and
//!       prints the public key to build releases with:
//!       MARGIN_LICENSE_PUBKEY=<hex> cargo build --release
//!
//!   margin-license issue <private-file> --name "Kim Dev" --email kim@example.com
//!                        [--updates-until 2027-11-11 | --years 1] [--id ORDER-123]
//!       Prints a licence key (one-time purchase: covers versions built until
//!       the date; default one year of updates).
//!
//!   margin-license verify <key> [--public <hex>]
//!       Checks a key and prints its contents.
//!
//! Without a private file, `--dev` signs with the public development seed
//! (keys only work in builds made without MARGIN_LICENSE_PUBKEY).

use margin_core::license::{self, License};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("issue") => issue(&args[1..]),
        Some("verify") => verify(&args[1..]),
        _ => Err("usage: margin-license keygen <dir> | issue <private-file|--dev> --name N --email E [--updates-until YYYY-MM-DD | --years N] [--id ID] | verify <key> [--public HEX]".into()),
    };
    if let Err(e) = res {
        eprintln!("margin-license: {e}");
        exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn keygen(args: &[String]) -> Result<(), String> {
    let dir = std::path::PathBuf::from(args.first().ok_or("keygen needs an output directory")?);
    let path = dir.join("margin-license.private");
    if path.exists() {
        return Err(format!("{} already exists; refusing to overwrite a signing key", path.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let seed = ed25519_compact::Seed::generate();
    let hex = license::to_hex(seed.as_ref());
    std::fs::write(&path, format!("{hex}\n")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let mut s = [0u8; 32];
    s.copy_from_slice(seed.as_ref());
    println!("Private key written to {} (keep it secret; back it up; never commit it).", path.display());
    println!("Public key (build releases with it):\n  MARGIN_LICENSE_PUBKEY={}", license::public_key_hex(&s));
    Ok(())
}

fn issue(args: &[String]) -> Result<(), String> {
    let seed = match args.first().map(String::as_str) {
        Some("--dev") => license::DEV_SEED,
        Some(file) => {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            license::seed_from_hex(&text).map_err(|e| e.0)?
        }
        None => return Err("issue needs the private key file (or --dev)".into()),
    };
    let name = flag(args, "--name").ok_or("--name is required")?;
    let email = flag(args, "--email").ok_or("--email is required")?;
    let today = margin_core::format_utc(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
    )[..10]
        .to_string();
    let until = match flag(args, "--updates-until") {
        Some(d) => d,
        None => {
            let years: i32 = flag(args, "--years").map(|y| y.parse().map_err(|_| "--years must be a number")).transpose()?.unwrap_or(1);
            let y: i32 = today[..4].parse().unwrap_or(2026);
            format!("{}{}", y + years, &today[4..])
        }
    };
    if until.len() != 10 || until.as_bytes()[4] != b'-' || until.as_bytes()[7] != b'-' {
        return Err("--updates-until must be YYYY-MM-DD".into());
    }
    let lic = License {
        v: 1,
        name,
        email,
        edition: "pro".into(),
        issued: today.clone(),
        updates_until: until,
        id: flag(args, "--id").unwrap_or_else(|| format!("{}-{}", today.replace('-', ""), std::process::id())),
    };
    println!("{}", license::sign(&lic, &seed).map_err(|e| e.0)?);
    Ok(())
}

fn verify(args: &[String]) -> Result<(), String> {
    let key = args.first().ok_or("verify needs a key")?;
    let pk = match flag(args, "--public") {
        Some(h) => ed25519_compact::PublicKey::from_slice(&license::from_hex(&h).ok_or("bad --public hex")?).map_err(|e| e.to_string())?,
        None => license::public_key(),
    };
    let lic = license::verify(key, &pk).map_err(|e| e.0)?;
    println!("{}", serde_json::to_string_pretty(&lic).map_err(|e| e.to_string())?);
    println!("covers this build ({}): {}", license::BUILD_DATE, lic.covers_build(license::BUILD_DATE));
    Ok(())
}
