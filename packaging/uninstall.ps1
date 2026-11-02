# Remove a per-user Wraith install (the folder this script is in). Settings, layout and logs under
# %APPDATA%\wraith and %LOCALAPPDATA%\wraith are kept; delete those yourself to remove everything.
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
$prefix = $PSScriptRoot
$target = [IO.Path]::GetFullPath((Join-Path $prefix 'wraith.exe'))
$running = Get-CimInstance Win32_Process -Filter "Name='wraith.exe'" | Where-Object { $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -eq $target) }
if ($running) { throw "Wraith is running from $prefix (pids $($running.ProcessId -join ', ')); close it (and run 'wr ctl server-stop') first" }
if ($DryRun) { "dry run: would remove $prefix from the user PATH, the Start Menu shortcut, the HKCU uninstall entry and $prefix"; return }

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
$kept = ($path -split ';') | Where-Object { $_ -and $_ -ne $prefix }
[Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
$lnk = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Wraith.lnk'
if ((Test-Path $lnk) -and ((New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath -eq $target)) { Remove-Item $lnk }
Remove-Item -Recurse -Force 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Wraith' -ErrorAction SilentlyContinue
# The script is inside the folder it deletes: finish from a detached shell.
Start-Process powershell.exe -WindowStyle Hidden -ArgumentList "-NoProfile -Command Start-Sleep 1; Remove-Item -Recurse -Force '$prefix'"
"Wraith removed"
