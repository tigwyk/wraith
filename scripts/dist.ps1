# Build a release and package it as dist\wraith-<version>-x64.zip (per-user install; spec M5).
#   scripts\dist.ps1            build, then package
#   scripts\dist.ps1 -NoBuild   package the existing target\release build
# Prints the zip's SHA256 and writes it into the local winget installer manifest.
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if (-not $NoBuild) {
    # libghostty-vt's pinned Ghostty needs Zig 0.17 (vendor/libghostty-rs/VENDORED.md).
    $zig = (& zig version) 2>$null
    if ($zig -notlike '0.17*') { throw "Zig 0.17.x must be first on PATH (found '$zig')" }
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

$manifest = Get-ChildItem "packaging\winget" -Recurse -Filter '*.installer.yaml' | Where-Object { $_.FullName -match [regex]::Escape($version) } | Select-Object -First 1
if ($manifest) {
    (Get-Content $manifest.FullName) -replace '^(\s*InstallerSha256:).*', "`$1 $sha" | Set-Content $manifest.FullName
}
"$zip"
"SHA256 $sha"
