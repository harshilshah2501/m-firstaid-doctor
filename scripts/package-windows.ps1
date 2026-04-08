#Requires -Version 5.1
<#
.SYNOPSIS
    Build the FirstAid Doctor Windows installer (.msi and/or .exe).

.DESCRIPTION
    Compiles the release binary, then produces:
      - An .msi via cargo-wix + WiX Toolset 3.x  (professional, MSI format)
      - A .exe via Inno Setup 6                   (simpler, self-extracting)
    Both outputs land in dist\windows\.

.PARAMETER Version
    Override the version string (default: read from Cargo.toml).

.PARAMETER Target
    Rust target triple. Default: x86_64-pc-windows-msvc.

.PARAMETER SkipMsi
    Skip the WiX MSI build (e.g. if WiX Toolset is not installed).

.PARAMETER SkipExe
    Skip the Inno Setup .exe build.

.PARAMETER Sign
    Codesign outputs. Requires CERT_PATH and CERT_PASSWORD env vars.

.EXAMPLE
    # Full build (both MSI and EXE):
    .\scripts\package-windows.ps1

    # MSI only, signed:
    $env:CERT_PATH     = "C:\certs\my.pfx"
    $env:CERT_PASSWORD = "hunter2"
    .\scripts\package-windows.ps1 -SkipExe -Sign

.NOTES
    Prerequisites:
      - Rust toolchain with target x86_64-pc-windows-msvc
      - cargo-wix: cargo install cargo-wix
      - WiX Toolset 3.x: https://wixtoolset.org/releases/ (for MSI)
      - Inno Setup 6:     https://jrsoftware.org/isdl.php  (for EXE)
      - Windows SDK (signtool.exe, for signing)
#>
[CmdletBinding()]
param(
    [string]$Version   = "",
    [string]$Target    = "x86_64-pc-windows-msvc",
    [switch]$SkipMsi,
    [switch]$SkipExe,
    [switch]$Sign
)

$ErrorActionPreference = "Stop"

# ── Locate repo root ──────────────────────────────────────────────────────────

$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $RepoRoot

try {

# ── Resolve version ───────────────────────────────────────────────────────────

if (-not $Version) {
    $Version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"(.+)"' |
                Select-Object -First 1).Matches.Groups[1].Value
}
if (-not $Version) { throw "Could not determine version from Cargo.toml" }
Write-Host "==> Building FirstAid Doctor $Version for Windows ($Target)" -ForegroundColor Cyan

# ── Output directory ──────────────────────────────────────────────────────────

$OutDir = Join-Path $RepoRoot "dist\windows"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# ── Step 1: Build release binary ─────────────────────────────────────────────

Write-Host ""
Write-Host "--> Compiling release binary…" -ForegroundColor Green
rustup target add $Target 2>$null
cargo build --release --target $Target -p agent-core

$BinPath = Join-Path $RepoRoot "target\$Target\release\firstaid.exe"
if (-not (Test-Path $BinPath)) { throw "Binary not found: $BinPath" }

$SizeMB = [math]::Round((Get-Item $BinPath).Length / 1MB, 1)
Write-Host "    Binary: $BinPath ($SizeMB MB)"
if ($SizeMB -gt 30) { Write-Warning "Binary exceeds 30 MB footprint target" }

# ── Step 2: Sign binary (optional) ───────────────────────────────────────────

function Invoke-SignTool([string]$FilePath) {
    if (-not $Sign) { return }
    $CertPath = $env:CERT_PATH
    $CertPass = $env:CERT_PASSWORD
    if (-not $CertPath) { throw "CERT_PATH env var required for signing" }
    $SignTool = (Get-Command signtool.exe -ErrorAction SilentlyContinue)?.Source
    if (-not $SignTool) {
        # Try common SDK paths
        $SDKPaths = @(
            "${env:ProgramFiles(x86)}\Windows Kits\10\bin\x64\signtool.exe",
            "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe"
        )
        $SignTool = $SDKPaths | Where-Object { Test-Path $_ } | Select-Object -First 1
    }
    if (-not $SignTool) { throw "signtool.exe not found. Install Windows SDK." }

    $SignArgs = @(
        "sign", "/fd", "SHA256",
        "/tr", "http://timestamp.digicert.com", "/td", "SHA256",
        "/f", $CertPath
    )
    if ($CertPass) { $SignArgs += @("/p", $CertPass) }
    $SignArgs += $FilePath

    Write-Host "    Signing $FilePath…"
    & $SignTool @SignArgs
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $FilePath" }
}

