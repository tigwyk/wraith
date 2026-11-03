//! Shell profiles (spec §4.3) and the command line that starts one with shell integration (spec §3.5).
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ShellKind {
    Pwsh,
    WindowsPowerShell,
    Cmd,
    Bash,
    Zsh,
    Wsl { distro: String },
    #[default]
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub name: String,
    pub kind: ShellKind,
    /// The command line, without integration arguments.
    pub command: String,
    /// Where a new tab of this profile starts.
    pub dir: Option<String>,
    /// An emoji, a glyph or an image path, for the new-tab menu.
    pub icon: Option<String>,
    /// The colour scheme's name.
    pub scheme: Option<String>,
    /// Font families, first wins.
    pub font: Option<Vec<String>>,
    pub font_size: Option<f32>,
    /// Left and top padding in pixels.
    pub padding: Option<(u16, u16)>,
    pub opacity: Option<f32>,
}

/// First word of a command line (quoted or not) and the rest.
fn split_command(command: &str) -> (&str, &str) {
    let c = command.trim_start();
    if let Some(rest) = c.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => (&rest[..end], rest[end + 1..].trim_start()),
            None => (rest, ""),
        };
    }
    match c.find(char::is_whitespace) {
        Some(i) => (&c[..i], c[i..].trim_start()),
        None => (c, ""),
    }
}

fn exe_stem(exe: &str) -> String {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase();
    name.strip_suffix(".exe").map_or(name.clone(), str::to_string)
}

/// A profile from `profile = name:command`; the kind comes from the executable's name.
pub fn from_config(name: &str, command: &str) -> Profile {
    let (exe, args) = split_command(command);
    let kind = match exe_stem(exe).as_str() {
        "pwsh" => ShellKind::Pwsh,
        "powershell" => ShellKind::WindowsPowerShell,
        "cmd" => ShellKind::Cmd,
        "bash" => ShellKind::Bash,
        "zsh" => ShellKind::Zsh,
        "wsl" => {
            let mut words = args.split_whitespace();
            let mut distro = String::new();
            while let Some(w) = words.next() {
                if matches!(w, "-d" | "--distribution") {
                    distro = words.next().unwrap_or_default().to_string();
                }
            }
            ShellKind::Wsl { distro }
        }
        _ => ShellKind::Other,
    };
    Profile { name: name.to_string(), kind, command: command.to_string(), ..Profile::default() }
}

/// The profile named `wanted` (any case), else `pwsh`, else the first. `profiles` is never empty
/// because `detect()` always finds cmd.
pub fn resolve<'a>(profiles: &'a [Profile], wanted: Option<&str>) -> &'a Profile {
    let find = |n: &str| profiles.iter().find(|p| p.name.eq_ignore_ascii_case(n));
    wanted.and_then(find).or_else(|| find("pwsh")).or_else(|| profiles.first()).expect("at least one profile")
}

/// Git for Windows' bash under `program_files`, if installed.
pub fn git_bash_profile(program_files: &Path) -> Option<Profile> {
    let exe = program_files.join("Git").join("bin").join("bash.exe");
    exe.is_file().then(|| Profile { name: "git-bash".into(), kind: ShellKind::Bash, command: format!("\"{}\" -i", exe.display()), ..Profile::default() })
}

/// Distro names from `wsl.exe -l -q` output (UTF-16LE). Docker's internal distros are not shells.
pub fn decode_wsl_list(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|l| l.trim().trim_matches('\0').to_string())
        .filter(|l| !l.is_empty() && !l.to_ascii_lowercase().starts_with("docker-desktop"))
        .collect()
}

fn on_path(exe: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
}

/// pwsh, Windows PowerShell, cmd, Git Bash and every WSL distro that is installed.
pub fn detect() -> Vec<Profile> {
    use std::os::windows::process::CommandExt;
    let mut out = Vec::new();
    if on_path("pwsh.exe") {
        out.push(Profile { name: "pwsh".into(), kind: ShellKind::Pwsh, command: "pwsh.exe".into(), ..Profile::default() });
    }
    let sysroot = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let ps = sysroot.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    if ps.is_file() {
        out.push(Profile { name: "powershell".into(), kind: ShellKind::WindowsPowerShell, command: format!("\"{}\"", ps.display()), ..Profile::default() });
    }
    let cmd = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    out.push(Profile { name: "cmd".into(), kind: ShellKind::Cmd, command: if cmd.contains(' ') { format!("\"{cmd}\"") } else { cmd }, ..Profile::default() });
    if let Some(pf) = std::env::var_os("ProgramFiles")
        && let Some(p) = git_bash_profile(Path::new(&pf))
    {
        out.push(p);
    }
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Ok(o) = std::process::Command::new("wsl.exe").args(["-l", "-q"]).creation_flags(CREATE_NO_WINDOW).output()
        && o.status.success()
    {
        for distro in decode_wsl_list(&o.stdout) {
            let command = format!("wsl.exe -d {distro}");
            out.push(Profile { name: distro.clone(), kind: ShellKind::Wsl { distro }, command, ..Profile::default() });
        }
    }
    out
}

