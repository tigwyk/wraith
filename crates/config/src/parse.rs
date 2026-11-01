//! Ghostty-syntax lines: `key = value`, `#` comments, optional quotes.

/// One `key = value` entry with its 1-based line number.
pub struct Line<'a> {
    pub no: usize,
    pub key: &'a str,
    pub value: &'a str,
}

/// Lines of `src`, ignoring a UTF-8 BOM, CRLF, blanks and comments.
/// A line without `=` comes back as `Err((line_no, message))`.
pub fn lines(src: &str) -> impl Iterator<Item = Result<Line<'_>, (usize, String)>> {
    src.strip_prefix('\u{feff}').unwrap_or(src).lines().enumerate().filter_map(|(i, raw)| {
        let no = i + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            return None;
        }
        let Some((k, v)) = t.split_once('=') else {
            return Some(Err((no, format!("expected `key = value`, got `{t}`"))));
        };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
        Some(Ok(Line { no, key: k.trim(), value: v }))
    })
}
