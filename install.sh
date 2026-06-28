#!/usr/bin/env bash
# Offerings — Install Script
# Builds from source and installs system-wide.
set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[✓]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
error() { echo -e "${RED}[✗]${NC} $*"; exit 1; }
step()  { echo -e "\n${BOLD}${CYAN}▶ $*${NC}"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

step "Installing build dependencies..."
NEEDED=(pkg-config libfontconfig1-dev libssl-dev build-essential)
MISSING=()
for dep in "${NEEDED[@]}"; do
    if ! dpkg -s "$dep" &>/dev/null 2>&1; then
        MISSING+=("$dep")
    fi
done
if [ ${#MISSING[@]} -gt 0 ]; then
    info "Installing: ${MISSING[*]}"
    sudo apt-get install -y "${MISSING[@]}" || \
        warn "apt-get failed — ensure ${MISSING[*]} are installed manually"
else
    info "All build dependencies present"
fi

if ! command -v cargo &>/dev/null; then
    error "Rust/Cargo not found. Install from https://rustup.rs"
fi
info "Rust $(rustc --version | cut -d' ' -f2) found"

step "Building Offerings from source..."
cd "$SCRIPT_DIR"
cargo build --release
if [[ ! -f "target/release/offerings" ]]; then
    error "Build failed — binary not found at target/release/offerings"
fi
info "Build complete"

step "Installing binary to /usr/local/bin/..."
sudo install -m 0755 target/release/offerings /usr/local/bin/offerings
info "Binary installed: /usr/local/bin/offerings"

step "Installing icon and desktop entry..."
ICON_SRC="$SCRIPT_DIR/assets/icon-logo.png"
if [[ -f "$ICON_SRC" ]]; then
    sudo mkdir -p /usr/share/pixmaps
    sudo cp "$ICON_SRC" /usr/share/pixmaps/offerings.png
    info "Icon installed"
fi

sudo tee /usr/share/applications/offerings.desktop > /dev/null << 'DESKTOP'
[Desktop Entry]
Type=Application
Name=Offerings
GenericName=App Store
Comment=Lilith Linux package store — Flatpak, Snap, AppImage, Deb, and more
Exec=offerings %U
Icon=offerings
Categories=System;PackageManager;Settings;
Keywords=store;install;packages;flatpak;snap;appimage;deb;
Terminal=false
StartupNotify=true
StartupWMClass=offerings
MimeType=application/vnd.debian.binary-package;application/x-debian-package;application/x-appimage;application/vnd.flatpak;application/x-flatpak;application/vnd.snap;application/x-snap;

[Desktop Action OpenLocalPackage]
Name=Open Local Package
Exec=offerings %f
DESKTOP

if command -v update-desktop-database &>/dev/null; then
    sudo update-desktop-database /usr/share/applications 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
    sudo gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true
fi

# Register Offerings as the handler for local package file types
step "Registering MIME type associations..."
MIME_TYPES=(
    "application/vnd.debian.binary-package"
    "application/x-debian-package"
    "application/x-appimage"
    "application/vnd.flatpak"
    "application/x-flatpak"
    "application/vnd.snap"
    "application/x-snap"
)
for mime in "${MIME_TYPES[@]}"; do
    if command -v xdg-mime &>/dev/null; then
        xdg-mime default offerings.desktop "$mime" 2>/dev/null || true
    fi
done
info "MIME associations registered (double-click .deb/.AppImage/.flatpak/.snap to open in Offerings)"
echo ""
echo -e "${GREEN}${BOLD}╔══════════════════════════════════════╗${NC}"
echo -e "${GREEN}${BOLD}║   Offerings installed successfully!  ║${NC}"
echo -e "${GREEN}${BOLD}╚══════════════════════════════════════╝${NC}"
echo ""
echo "  Run: offerings"
echo "  Or find 'Offerings' in your application menu."
