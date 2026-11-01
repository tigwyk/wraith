//! Shell integration scripts (spec §3.5): embedded in the binary, written once to a stable folder
//! so a shell can be pointed at them.
use std::io;
use std::path::{Path, PathBuf};

/// (path relative to the scripts folder, contents)
pub fn scripts() -> &'static [(&'static str, &'static str)] {
    &[
        ("integration.ps1", include_str!("../assets/shell/integration.ps1")),
        ("integration.bash", include_str!("../assets/shell/integration.bash")),
        ("zsh/.zshrc", include_str!("../assets/shell/zsh/.zshrc")),
    ]
}

/// Write the scripts to `base/v<version>/`, replacing any file whose contents differ. Returns that folder.
pub fn ensure_scripts_in(base: &Path) -> io::Result<PathBuf> {
    let dir = base.join(format!("v{}", env!("CARGO_PKG_VERSION")));
    for (rel, body) in scripts() {
        let path = dir.join(rel);
        if std::fs::read_to_string(&path).is_ok_and(|have| have == *body) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)?;
    }
    Ok(dir)
}

/// `%LOCALAPPDATA%\wraith\shell-integration\v<version>\`
pub fn ensure_scripts() -> io::Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    ensure_scripts_in(&local.join("wraith").join("shell-integration"))
}
