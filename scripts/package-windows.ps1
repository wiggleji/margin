# Build Margin for Windows (x64) and package it:
#   dist\Margin-<version>-windows-x64-setup.exe   installer (Inno Setup 6)
#   dist\Margin-<version>-windows-x64.zip         portable: unzip and run margin.exe
# each with a .sha256.
#
#   scripts\package-windows.ps1             build, then package
#   scripts\package-windows.ps1 -NoBuild    package an existing target\release build
#   scripts\package-windows.ps1 -NoInstaller  only the zip (no Inno Setup needed)
param([switch]$NoBuild, [switch]$NoInstaller)
$ErrorActionPreference = "Stop"

$root = (Resolve-Path "$PSScriptRoot\..").Path
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$dist = Join-Path $root "dist"
$bin = Join-Path $root "target\release"

if (-not $NoBuild) {
    Write-Host "==> Building Margin $version (release)"
    Push-Location $root
    cargo build --release --locked -p margin-app -p margin-mcp
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    Pop-Location
}
foreach ($b in "margin.exe", "margin-mcp.exe") {
    if (-not (Test-Path (Join-Path $bin $b))) { throw "$bin\$b not found (build first)" }
}
New-Item -ItemType Directory -Force $dist | Out-Null

function Write-Sha256($file) {
    $h = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower()
    "$h  $(Split-Path -Leaf $file)" | Out-File -Encoding ascii -NoNewline "$file.sha256"
}

# ------------------------------------------------------------------- zip
$name = "Margin-$version-windows-x64"
Write-Host "==> Building $name.zip"
$stage = Join-Path $dist $name
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item (Join-Path $bin "margin.exe"), (Join-Path $bin "margin-mcp.exe") $stage
Copy-Item (Join-Path $root "LICENSE") (Join-Path $stage "LICENSE.txt")
Copy-Item (Join-Path $root "THIRD_PARTY_NOTICES.md"), (Join-Path $root "README.md") $stage
$zip = Join-Path $dist "$name.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item -Recurse -Force $stage
Write-Sha256 $zip

# ------------------------------------------------------------- installer
if (-not $NoInstaller) {
    Write-Host "==> Building $name-setup.exe"
    $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { $iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source }
    if (-not $iscc) { throw "Inno Setup 6 not found (choco install innosetup), or pass -NoInstaller" }
    & $iscc /Q "/DAppVersion=$version" (Join-Path $root "packaging\windows\margin.iss")
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
    Write-Sha256 (Join-Path $dist "$name-setup.exe")
}

Write-Host "==> Done"
Get-ChildItem $dist -Filter "Margin-$version-windows-*" | ForEach-Object { Write-Host "  $($_.Name)" }
