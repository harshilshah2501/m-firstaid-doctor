#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# package-mac.sh  —  Build the FirstAid Doctor macOS distribution:
#
#   Default: Apple Silicon (arm64 / M1+) binary
#   Optional: Universal binary (arm64 + x86_64) via --universal
#
#   Outputs:
#     dist/macos/firstaid-<version>-arm64.pkg   (or -universal.pkg)
#     dist/macos/firstaid-<version>-arm64.dmg   (or -universal.dmg)
#
# Prerequisites (install once):
#   rustup target add aarch64-apple-darwin
#   rustup target add x86_64-apple-darwin   # only needed with --universal
#   Xcode command-line tools: xcode-select --install
#
# Optional (for signing and notarization):
#   export CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAM_ID)"
#   export NOTARIZE_APPLE_ID="you@example.com"
#   export NOTARIZE_TEAM_ID="TEAM_ID"
#   export NOTARIZE_PASSWORD="@keychain:AC_PASSWORD"
#
# Usage:
#   bash scripts/package-mac.sh                     # M1/arm64 (default)
#   bash scripts/package-mac.sh --universal         # arm64 + x86_64 fat binary
#   bash scripts/package-mac.sh --skip-sign         # skip codesign (local test)
#   bash scripts/package-mac.sh --version 1.2.3     # override version
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ── Parse args ────────────────────────────────────────────────────────────────
SKIP_SIGN=false
UNIVERSAL=false
VERSION=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-sign) SKIP_SIGN=true ;;
        --universal) UNIVERSAL=true ;;
        --version)   VERSION="$2"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
    shift
done

# Derive version from Cargo.toml if not provided
if [ -z "$VERSION" ]; then
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*= *"\(.*\)"/\1/')
fi

APP_NAME="FirstAid Doctor"
BUNDLE_ID="com.firstaid.doctor"
BINARY_NAME="firstaid"
OUT_DIR="$REPO_ROOT/dist/macos"
PKG_SCRIPTS="$REPO_ROOT/packaging/macos"

if $UNIVERSAL; then
    ARCH_LABEL="universal"
else
    ARCH_LABEL="arm64"
fi

echo "==> Building $APP_NAME $VERSION for macOS ($ARCH_LABEL)"
echo ""

# ── Step 1: Build binary ──────────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

echo "--> Building arm64 (Apple Silicon / M1+) binary…"
rustup target add aarch64-apple-darwin 2>/dev/null || true
cargo build --release --target aarch64-apple-darwin -p agent-core

if $UNIVERSAL; then
    echo "--> Building x86_64 (Intel) binary…"
    rustup target add x86_64-apple-darwin 2>/dev/null || true
    cargo build --release --target x86_64-apple-darwin -p agent-core

    echo "--> Creating universal binary via lipo…"
    FINAL_BIN="$OUT_DIR/${BINARY_NAME}-universal"
    lipo -create \
        "target/aarch64-apple-darwin/release/$BINARY_NAME" \
        "target/x86_64-apple-darwin/release/$BINARY_NAME" \
        -output "$FINAL_BIN"

    lipo -archs "$FINAL_BIN" | grep -q "x86_64" || { echo "ERROR: x86_64 missing from fat binary"; exit 1; }
    lipo -archs "$FINAL_BIN" | grep -q "arm64"  || { echo "ERROR: arm64 missing from fat binary";  exit 1; }
    echo "    Architectures: $(lipo -archs "$FINAL_BIN")"
else
    FINAL_BIN="$OUT_DIR/${BINARY_NAME}-arm64"
    cp "target/aarch64-apple-darwin/release/$BINARY_NAME" "$FINAL_BIN"

    # Confirm it's really arm64
    ARCH=$(file "$FINAL_BIN" | grep -o "arm64\|x86_64" | head -1)
    echo "    Architecture : $ARCH"
    [[ "$ARCH" == "arm64" ]] || { echo "ERROR: expected arm64, got $ARCH"; exit 1; }
fi

SIZE_MB=$(stat -f%z "$FINAL_BIN" | awk '{printf "%.1f", $1/1048576}')
echo "    Binary size  : ${SIZE_MB} MB"
if (( $(stat -f%z "$FINAL_BIN") > 30*1024*1024 )); then
    echo "WARNING: binary exceeds 30 MB footprint target"
fi

# ── Step 2: Code sign ─────────────────────────────────────────────────────────
if [ "$SKIP_SIGN" = false ] && [ -n "${CODESIGN_IDENTITY:-}" ]; then
    echo "--> Code signing with identity: $CODESIGN_IDENTITY"
    codesign \
        --sign "$CODESIGN_IDENTITY" \
        --entitlements "$PKG_SCRIPTS/entitlements.plist" \
        --options runtime \
        --timestamp \
        --force \
        "$FINAL_BIN"
    echo "    Signed OK"
else
    echo "--> Skipping code sign (set CODESIGN_IDENTITY or pass --skip-sign)"
    codesign --sign "-" "$FINAL_BIN"   # ad-hoc sign for local testing
fi

# ── Step 3: Build .pkg ────────────────────────────────────────────────────────
echo "--> Building .pkg installer…"

PKG_ROOT="$OUT_DIR/pkgroot"
rm -rf "$PKG_ROOT"
mkdir -p "$PKG_ROOT/usr/local/bin"
mkdir -p "$PKG_ROOT/Library/LaunchDaemons"

cp "$FINAL_BIN" "$PKG_ROOT/usr/local/bin/$BINARY_NAME"
cp "$PKG_SCRIPTS/com.firstaid.doctor.daemon.plist" \
   "$PKG_ROOT/Library/LaunchDaemons/com.firstaid.doctor.daemon.plist"

