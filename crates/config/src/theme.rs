//! Ghostty theme files: the config syntax restricted to color keys.
use std::path::Path;

use crate::{Config, keys, parse};

/// Keys a theme may set; anything else in a theme file is ignored.
pub const COLOR_KEYS: &[&str] = &[
    "palette",
    "background",
    "foreground",
    "cursor-color",
    "cursor-text",
    "selection-background",
    "selection-foreground",
];

/// Apply theme `name` from `dir` onto `c`.
pub fn apply(c: &mut Config, dir: &Path, name: &str) -> Result<(), String> {
    // Theme names are file names; refuse path tricks.
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("invalid theme name `{name}`"));
    }
    let src = std::fs::read_to_string(dir.join(name)).map_err(|_| format!("unknown theme `{name}`"))?;
    for line in parse::lines(&src).flatten() {
        if COLOR_KEYS.contains(&line.key) {
            keys::apply(c, line.key, line.value)
                .map_err(|e| format!("theme `{name}` line {}: {e}", line.no))?;
        }
    }
    Ok(())
}
