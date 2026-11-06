//! `wr update`: install the latest GitHub release over this install, as a graceful restart. The server
//! stops (saving every tab and its history), the windows close, the published installer
//! (packaging/install.ps1) runs, and the windows come back with their tabs restored around new shells.
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NEW_CONSOLE: u32 = 0x10;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$prefix = $env:WR_PREFIX
$wr = Join-Path $prefix 'wr.exe'
$old = Join-Path $prefix 'wr.old.exe'
$exe = Join-Path $prefix 'wraith.exe'
$tag = (Invoke-RestMethod 'https://api.github.com/repos/tigwyk/wraith/releases/latest').tag_name
if ($tag.TrimStart('v') -eq $env:WR_VERSION) { "Wraith $env:WR_VERSION is the latest release"; return }
"updating Wraith $env:WR_VERSION -> $tag"

function Ours { Get-CimInstance Win32_Process -Filter "Name='wraith.exe'" | Where-Object { $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath) -eq $exe } }
# The quick terminal isn't counted: the first window starts it again.
$windows = @(Ours | Where-Object { $_.CommandLine -notlike '*--server*' -and $_.CommandLine -notlike '*--quick-terminal*' }).Count
if (Ours | Where-Object CommandLine -Like '*--server*') {
    "stopping the server (tabs and history are saved)"
    & $wr ctl server-stop
    Ours | Where-Object CommandLine -Like '*--server*' | ForEach-Object { Wait-Process -Id $_.ProcessId -Timeout 20 -ErrorAction SilentlyContinue }
}
# The windows only show "[server stopped]" now; they hold nothing the server didn't save.
Ours | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; Wait-Process -Id $_.ProcessId -Timeout 10 -ErrorAction SilentlyContinue }

# Windows can't overwrite a running exe (this wr.exe, or a `wr hook` in flight) but can rename one.
Remove-Item $old -ErrorAction SilentlyContinue
Rename-Item $wr 'wr.old.exe'
try {
    & ([scriptblock]::Create((Invoke-RestMethod 'https://tigwyk.github.io/wraith/install.ps1'))) -Prefix $prefix
} finally {
    if (-not (Test-Path $wr)) { Rename-Item $old 'wr.exe' }
    # Each new window attaches the most recently detached workspace.
    # shortcut: a fixed pause between launches, not a wait for each attach; upgrade if windows swap workspaces.
    for ($i = 0; $i -lt $windows; $i++) { Start-Process $exe; Start-Sleep 2 }
}
"#;

pub fn run() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("wr.exe has no folder")?;
    // Only an installed copy (cargo builds have no installer alongside) gets overwritten.
    if !dir.join("install.ps1").exists() {
        return Err(format!("{} is not an installed Wraith; install with: irm https://tigwyk.github.io/wraith/install.ps1 | iex", dir.display()));
    }
    let mut ps = Command::new("powershell.exe");
    ps.env("WR_VERSION", env!("CARGO_PKG_VERSION")).env("WR_PREFIX", dir);
    // Inside a pane, stopping the server kills this console and everything in the pane's job: run
    // the update in its own window, outside that job, and leave it open to show how it went.
    if std::env::var_os("WRAITH_PANE_ID").is_some() {
        ps.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NoExit", "-Command", SCRIPT])
            .creation_flags(CREATE_NEW_CONSOLE | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .map_err(|e| format!("could not start the updater outside this pane ({e}); run 'wr update' from a terminal outside Wraith"))?;
        println!("updating in a new window; Wraith restarts with your tabs when it is done");
        return Ok(());
    }
    match ps.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT]).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("update failed ({s})")),
        Err(e) => Err(format!("could not run powershell.exe: {e}")),
    }
}
