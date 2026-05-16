#!/bin/bash
# Offerings App Store Installer
# Deploys the release binary, icon, and desktop launcher so the app appears
# in the application menu immediately after install.

set -e

PROJECT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_THEME_DIR="$HOME/.local/share/icons/hicolor"

echo "🚀 Installing Offerings App Store..."

# 1. Ensure directories exist
mkdir -p "$BIN_DIR"
mkdir -p "$APP_DIR"
mkdir -p "$ICON_THEME_DIR/256x256/apps"
mkdir -p "$ICON_THEME_DIR/scalable/apps"

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

# 4. Install icon to hicolor theme directory (standard name resolution)
ICON_SRC="$PROJECT_DIR/assets/icon-logo.png"
if [ -f "$ICON_SRC" ]; then
    echo "🖼️  Installing icon..."
    # Resize to 256x256 if ImageMagick is available
    if command -v convert &>/dev/null; then
        convert -resize 256x256! "$ICON_SRC" "$ICON_THEME_DIR/256x256/apps/offerings.png"
    else
        cp "$ICON_SRC" "$ICON_THEME_DIR/256x256/apps/offerings.png"
    fi
    # Also copy SVG placeholder as scalable fallback if only PNG exists
    cp "$ICON_THEME_DIR/256x256/apps/offerings.png" "$ICON_THEME_DIR/scalable/apps/offerings.png" 2>/dev/null || true
fi

# 5. Update icon cache so the desktop environment resolves "offerings" by name
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$ICON_THEME_DIR" 2>/dev/null || true
fi
if command -v update-icon-caches &>/dev/null; then
    update-icon-caches "$ICON_THEME_DIR" 2>/dev/null || true
fi

# 6. Create/Update desktop file using the standard icon name (no hardcoded paths)
echo "🖥️  Updating desktop launcher..."
cat <<EOF > "$APP_DIR/offerings.desktop"
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Lilith Linux package store — Flatpak, Snap, Homebrew, GitHub Releases, and more
Exec=$BIN_DIR/offerings %U
Icon=offerings
Categories=System;PackageManager;Settings;
Keywords=store;install;packages;flatpak;snap;homebrew;
Terminal=false
StartupNotify=true
StartupWMClass=offerings
EOF

# 7. Refresh desktop database
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR"
fi

echo "✅ Installation complete! You can now launch Offerings from your application menu."
