#!/usr/bin/env bash
# Build the agent for macOS (universal binary: arm64 + x86_64)
set -euo pipefail

echo "Building firstaid for macOS..."

# Add targets if not present
rustup target add aarch64-apple-darwin x86_64-apple-darwin 2>/dev/null || true

# Build for both architectures
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# Create universal binary
lipo -create \
    target/aarch64-apple-darwin/release/firstaid \
    target/x86_64-apple-darwin/release/firstaid \
    -output target/firstaid-universal

echo "Universal binary: target/firstaid-universal"

# Check binary size
SIZE=$(stat -f%z target/firstaid-universal)
SIZE_MB=$(echo "scale=1; $SIZE / 1048576" | bc)
echo "Binary size: ${SIZE_MB} MB"
if [ "$SIZE" -gt $((20 * 1024 * 1024)) ]; then
    echo "WARNING: binary exceeds 20 MB footprint target"
fi

echo "Done."
