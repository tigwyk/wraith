# Measure a Wraith build and print a markdown table: startup, throughput, key-to-echo latency,
# frame cost under a redrawing TUI, and memory. Each run gets its own pipe, APPDATA and LOCALAPPDATA,
# so your own Wraith, config and layout are never touched. Windows open on screen while it runs.
#   scripts\bench.ps1                          bench target\release\wraith.exe
#   scripts\bench.ps1 -Exe path\to\wraith.exe  bench another build
# Numbers come from the debug log lines the app already writes (see docs\benchmarks.md).
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\wraith.exe'),
    [int]$MB = 32,
    [int]$Keys = 200,
    [switch]$Keep # keep the logs and data in %TEMP%\wraith-bench-<pid>
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$wr = Join-Path (Split-Path $Exe) 'wr.exe'
$work = Join-Path ([IO.Path]::GetTempPath()) "wraith-bench-$PID"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class BenchW {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static IntPtr Of(uint pid) {
    IntPtr r = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); var sb = new System.Text.StringBuilder(64); GetClassName(h, sb, 64);
      if (p == pid && sb.ToString() == "WraithWindow" && IsWindowVisible(h)) { r = h; return false; } return true; }, IntPtr.Zero);
    return r;
  }
}
"@

$log = 'info,wraith_client::app=debug,wraith_client::reload=debug,wraith_server::pane=debug'

# One isolated environment: its own pipe, config and logs. Returns the paths.
function New-Env($name) {
    $dir = Join-Path $work $name
    New-Item -ItemType Directory -Force (Join-Path $dir 'wraith\logs') | Out-Null # the log pruner errors without it
    # No quick terminal: it would start an extra process and hold Win+` during the run.
    Set-Content (Join-Path $dir 'wraith\config') 'keybind = global:super+backquote=unbind' -Encoding utf8
    @{ Dir = $dir; Pipe = "\\.\pipe\wraith-bench-$name-$PID"; Logs = Join-Path $dir 'wraith\logs' }
}

function Start-Wraith($e, [string[]]$cmd, $level = $log) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo $Exe
    foreach ($a in @('-e') + $cmd) { $psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['APPDATA'] = $e.Dir
    $psi.EnvironmentVariables['LOCALAPPDATA'] = $e.Dir
    $psi.EnvironmentVariables['WRAITH_PIPE_NAME'] = $e.Pipe
    $psi.EnvironmentVariables['WRAITH_LAYOUT_PATH'] = Join-Path $e.Dir 'layout.json'
    $psi.EnvironmentVariables['WRAITH_LOG'] = $level
    $psi.EnvironmentVariables.Remove('NO_COLOR')
    [System.Diagnostics.Process]::Start($psi)
}

# Stop the server for this environment, then anything of this build still running from it.
function Stop-Env($e, $since) {
    $env:WRAITH_PIPE_NAME = $e.Pipe
    & $wr ctl server-stop 2>&1 | Out-Null
    Remove-Item Env:WRAITH_PIPE_NAME
    Start-Sleep -Milliseconds 500
    Get-Process wraith -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe -and $_.StartTime -ge $since } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}

function Lines($e, $pattern) {
    if (-not (Test-Path $e.Logs)) { return @() }
    Get-ChildItem $e.Logs -Filter *.log | ForEach-Object { Get-Content $_.FullName } | Where-Object { $_ -match $pattern }
}
function Field($lines, $name) { $lines | ForEach-Object { if ($_ -match "$name=(\d+)") { [double]$Matches[1] } } }
function Pct($xs, $p) {
    $s = @($xs | Sort-Object)
    if (-not $s.Count) { return [double]::NaN }
    $s[[math]::Min($s.Count - 1, [math]::Floor($s.Count * $p))]
}
function Wait-Until($what, $secs, [scriptblock]$cond) {
    $deadline = (Get-Date).AddSeconds($secs)
    while (-not (& $cond)) {
        if ((Get-Date) -gt $deadline) { throw "timed out waiting for $what" }
        Start-Sleep -Milliseconds 100
    }
}
function Ms($us) { '{0:N1} ms' -f ($us / 1000) }

$rows = [System.Collections.Generic.List[object]]::new()
function Row($what, $value, $note) { $rows.Add([pscustomobject]@{ What = $what; Value = $value; Note = $note }) }

# --- Startup: process start to the first presented frame, cold (starts the server) and warm.
$e = New-Env 'startup'; $since = Get-Date
foreach ($kind in 'cold', 'warm') {
    $before = @(Lines $e 'presented').Count
    $p = Start-Wraith $e @('cmd.exe', '/k')
    $t0 = $p.StartTime.ToUniversalTime()
    Wait-Until "first frame ($kind)" 30 { @(Lines $e 'presented').Count -gt $before }
    $first = @(Lines $e 'presented')[$before]
    $t1 = [datetime]::Parse(($first.Trim() -split '\s+')[0]).ToUniversalTime()
    Row "Startup, $kind" (Ms (($t1 - $t0).TotalMilliseconds * 1000)) $(if ($kind -eq 'cold') { 'starts the server, window and shell' } else { 'a second window; the server is already running' })
}
Stop-Env $e $since