# Make scripts executable
chmod +x "$PKG_SCRIPTS/preinstall" "$PKG_SCRIPTS/postinstall"

# Component package
COMPONENT_PKG="$OUT_DIR/${BINARY_NAME}-component.pkg"
pkgbuild \
    --root "$PKG_ROOT" \
    --identifier "${BUNDLE_ID}" \
    --version "$VERSION" \
    --scripts "$PKG_SCRIPTS" \
    --install-location "/" \
    "$COMPONENT_PKG"

# Distribution package (wraps the component, adds welcome/license screen)
DIST_XML="$OUT_DIR/distribution.xml"
cat > "$DIST_XML" <<DIST_XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>FirstAid Doctor ${VERSION}</title>
    <organization>com.firstaid.doctor</organization>
    <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
    <options customize="never" require-scripts="true" hostArchitectures="arm64"/>
    <choices-outline>
        <line choice="default">
            <line choice="com.firstaid.doctor"/>
        </line>
    </choices-outline>
    <choice id="default"/>
    <choice id="com.firstaid.doctor" visible="false">
        <pkg-ref id="com.firstaid.doctor"/>
    </choice>
    <pkg-ref id="com.firstaid.doctor" version="${VERSION}" onConclusion="none">${BINARY_NAME}-component.pkg</pkg-ref>
</installer-gui-script>
DIST_XML

PRODUCT_PKG="$OUT_DIR/${BINARY_NAME}-${VERSION}-${ARCH_LABEL}.pkg"
productbuild \
    --distribution "$DIST_XML" \
    --package-path "$OUT_DIR" \
    "$PRODUCT_PKG"

echo "    Package: $PRODUCT_PKG"

# ── Step 4: Sign the .pkg ─────────────────────────────────────────────────────
if [ "$SKIP_SIGN" = false ] && [ -n "${CODESIGN_IDENTITY:-}" ]; then
    PKG_IDENTITY="${CODESIGN_IDENTITY/Application/Installer}"   # Installer cert
    SIGNED_PKG="$OUT_DIR/${BINARY_NAME}-${VERSION}-signed.pkg"
    productsign --sign "$PKG_IDENTITY" "$PRODUCT_PKG" "$SIGNED_PKG"
    PRODUCT_PKG="$SIGNED_PKG"
    echo "    Signed package: $PRODUCT_PKG"
fi

# ── Step 5: Build .dmg ────────────────────────────────────────────────────────
echo "--> Creating .dmg…"

DMG_STAGING="$OUT_DIR/dmg_contents"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"

cp "$PRODUCT_PKG"                 "$DMG_STAGING/"
cp "$PKG_SCRIPTS/uninstall.sh"    "$DMG_STAGING/Uninstall FirstAid Doctor.sh"
cat > "$DMG_STAGING/README.txt" <<README
FirstAid Doctor ${VERSION}
==========================

INSTALL
  Double-click '${BINARY_NAME}-${VERSION}-${ARCH_LABEL}.pkg' and follow the prompts.
  The installer requires administrator privileges.

  The background daemon will start automatically after installation.
  Binary is installed to /usr/local/bin/firstaid.

USAGE
  firstaid diagnose        Run a full diagnostic cycle
  firstaid status          Show last known health state
  firstaid history         Show recent issue history
  firstaid daemon          Start background daemon (or use the launchd service)

UNINSTALL
  sudo bash 'Uninstall FirstAid Doctor.sh'

LOGS
  /var/log/firstaid/firstaid.log
  /var/log/firstaid/firstaid.err
README

# Create read/write DMG from the staging folder
TEMP_DMG="$OUT_DIR/tmp_rw.dmg"
hdiutil create \
    -srcfolder "$DMG_STAGING" \
    -volname "$APP_NAME $VERSION" \
    -fs HFS+ \
    -fsargs "-c c=64,a=16,b=16" \
    -format UDRW \
    -size 100m \
    "$TEMP_DMG"

# Convert to compressed read-only DMG
FINAL_DMG="$OUT_DIR/${BINARY_NAME}-${VERSION}-${ARCH_LABEL}.dmg"
hdiutil convert "$TEMP_DMG" \
    -format UDZO \
    -imagekey zlib-level=9 \
    -o "$FINAL_DMG"
rm -f "$TEMP_DMG"

# ── Step 6: Notarize (optional) ───────────────────────────────────────────────
if [ "$SKIP_SIGN" = false ] \
   && [ -n "${NOTARIZE_APPLE_ID:-}" ] \
   && [ -n "${NOTARIZE_TEAM_ID:-}" ] \
   && [ -n "${NOTARIZE_PASSWORD:-}" ]; then
    echo "--> Submitting for notarization (this may take several minutes)…"
    xcrun notarytool submit "$FINAL_DMG" \
        --apple-id  "$NOTARIZE_APPLE_ID" \
        --team-id   "$NOTARIZE_TEAM_ID" \
        --password  "$NOTARIZE_PASSWORD" \
        --wait
    xcrun stapler staple "$FINAL_DMG"
    echo "    Notarized and stapled."
else
    echo "--> Skipping notarization (set NOTARIZE_* env vars to enable)"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "==> Done."
echo ""
echo "    Package : $PRODUCT_PKG"
echo "    DMG     : $FINAL_DMG"
echo ""
echo "    To test locally:"
echo "      sudo installer -pkg '$PRODUCT_PKG' -target /"
echo "      firstaid diagnose"
