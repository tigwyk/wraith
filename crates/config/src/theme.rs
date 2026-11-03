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

/// Theme `name` from `dir` as a scheme (the colours it sets over Wraith's defaults).
pub fn scheme_from_theme(dir: &Path, name: &str) -> Option<crate::wt::Scheme> {
    let mut c = Config::default();
    apply(&mut c, dir, name).ok()?;
    Some(crate::wt::Scheme { fg: c.foreground, bg: c.background, cursor: c.cursor_color, palette: c.palette[..16].try_into().ok()? })
}

/// Apply theme `name` from `dir` onto `c`.
pub fn apply(c: &mut Config, dir: &Path, name: &str) -> Result<(), String> {
    // Theme names are file names; refuse path tricks.
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("invalid theme name `{name}`"));
    }
    // The user's own themes (`wr import-wt` writes there) shadow the bundled ones.
    let user = std::env::var_os("APPDATA").map(|a| Path::new(&a).join("wraith").join("themes").join(name));
    let src = user
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| std::fs::read_to_string(dir.join(name)).ok())
        .ok_or_else(|| format!("unknown theme `{name}`"))?;
    for line in parse::lines(&src).flatten() {
        if COLOR_KEYS.contains(&line.key) {
            keys::apply(c, line.key, line.value)
                .map_err(|e| format!("theme `{name}` line {}: {e}", line.no))?;
        }
    }
    Ok(())
}
