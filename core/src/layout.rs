//! Destination-folder template rendering and capture-date extraction.

use chrono::{DateTime, Local, Utc};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::SystemTime;

/// What a file contributes to templates: its capture date/time and camera model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capture {
    pub year: String, // YYYY
    pub date: String, // YYYY-MM-DD
    pub time: String, // HHMMSS
    pub camera: String, // EXIF Model, "" when unknown
}

/// Capture metadata for `path`: EXIF `DateTimeOriginal` when the file has it (photos,
/// TIFF-based RAWs like ARW/NEF/DNG/CR2, HEIF), else the file's mtime. `None` only when
/// the file can't be stat'd. A Lightroom export keeps the shot's date this way instead
/// of the export time.
pub fn capture_of(path: &Path) -> Option<Capture> {
    let exif = read_exif(path);
    let field = |tag| {
        let f = exif.as_ref()?.get_field(tag, exif::In::PRIMARY)?;
        match &f.value {
            exif::Value::Ascii(v) => v.first().map(|b| String::from_utf8_lossy(b).trim().to_string()),
            _ => None,
        }
    };
    let mut cap = match field(exif::Tag::DateTimeOriginal).and_then(|s| parse_exif_datetime(&s)) {
        Some(c) => c,
        None => from_mtime(std::fs::metadata(path).ok()?.modified().ok()?),
    };
    cap.camera = field(exif::Tag::Model).unwrap_or_default().replace(['/', '\\'], "-");
    Some(cap)
}

/// How much of a TIFF-based file to read for its EXIF: the IFDs sit near the start, so a
/// prefix avoids reading a whole 50 MB RAW off the card just for its date.
const TIFF_PREFIX: u64 = 256 * 1024;

fn read_exif(path: &Path) -> Option<exif::Exif> {
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.try_clone().ok()?.take(TIFF_PREFIX).read_to_end(&mut head).ok()?;
    let mut reader = exif::Reader::new();
    reader.continue_on_error(true); // a truncated prefix still yields the early fields
    let partial = |r: Result<exif::Exif, exif::Error>| r.or_else(|e| e.distill_partial_result(|_| {})).ok();
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return partial(reader.read_raw(head));
    }
    // JPEG/HEIF/PNG/WebP parsers stop at the EXIF block; anything else fails fast.
    partial(reader.read_from_container(&mut BufReader::new(file)))
}

/// `"2026:06:20 14:03:22"` → capture parts; `None` for a malformed or blank stamp.
pub fn parse_exif_datetime(s: &str) -> Option<Capture> {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).filter(|d| d.iter().all(u8::is_ascii_digit)).map(|_| &s[r]);
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if y == "0000" || m == "00" || d == "00" {
        return None; // cameras write zeros when the clock was never set
    }
    let time = match (digits(11..13), digits(14..16), digits(17..19)) {
        (Some(h), Some(mi), Some(se)) => format!("{h}{mi}{se}"),
        _ => String::new(),
    };
    Some(Capture { year: y.into(), date: format!("{y}-{m}-{d}"), time, camera: String::new() })
}

fn from_mtime(mtime: SystemTime) -> Capture {
    let dt: DateTime<Local> = DateTime::<Utc>::from(mtime).with_timezone(&Local);
    Capture {
        year: dt.format("%Y").to_string(),
        date: dt.format("%Y-%m-%d").to_string(),
        time: dt.format("%H%M%S").to_string(),
        camera: String::new(),
    }
}

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
    render_cap(template, &Capture { year: year.into(), date: date.into(), ..Default::default() }, name)
}

/// [`render`] with every capture token: `{year}`, `{date}`, `{time}`, `{camera}`, `{name}`.
pub fn render_cap(template: &str, cap: &Capture, name: &str) -> String {
    // On Windows the rendered segments must also be legal NTFS/exFAT names, or the
    // path we verify against desyncs from the path the filesystem actually creates.
    #[cfg(windows)]
    return render_segments(template, cap, name)
        .iter()
        .map(|s| crate::windows::sanitize_segment(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    #[cfg(not(windows))]
    render_segments(template, cap, name).join("/")
}

/// Like [`render`], but every segment is passed through
/// [`crate::windows::sanitize_segment`] — what [`render`] does on Windows builds.
/// Exposed on all platforms so the behaviour is testable everywhere.
pub fn render_windows_safe(template: &str, year: &str, date: &str, name: &str) -> String {
    let cap = Capture { year: year.into(), date: date.into(), ..Default::default() };
    render_segments(template, &cap, name)
        .iter()
        .map(|s| crate::windows::sanitize_segment(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Substitute tokens and split into cleaned segments (shared by both renderers).
fn render_segments(template: &str, cap: &Capture, name: &str) -> Vec<String> {
    let raw = substitute(template, &cap_tokens(cap, name));
    raw.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .map(str::to_string)
        .collect()
}

/// Template tokens shared by folder layouts and filenames. `{name}` goes last so a
/// user-typed name containing a token string is never expanded.
fn cap_tokens<'a>(cap: &'a Capture, name: &'a str) -> [(&'static str, &'a str); 5] {
    [
        ("{year}", &cap.year),
        ("{date}", &cap.date),
        ("{time}", &cap.time),
        ("{camera}", &cap.camera),
        ("{name}", name),
    ]
}

/// Render a filename template like `{date}_{seq}` into a single filename.
///
/// Same tokens as [`render_cap`] plus `{seq}` (a 1-based sequence, zero-padded to 4) and
/// `{orig}` (the original file name without its extension).
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
    let cap = Capture { year: year.into(), date: date.into(), ..Default::default() };
    render_filename_cap(template, &cap, name, seq, ext, "")
}

/// [`render_filename`] with every capture token plus `{orig}`.
pub fn render_filename_cap(
    template: &str,
    cap: &Capture,
    name: &str,
    seq: usize,
    ext: &str,
    orig: &str,
) -> String {
    let seq_str = format!("{seq:04}");
    let [y, d, t, c, n] = cap_tokens(cap, name);
    let stem = substitute(template, &[y, d, t, c, ("{seq}", &seq_str), ("{orig}", orig), n])
        .replace('/', "-");
    let stem = stem.trim();
    // A bare "." or ".." would resolve to the dest dir or its parent once joined — never a
    // real filename. Fall back to the sequence, same as an empty render.
    let stem = if stem.is_empty() || stem == "." || stem == ".." { seq_str.as_str() } else { stem };
    // Same Windows-legality rule as folder segments in [`render`].
    #[cfg(windows)]
    let stem = {
        let s = crate::windows::sanitize_segment(stem);
        if s.is_empty() { seq_str.clone() } else { s }
    };
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
    let c = from_mtime(mtime);
    (c.year, c.date)
}
