// Embeds the build date (licence keys cover versions built before their
// "updates until" date) and the release licence public key.
fn main() {
    println!("cargo:rerun-if-env-changed=MARGIN_LICENSE_PUBKEY");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let secs: i64 = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);
    // civil date from days (Howard Hinnant)
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    println!("cargo:rustc-env=MARGIN_BUILD_DATE={y:04}-{m:02}-{d:02}");
    if std::env::var("MARGIN_LICENSE_PUBKEY").is_err() && std::env::var("PROFILE").as_deref() == Ok("release") {
        println!("cargo:warning=MARGIN_LICENSE_PUBKEY not set: this release build accepts DEVELOPMENT licence keys only. Set it before shipping.");
    }
}
