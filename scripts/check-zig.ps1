# Fail unless the Zig on PATH is the pinned Ghostty's minimum_zig_version (it builds libghostty-vt).
#   check-zig.ps1         check the Zig on PATH
#   check-zig.ps1 -Want   print the wanted version only (CI installs it)
param([switch]$Want)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$commit = (Select-String -Path (Join-Path $root 'vendor/libghostty-rs/crates/libghostty-vt-sys/build.rs') -Pattern 'GHOSTTY_COMMIT: &str = "([0-9a-f]+)"').Matches[0].Groups[1].Value
$zon = (Invoke-WebRequest -UseBasicParsing "https://raw.githubusercontent.com/ghostty-org/ghostty/$commit/build.zig.zon").Content
$wanted = [regex]::Match($zon, 'minimum_zig_version = "([^"]+)"').Groups[1].Value
if (-not $wanted) { throw "no minimum_zig_version in Ghostty $commit build.zig.zon" }
if ($Want) { return $wanted }
$have = (zig version).Trim()
"Ghostty $commit wants Zig $wanted; installed $have"
if ($have -ne $wanted) { throw "Zig $have does not match Ghostty's minimum_zig_version $wanted" }
