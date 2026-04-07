# Build the agent for Windows (x86_64)
$ErrorActionPreference = "Stop"

Write-Host "Building firstaid for Windows..."

# Ensure target is installed
rustup target add x86_64-pc-windows-msvc 2>$null

# Build release
cargo build --release --target x86_64-pc-windows-msvc

$binary = "target\x86_64-pc-windows-msvc\release\firstaid.exe"
$size = (Get-Item $binary).Length
$sizeMB = [math]::Round($size / 1MB, 1)

Write-Host "Binary: $binary"
Write-Host "Binary size: ${sizeMB} MB"

if ($size -gt (20 * 1024 * 1024)) {
    Write-Warning "Binary exceeds 20 MB footprint target"
}

Write-Host "Done."
