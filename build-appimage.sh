#!/bin/bash
# Build Offerings AppImage
# Usage: ./build-appimage.sh [version]
# Works in CI without FUSE via APPIMAGE_EXTRACT_AND_RUN=1

set -e

VERSION=${1:-$(git describe --tags --always --dirty 2>/dev/null || echo "dev")}
ARCH=$(uname -m)
BUILD_DIR="$(pwd)/appimage-build"
OUTPUT_DIR="$(pwd)/dist"

echo "=== Building Offerings AppImage ==="
echo "Version: $VERSION"
echo "Architecture: $ARCH"

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR" "$OUTPUT_DIR"

# Build release binary if not already done
if [ ! -f "target/release/offerings" ]; then
    echo "Building release binary..."
    cargo build --release
fi

# Create AppDir structure
APPDIR="$BUILD_DIR/AppDir"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$APPDIR/usr/share/metainfo"

cp "target/release/offerings" "$APPDIR/usr/bin/offerings"
chmod +x "$APPDIR/usr/bin/offerings"

# Icon
if [ -f "assets/icon-logo.png" ]; then
    cp "assets/icon-logo.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/offerings.png"
    cp "assets/icon-logo.png" "$APPDIR/offerings.png"
    ICON_ARG="--icon-file=$APPDIR/usr/share/icons/hicolor/256x256/apps/offerings.png"
else
    cat > "$APPDIR/usr/share/icons/hicolor/scalable/apps/offerings.svg" << 'SVGEOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect width="128" height="128" rx="14" fill="#8b0000"/>
  <text x="64" y="85" font-family="Arial" font-size="60" font-weight="bold" fill="white" text-anchor="middle">O</text>
</svg>
SVGEOF
    cp "$APPDIR/usr/share/icons/hicolor/scalable/apps/offerings.svg" "$APPDIR/offerings.svg"
    ICON_ARG="--icon-file=$APPDIR/usr/share/icons/hicolor/scalable/apps/offerings.svg"
fi

# Desktop file
cat > "$APPDIR/usr/share/applications/offerings.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Unified package manager for Flatpak, Snap, AppImage and more
Exec=offerings %U
Icon=offerings
Categories=Utility;PackageManager;
Terminal=false
StartupNotify=true
EOF

cp "$APPDIR/usr/share/applications/offerings.desktop" "$APPDIR/offerings.desktop"

# AppStream metadata
cat > "$APPDIR/usr/share/metainfo/com.lilithlinux.Offerings.metainfo.xml" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>com.lilithlinux.Offerings</id>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>MIT</project_license>
  <name>Offerings</name>
  <summary>Unified package manager for Linux</summary>
  <launchable type="desktop-id">offerings.desktop</launchable>
  <url type="homepage">https://github.com/BlancoBAM/Offerings</url>
  <releases>
    <release version="${VERSION}" date="$(date -I)"/>
  </releases>
  <content_rating type="oars-1.1"/>
</component>
EOF

# AppRun entry point
cat > "$APPDIR/AppRun" << 'APPRUNEOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH}"
if [ -z "$DISPLAY" ] && [ -z "$WAYLAND_DISPLAY" ]; then
    export DISPLAY=:0
fi
exec "${HERE}/usr/bin/offerings" "$@"
APPRUNEOF
chmod +x "$APPDIR/AppRun"

# Download linuxdeploy
echo "Downloading linuxdeploy..."
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage"
curl -L --retry 3 -o "$BUILD_DIR/linuxdeploy.AppImage" "$LINUXDEPLOY_URL"
chmod +x "$BUILD_DIR/linuxdeploy.AppImage"

# Build AppImage (APPIMAGE_EXTRACT_AND_RUN=1 avoids FUSE in CI)
echo "Building AppImage..."
cd "$BUILD_DIR"
APPIMAGE_EXTRACT_AND_RUN=1 LINUXDEPLOY_OUTPUT_VERSION="$VERSION" \
    ./linuxdeploy.AppImage \
    --appdir AppDir \
    --output appimage \
    --desktop-file="AppDir/offerings.desktop" \
    $ICON_ARG

APPIMAGE_FILE=$(ls Offerings-*.AppImage 2>/dev/null | head -1 || ls *.AppImage 2>/dev/null | head -1 || true)
if [ -n "$APPIMAGE_FILE" ]; then
    mv "$APPIMAGE_FILE" "$OUTPUT_DIR/Offerings-${VERSION}-${ARCH}.AppImage"
    echo "=== Build Complete ==="
    echo "AppImage: $OUTPUT_DIR/Offerings-${VERSION}-${ARCH}.AppImage"
    ls -lh "$OUTPUT_DIR/"
else
    echo "ERROR: No AppImage produced"
    ls -la
    exit 1
fi
