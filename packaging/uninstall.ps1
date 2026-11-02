# Remove a per-user Wraith install (the folder this script is in). Settings, layout and logs under
# %APPDATA%\wraith and %LOCALAPPDATA%\wraith are kept; delete those yourself to remove everything.
# Only what install.ps1 put there is deleted: the folder itself goes only if nothing else is in it.
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
$prefix = $PSScriptRoot
$target = [IO.Path]::GetFullPath((Join-Path $prefix 'wraith.exe'))
$running = Get-CimInstance Win32_Process -Filter "Name='wraith.exe'" | Where-Object { $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -eq $target) }
if ($running) { throw "Wraith is running from $prefix (pids $($running.ProcessId -join ', ')); close it (and run 'wr ctl server-stop') first" }

# Keep in step with install.ps1's file list.
$installed = 'wraith.exe', 'wr.exe', 'OpenConsole.exe', 'conpty.dll', 'install.ps1', 'uninstall.ps1', 'THIRD-PARTY-NOTICES.txt', 'LICENSE-libghostty-rs.txt', 'themes'
$remove = @($installed | ForEach-Object { Join-Path $prefix $_ } | Where-Object { Test-Path $_ })
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Wraith'
$ours = (Get-ItemProperty $key -ErrorAction SilentlyContinue).InstallLocation -eq $prefix
if ($DryRun) {
    "dry run: would remove $prefix from the user PATH, the Start Menu shortcut if it points here$(if ($ours) { ', the HKCU uninstall entry' }), and:"
    $remove
    "then $prefix itself if nothing else is left in it"
    return
}

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($path -split ';') -contains $prefix) {
    [Environment]::SetEnvironmentVariable('Path', ((($path -split ';') | Where-Object { $_ -and $_ -ne $prefix }) -join ';'), 'User')
}
$lnk = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Wraith.lnk'
if ((Test-Path $lnk) -and ((New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath -eq $target)) { Remove-Item $lnk }
# A copy run from elsewhere (e.g. the unpacked zip) must not drop the real install's entry.
if ($ours) { Remove-Item -Recurse -Force $key }
# This script is one of the files: finish from a detached shell.
$q = { "'" + ($args[0] -replace "'", "''") + "'" }
$list = ($remove | ForEach-Object { & $q $_ }) -join ','
$dir = & $q $prefix
Start-Process powershell.exe -WindowStyle Hidden -ArgumentList "-NoProfile -Command Start-Sleep 1; Remove-Item -Recurse -Force $list; if (-not (Get-ChildItem -Force $dir)) { Remove-Item $dir }"
"Wraith removed"
