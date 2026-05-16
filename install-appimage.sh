#!/bin/bash
# Offerings AppImage Desktop Integration Installer
# Run this script after downloading the AppImage to register it
# as a proper desktop application with icon.
#
# Usage: ./install-appimage.sh [path/to/Offerings-*.AppImage]

set -e

APPIMAGE_PATH="${1:-}"
ICON_THEME_DIR="$HOME/.local/share/icons/hicolor"
APP_DIR="$HOME/.local/share/applications"

# Auto-detect AppImage in current directory if not provided
if [ -z "$APPIMAGE_PATH" ]; then
    APPIMAGE_PATH=$(ls ./dist/Offerings-*.AppImage 2>/dev/null | head -1 || ls ./Offerings-*.AppImage 2>/dev/null | head -1 || true)
fi

if [ -z "$APPIMAGE_PATH" ] || [ ! -f "$APPIMAGE_PATH" ]; then
    echo "❌ Error: AppImage not found. Usage: $0 /path/to/Offerings-*.AppImage"
    exit 1
fi

APPIMAGE_ABS="$(realpath "$APPIMAGE_PATH")"
chmod +x "$APPIMAGE_ABS"

echo "🚀 Installing Offerings AppImage desktop integration..."
echo "   AppImage: $APPIMAGE_ABS"

# Ensure directories exist
mkdir -p "$ICON_THEME_DIR/256x256/apps"
mkdir -p "$APP_DIR"

# Extract icon from the AppImage's embedded assets
echo "🖼️  Extracting icon..."
SQUASHFS_ROOT=$(APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE_ABS" --appimage-extract 2>/dev/null; echo "squashfs-root")
ICON_SRC=""
if [ -f "squashfs-root/usr/share/icons/hicolor/256x256/apps/offerings.png" ]; then
    ICON_SRC="squashfs-root/usr/share/icons/hicolor/256x256/apps/offerings.png"
elif [ -f "squashfs-root/offerings.png" ]; then
    ICON_SRC="squashfs-root/offerings.png"
fi

if [ -n "$ICON_SRC" ] && [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$ICON_THEME_DIR/256x256/apps/offerings.png"
    echo "   Icon installed to $ICON_THEME_DIR/256x256/apps/offerings.png"
fi

# Clean up extracted squashfs
rm -rf squashfs-root

# Update icon cache
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$ICON_THEME_DIR" 2>/dev/null || true
fi

# Create desktop file pointing to this AppImage
echo "🖥️  Creating desktop launcher..."
cat <<EOF > "$APP_DIR/offerings.desktop"
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Lilith Linux package store — Flatpak, Snap, Homebrew, GitHub Releases, and more
Exec=$APPIMAGE_ABS %U
Icon=offerings
Categories=System;PackageManager;Settings;
Keywords=store;install;packages;flatpak;snap;homebrew;
Terminal=false
StartupNotify=true
StartupWMClass=offerings
EOF

# Refresh desktop database
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR"
fi

echo "✅ Done! Offerings now appears in your application menu."
echo "   To launch: offerings (if on PATH) or $APPIMAGE_ABS"
