#!/bin/bash
# Build Offerings AppImage
# Usage: ./build-appimage.sh [version]

set -e

VERSION=${1:-$(git describe --tags --always --dirty 2>/dev/null || echo "dev")}
ARCH=$(uname -m)
APP_NAME="Offerings"
BUILD_DIR="$(pwd)/appimage-build"
OUTPUT_DIR="$(pwd)/dist"

echo "=== Building Offerings AppImage ==="
echo "Version: $VERSION"
echo "Architecture: $ARCH"

# Clean previous builds
rm -rf "$BUILD_DIR" "$OUTPUT_DIR"
mkdir -p "$BUILD_DIR" "$OUTPUT_DIR"

# Build release binary
echo "Building release binary..."
cargo build --release

# Copy binary
cp target/release/offerings "$BUILD_DIR/"

# Create AppDir structure
mkdir -p "$BUILD_DIR/AppDir/usr/bin"
mkdir -p "$BUILD_DIR/AppDir/usr/share/applications"
mkdir -p "$BUILD_DIR/AppDir/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$BUILD_DIR/AppDir/usr/share/metainfo"

# Copy binary to AppDir
cp "$BUILD_DIR/offerings" "$BUILD_DIR/AppDir/usr/bin/"

# Create desktop file
cat > "$BUILD_DIR/AppDir/usr/share/applications/offerings.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Unified package manager for Flatpak, Snap, AppImage and more
Exec=offerings %U
Icon=offerings
Categories=System;PackageManager;Utility;
Keywords=store;install;packages;flatpak;snap;appimage;
Terminal=false
StartupNotify=true
MimeType=x-scheme-handler/offerings;
EOF

# Create AppImage desktop file (for the AppImage itself)
cat > "$BUILD_DIR/AppDir/offerings.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Unified package manager for Flatpak, Snap, AppImage and more
Exec=offerings %U
Icon=offerings
Categories=System;PackageManager;Utility;
Keywords=store;install;packages;flatpak;snap;appimage;
Terminal=false
StartupNotify=true
EOF

# Create icon (simple SVG for now)
cat > "$BUILD_DIR/AppDir/usr/share/icons/hicolor/scalable/apps/offerings.svg" << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#8b0000"/>
      <stop offset="100%" style="stop-color:#5c0000"/>
    </linearGradient>
  </defs>
  <rect width="128" height="128" rx="14" fill="url(#bg)"/>
  <text x="64" y="85" font-family="Arial, sans-serif" font-size="60" font-weight="bold" fill="white" text-anchor="middle">O</text>
  <text x="64" y="110" font-family="Arial, sans-serif" font-size="14" fill="#e0e0e0" text-anchor="middle">OFFERINGS</text>
</svg>
EOF

# Copy icon to root of AppDir
cp "$BUILD_DIR/AppDir/usr/share/icons/hicolor/scalable/apps/offerings.svg" "$BUILD_DIR/AppDir/offerings.svg"

# Create AppStream metadata
cat > "$BUILD_DIR/AppDir/usr/share/metainfo/offerings.appdata.xml" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>offerings</id>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>MIT</project_license>
  <name>Offerings</name>
  <summary>Unified package manager for Linux</summary>
  <description>
    <p>
      Offerings is a unified package manager GUI for Linux that combines multiple package sources
      including Flatpak, Snap, AppImage, and more into a single, beautiful interface.
    </p>
    <p>Features:</p>
    <ul>
      <li>Browse packages from multiple sources</li>
      <li>One-click install and removal</li>
      <li>Category-based browsing</li>
      <li>Search functionality</li>
      <li>Automatic updates</li>
      <li>Source selection for duplicate packages</li>
    </ul>
  </description>
  <launchable type="desktop-id">offerings.desktop</launchable>
  <screenshots>
    <screenshot type="default">
      <caption>Main interface showing package categories</caption>
    </screenshot>
  </screenshots>
  <url type="homepage">https://github.com/BlancoBAM/Offerings</url>
  <url type="bugtracker">https://github.com/BlancoBAM/Offerings/issues</url>
  <releases>
    <release version="$VERSION" date="$(date -I)"/>
  </releases>
  <content_rating type="oars-1.1"/>
</component>
EOF

# Create AppRun script
cat > "$BUILD_DIR/AppDir/AppRun" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH}"
exec "${HERE}/usr/bin/offerings" "$@"
EOF
chmod +x "$BUILD_DIR/AppDir/AppRun"

# Download and setup linuxdeploy
echo "Downloading linuxdeploy..."
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage"
curl -L -o "$BUILD_DIR/linuxdeploy.AppImage" "$LINUXDEPLOY_URL"
chmod +x "$BUILD_DIR/linuxdeploy.AppImage"

# Build AppImage
echo "Building AppImage..."
cd "$BUILD_DIR"
./linuxdeploy.AppImage \
    --appdir AppDir \
    --output appimage \
    --desktop-file=AppDir/usr/share/applications/offerings.desktop \
    --icon-file=AppDir/usr/share/icons/hicolor/scalable/apps/offerings.svg \
    --appimage-version="$VERSION"

# Move output to dist directory
mv Offerings-*.AppImage "$OUTPUT_DIR/Offerings-${VERSION}-${ARCH}.AppImage"

echo ""
echo "=== Build Complete ==="
echo "AppImage: $OUTPUT_DIR/Offerings-${VERSION}-${ARCH}.AppImage"
ls -lh "$OUTPUT_DIR/"
