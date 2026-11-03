#![cfg(windows)]
use std::path::Path;

use wraith_server::profile::{Profile, ShellKind, decode_wsl_list, detect, from_config, git_bash_profile, process_cwd, resolve, spawn_command};
use wraith_server::shell::{ensure_scripts_in, scripts};

fn p(name: &str, kind: ShellKind, command: &str) -> Profile {
    Profile { name: name.into(), kind, command: command.into(), ..Profile::default() }
}

const SCRIPTS: &str = r"C:\Users\x\wraith\v1";

#[test]
fn pwsh_gets_noexit_command_dot_source() {
    let cmd = spawn_command(&p("pwsh", ShellKind::Pwsh, "pwsh.exe"), None, Path::new(SCRIPTS), true);
    assert_eq!(cmd, r#"pwsh.exe -NoExit -Command ". 'C:\Users\x\wraith\v1\integration.ps1'""#);
    let ps = spawn_command(&p("powershell", ShellKind::WindowsPowerShell, "powershell.exe"), None, Path::new(SCRIPTS), true);
    assert!(ps.starts_with("powershell.exe -NoExit -Command ") && ps.contains("integration.ps1"));
}

#[test]
fn apostrophes_in_the_scripts_path_are_doubled() {
    let cmd = spawn_command(&p("pwsh", ShellKind::Pwsh, "pwsh.exe"), None, Path::new(r"C:\Users\O'Brien\w"), true);
    assert!(cmd.contains(r"C:\Users\O''Brien\w\integration.ps1"), "{cmd}");
}

#[test]
fn bash_gets_rcfile() {
    let cmd = spawn_command(&p("bash", ShellKind::Bash, "bash.exe -i"), None, Path::new(SCRIPTS), true);
    assert_eq!(cmd, r#"bash.exe -i --rcfile "C:/Users/x/wraith/v1/integration.bash""#);
}

#[test]
fn git_bash_path_is_quoted() {
    let dir = std::env::temp_dir().join(format!("wraith-gitbash-{}", std::process::id()));
    let bin = dir.join("Git").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("bash.exe"), b"").unwrap();
    let prof = git_bash_profile(&dir).unwrap();
    assert_eq!(prof.name, "git-bash");
    assert_eq!(prof.kind, ShellKind::Bash);
    assert_eq!(prof.command, format!("\"{}\" -i", bin.join("bash.exe").display()));
    assert!(git_bash_profile(&dir.join("nowhere")).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wsl_gets_distro_cd_and_translated_rcfile() {
    let wsl = p("Ubuntu", ShellKind::Wsl { distro: "Ubuntu".into() }, "wsl.exe -d Ubuntu");
    let cmd = spawn_command(&wsl, Some("/home/lee/app"), Path::new(SCRIPTS), true);
    assert!(cmd.starts_with(r#"wsl.exe -d Ubuntu --cd /home/lee/app -e sh -c ""#), "{cmd}");
    assert!(cmd.contains("--rcfile '/mnt/c/Users/x/wraith/v1/integration.bash'"), "{cmd}");
    assert!(cmd.contains("ZDOTDIR='/mnt/c/Users/x/wraith/v1/zsh'"), "{cmd}");
    assert!(cmd.contains("${SHELL:-/bin/sh}"), "keeps the user's own shell: {cmd}");
    let win = spawn_command(&wsl, Some(r"C:\src\api"), Path::new(SCRIPTS), true);
    assert!(win.contains("--cd /mnt/c/src/api "), "{win}");
    let spaced = spawn_command(&wsl, Some("/home/lee/my app"), Path::new(SCRIPTS), true);
    assert!(spaced.contains(r#"--cd "/home/lee/my app" "#), "{spaced}");
    let none = spawn_command(&wsl, None, Path::new(SCRIPTS), true);
    assert!(!none.contains("--cd"), "{none}");
}

#[test]
fn cmd_and_other_are_untouched() {
    for (k, c) in [(ShellKind::Cmd, "cmd.exe"), (ShellKind::Other, "nu.exe --lsp"), (ShellKind::Zsh, "zsh.exe")] {
        assert_eq!(spawn_command(&p("x", k, c), Some("C:/x"), Path::new(SCRIPTS), true), c);
    }
}

#[test]
fn integration_off_or_commands_with_own_args_are_untouched() {
    let pw = p("pwsh", ShellKind::Pwsh, "pwsh.exe");
    assert_eq!(spawn_command(&pw, None, Path::new(SCRIPTS), false), "pwsh.exe");
    for (k, c) in [
        (ShellKind::Pwsh, "pwsh.exe -Command Get-Date"),
        (ShellKind::Pwsh, "pwsh.exe -File x.ps1"),
        (ShellKind::Bash, r#""C:\Git\bin\bash.exe" --login -i"#),
        (ShellKind::Bash, "bash.exe --rcfile x"),
    ] {
        assert_eq!(spawn_command(&p("x", k, c), None, Path::new(SCRIPTS), true), c);
    }
}

#[test]
fn from_config_infers_kind_by_exe_name() {
    let kind = |c: &str| from_config("n", c).kind;
    assert_eq!(kind(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#), ShellKind::Pwsh);
    assert_eq!(kind(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"), ShellKind::WindowsPowerShell);
    assert_eq!(kind(r"C:\Windows\System32\cmd.exe /k"), ShellKind::Cmd);
    assert_eq!(kind(r#""C:\Program Files\Git\bin\bash.exe" --login"#), ShellKind::Bash);
    assert_eq!(kind("zsh"), ShellKind::Zsh);
    assert_eq!(kind("wsl.exe -d Debian"), ShellKind::Wsl { distro: "Debian".into() });
    assert_eq!(kind("wsl.exe --distribution Arch"), ShellKind::Wsl { distro: "Arch".into() });
    assert_eq!(kind("nu.exe"), ShellKind::Other);
    assert_eq!(from_config("n", "pwsh.exe").name, "n");
}

#[test]
fn resolve_prefers_named_then_pwsh_then_first() {
    let list = vec![p("cmd", ShellKind::Cmd, "cmd.exe"), p("pwsh", ShellKind::Pwsh, "pwsh.exe"), p("Dev", ShellKind::Other, "dev.exe")];
    assert_eq!(resolve(&list, Some("dev")).name, "Dev");
    assert_eq!(resolve(&list, Some("nope")).name, "pwsh");
    assert_eq!(resolve(&list, None).name, "pwsh");
    assert_eq!(resolve(&list[..1], None).name, "cmd");
}

#[test]
fn decode_wsl_list_handles_utf16_and_crlf() {
    let utf16: Vec<u8> = "Ubuntu\r\ndocker-desktop\r\nDebian\r\n\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(decode_wsl_list(&utf16), vec!["Ubuntu", "Debian"]);
    let with_bom: Vec<u8> = std::iter::once(0xfeffu16).chain("Arch\r\n".encode_utf16()).flat_map(u16::to_le_bytes).collect();
    assert_eq!(decode_wsl_list(&with_bom), vec!["Arch"]);
    assert!(decode_wsl_list(&[]).is_empty());
    assert!(decode_wsl_list(&[1]).is_empty(), "odd byte counts must not panic");
}

#[test]
fn detect_always_has_cmd() {
    assert!(detect().iter().any(|p| p.kind == ShellKind::Cmd));
}

#[test]
fn process_cwd_only_passes_windows_paths_to_native_shells() {
    let native = p("pwsh", ShellKind::Pwsh, "pwsh.exe");
    assert_eq!(process_cwd(&native, Some(r"C:\src")).as_deref(), Some(r"C:\src"));
    assert_eq!(process_cwd(&native, Some("/home/x")), None);
    assert_eq!(process_cwd(&native, None), None);
    let wsl = p("U", ShellKind::Wsl { distro: "U".into() }, "wsl.exe");
    assert_eq!(process_cwd(&wsl, Some("/home/x")), None);
    assert_eq!(process_cwd(&wsl, Some(r"C:\src")), None);
}

#[test]
fn ensure_scripts_is_idempotent_and_versioned() {
    let base = std::env::temp_dir().join(format!("wraith-scripts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = ensure_scripts_in(&base).unwrap();
    assert!(dir.ends_with(format!("v{}", env!("CARGO_PKG_VERSION"))), "{dir:?}");
    for (rel, body) in scripts() {
        assert_eq!(std::fs::read_to_string(dir.join(rel)).unwrap(), *body, "{rel}");
    }
    assert_eq!(ensure_scripts_in(&base).unwrap(), dir);
    // A damaged script is rewritten.
    std::fs::write(dir.join("integration.ps1"), "tampered").unwrap();
    ensure_scripts_in(&base).unwrap();
    assert_ne!(std::fs::read_to_string(dir.join("integration.ps1")).unwrap(), "tampered");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn scripts_emit_osc7_and_osc133() {
    for (rel, body) in scripts() {
        assert!(body.contains("]7;"), "{rel} lacks OSC 7");
        assert!(body.contains("]133;A"), "{rel} lacks OSC 133;A");
        assert!(body.contains("]133;D"), "{rel} lacks OSC 133;D");
        assert!(body.contains("]133;B"), "{rel} lacks OSC 133;B");
    }
}

#[test]
fn ps1_script_parses() {
    let base = std::env::temp_dir().join(format!("wraith-parse-{}", std::process::id()));
    let dir = ensure_scripts_in(&base).unwrap();
    let path = dir.join("integration.ps1");
    let check = format!(
        "$e = $null; [System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$null, [ref]$e) | Out-Null; if ($e) {{ $e | Out-String | Write-Host; exit 1 }}",
        path.display()
    );
    let out = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &check]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn wslenv_forwards_integration_variables_once() {
    use wraith_server::profile::wslenv_with;
    // WRAITH_PIPE too: an agent hook inside WSL runs wr.exe, which needs it to reach the server.
    assert_eq!(wslenv_with(None), "TERM_PROGRAM:WRAITH_PANE_ID:WRAITH_PIPE");
    assert_eq!(wslenv_with(Some("A/p:TERM_PROGRAM")), "A/p:TERM_PROGRAM:WRAITH_PANE_ID:WRAITH_PIPE");
    assert_eq!(wslenv_with(Some("")), "TERM_PROGRAM:WRAITH_PANE_ID:WRAITH_PIPE");
}
