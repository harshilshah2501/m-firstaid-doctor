# FirstAid Doctor — macOS Installation Guide

## System Requirements

| Requirement | Minimum |
|-------------|---------|
| macOS       | 12 Monterey or later |
| Architecture | Apple Silicon (M1/M2/M3) — arm64 |
| Disk space  | 5 MB for binary; ~50 MB for diagnostic database over time |
| Privileges  | Administrator (sudo) required for installation |

> **Intel Mac (x86_64):** Build with `--universal` flag for a fat binary that runs on both.
> See [Building from Source](#building-from-source).

---

## Installing from the DMG

1. Open `firstaid-<version>-arm64.dmg`.
2. Double-click `firstaid-<version>-arm64.pkg` inside the mounted disk image.
3. Follow the installer prompts — administrator password required.
4. After installation completes, open **Terminal** and verify:

```bash
firstaid --version
firstaid status
```

The background daemon starts automatically after the installer finishes.

---

## What Gets Installed

| Path | Description |
|------|-------------|
| `/usr/local/bin/firstaid` | CLI binary |
| `/Library/LaunchDaemons/com.firstaid.doctor.daemon.plist` | launchd daemon definition |
| `/var/lib/firstaid/` | SQLite diagnostic database (`firstaid.db`) |
| `/var/log/firstaid/` | Log files (`firstaid.log`, `firstaid.err`) |

---

## Daemon Management

FirstAid Doctor runs as a **LaunchDaemon** (system-level background service, starts at boot, runs as root).

```bash
# Check daemon status
sudo launchctl list | grep com.firstaid.doctor

# Stop the daemon
sudo launchctl unload /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist

# Start the daemon
sudo launchctl load /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist

# View live logs
tail -f /var/log/firstaid/firstaid.log
tail -f /var/log/firstaid/firstaid.err
```

---

## CLI Usage

```bash
firstaid diagnose            # Run a full diagnostic cycle now
firstaid status              # Show the most recent health snapshot
firstaid history             # List recent issues and remediations
firstaid history --limit 50  # Show last 50 entries
firstaid daemon              # Run daemon in the foreground (debug mode)
firstaid --help              # Full command reference
```

---

## Configuration

The daemon reads environment variables set in its plist. To customize:

```bash
sudo nano /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist
```

Key environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `FIRSTAID_DB` | `/var/lib/firstaid/firstaid.db` | SQLite database path |
| `RUST_LOG` | `info` | Log level: `error`, `warn`, `info`, `debug`, `trace` |

After editing the plist, reload the daemon:

```bash
sudo launchctl unload /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist
sudo launchctl load  /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist
```

---

## Uninstalling

Run the uninstaller script bundled in the DMG:

```bash
sudo bash "/Volumes/FirstAid Doctor <version>/Uninstall FirstAid Doctor.sh"
```

Or, if you no longer have the DMG mounted:

```bash
# Stop and unload the daemon
sudo launchctl unload /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist

# Remove binary and daemon plist
sudo rm -f /usr/local/bin/firstaid
sudo rm -f /Library/LaunchDaemons/com.firstaid.doctor.daemon.plist

# Optionally remove data and logs
sudo rm -rf /var/lib/firstaid /var/log/firstaid
```

---

## Building from Source

### Prerequisites

```bash
# Rust toolchain
curl https://sh.rustup.rs -sSf | sh
rustup target add aarch64-apple-darwin

# For universal binary (Intel + Apple Silicon)
rustup target add x86_64-apple-darwin

# Xcode command-line tools (for codesign, pkgbuild, productbuild, hdiutil)
xcode-select --install
```

### Build commands

```bash
# M1/arm64 only (default, recommended)
bash scripts/package-mac.sh

# Universal binary (arm64 + x86_64)
bash scripts/package-mac.sh --universal

# Skip code signing (local test)
bash scripts/package-mac.sh --skip-sign

# Override version
bash scripts/package-mac.sh --version 1.2.3
```

Output is placed in `dist/macos/`.

### Code Signing and Notarization

For distribution outside of direct downloads (e.g., via your own update mechanism), set:

```bash
export CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAM_ID)"
export NOTARIZE_APPLE_ID="you@example.com"
export NOTARIZE_TEAM_ID="TEAM_ID"
export NOTARIZE_PASSWORD="@keychain:AC_PASSWORD"

bash scripts/package-mac.sh
```

The script will:
1. Sign the binary with your Developer ID.
2. Sign the `.pkg` with the matching Installer certificate.
3. Submit the `.dmg` to Apple's notary service and staple the ticket.

---

## Troubleshooting

### "firstaid: command not found" after install

`/usr/local/bin` may not be in your `PATH`. Add it:

```bash
echo 'export PATH="/usr/local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Daemon not starting

Check for errors in the daemon log:

```bash
sudo cat /var/log/firstaid/firstaid.err
```

Verify the plist loaded correctly:

```bash
sudo launchctl list com.firstaid.doctor.daemon
```

A missing exit code means the daemon is running. A non-zero exit code indicates a crash — check the `.err` log.

### "Package is damaged" or Gatekeeper warning

If Gatekeeper blocks the installer (unsigned build):

```bash
xattr -d com.apple.quarantine firstaid-<version>-arm64.pkg
```

For production use, build with a valid Developer ID and notarize.

### Database corruption

If the SQLite database is corrupted:

```bash
sudo rm /var/lib/firstaid/firstaid.db
sudo launchctl kickstart system/com.firstaid.doctor.daemon
```

The daemon will recreate the schema on startup.
