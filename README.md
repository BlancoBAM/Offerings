# Offerings

[![Build Status](https://github.com/BlancoBAM/Offerings/actions/workflows/ci.yml/badge.svg)](https://github.com/BlancoBAM/Offerings/actions)
[![Latest Release](https://img.shields.io/github/v/release/BlancoBAM/Offerings)](https://github.com/BlancoBAM/Offerings/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Offerings** is a unified GUI interface for easy desktop app management designed for Lilith Linux. It combines multiple package sources into a single, beautiful interface, allowing users to discover, install, and manage applications from Flatpak, Snap, AppImage, SOAR/PkgForge, and GitHub Releases.

## ✨ Features

- 📦 **Multi-Source Support**: Seamless integration with Flatpak, Snap, AppImage (AM), SOAR/pkgforge, and GitHub Releases.
- 🎨 **Modern Interface**: Built with Slint for a fast, responsive, and native-feeling experience.
- 🛡️ **Lilith Curated Section**: Hand-picked applications specifically chosen for Lilith Linux users.
- 🔄 **Automated Updates**: Background synchronization ensures you always have the latest versions and package metadata.
- 🔍 **Powerful Search**: Fuzzy matching across all enabled sources to find exactly what you need.
- 🗂️ **Smart Categorization**: Automated classification of thousands of apps into intuitive categories.
- ⚡ **Performance First**: Efficient SQLite-backed caching and incremental loading for smooth browsing of massive catalogs.

## 🚀 Installation

### AppImage (Recommended)

The easiest way to use Offerings on any Linux distribution:

```bash
wget https://github.com/BlancoBAM/Offerings/releases/latest/download/offerings-x86_64.AppImage
chmod +x offerings-x86_64.AppImage
./offerings-x86_64.AppImage
```

### From Source

Ensure you have the Rust toolchain and `slint` dependencies installed:

```bash
git clone https://github.com/BlancoBAM/Offerings.git
cd Offerings
cargo build --release
./target/release/offerings
```

### Via Cargo

```bash
cargo install offerings
```

## 🛠️ Configuration for Developers

### Lilith Curated Section

The Lilith section is populated via `assets/metadata-catalog.json`. To curate apps for this section:

1. Add an entry to `assets/metadata-catalog.json`.
2. Include `"Lilith"` in the `categories` array.
3. The app will automatically populate the Lilith section upon the next refresh.

### Package Sources

Offerings supports the following sources out of the box:
- **Flathub** (Flatpak)
- **Snap Store** (Snap)
- **AM / AppImage** (AppImage)
- **PkgForge** (SOAR)
- **GitHub Releases**

## 🏗️ Project Structure

- `src/`: Core application logic in Rust.
- `ui/`: User interface definitions using Slint.
- `assets/`: Icons, metadata catalogs, and desktop integration files.
- `docs/`: Technical documentation and deployment guides.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request or open an issue for bug reports and feature requests.

---

**Built by [BlancoBAM](https://github.com/BlancoBAM) for the Lilith Linux community.**
