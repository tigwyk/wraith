//! In-place edits of the config file for the settings panel: only the edited key's lines change, and
//! the file keeps its comments, order, line endings and encoding.
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

/// The first line of a config file that Settings creates.
pub const HEADER: &str = "# Wraith config — edited by hand and by Settings (ctrl+comma)";

/// Which line of a repeated key wins: list keys (`font-family`) take the first, the rest the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    First,
    Last,
}

/// Where a key's value in effect comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Here { line: usize },
    Included { file: PathBuf, line: usize },
    Default,
}

/// The same rule as `parse::lines`: not a comment, and the text before `=` is the key.
fn is_key(line: &str, key: &str) -> bool {
    let t = line.trim();
    !t.starts_with('#') && t.split_once('=').is_some_and(|(k, _)| k.trim() == key)
}

/// `src` without a UTF-8 BOM, the BOM, and the file's line ending.
fn split_bom(src: &str) -> (&str, &str, &'static str) {
    let (bom, body) = match src.strip_prefix('\u{feff}') {
        Some(b) => ("\u{feff}", b),
        None => ("", src),
    };
    (bom, body, if src.contains("\r\n") { "\r\n" } else { "\n" })
}

/// Rewrite the winning `key = …` line to `key = value`, or append one.
pub fn set(src: &str, key: &str, value: &str, which: Which) -> String {
    let (bom, body, nl) = split_bom(src);
    let mut lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut matches = (0..lines.len()).filter(|&i| is_key(lines[i], key));
    let hit = match which {
        Which::First => matches.next(),
        Which::Last => matches.next_back(),
    };
    let new = format!("{key} = {value}");
    let mut out = String::from(bom);
    match hit {
        Some(i) => {
            let old = lines[i];
            let ending = &old[old.trim_end_matches(['\r', '\n']).len()..];
            let line = format!("{new}{ending}");
            lines[i] = &line;
            out.extend(lines);
        }
        None => {
            out.push_str(body);
            if !body.is_empty() && !body.ends_with('\n') {
                out.push_str(nl);
            }
            out.push_str(&new);
            out.push_str(nl);
        }
    }
    out
}

/// Remove every `key = …` line.
pub fn unset(src: &str, key: &str) -> String {
    let (bom, body, _) = split_bom(src);
    let mut out = String::from(bom);
    out.extend(body.split_inclusive('\n').filter(|l| !is_key(l, key)));
    out
}

/// Where each of `keys` gets its value, following `config-file` includes in load order.
pub fn origins(path: &Path, keys: &[(&str, Which)]) -> Vec<Origin> {
    let mut entries = Vec::new();
    if let Ok(src) = read(path) {
        let mut stack = vec![crate::canon(path)];
        crate::collect(&src, path, &mut stack, &mut entries, &mut Vec::new());
    }
    let me = crate::canon(path);
    keys.iter()
        .map(|&(key, which)| {
            let mut m = entries.iter().filter(|e| e.key == key);
            let e = match which {
                Which::First => m.next(),
                Which::Last => m.next_back(),
            };
            match e {
                None => Origin::Default,
                Some(e) if crate::canon(&e.file) == me => Origin::Here { line: e.line },
                Some(e) => Origin::Included { file: e.file.clone(), line: e.line },
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

fn decode(bytes: &[u8]) -> Result<(String, Encoding)> {
    let utf16 = |be: bool| {
        if !bytes.len().is_multiple_of(2) {
            return Err(Error::new(ErrorKind::InvalidData, "the UTF-16 config file has a stray byte at the end"));
        }
        let units = bytes[2..].as_chunks::<2>().0.iter().map(|&c| if be { u16::from_be_bytes(c) } else { u16::from_le_bytes(c) });
        char::decode_utf16(units).collect::<std::result::Result<String, _>>().map_err(|e| Error::new(ErrorKind::InvalidData, e))
    };
    match bytes {
        [0xFF, 0xFE, ..] => Ok((utf16(false)?, Encoding::Utf16Le)),
        [0xFE, 0xFF, ..] => Ok((utf16(true)?, Encoding::Utf16Be)),
        _ => String::from_utf8(bytes.to_vec())
            .map(|s| (s, Encoding::Utf8))
            .map_err(|_| Error::new(ErrorKind::InvalidData, "the config file is not valid UTF-8")),
    }
}

fn read_enc(path: &Path) -> Result<(String, Encoding)> {
    match std::fs::read(path) {
        Ok(b) => decode(&b),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok((format!("{HEADER}\n"), Encoding::Utf8)),
        Err(e) => Err(e),
    }
}

/// The file's text. A missing file reads as just the header; a UTF-8 BOM stays as `\u{feff}`.
pub fn read(path: &Path) -> Result<String> {
    read_enc(path).map(|(s, _)| s)
}

/// Read the file fresh, apply `edit`, and replace the file atomically in its own encoding.
pub fn write(path: &Path, edit: impl FnOnce(&str) -> String) -> Result<()> {
    // Write through a symlinked config (dotfiles) rather than replacing the link with a file.
    let resolved = std::fs::canonicalize(path).ok();
    let path = resolved.as_deref().unwrap_or(path);
    // A rename replaces a read-only file on NTFS, so check first.
    if std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(Error::new(ErrorKind::PermissionDenied, "the config file is read-only"));
    }
    let (src, enc) = read_enc(path)?;
    let text = edit(&src);
    let bytes: Vec<u8> = match enc {
        Encoding::Utf8 => text.into_bytes(),
        Encoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
        Encoding::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    // `rename` is MoveFileExW(MOVEFILE_REPLACE_EXISTING) on Windows.
    let r = std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, path));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}
