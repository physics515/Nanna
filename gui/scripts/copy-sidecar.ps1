# Copy nanna-daemon to sidecar binaries folder for development
# Run from gui folder: .\scripts\copy-sidecar.ps1

$target = $env:CARGO_TARGET_DIR
if (-not $target) {
    $target = "..\..\target"
}

$source = "$target\debug\nanna-daemon.exe"
$destDir = "src-tauri\binaries"
$dest = "$destDir\nanna-daemon-x86_64-pc-windows-msvc.exe"

# Create binaries folder if needed
if (-not (Test-Path $destDir)) {
    New-Item -ItemType Directory -Path $destDir -Force | Out-Null
}

# Build daemon if not exists
if (-not (Test-Path $source)) {
    Write-Host "Building nanna-daemon..."
    Push-Location ..
    cargo build -p nanna-daemon
    Pop-Location
}

# Copy
if (Test-Path $source) {
    Copy-Item $source $dest -Force
    Write-Host "Copied daemon to $dest"
} else {
    Write-Error "Failed to find daemon binary at $source"
    exit 1
}
