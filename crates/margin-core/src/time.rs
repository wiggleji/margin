//! Tiny date helpers so we don't need chrono.

/// Format a unix timestamp as `YYYY-MM-DD HH:MM` (UTC).
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Offset of the machine's time zone from UTC (seconds) at `secs`, including
/// daylight saving. 0 where it can't be read.
pub fn local_offset(secs: i64) -> i64 {
    local_tm(secs).map_or(0, |(off, _)| off)
}

/// Short name of the machine's time zone at `secs` ("KST", "PDT"), or the
/// numeric offset ("+09:00") when the system has no abbreviation.
pub fn local_zone(secs: i64) -> String {
    match local_tm(secs) {
        Some((_, Some(name))) => name,
        Some((off, None)) => format!("{}{:02}:{:02}", if off < 0 { '-' } else { '+' }, off.abs() / 3600, off.abs() % 3600 / 60),
        None => "UTC".into(),
    }
}

/// Format a unix timestamp as `YYYY-MM-DD HH:MM` in the machine's time zone.
pub fn format_local(secs: i64) -> String {
    format_utc(secs + local_offset(secs))
}

fn local_tm(secs: i64) -> Option<(i64, Option<String>)> {
    crate::platform::local_offset(secs)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00");
        assert_eq!(super::format_utc(1_762_758_000), "2025-11-10 07:00");
    }

    /// Run with TZ set (e.g. `TZ=Asia/Seoul cargo test local_time`).
    #[test]
    #[cfg(unix)]
    fn local_time() {
        match std::env::var("TZ").as_deref() {
            Ok("Asia/Seoul") => {
                assert_eq!(super::local_offset(1_762_758_000), 9 * 3600);
                assert_eq!(super::format_local(1_762_758_000), "2025-11-10 16:00");
                assert_eq!(super::local_zone(1_762_758_000), "KST");
            }
            Ok("America/Los_Angeles") => {
                // November 10: PST (UTC-8); July: PDT (UTC-7)
                assert_eq!(super::local_offset(1_762_758_000), -8 * 3600);
                assert_eq!(super::local_offset(1_752_000_000), -7 * 3600);
            }
            _ => {}
        }
    }
}
