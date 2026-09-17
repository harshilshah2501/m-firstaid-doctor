#Requires -Version 5.1
<#
.SYNOPSIS
    End-to-end Windows bootstrap for m-firstaid-doctor on a clean machine.

.DESCRIPTION
    This script can:
      1) Install prerequisites with winget (Git, Rust, Python, VS Build Tools)
      2) Prepare Python virtual environment and training dependencies
      3) Train/export ONNX models locally
      4) Verify SHA-256 hashes against models/manifest.json
      5) Build firstaid.exe for x86_64-pc-windows-msvc
      6) Run fixture-based diagnose validation
      7) Optionally package Windows installers

.NOTES
    - Run from an elevated PowerShell when using -InstallPrereqs.
    - If VS Build Tools are installed in this run, open a new terminal and rerun
      with -SkipInstallPrereqs if compiler tools are not yet on PATH.
#>

[CmdletBinding()]
param(
    [string]$RepoRoot = "",
    [switch]$InstallPrereqs,
    [switch]$SkipInstallPrereqs,
    [string]$PythonExe = "py -3.11",
    [string]$Target = "x86_64-pc-windows-msvc",
    [switch]$DoPackaging,
    [switch]$SkipModelTraining,
    [switch]$SkipHashVerification,
    [switch]$SkipFixtureRun
)

$ErrorActionPreference = "Stop"

function Step([string]$Message) {
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Ensure-Command([string]$Name, [string]$InstallHint) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Missing command '$Name'. $InstallHint"
    }
}

function Exec([string]$Cmd, [string]$ErrMsg) {
    Write-Host "   $Cmd"
    Invoke-Expression $Cmd
    if ($LASTEXITCODE -ne 0) {
        throw "$ErrMsg (exit code $LASTEXITCODE)"
    }
}

function Try-Exec([string]$Cmd) {
    Write-Host "   $Cmd"
    Invoke-Expression $Cmd | Out-Host
    return ($LASTEXITCODE -eq 0)
}

