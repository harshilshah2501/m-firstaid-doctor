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
Write-Host "--> Compiling release binary..." -ForegroundColor Green

# Avoid stderr-only informational output from rustup being treated as an error
# under strict PowerShell error settings.
$InstalledTargets = (& rustup target list --installed 2>$null)
if ($LASTEXITCODE -ne 0) {
    throw "rustup target list failed. Ensure rustup is installed and available in PATH."
}
if (-not ($InstalledTargets -contains $Target)) {
    & rustup target add $Target 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "rustup target add failed for target '$Target'"
    }
}
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
    $SignToolCmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    $SignTool = $null
    if ($SignToolCmd) { $SignTool = $SignToolCmd.Source }
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
$CanUseCargoWix = $false
$IsUncRepoPath = $RepoRoot -match '^\\\\'
if (-not $SkipMsi) {
    Write-Host ""
    Write-Host "--> Building MSI with cargo-wix…" -ForegroundColor Green

    # cargo-wix is optional now; if unavailable or it fails, we fall back
    # to a direct candle/light build with local staging.
    # On UNC/WSL paths, cargo-wix commonly fails at light/cab creation.
    # Prefer the direct WiX fallback immediately for cleaner output.
    if ($IsUncRepoPath) {
        Write-Warning "UNC/WSL repository path detected; skipping cargo-wix and using direct WiX build path."
    } else {
        $CargoWixCmd = Get-Command "cargo-wix" -ErrorAction SilentlyContinue
        if ($CargoWixCmd) {
            $CanUseCargoWix = $true
        } else {
            Write-Warning "cargo-wix not found. Will use direct WiX toolchain fallback."
        }
    }
}

