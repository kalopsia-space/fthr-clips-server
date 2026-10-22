use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Utc};

/// (shape, chrono format); `d` in a shape means an ASCII digit, anything else is literal.
const PATTERNS: [(&str, &str); 4] = [
    ("dddd-dd-dd dd-dd-dd", "%Y-%m-%d %H-%M-%S"),
    ("dddd-dd-dd_dd-dd-dd", "%Y-%m-%d_%H-%M-%S"),
    ("dddddddd_dddddd", "%Y%m%d_%H%M%S"),
    ("dddd-dd-dd", "%Y-%m-%d"),
];

fn matches_shape(s: &[u8], shape: &str) -> bool {
    s.len() == shape.len()
        && s.iter().zip(shape.bytes()).all(|(c, p)| {
            if p == b'd' {
                c.is_ascii_digit()
            } else {
                *c == p
            }
        })
}

fn parse_at(name: &str, start: usize) -> Option<NaiveDateTime> {
    for (shape, fmt) in PATTERNS {
        let Some(slice) = name.get(start..start + shape.len()) else {
            continue;
        };
        if !matches_shape(slice.as_bytes(), shape) {
            continue;
        }
        let parsed = if shape.len() == 10 {
            NaiveDate::parse_from_str(slice, fmt)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        } else {
            NaiveDateTime::parse_from_str(slice, fmt).ok()
        };
        if parsed.is_some() {
            return parsed;
        }
    }
    None
}

/// Extracts a recording time embedded in a file name (local time, converted to UTC).
pub fn recorded_at_from_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.char_indices()
        .find_map(|(i, _)| parse_at(stem, i))
        .and_then(|naive| naive.and_local_timezone(Local).earliest())
        .map(|local| local.with_timezone(&Utc))
}
