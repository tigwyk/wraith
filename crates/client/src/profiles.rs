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

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub families: Vec<String>,
    pub size_pt: f32,
    pub pad: (u16, u16),
    pub opacity: f32,
    /// The profile names fonts but none is installed, so only the window's families are used.
    pub fell_back: bool,
}

/// `p`'s appearance over the global config, plus a zoom delta; size clamped 4–72. `installed` says
/// whether a family exists; when none of the profile's does, the window families are used.
pub fn appearance(p: Option<&Profile>, c: &Config, zoom: f32, installed: &dyn Fn(&str) -> bool) -> Appearance {
    let named = p.and_then(|p| p.font.as_ref());
    let own = named.filter(|f| f.iter().any(|f| installed(f)));
    let mut families: Vec<String> = own.into_iter().flatten().cloned().collect();
    for f in &c.font_family {
        if !families.iter().any(|g| g.eq_ignore_ascii_case(f)) {
            families.push(f.clone());
        }
    }
    Appearance {
        families,
        size_pt: crate::app::clamp_font_size(p.and_then(|p| p.font_size).unwrap_or(c.font_size) + zoom),
        pad: p.and_then(|p| p.padding).unwrap_or((c.window_padding_x, c.window_padding_y)),
        opacity: p.and_then(|p| p.opacity).unwrap_or(c.background_opacity),
        fell_back: named.is_some() && own.is_none(),
    }
}
