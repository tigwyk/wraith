//! The profile list a window offers (wt-profiles spec §3): Windows Terminal's profiles when its
//! settings could be read, else the detected shells; then the config's `profile` and `profile-*` lines.
use wraith_config::wt::WtSettings;
use wraith_config::{Config, ProfileExtra, font_list, font_size, opacity, padding};
use wraith_server::profile::{Profile, from_config};

pub struct Merged {
    pub profiles: Vec<Profile>,
    pub default: Option<String>,
    /// Problems worth a log line: WT settings that did not parse, `profile-*` lines naming no profile.
    pub warnings: Vec<String>,
}

/// WT's profiles replace detection (WT already lists the same shells under its own names, so both
/// would show each WSL distro twice); `detected` runs only without them.
pub fn merge(wt: Option<&Result<WtSettings, String>>, detected: impl FnOnce() -> Vec<Profile>, config: &Config) -> Merged {
    let mut warnings = Vec::new();
    let mut detected = Some(detected);
    let mut detect = || detected.take().map_or_else(Vec::new, |f| f());
    let wt = wt.filter(|_| config.windows_terminal_profiles);
    let (mut profiles, wt_default) = match wt {
        Some(Ok(s)) => {
            let ps = s
                .profiles
                .iter()
                .map(|p| Profile { dir: p.dir.clone(), icon: p.icon.clone(), scheme: p.scheme.clone(), font: p.font.clone(), font_size: p.font_size, padding: p.padding, opacity: p.opacity, ..from_config(&p.name, &p.command) })
                .collect();
            (ps, s.default.clone())
        }
        Some(Err(e)) => {
            warnings.push(format!("Windows Terminal profiles not loaded: {e}"));
            (if config.profile_detect { detect() } else { Vec::new() }, None)
        }
        None => (if config.profile_detect { detect() } else { Vec::new() }, None),
    };
    for (name, cmd) in &config.profiles {
        let p = from_config(name, cmd);
        match profiles.iter().position(|q| q.name.eq_ignore_ascii_case(name)) {
            Some(i) => profiles[i] = p,
            None => profiles.push(p),
        }
    }
    for (name, kind, value) in &config.profile_extras {
        let Some(p) = profiles.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) else {
            let key = match kind {
                ProfileExtra::Dir => "profile-dir",
                ProfileExtra::Theme => "profile-theme",
                ProfileExtra::Icon => "profile-icon",
                ProfileExtra::Font => "profile-font",
                ProfileExtra::FontSize => "profile-font-size",
                ProfileExtra::Padding => "profile-padding",
                ProfileExtra::Opacity => "profile-opacity",
            };
            warnings.push(format!("{key} names `{name}`, which is not a profile"));
            continue;
        };
        // keys.rs already rejected values that do not parse, so a `None` here is ignored.
        match kind {
            ProfileExtra::Dir => p.dir = Some(value.clone()),
            ProfileExtra::Theme => p.scheme = Some(value.clone()),
            ProfileExtra::Icon => p.icon = Some(value.clone()),
            ProfileExtra::Font => p.font = font_list(value).or(p.font.take()),
            ProfileExtra::FontSize => p.font_size = font_size(value).or(p.font_size),
            ProfileExtra::Padding => p.padding = padding(value).or(p.padding),
            ProfileExtra::Opacity => p.opacity = opacity(value).or(p.opacity),
        }
    }
    if profiles.is_empty() {
        // A window always needs a shell to start.
        profiles = detect();
    }
    Merged { profiles, default: config.default_profile.clone().or(wt_default), warnings }
}
