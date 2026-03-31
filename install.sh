#!/bin/bash
# Offerings App Store Installer
# Deploys the release binary and desktop launcher

set -e

PROJECT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_PATH="$PROJECT_DIR/assets/icon-logo.png"

echo "🚀 Installing Offerings App Store..."

# 1. Ensure directories exist
mkdir -p "$BIN_DIR"
mkdir -p "$APP_DIR"

# 2. Check for release binary
RELEASE_BIN="$PROJECT_DIR/target/release/offerings"
if [ ! -f "$RELEASE_BIN" ]; then
    echo "❌ Error: Release binary not found at $RELEASE_BIN"
    echo "Please run 'cargo build --release' first."
    exit 1
fi

# 3. Copy binary
echo "📦 Copying binary to $BIN_DIR/offerings..."
cp "$RELEASE_BIN" "$BIN_DIR/offerings"
chmod +x "$BIN_DIR/offerings"

# 4. Create/Update desktop file
echo "🖥️  Updating desktop launcher..."
cat <<EOF > "$APP_DIR/offerings.desktop"
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Lilith Linux package store — Flatpak, Snap, Homebrew, GitHub Releases, and more
Exec=$BIN_DIR/offerings
Icon=$ICON_PATH
Categories=System;PackageManager;Settings;
Keywords=store;install;packages;flatpak;snap;homebrew;
Terminal=false
StartupNotify=true
StartupWMClass=offerings
EOF

# 5. Refresh desktop database
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR"
fi

echo "✅ Installation complete! You can now launch Offerings from your application menu."