Invoke-SignTool $BinPath

# ── Step 3: Build MSI via cargo-wix ─────────────────────────────────────────

$MsiPath = $null
if (-not $SkipMsi) {
    Write-Host ""
    Write-Host "--> Building MSI with cargo-wix…" -ForegroundColor Green

    # Verify prerequisites
    if (-not (Get-Command "cargo-wix" -ErrorAction SilentlyContinue)) {
        Write-Warning "cargo-wix not found. Run: cargo install cargo-wix"
        Write-Warning "Skipping MSI build."
        $SkipMsi = $true
    }
}

if (-not $SkipMsi) {
    # cargo-wix expects to run from the crate directory
    Push-Location (Join-Path $RepoRoot "crates\agent-core")
    try {
        # Point cargo-wix at our pre-authored WiX source
        $WxsSource = Join-Path $RepoRoot "packaging\windows\main.wxs"
        $MsiOut     = Join-Path $OutDir "firstaid-$Version-$Target.msi"

        cargo wix `
            --no-build `
            --nocapture `
            --target $Target `
            --output $MsiOut `
            $WxsSource

        if ($LASTEXITCODE -ne 0) { throw "cargo wix failed" }
        $MsiPath = $MsiOut
        Write-Host "    MSI: $MsiPath"
        Invoke-SignTool $MsiPath
    } finally {
        Pop-Location
    }
}

# ── Step 4: Build setup.exe via Inno Setup ───────────────────────────────────

$ExePath = $null
if (-not $SkipExe) {
    Write-Host ""
    Write-Host "--> Building setup.exe with Inno Setup…" -ForegroundColor Green

    # Locate ISCC.exe
    $ISSCPaths = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    )
    $ISCC = $ISSCPaths | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $ISCC) {
        $ISCC = (Get-Command ISCC.exe -ErrorAction SilentlyContinue)?.Source
    }

    if (-not $ISCC) {
        Write-Warning "Inno Setup (ISCC.exe) not found."
        Write-Warning "Download from https://jrsoftware.org/isdl.php"
        Write-Warning "Skipping .exe build."
        $SkipExe = $true
    }
}

if (-not $SkipExe) {
    $IssFile = Join-Path $RepoRoot "packaging\windows\firstaid.iss"

    & $ISCC `
        "/DAppVersion=$Version" `
        "/DBinDir=$(Join-Path $RepoRoot "target\$Target\release")" `
        $IssFile

    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

    $ExeOut = Join-Path $OutDir "firstaid-$Version-setup.exe"
    if (Test-Path $ExeOut) {
        $ExePath = $ExeOut
        Write-Host "    EXE: $ExePath"
        Invoke-SignTool $ExePath
    }
}

# ── Summary ───────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "==> Done." -ForegroundColor Cyan
Write-Host ""
if ($MsiPath -and (Test-Path $MsiPath)) {
    Write-Host "    MSI installer : $MsiPath"
}
if ($ExePath -and (Test-Path $ExePath)) {
    Write-Host "    EXE installer : $ExePath"
}
if ((-not $MsiPath) -and (-not $ExePath)) {
    Write-Warning "No packages were produced. Check prerequisite tools."
}
Write-Host ""
Write-Host "    To test the MSI silently:"
Write-Host "      msiexec /i '$MsiPath' /quiet /qn"
Write-Host ""
Write-Host "    To verify the service was installed:"
Write-Host "      sc query firstaid"
Write-Host "      firstaid status"

} finally {
    Pop-Location
}
