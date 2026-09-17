# FirstAid Doctor — Windows Installation Guide

## System Requirements

| Requirement | Minimum |
|-------------|---------|
| Windows     | 10 version 1809 (build 17763) or later |
| Architecture | x86_64 (64-bit) |
| Disk space  | 10 MB for binary; ~50 MB for diagnostic database over time |
| Privileges  | Administrator required for installation |

---

## Choosing an Installer Format

Two installer formats are provided. Both install identical software.

| Format | File | Best for |
|--------|------|----------|
| **MSI** | `firstaid-<version>-x86_64-pc-windows-msvc.msi` | Enterprise deployment, SCCM/Intune, Group Policy, silent installs |
| **EXE** | `firstaid-<version>-setup.exe` | Individual end-user installation, wizard experience |

---

## Installing with the EXE (Wizard)

1. Download `firstaid-<version>-setup.exe`.
2. Right-click → **Run as administrator**.
3. Follow the wizard prompts.
4. On the final screen, leave **"Add to system PATH"** checked (recommended).
5. After the wizard finishes, open a new **Command Prompt** or **PowerShell** window:

```powershell
firstaid --version
firstaid status
```

The Windows Service starts automatically after installation.

---

## Installing with the MSI (Silent / Enterprise)

### Interactive install

```powershell
msiexec /i firstaid-<version>-x86_64-pc-windows-msvc.msi
```

### Silent install (no UI)

```powershell
msiexec /i firstaid-<version>-x86_64-pc-windows-msvc.msi /quiet /qn /norestart
```

### Silent install with log

```powershell
msiexec /i firstaid-<version>-x86_64-pc-windows-msvc.msi /quiet /qn /l*v C:\Temp\firstaid-install.log
```

### SCCM / Intune deployment

Use the standard MSI detection method: check for `SOFTWARE\FirstAidDoctor` → `Version` in the registry (`HKLM`).

---

## What Gets Installed

| Path | Description |
|------|-------------|
| `%ProgramFiles%\FirstAid Doctor\firstaid.exe` | Main binary |
| `%ProgramFiles%\FirstAid Doctor\onnxruntime.dll` | ML runtime (if bundled) |
| `%ProgramData%\FirstAid Doctor\` | Diagnostic database (`firstaid.db`) |
| `%ProgramData%\FirstAid Doctor\logs\` | Log files |
| `HKLM\SOFTWARE\FirstAidDoctor` | Version stamp and path registry keys |
| Windows Service: `firstaid` | Auto-start service running as LocalSystem |

The install directory is added to the **system PATH** so `firstaid` is available in all terminals.

---

## Service Management

FirstAid Doctor runs as a **Windows Service** (`firstaid`), set to automatic start.

```powershell
# Check service status
sc query firstaid

# Stop the service
net stop firstaid
# or
Stop-Service firstaid

# Start the service
net start firstaid
# or
Start-Service firstaid

# Restart the service
Restart-Service firstaid

# View service details
Get-Service firstaid | Format-List *
```

### Event Viewer

Service events (start, stop, crashes) appear in:  
`Event Viewer → Windows Logs → Application` — filter by Source: `firstaid`

---

## CLI Usage

Open any terminal (Command Prompt, PowerShell, Windows Terminal):

```powershell
firstaid diagnose            # Run a full diagnostic cycle now
firstaid status              # Show the most recent health snapshot
firstaid history             # List recent issues and remediations
firstaid history --limit 50  # Show last 50 entries
firstaid daemon              # Run the daemon in the foreground (debug)
firstaid --help              # Full command reference
```

---

## Configuration

The service reads standard Windows environment variables. To set persistent configuration:

```powershell
# Set database path (system-wide, requires admin)
[System.Environment]::SetEnvironmentVariable("FIRSTAID_DB", "D:\data\firstaid.db", "Machine")

