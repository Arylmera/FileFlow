//! Destination-folder template rendering and capture-date extraction.

use chrono::{DateTime, Local, Utc};
use std::time::SystemTime;

/// Replace `{token}` placeholders in `template` with their values.
fn substitute(template: &str, tokens: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (k, v) in tokens {
        s = s.replace(k, v);
    }
    s
}

/// Render a layout template like `{year}/{date} {name}` into a relative folder path.
///
/// Empty `name` leaves no trailing junk: each `/`-separated segment is trimmed and
/// empty segments are dropped, so `{date} {name}` with no name becomes just the date.
/// `.` and `..` segments are also dropped, so a user-typed `{name}` (free text from the
/// naming form) can never climb out of the destination root — the result is always a
/// relative path with no traversal components.
pub fn render(template: &str, year: &str, date: &str, name: &str) -> String {
    let raw = substitute(template, &[("{year}", year), ("{date}", date), ("{name}", name)]);
    raw.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// Render a filename template like `{date}_{seq}` into a single filename.
///
/// Same tokens as [`render`] plus `{seq}` (a 1-based sequence, zero-padded to 4).
/// Path separators are flattened to `-` (a filename has no folders) and the original
/// `ext` is always re-appended, so a template can never drop the extension. A template
/// that renders to nothing falls back to the sequence, never a hidden/empty name.
// ponytail: {seq} is fixed 4-wide; add {seq:N} only if someone shoots >9999/day and cares.
pub fn render_filename(
    template: &str,
    year: &str,
    date: &str,
    name: &str,
    seq: usize,
    ext: &str,
) -> String {
    let seq_str = format!("{seq:04}");
    let stem = substitute(
        template,
        &[("{year}", year), ("{date}", date), ("{name}", name), ("{seq}", &seq_str)],
    )
    .replace('/', "-");
    let stem = stem.trim();
    // A bare "." or ".." would resolve to the dest dir or its parent once joined — never a
    // real filename. Fall back to the sequence, same as an empty render.
    let stem = if stem.is_empty() || stem == "." || stem == ".." { seq_str.as_str() } else { stem };
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

/// `(year, date)` = `("YYYY", "YYYY-MM-DD")` from a file's mtime, in local time.
///
/// Camera-written mtime ≈ capture time, and FAT/exFAT stores wall-clock local time,
/// so interpreting in the machine's local zone round-trips correctly.
pub fn date_parts(mtime: SystemTime) -> (String, String) {
    let dt: DateTime<Local> = DateTime::<Utc>::from(mtime).with_timezone(&Local);
    (dt.format("%Y").to_string(), dt.format("%Y-%m-%d").to_string())
}
