//! Destination-folder template rendering and capture-date extraction.

use chrono::{DateTime, Local, Utc};
use std::time::SystemTime;

/// Render a layout template like `{year}/{date} {name}` into a relative folder path.
///
/// Empty `name` leaves no trailing junk: each `/`-separated segment is trimmed and
/// empty segments are dropped, so `{date} {name}` with no name becomes just the date.
pub fn render(template: &str, year: &str, date: &str, name: &str) -> String {
    let raw = template
        .replace("{year}", year)
        .replace("{date}", date)
        .replace("{name}", name);
    raw.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// `(year, date)` = `("YYYY", "YYYY-MM-DD")` from a file's mtime, in local time.
///
/// Camera-written mtime ≈ capture time, and FAT/exFAT stores wall-clock local time,
/// so interpreting in the machine's local zone round-trips correctly.
pub fn date_parts(mtime: SystemTime) -> (String, String) {
    let dt: DateTime<Local> = DateTime::<Utc>::from(mtime).with_timezone(&Local);
    (dt.format("%Y").to_string(), dt.format("%Y-%m-%d").to_string())
}