# Set log level
[System.Environment]::SetEnvironmentVariable("RUST_LOG", "debug", "Machine")
```

After changing environment variables, restart the service:

```powershell
Restart-Service firstaid
```

| Variable | Default | Description |
|----------|---------|-------------|
| `FIRSTAID_DB` | `%ProgramData%\FirstAid Doctor\firstaid.db` | SQLite database path |
| `RUST_LOG` | `info` | Log level: `error`, `warn`, `info`, `debug`, `trace` |

---

## Uninstalling

### Via Settings (GUI)

`Settings → Apps → Installed apps → FirstAid Doctor → Uninstall`

### Via Command Line (silent)

```powershell
# With EXE installer — find the uninstall string in registry
$uninst = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*" |
           Where-Object DisplayName -eq "FirstAid Doctor").UninstallString
Start-Process $uninst -ArgumentList "/SILENT" -Wait

# With MSI
$productCode = "{E3A7C1D2-8F4B-4E9A-B2C5-6D0F3A1E7B94}"
msiexec /x $productCode /quiet /qn /norestart
```

Uninstall stops and removes the Windows Service automatically.  
The `%ProgramData%\FirstAid Doctor\` data directory is **preserved** by default (uninstall does not delete your diagnostic history).

To remove data as well:

```powershell
Remove-Item -Recurse -Force "$env:ProgramData\FirstAid Doctor"
```

---

## Building from Source

### Prerequisites

```powershell
# Rust toolchain (from https://rustup.rs)
rustup target add x86_64-pc-windows-msvc

# For MSI builds
cargo install cargo-wix
# WiX Toolset 3.x: https://wixtoolset.org/releases/

# For EXE builds
# Inno Setup 6: https://jrsoftware.org/isdl.php
```

### Build commands

```powershell
# Both MSI and EXE (default)
.\scripts\package-windows.ps1

# MSI only
.\scripts\package-windows.ps1 -SkipExe

# EXE only
.\scripts\package-windows.ps1 -SkipMsi

# Override version
.\scripts\package-windows.ps1 -Version 1.2.3

# Sign outputs (requires CERT_PATH and CERT_PASSWORD env vars)
$env:CERT_PATH     = "C:\certs\my.pfx"
$env:CERT_PASSWORD = "hunter2"
.\scripts\package-windows.ps1 -Sign
```

Output is placed in `dist\windows\`.

---

## Troubleshooting

### "firstaid is not recognized as a command"

The PATH change takes effect in **new** terminal windows. Close and reopen your terminal, or:

```powershell
# Refresh PATH in current session
$env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" +
            [System.Environment]::GetEnvironmentVariable("Path","User")
```

### Service fails to start

Check Windows Event Viewer for the error, or run the binary in the foreground to see the error directly:

```powershell
& "$env:ProgramFiles\FirstAid Doctor\firstaid.exe" daemon
```

### EXE window closes immediately when double-clicked

`firstaid.exe` is primarily a CLI tool. When launched without arguments, it now runs a default `diagnose` pass and then waits for Enter before closing.

For regular usage, open PowerShell and run explicit commands:

```powershell
firstaid diagnose
firstaid status
firstaid --help
```

### Database corruption

If the SQLite database is corrupted:

```powershell
Stop-Service firstaid
Remove-Item "$env:ProgramData\FirstAid Doctor\firstaid.db"
Start-Service firstaid
```

The service recreates the schema on startup.

### Installation fails with "another version is already installed"

A newer version is present. Uninstall the existing version first:

```powershell
msiexec /x "{E3A7C1D2-8F4B-4E9A-B2C5-6D0F3A1E7B94}" /quiet /qn
```

Then install the new version.

### Antivirus false positive

Some AV engines flag unsigned Rust binaries. To verify authenticity, check the SHA-256 hash against `models/manifest.json` in the release, or build from source. For enterprise deployments, sign with your organisation's code-signing certificate using the `-Sign` flag.
