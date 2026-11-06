# Build a release and package it as dist\wraith-<version>-x64.zip (per-user install; spec M5).
#   scripts\dist.ps1            build, then package
#   scripts\dist.ps1 -NoBuild   package the existing target\release build
# Prints the zip's SHA256.
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if (-not $NoBuild) {
    & (Join-Path $PSScriptRoot 'check-zig.ps1') | Out-Null
    cargo build --release -p wraith-client -p wraith-cli
    if ($LASTEXITCODE) { throw "cargo build failed" }
}

$name = "wraith-$version-x64"
$stage = Join-Path $root "dist\$name"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($f in 'wraith.exe', 'wr.exe', 'OpenConsole.exe', 'conpty.dll') {
    Copy-Item (Join-Path 'target\release' $f) $stage
}
Copy-Item -Recurse 'assets\themes' (Join-Path $stage 'themes')
Copy-Item 'packaging\install.ps1', 'packaging\uninstall.ps1', 'packaging\THIRD-PARTY-NOTICES.txt' $stage
Copy-Item 'vendor\libghostty-rs\LICENSE' (Join-Path $stage 'LICENSE-libghostty-rs.txt')

$zip = Join-Path $root "dist\$name.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
$sha = (Get-FileHash $zip -Algorithm SHA256).Hash

"$zip"
"SHA256 $sha"
