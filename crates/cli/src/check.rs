//! `wr config check [path]` (spec §7): print every config error as `file:line: message`; any error is exit 1.
use std::path::{Path, PathBuf};

pub fn run(args: &[&str]) -> Result<(), String> {
    let path = match args {
        [] => wraith_config::default_path(),
        [p] => PathBuf::from(p),
        _ => return Err("usage: wr config check [path]".into()),
    };
    let diags = match wraith_config::load_checked(&path, &themes_dir()) {
        Ok((_, d)) => d,
        Err(d) => vec![d],
    };
    if diags.is_empty() {
        println!("wr: config ok ({})", path.display());
        return Ok(());
    }
    for d in &diags {
        eprintln!("{}:{}: {}", d.file.display(), d.line, d.message);
    }
    Err(format!("config has {} error(s)", diags.len()))
}

/// Themes ship beside the binaries; a dev build reads the repo's copy (as the client does).
pub fn themes_dir() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("themes")));
    beside.filter(|d| d.is_dir()).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes"))
}
