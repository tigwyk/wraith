# Install Wraith for the current user (no admin): run from the unpacked release folder.
#   install.ps1 [-Prefix <dir>] [-DryRun]
# Copies the files to <Prefix> (default %LOCALAPPDATA%\Programs\Wraith), puts wr.exe on the user PATH,
# creates the Start Menu shortcut (with the app id toasts need) and an "Apps & features" entry.
# -DryRun copies the files but changes nothing else (PATH, Start Menu, registry) and says what it would do.
param([string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\Wraith'), [switch]$DryRun)
$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
$files = 'wraith.exe', 'wr.exe', 'OpenConsole.exe', 'conpty.dll', 'install.ps1', 'uninstall.ps1', 'THIRD-PARTY-NOTICES.txt', 'LICENSE-libghostty-rs.txt'
foreach ($f in $files) { if (-not (Test-Path (Join-Path $src $f))) { throw "missing $f next to install.ps1" } }

# Replacing files under a running Wraith would fail half-way; refuse instead.
$target = [IO.Path]::GetFullPath((Join-Path $Prefix 'wraith.exe'))
$running = Get-CimInstance Win32_Process -Filter "Name='wraith.exe'" | Where-Object { $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -eq $target) }
if ($running) { throw "Wraith is running from $Prefix (pids $($running.ProcessId -join ', ')); close it (and run 'wr ctl server-stop') first" }

New-Item -ItemType Directory -Force $Prefix | Out-Null
foreach ($f in $files) { Copy-Item (Join-Path $src $f) $Prefix -Force }
Remove-Item -Recurse -Force (Join-Path $Prefix 'themes') -ErrorAction SilentlyContinue
Copy-Item -Recurse (Join-Path $src 'themes') (Join-Path $Prefix 'themes')
"installed to $Prefix"

$version = (Get-Item (Join-Path $Prefix 'wraith.exe')).VersionInfo.ProductVersion
if (-not $version) { $version = '0.2.0' }
if ($DryRun) {
    "dry run: would add $Prefix to the user PATH, run 'wraith.exe --register' (Start Menu shortcut) and write HKCU uninstall entry 'Wraith'"
    return
}

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($path -split ';') -contains $Prefix)) {
    [Environment]::SetEnvironmentVariable('Path', (($path.TrimEnd(';'), $Prefix) -join ';').TrimStart(';'), 'User')
    "added $Prefix to the user PATH (new terminals see it)"
}
& (Join-Path $Prefix 'wraith.exe') --register | Out-Null
if ($LASTEXITCODE) { Write-Warning "could not create the Start Menu shortcut; Wraith creates it on first start" }

$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Wraith'
New-Item -Force $key | Out-Null
$props = @{
    DisplayName = 'Wraith'
    DisplayVersion = $version
    Publisher = 'Wraith'
    InstallLocation = $Prefix
    DisplayIcon = (Join-Path $Prefix 'wraith.exe')
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $Prefix 'uninstall.ps1')`""
    NoModify = 1
    NoRepair = 1
}
foreach ($k in $props.Keys) { New-ItemProperty -Force -Path $key -Name $k -Value $props[$k] | Out-Null }
"done: start Wraith from the Start Menu, or run 'wraith' / 'wr' in a new terminal"