if (-not $SkipMsi) {
    # Detect WiX Toolset bin folder (must contain candle.exe and light.exe)
    $WixBinPath = $null
    $CandleCmd = Get-Command candle.exe -ErrorAction SilentlyContinue
    if ($CandleCmd) {
        $WixBinPath = Split-Path -Parent $CandleCmd.Source
    }
    if (-not $WixBinPath -and $env:WIX) {
        $Candidate = Join-Path $env:WIX "bin"
        if (Test-Path (Join-Path $Candidate "candle.exe")) {
            $WixBinPath = $Candidate
        } elseif (Test-Path (Join-Path $env:WIX "candle.exe")) {
            $WixBinPath = $env:WIX
        }
    }
    if (-not $WixBinPath) {
        $WixCandidates = @(
            "${env:ProgramFiles(x86)}\WiX Toolset v3.11\bin",
            "${env:ProgramFiles(x86)}\WiX Toolset v3.14\bin",
            "${env:ProgramFiles}\WiX Toolset v3.11\bin",
            "${env:ProgramFiles}\WiX Toolset v3.14\bin"
        )
        $WixBinPath = $WixCandidates |
            Where-Object { Test-Path (Join-Path $_ "candle.exe") } |
            Select-Object -First 1
    }
    if (-not $WixBinPath) {
        throw "WiX Toolset compiler (candle.exe) not found. Install WiX Toolset 3.x or set WIX/bin in PATH."
    }

    Push-Location (Join-Path $RepoRoot "crates\agent-core")
    try {
        $WxsSource = Join-Path $RepoRoot "packaging\windows\main.wxs"
        $MsiOut    = Join-Path $OutDir "firstaid-$Version-$Target.msi"
        $MsiBuilt  = $false

        if (-not (Test-Path $WxsSource)) { throw "WiX source not found: $WxsSource" }
        if (-not (Test-Path $BinPath))   { throw "Binary not found: $BinPath" }

        # cargo-wix auto-discovers wix\main.wxs inside the crate directory.
        # Copy our canonical WXS there so it is found without any extra flags.
        $WixDir = Join-Path (Get-Location) "wix"
        New-Item -ItemType Directory -Force -Path $WixDir | Out-Null
        Copy-Item -Path $WxsSource -Destination (Join-Path $WixDir "main.wxs") -Force

        if ($CanUseCargoWix) {
            Write-Host "    cargo wix -p agent-core --bin-path $WixBinPath --nocapture (using wix\main.wxs)"
            & cargo wix -p agent-core --bin-path $WixBinPath --nocapture

            if ($LASTEXITCODE -ne 0) {
                Write-Warning "cargo wix failed (exit $LASTEXITCODE). Falling back to direct candle/light build."
            } else {
                # cargo-wix typically writes to target\wix\ — find and relocate the MSI.
                $SearchRoots = @(
                    (Join-Path (Get-Location) "target\wix"),
                    (Join-Path $RepoRoot "target\wix")
                )
                $ProducedMsi = $null
                foreach ($root in $SearchRoots) {
                    $found = Get-ChildItem -Path $root -Filter "*.msi" -ErrorAction SilentlyContinue |
                             Sort-Object LastWriteTime -Descending |
                             Select-Object -First 1
                    if ($found) { $ProducedMsi = $found; break }
                }

                if ($ProducedMsi) {
                    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
                    Copy-Item $ProducedMsi.FullName -Destination $MsiOut -Force
                    $MsiPath = $MsiOut
                    Write-Host "    MSI: $MsiPath"
                    Invoke-SignTool $MsiPath
                    $MsiBuilt = $true
                } else {
                    Write-Warning "cargo wix succeeded but no .msi file was found under target\wix\"
                }
            }
        }

        if (-not $MsiBuilt) {
            try {
                # Direct WiX fallback: stage files under local TEMP to avoid UNC/WSL path issues.
                $StageRoot = Join-Path $env:TEMP ("firstaid-wix-" + [guid]::NewGuid().ToString("N"))
                New-Item -ItemType Directory -Force -Path $StageRoot | Out-Null

                $StageWxs = Join-Path $StageRoot "main.wxs"
                $StageExe = Join-Path $StageRoot "firstaid.exe"
                $StageObj = Join-Path $StageRoot "main.wixobj"
                $StageMsi = Join-Path $StageRoot "firstaid.msi"

                Copy-Item -Path $BinPath -Destination $StageExe -Force

                $WxsContent = Get-Content -Raw -Path $WxsSource
                $WxsContent = $WxsContent -replace 'Source="\.\.\\\.\.\\target\\x86_64-pc-windows-msvc\\release\\\$\(var\.BinaryName\)"', 'Source="firstaid.exe"'
                Set-Content -Path $StageWxs -Value $WxsContent -Encoding UTF8

                $CandleExe = Join-Path $WixBinPath "candle.exe"
                $LightExe  = Join-Path $WixBinPath "light.exe"

                Write-Host "    candle.exe -arch x64 $StageWxs"
                & $CandleExe -nologo -arch x64 -out $StageObj $StageWxs
                if ($LASTEXITCODE -ne 0) { throw "candle failed (exit $LASTEXITCODE)" }

                Write-Host "    light.exe $StageObj"
                & $LightExe -nologo -ext WixUIExtension -out $StageMsi $StageObj
                if ($LASTEXITCODE -ne 0) { throw "light failed (exit $LASTEXITCODE)" }

                if (-not (Test-Path $StageMsi)) {
                    throw "Direct WiX build completed but MSI not found: $StageMsi"
                }

                New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
                Copy-Item -Path $StageMsi -Destination $MsiOut -Force
                $MsiPath = $MsiOut
                Write-Host "    MSI (direct WiX fallback): $MsiPath"
                Invoke-SignTool $MsiPath
            } catch {
                Write-Warning "Direct WiX fallback failed: $($_.Exception.Message)"
                Write-Warning "Continuing to EXE build."
            }
        }
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
        $ISCCCmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($ISCCCmd) { $ISCC = $ISCCCmd.Source }
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
if ($MsiPath -and (Test-Path $MsiPath)) {
    Write-Host "    To test the MSI silently:"
    Write-Host "      msiexec /i '$MsiPath' /quiet /qn"
    Write-Host ""
}
if ($MsiPath -or $ExePath) {
    Write-Host "    To verify the service was installed:"
    Write-Host "      sc query firstaid"
    Write-Host "      firstaid status"
}

} finally {
    Pop-Location
}