fn is_drive_path(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/')
}

/// `C:\a\b` → `/mnt/c/a/b`; a POSIX path is returned as is.
pub fn to_wsl_path(p: &str) -> String {
    if is_drive_path(p) {
        let rest = p[2..].replace('\\', "/");
        return format!("/mnt/{}{}", p[..1].to_ascii_lowercase(), rest.trim_end_matches('/'));
    }
    p.to_string()
}

/// The working directory to give CreateProcess: a Windows path for a native shell, nothing for WSL
/// (whose directory goes on the command line) or for a POSIX path a native shell cannot enter.
pub fn process_cwd(p: &Profile, cwd: Option<&str>) -> Option<String> {
    match p.kind {
        ShellKind::Wsl { .. } => None,
        _ => cwd.filter(|c| is_drive_path(c) || c.starts_with(r"\\")).map(str::to_string),
    }
}

/// `existing` WSLENV plus the variables the integration scripts and agent hooks (`wr.exe hook`) read.
pub fn wslenv_with(existing: Option<&str>) -> String {
    let mut parts: Vec<&str> = existing.unwrap_or("").split(':').filter(|s| !s.is_empty()).collect();
    for v in ["TERM_PROGRAM", "WRAITH_PANE_ID", "WRAITH_PIPE"] {
        if !parts.contains(&v) {
            parts.push(v);
        }
    }
    parts.join(":")
}

fn quote_arg(s: &str) -> String {
    if s.contains([' ', '\t', '"']) { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
}

/// The command line that starts `p`. With `integrate`, a recognised shell is started so it loads the
/// integration scripts in `scripts`; a command that already carries its own arguments is left alone.
pub fn spawn_command(p: &Profile, cwd: Option<&str>, scripts: &Path, integrate: bool) -> String {
    let (_, args) = split_command(&p.command);
    let args = format!(" {} ", args.to_ascii_lowercase());
    let has = |flags: &[&str]| flags.iter().any(|f| args.contains(&format!(" {f} ")) || args.contains(&format!(" {f}=")));
    let script = |name: &str| scripts.join(name);
    match &p.kind {
        ShellKind::Pwsh | ShellKind::WindowsPowerShell if integrate && !has(&["-command", "-c", "-file", "-f", "-encodedcommand", "-noexit"]) => {
            let path = script("integration.ps1").display().to_string().replace('\'', "''");
            format!("{} -NoExit -Command \". '{path}'\"", p.command)
        }
        ShellKind::Bash if integrate && !has(&["--rcfile", "--init-file", "--login", "-l", "-c", "--norc"]) => {
            let path = script("integration.bash").display().to_string().replace('\\', "/");
            format!("{} --rcfile \"{path}\"", p.command)
        }
        ShellKind::Wsl { .. } => {
            let mut cmd = p.command.clone();
            if let Some(dir) = cwd {
                cmd.push_str(&format!(" --cd {}", quote_arg(&to_wsl_path(dir))));
            }
            if integrate && !has(&["-e", "--exec", "--", "-u", "--user"]) {
                let rc = to_wsl_path(&script("integration.bash").display().to_string());
                let zdir = to_wsl_path(&script("zsh").display().to_string());
                // Run the user's own shell (bash or zsh) with the integration loaded; anything else, plain.
                cmd.push_str(&format!(
                    " -e sh -c \"s=${{SHELL:-/bin/sh}}; case $s in *bash) exec $s --rcfile '{rc}' -i;; *zsh) WRAITH_ORIG_ZDOTDIR=${{ZDOTDIR:-$HOME}} ZDOTDIR='{zdir}' exec $s -i;; *) exec $s -l;; esac\""
                ));
            }
            cmd
        }
        _ => p.command.clone(),
    }
}
