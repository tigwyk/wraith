//! Wraith config: Ghostty `key = value` syntax and key names (spec §7).
mod keys;
mod parse;
mod theme;

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
    BlockHollow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightClick {
    Paste,
    Menu,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub font_family: Vec<String>,
    font_family_set: bool,
    pub font_size: f32,
    pub font_feature: Vec<String>,
    pub theme: Option<String>,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor_color: Option<Rgb>,
    pub cursor_text: Option<Rgb>,
    pub selection_background: Option<Rgb>,
    pub selection_foreground: Option<Rgb>,
    pub palette: [Rgb; 256],
    pub background_opacity: f32,
    pub cursor_style: CursorStyle,
    pub cursor_style_blink: bool,
    pub window_padding_x: u16,
    pub window_padding_y: u16,
    /// Lines (libghostty-vt counts scrollback in lines).
    pub scrollback_limit: usize,
    pub copy_on_select: bool,
    pub right_click_action: RightClick,
    pub command: Option<String>,
    pub close_on_exit: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            font_family: vec!["Cascadia Mono".into()],
            font_family_set: false,
            font_size: 12.0,
            font_feature: Vec::new(),
            theme: None,
            background: Rgb(0x28, 0x2c, 0x34),
            foreground: Rgb(0xff, 0xff, 0xff),
            cursor_color: None,
            cursor_text: None,
            selection_background: None,
            selection_foreground: None,
            palette: xterm_palette(),
            background_opacity: 1.0,
            cursor_style: CursorStyle::Block,
            cursor_style_blink: true,
            window_padding_x: 2,
            window_padding_y: 2,
            scrollback_limit: 10_000,
            copy_on_select: false,
            right_click_action: RightClick::Paste,
            command: None,
            close_on_exit: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

/// `%APPDATA%\wraith\config`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("wraith").join("config")
}

/// Load `path` (a missing file means defaults, no diagnostics), following `config-file` includes.
pub fn load(path: &Path, themes_dir: &Path) -> (Config, Vec<Diagnostic>) {
    let src = std::fs::read_to_string(path).unwrap_or_default();
    parse_str(&src, path, themes_dir)
}

/// Parse `src` as if read from `file`. Bad lines keep their defaults and produce a diagnostic.
pub fn parse_str(src: &str, file: &Path, themes_dir: &Path) -> (Config, Vec<Diagnostic>) {
    let mut entries = Vec::new();
    let mut diags = Vec::new();
    let mut stack = vec![canon(file)];
    collect(src, file, &mut stack, &mut entries, &mut diags);

    let mut c = Config::default();
    // Theme first, so explicit color keys win regardless of line order.
    if let Some(e) = entries.iter().rev().find(|e| e.key == "theme")
        && !e.value.is_empty()
    {
        match theme::apply(&mut c, themes_dir, &e.value) {
            Ok(()) => c.theme = Some(e.value.clone()),
            Err(message) => diags.push(Diagnostic { file: e.file.clone(), line: e.line, message }),
        }
    }
    for e in entries.iter().filter(|e| e.key != "theme") {
        if let Err(message) = keys::apply(&mut c, &e.key, &e.value) {
            diags.push(Diagnostic { file: e.file.clone(), line: e.line, message });
        }
    }
    (c, diags)
}

struct Entry {
    file: PathBuf,
    line: usize,
    key: String,
    value: String,
}

const MAX_INCLUDE_DEPTH: usize = 8;

fn collect(src: &str, file: &Path, stack: &mut Vec<PathBuf>, out: &mut Vec<Entry>, diags: &mut Vec<Diagnostic>) {
    for line in parse::lines(src) {
        let line = match line {
            Ok(l) => l,
            Err((no, message)) => {
                diags.push(Diagnostic { file: file.into(), line: no, message });
                continue;
            }
        };
        if line.key != "config-file" {
            out.push(Entry { file: file.into(), line: line.no, key: line.key.into(), value: line.value.into() });
            continue;
        }
        let (optional, rel) = match line.value.strip_prefix('?') {
            Some(r) => (true, r),
            None => (false, line.value),
        };
        let inc = file.parent().unwrap_or(Path::new(".")).join(rel);
        let mut report = |message: String| diags.push(Diagnostic { file: file.into(), line: line.no, message });
        if stack.contains(&canon(&inc)) {
            report(format!("config-file cycle: {}", inc.display()));
        } else if stack.len() > MAX_INCLUDE_DEPTH {
            report("config-file nesting too deep".into());
        } else {
            match std::fs::read_to_string(&inc) {
                Ok(s) => {
                    stack.push(canon(&inc));
                    collect(&s, &inc, stack, out, diags);
                    stack.pop();
                }
                Err(_) if optional => {}
                Err(e) => report(format!("config-file {}: {e}", inc.display())),
            }
        }
    }
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// xterm's default 256-color palette.
fn xterm_palette() -> [Rgb; 256] {
    const BASE: [Rgb; 16] = [
        Rgb(0x00, 0x00, 0x00),
        Rgb(0xcd, 0x00, 0x00),
        Rgb(0x00, 0xcd, 0x00),
        Rgb(0xcd, 0xcd, 0x00),
        Rgb(0x00, 0x00, 0xee),
        Rgb(0xcd, 0x00, 0xcd),
        Rgb(0x00, 0xcd, 0xcd),
        Rgb(0xe5, 0xe5, 0xe5),
        Rgb(0x7f, 0x7f, 0x7f),
        Rgb(0xff, 0x00, 0x00),
        Rgb(0x00, 0xff, 0x00),
        Rgb(0xff, 0xff, 0x00),
        Rgb(0x5c, 0x5c, 0xff),
        Rgb(0xff, 0x00, 0xff),
        Rgb(0x00, 0xff, 0xff),
        Rgb(0xff, 0xff, 0xff),
    ];
    let mut p = [Rgb(0, 0, 0); 256];
    p[..16].copy_from_slice(&BASE);
    let lvl = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
    for i in 0..216 {
        p[16 + i] = Rgb(lvl(i / 36), lvl(i / 6 % 6), lvl(i % 6));
    }
    for i in 0..24 {
        let v = (8 + i * 10) as u8;
        p[232 + i] = Rgb(v, v, v);
    }
    p
}