# --- Throughput: a shell writes a big file to its console; time from first byte to last.
$e = New-Env 'throughput'; $since = Get-Date
$data = Join-Path $work 'data.txt'; $result = Join-Path $work 'throughput.txt'
$line = "$([char]27)[32mINFO$([char]27)[0m 2026-11-04 12:00:00 request handled path=/api/v1/items status=200 took=12ms user=example`r`n"
$block = [Text.Encoding]::UTF8.GetBytes(($line * 1000))
$f = [IO.File]::Create($data)
while ($f.Length -lt $MB * 1MB) { $f.Write($block, 0, $block.Length) }
$f.Close()
Set-Content (Join-Path $work 'write.ps1') @'
param($data, $result)
$b = [IO.File]::ReadAllBytes($data); $o = [Console]::OpenStandardOutput()
$s = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt $b.Length; $i += 65536) { $o.Write($b, $i, [math]::Min(65536, $b.Length - $i)) }
$o.Flush(); $s.Stop()
Set-Content $result $s.Elapsed.TotalSeconds
'@
$p = Start-Wraith $e @('powershell.exe', '-NoProfile', '-File', (Join-Path $work 'write.ps1'), $data, $result) 'info'
Wait-Until 'the throughput run' 300 { Test-Path $result }
$secs = [double](Get-Content $result)
$size = (Get-Item $data).Length / 1MB
Row 'Throughput' ('{0:N0} MB/s' -f ($size / $secs)) ("{0:N0} MB of colored log lines in {1:N2} s, through ConPTY" -f $size, $secs)
Stop-Env $e $since

# --- Key-to-echo latency: post keys to the window (no global input, no focus change), cmd.exe echoes them.
$e = New-Env 'echo'; $since = Get-Date
$p = Start-Wraith $e @('cmd.exe', '/k')
$h = [IntPtr]::Zero
Wait-Until 'the window' 30 { $script:h = [BenchW]::Of([uint32]$p.Id); $script:h -ne [IntPtr]::Zero }
Start-Sleep -Seconds 2 # let the prompt draw
$rand = [Random]::new(1)
for ($i = 0; $i -lt $Keys; $i++) {
    $vk, $sc = if ($i % 2) { 0x08, 0x0E } else { 0x41, 0x1E } # 'a', then Backspace, so the line never grows
    [BenchW]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr](1 -bor ($sc -shl 16))) | Out-Null
    [BenchW]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr](1 -bor ($sc -shl 16) -bor (3 -shl 30))) | Out-Null
    Start-Sleep -Milliseconds (40 + $rand.Next(30)) # off any vsync rhythm
}
Start-Sleep -Seconds 1
$echo = Field (Lines $e 'key echoed') 'echo_us'
Row 'Key to echo, median' (Ms (Pct $echo 0.5)) "$(@($echo).Count) of $Keys keys echoed by cmd.exe; timed to the Present call, the screen shows it at its next refresh"
Row 'Key to echo, p99' (Ms (Pct $echo 0.99)) ''
$server = Get-Process wraith | Where-Object { $_.Path -eq $Exe -and $_.StartTime -ge $since -and $_.Id -ne $p.Id }
$client = Get-Process -Id $p.Id
Row 'Memory, window' ('{0:N0} MB' -f ($client.PrivateMemorySize64 / 1MB)) 'private bytes, one tab, idle'
Row 'Memory, server' ('{0:N0} MB' -f (($server | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB)) 'private bytes, one pane, idle'
Stop-Env $e $since

# --- Redrawing TUI: 300 full-screen frames, one every ~15 ms, like an app streaming tokens.
$e = New-Env 'tui'; $since = Get-Date
$done = Join-Path $work 'tui.done'
Set-Content (Join-Path $work 'tui.ps1') @'
param($done)
$e = [char]27; $o = [Console]::Out
for ($i = 0; $i -lt 300; $i++) {
    $f = -join (0..39 | ForEach-Object { "$e[3$($_ % 7 + 1)m row $_ frame $i the quick brown fox$e[0m$e[K`r`n" })
    $o.Write("$e[H$f"); $o.Flush(); Start-Sleep -Milliseconds 15
}
Start-Sleep 1; Set-Content $done ok
'@
$p = Start-Wraith $e @('powershell.exe', '-NoProfile', '-File', (Join-Path $work 'tui.ps1'), $done)
Wait-Until 'the TUI run' 120 { Test-Path $done }
$present = Field (Lines $e 'presented') 'total_us'
$apply = Field (Lines $e 'frame arrived') 'apply_us'
$held = Field (Lines $e 'frame sent') 'held_us'
Row 'Frame build, median' (Ms (Pct $present 0.5)) "$(@($present).Count) presents; CPU time to draw and submit one frame"
Row 'Frame build, p99' (Ms (Pct $present 0.99)) ''
Row 'Frame apply, median' (Ms (Pct $apply 0.5)) 'the window applying a frame from the server'
Row 'Output held, median' (Ms (Pct $held 0.5)) 'how long the pane waited for the window to take its next frame'
Stop-Env $e $since

$gpu = (Get-CimInstance Win32_VideoController | Select-Object -First 1).Name
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
$ver = (Select-String -Path (Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
"Wraith $ver · $cpu · $gpu · $(Get-Date -Format yyyy-MM-dd)"
''
'| Measure | Result | Notes |'
'| --- | --- | --- |'
$rows | ForEach-Object { "| $($_.What) | $($_.Value) | $($_.Note) |" }
if ($Keep) { "logs kept in $work" } else { Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue }
