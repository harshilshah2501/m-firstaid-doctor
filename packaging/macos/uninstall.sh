#!/bin/bash
# FirstAid Doctor — macOS uninstaller
# Run as root: sudo bash uninstall.sh
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "Error: this script must be run as root (sudo)." >&2
    exit 1
fi

DAEMON_LABEL="com.firstaid.doctor.daemon"
PLIST="/Library/LaunchDaemons/${DAEMON_LABEL}.plist"

echo "Uninstalling FirstAid Doctor…"

# Stop and unload daemon
if launchctl list | grep -q "$DAEMON_LABEL" 2>/dev/null; then
    echo "  Stopping daemon…"
    launchctl unload "$PLIST" 2>/dev/null || true
fi

# Remove installed files
FILES=(
    "/usr/local/bin/firstaid"
    "$PLIST"
)
for f in "${FILES[@]}"; do
    if [ -e "$f" ]; then
        echo "  Removing $f"
        rm -f "$f"
    fi
done

# Ask before removing data (non-fatal)
echo ""
read -r -p "Remove diagnostic database and logs in /var/lib/firstaid and /var/log/firstaid? [y/N] " CONFIRM
if [[ "$CONFIRM" =~ ^[Yy]$ ]]; then
    rm -rf /var/lib/firstaid /var/log/firstaid
    echo "  Data directories removed."
else
    echo "  Data kept at /var/lib/firstaid and /var/log/firstaid."
fi

echo ""
echo "FirstAid Doctor has been uninstalled."
