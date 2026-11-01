//! Typed key table: key name → setter on `Config`.
use crate::{Config, CursorStyle, Rgb, RightClick};

/// Set `key` from `value`. An empty value resets the key to its default.
pub fn apply(c: &mut Config, key: &str, value: &str) -> Result<(), String> {
    let d = Config::default();
    macro_rules! set {
        ($field:ident, $parse:expr) => {{
            c.$field = if value.is_empty() { d.$field } else { $parse(value)? };
        }};
    }
    match key {
        "font-family" => {
            if value.is_empty() {
                c.font_family.clear();
                c.font_family_set = true;
            } else {
                // The first explicit family replaces the default list.
                if !c.font_family_set {
                    c.font_family.clear();
                    c.font_family_set = true;
                }
                c.font_family.push(value.to_string());
            }
        }
        "font-feature" => {
            if value.is_empty() {
                c.font_feature.clear();
            } else {
                c.font_feature.push(value.to_string());
            }
        }
        "font-size" => set!(font_size, |v: &str| num(v).filter(|n| *n > 0.0).ok_or_else(|| bad(v))),
        "background" => set!(background, rgb),
        "foreground" => set!(foreground, rgb),
        "cursor-color" => set!(cursor_color, |v| rgb(v).map(Some)),
        "cursor-text" => set!(cursor_text, |v| rgb(v).map(Some)),
        "selection-background" => set!(selection_background, |v| rgb(v).map(Some)),
        "selection-foreground" => set!(selection_foreground, |v| rgb(v).map(Some)),
        "palette" => {
            if value.is_empty() {
                c.palette = d.palette;
            } else {
                let (i, col) = value.split_once('=').ok_or_else(|| bad(value))?;
                let i: u8 = i.trim().parse().map_err(|_| bad(value))?;
                c.palette[i as usize] = rgb(col.trim())?;
            }
        }
        "background-opacity" => set!(background_opacity, |v: &str| num(v)
            .filter(|n| (0.0..=1.0).contains(n))
            .ok_or_else(|| bad(v))),
        "cursor-style" => set!(cursor_style, |v| match v {
            "block" => Ok(CursorStyle::Block),
            "bar" => Ok(CursorStyle::Bar),
            "underline" => Ok(CursorStyle::Underline),
            "block_hollow" => Ok(CursorStyle::BlockHollow),
            _ => Err(bad(v)),
        }),
        "cursor-style-blink" => set!(cursor_style_blink, boolean),
        "window-padding-x" => set!(window_padding_x, |v: &str| v.parse().map_err(|_| bad(v))),
        "window-padding-y" => set!(window_padding_y, |v: &str| v.parse().map_err(|_| bad(v))),
        "scrollback-limit" => {
            // Bytes, like Ghostty. Capped so a typo cannot make memory unbounded (spec 6.3).
            let n: u64 = if value.is_empty() { d.scrollback_limit as u64 } else { value.parse().map_err(|_| bad(value))? };
            c.scrollback_limit = n.min(crate::MAX_SCROLLBACK_BYTES as u64) as usize;
            if n > crate::MAX_SCROLLBACK_BYTES as u64 {
                return Err(format!("scrollback-limit {n} exceeds 1 GiB; using 1 GiB"));
            }
        }
        "copy-on-select" => set!(copy_on_select, boolean),
        "right-click-action" => set!(right_click_action, |v| match v {
            "paste" => Ok(RightClick::Paste),
            "context-menu" | "menu" => Ok(RightClick::Menu),
            _ => Err(bad(v)),
        }),
        "command" => set!(command, |v: &str| Ok::<_, String>(Some(v.to_string()))),
        "close-on-exit" => set!(close_on_exit, boolean),
        "keybind" => crate::keybind::apply(&mut c.keybinds, value)?,
        _ => return Err(format!("unknown key `{key}`")),
    }
    Ok(())
}

fn bad(v: &str) -> String {
    format!("invalid value `{v}`")
}

fn num(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|n| n.is_finite())
}

fn boolean(v: &str) -> Result<bool, String> {
    match v {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(bad(v)),
    }
}

pub fn rgb(v: &str) -> Result<Rgb, String> {
    let h = v.strip_prefix('#').unwrap_or(v);
    if h.len() != 6 || !h.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(bad(v));
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap();
    Ok(Rgb(b(0), b(2), b(4)))
}