function Ensure-WingetPackage([string]$Id, [string]$Name, [string]$Override = "") {
    $installed = winget list --id $Id -e | Out-String
    if ($installed -match [regex]::Escape($Id)) {
        Write-Host "   $Name already installed"
        return
    }

    $cmd = "winget install --id $Id -e --accept-package-agreements --accept-source-agreements"
    if ($Override) {
        $cmd += " --override `"$Override`""
    }
    Exec $cmd "Failed installing $Name"
}

function Get-DefaultRepoRoot {
    if ($PSScriptRoot) {
        $scriptRoot = $PSScriptRoot
    } elseif ($PSCommandPath) {
        $scriptRoot = Split-Path -Parent $PSCommandPath
    } else {
        throw "Unable to determine the script directory. Pass -RepoRoot explicitly."
    }
    return Split-Path -Parent $scriptRoot
}

function Resolve-PythonCommand([string]$RequestedPythonExe) {
    $candidates = @()
    if ($RequestedPythonExe) {
        $candidates += $RequestedPythonExe
    }

    foreach ($candidate in @("py -3.11", "python", "python3", "py")) {
        if ($candidates -notcontains $candidate) {
            $candidates += $candidate
        }
    }

    foreach ($candidate in $candidates) {
        if (Try-Exec "$candidate --version") {
            return $candidate
        }
    }

    throw "Python 3.11+ is required. Install it with 'py install 3.11' or rerun with -InstallPrereqs, or pass -PythonExe to a working interpreter."
}

if (-not $RepoRoot) {
    $RepoRoot = Get-DefaultRepoRoot
}

if (-not (Test-Path (Join-Path $RepoRoot "Cargo.toml"))) {
    throw "Repo root does not look valid: $RepoRoot"
}

Push-Location $RepoRoot
try {
    Step "Workspace: $RepoRoot"

    if ($InstallPrereqs -and $SkipInstallPrereqs) {
        throw "Use either -InstallPrereqs or -SkipInstallPrereqs, not both."
    }

    if (-not $SkipInstallPrereqs) {
        if (-not $InstallPrereqs) {
            Write-Warning "Prerequisite installation not explicitly requested."
            Write-Warning "Proceeding with checks only. Use -InstallPrereqs to auto-install via winget."
        } else {
            Step "Installing prerequisites via winget"
            Ensure-Command "winget" "Install App Installer from Microsoft Store."

            Ensure-WingetPackage "Git.Git" "Git"
            Ensure-WingetPackage "Rustlang.Rustup" "Rustup"
            Ensure-WingetPackage "Python.Python.3.11" "Python 3.11"
            Ensure-WingetPackage "Microsoft.VisualStudio.2022.BuildTools" "VS 2022 Build Tools" "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
        }
    }

    Step "Validating command-line prerequisites"
    Ensure-Command "cargo" "Install Rust via rustup."
    Ensure-Command "rustup" "Install Rust via rustup."
    Ensure-Command "git" "Install Git."

    Step "Configuring Rust target"
    Exec "rustup target add $Target" "Failed to add Rust target $Target"

    $NeedsPython = (-not $SkipModelTraining) -or (-not $SkipHashVerification)
    $VenvPython = $null
    if ($NeedsPython) {
        Step "Preparing Python environment"
        $PythonExe = Resolve-PythonCommand $PythonExe
        Write-Host "   Using Python command: $PythonExe"

        $venvDir = Join-Path $RepoRoot ".venv"
        if (-not (Test-Path $venvDir)) {
            Exec "$PythonExe -m venv .venv" "Failed to create virtual environment"
        } else {
            Write-Host "   Reusing existing .venv"
        }

        $VenvPython = Join-Path $venvDir "Scripts\python.exe"
        if (-not (Test-Path $VenvPython)) {
            throw "Virtual environment python not found at $VenvPython"
        }

        Exec "& `"$VenvPython`" -m pip install --upgrade pip" "Failed to upgrade pip"
        Exec "& `"$VenvPython`" -m pip install torch onnx numpy scikit-learn skl2onnx" "Failed to install Python training dependencies"
    } else {
        Write-Warning "Skipping Python environment setup because both model training and hash verification are disabled."
    }

    if (-not $SkipModelTraining) {
        Step "Training and exporting ONNX models"
        Exec "& `"$VenvPython`" scripts/train_crash_classifier.py --output models/crash_classifier_v1.onnx" "Crash classifier training failed"
        Exec "& `"$VenvPython`" scripts/train_process_anomaly.py --output models/process_anomaly_v1.onnx" "Process anomaly training failed"
        Exec "& `"$VenvPython`" scripts/train_temporal.py --output models/temporal_anomaly_v1.onnx" "Temporal model training failed"
        Exec "& `"$VenvPython`" scripts/train_causal_ranker.py" "Causal ranker training failed"
        Exec "& `"$VenvPython`" scripts/train_remediation_selector.py" "Remediation selector training failed"
    } else {
        Write-Warning "Skipping model training due to -SkipModelTraining"
    }

    if (-not $SkipHashVerification) {
        Step "Verifying model hashes against manifest"
        $verifyScript = @"
import hashlib
import json
import pathlib
import sys

manifest = pathlib.Path('models/manifest.json')
if not manifest.exists():
    print('ERROR: models/manifest.json not found')
    sys.exit(1)

data = json.loads(manifest.read_text(encoding='utf-8'))
ok = True
for entry in data.get('models', []):
    name = entry.get('name', '<unknown>')
    path = pathlib.Path(entry.get('path', ''))
    expected = str(entry.get('sha256', '')).lower()

    if not path.exists():
        print(f'MISSING {name} {path}')
        ok = False
        continue

    actual = hashlib.sha256(path.read_bytes()).hexdigest().lower()
    if actual != expected:
        print(f'FAIL   {name} expected={expected} actual={actual}')
        ok = False
    else:
        print(f'OK     {name} {actual}')

sys.exit(0 if ok else 2)
"@

        $tmpVerify = Join-Path $env:TEMP "firstaid_verify_hashes.py"
        Set-Content -Path $tmpVerify -Value $verifyScript -Encoding UTF8
        Exec "& `"$VenvPython`" `"$tmpVerify`"" "Model hash verification failed"
    } else {
        Write-Warning "Skipping hash verification due to -SkipHashVerification"
    }

    Step "Building firstaid for Windows"
    Exec "cargo build --release --target $Target -p agent-core" "Rust build failed"

    $binary = Join-Path $RepoRoot "target\$Target\release\firstaid.exe"
    if (-not (Test-Path $binary)) {
        throw "Build succeeded but binary missing at $binary"
    }
    $sizeMB = [math]::Round((Get-Item $binary).Length / 1MB, 2)
    Write-Host "   Built binary: $binary ($sizeMB MB)"

    if (-not $SkipFixtureRun) {
        Step "Running fixture validation"
        $fixture = "fixtures\scenarios\memory_critical_pressure.json"
        Exec "& `"$binary`" --fixture $fixture diagnose --json" "Fixture diagnose failed"
    } else {
        Write-Warning "Skipping fixture run due to -SkipFixtureRun"
    }

    if ($DoPackaging) {
        Step "Packaging installers"
        Exec ".\scripts\package-windows.ps1" "Windows packaging failed"
    }

    Step "Completed"
    Write-Host "All requested steps completed successfully." -ForegroundColor Green
    Write-Host ""
    Write-Host "Next useful commands:"
    Write-Host "  $binary diagnose"
    Write-Host "  $binary status"
    Write-Host "  $binary verify-audit"
}
finally {
    Pop-Location
}
