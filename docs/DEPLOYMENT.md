# Offerings Deployment Guide

## Overview

This document describes how to build, deploy, and distribute Offerings across multiple channels.

## Build Options

### 1. Local Development Build

```bash
cd offerings
cargo build --release
./target/release/offerings
```

Before packaging, run the pre-release validation flow in [docs/TESTING.md](./TESTING.md).

### 2. System Installation

```bash
# Build and install
cargo build --release
sudo ./install.sh

# Or manually
sudo cp target/release/offerings /usr/local/bin/
sudo cp offerings.desktop /usr/share/applications/
sudo cp assets/icon-logo.png /usr/share/icons/hicolor/512x512/apps/offerings.png
```

### 3. AppImage Distribution

```bash
# Build AppImage
./build-appimage.sh v1.0.0

# Output: dist/Offerings-1.0.0-x86_64.AppImage
```

### 4. GitHub Releases (Automated via CI/CD)

When you push a tag starting with `v`:
- CI builds the release binary
- CI builds the AppImage
- CI creates a GitHub Release with artifacts
- Checksums are automatically generated

## CI/CD Pipeline

The GitHub Actions workflow (`.github/workflows/ci.yml`) handles:

1. **Build Job**
   - Checks code formatting
   - Runs Clippy lints
   - Builds release binary
   - Runs tests
   - Uploads binary artifact

2. **AppImage Job**
   - Installs dependencies
   - Builds release binary
   - Creates AppImage with linuxdeploy
   - Uploads AppImage artifact

3. **Release Job** (on tags only)
   - Downloads artifacts
   - Creates checksums
   - Creates GitHub Release with all assets

## Creating a Release

```bash
# 1. Update version in Cargo.toml
# 2. Commit changes
git commit -am "Release v1.0.0"

# 3. Create and push tag
git tag v1.0.0
git push origin v1.0.0

# CI will automatically:
# - Build everything
# - Create GitHub Release
# - Attach binaries and AppImage
```

## Package Sources Integration

Offerings aggregates packages from:

| Source | Adapter | Update Frequency |
|--------|---------|------------------|
| Flathub | FlatpakAdapter | Daily |
| AM | AppImageAdapter | Daily |
| SOAR | SoarAdapter | Every 12 hours |
| Snap Store | SnapAdapter | Daily |
| Homebrew | HomebrewAdapter | Daily |
| GitHub Releases | GitHubReleaseAdapter | Every 12 hours |

## Adding New Sources

1. Create adapter in `src/adapters/`:
```rust
// src/adapters/newsource.rs
use super::{PackageAdapter, Package, OperationResult};

pub struct NewSourceAdapter { ... }

#[async_trait]
impl PackageAdapter for NewSourceAdapter {
    // Implement required methods
}
```

2. Register in `src/backend.rs`:
```rust
let adapters: Vec<Arc<dyn PackageAdapter>> = vec![
    // ... existing adapters
    Arc::new(NewSourceAdapter::new()),
];
```

3. Update documentation

## Desktop Integration

The desktop entry (`assets/offerings.desktop`) provides:
- Application menu entry
- MIME type handling (`x-scheme-handler/offerings://`)
- Quick actions (Check for Updates, Refresh)

## Icon Assets

Icons are provided in:
- SVG (scalable): `assets/offerings.svg`
- PNG variants can be generated from SVG

## Troubleshooting

### AppImage won't run
```bash
# Make executable
chmod +x Offerings-*.AppImage

# If still issues, try with fuse
./Offerings-*.AppImage --appimage-extract
./squashfs-root/AppRun
```

### Binary crashes on startup
```bash
# Check dependencies
ldd target/release/offerings

# Run with debug output
RUST_BACKTRACE=1 ./target/release/offerings
```

### Packages not showing
```bash
# Force refresh
offerings --refresh

# Run headless diagnostics
offerings --self-test

# Check logs
journalctl -f | grep offerings
```

## Performance Considerations

- Initial cache refresh: ~30-60 seconds (5000+ packages)
- Subsequent launches: <5 seconds (cached data)
- Memory usage: ~100-200MB
- Disk usage: ~50MB (binary + cache)

## Security

- All package operations use system package managers (flatpak, snap, etc.)
- No elevated privileges required for most operations
- AppImage is sandboxed by default
- Source code is open for audit

## Support Channels

- GitHub Issues: Bug reports and feature requests
- GitHub Discussions: Questions and community support
- Documentation: `docs/` directory

---

Last updated: March 29, 2026
