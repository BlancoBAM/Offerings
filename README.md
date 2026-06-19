<div align="center">

<img src="assets/icon-logo.png" alt="Offerings Icon" width="200" />

<br/><br/>

<img src="assets/logo.png" alt="Offerings Banner" width="700" />

<br/><br/>

**A unified GUI interface for easy desktop app management — designed for Lilith Linux**

[![Build Status](https://github.com/BlancoBAM/Offerings/actions/workflows/ci.yml/badge.svg)](https://github.com/BlancoBAM/Offerings/actions)
[![Latest Release](https://img.shields.io/github/v/release/BlancoBAM/Offerings?include_prereleases)](https://github.com/BlancoBAM/Offerings/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

</div>

---

> [!WARNING]
> **This is a beta release.** I make no guarantees that the app works as intended at this time, and welcome testing and issues. Please give your feedback! It definitely needs some polish and code cleanup — I'm sure that's not all. This was designed for an upcoming distro named **Lilith Linux**, after the demoness mythology, designed for someone specific and the theming reflects the demonic/satanic-ish theming of the distro. It's built in Rust, as is pretty much everything I write these days, with [Slint](https://slint.dev/) for the GUI.

**Offerings** is a unified GUI interface for easy desktop app management designed for Lilith Linux. It combines multiple package sources into a single, beautiful interface, allowing users to discover, install, and manage applications from Flatpak, Snap, AppImage, SOAR/PkgForge, and GitHub Releases. Catalogue of over 9,000 apps!

## ✨ Features

- 📦 **Multi-Source Support**: Seamless integration with Flatpak, Snap, AppImage (AM), SOAR/pkgforge, and GitHub Releases.
- 🎨 **Modern Interface**: Built with Slint for a fast, responsive, and native-feeling experience.
- 🛡️ **Lilith Curated Section**: Hand-picked applications specifically chosen for Lilith Linux users.
- 🔄 **Automated Updates**: Background synchronization ensures you always have the latest versions and package metadata.
- 🔍 **Powerful Search**: Fuzzy matching across all enabled sources to find exactly what you need.
- 🗂️ **Smart Categorization**: Automated classification of thousands of apps into intuitive categories.

>[!NOTE]
As of yet,I haven't taken the time to manually categorize packages, so Qwen automated the process, and (go figure) there are errors with a model performing the task. For example, titles containing 'mAIl are placed in the AI tab,and Miscellaneous has 3,000+ apps. Help is more than welcome!

- ⚡ **Performance First**: Efficient SQLite-backed caching and incremental loading for smooth browsing of massive catalogs.

## 🚀 Installation

### Binary (Recommended)

Download the latest pre-built binary from the [Releases page](https://github.com/BlancoBAM/Offerings/releases/latest):

```bash
wget https://github.com/BlancoBAM/Offerings/releases/latest/download/offerings-linux-amd64
sudo install -m 0755 offerings-linux-amd64 /usr/local/bin/offerings
```

### From Source

```bash
# Clone the repository
git clone https://github.com/BlancoBAM/Offerings.git
cd Offerings

# Run the install script (handles deps, build, and desktop integration)
bash install.sh
```

> **Note:** The `cargo install offerings` and AppImage methods are not currently supported.
> Use the binary download or build from source via `install.sh`.

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

## Credits and Appreciation

Huge thanks go to these projects and developers, first and foremost:

- **[SOAR](https://github.com/pkgforge/soar)** — An incredible terminal package manager I've used for years while distro-hopping.
- **[AM](https://github.com/ivan-hc/AM)** — The AppImage project providing thousands of apps, with its own [GUI](https://github.com/Shikakiben/AM-GUI) and SOAR integration.
- **[Flathub](https://flathub.org/en)** — And dedicated stores built on top of it, such as [Bazaar](https://flathub.org/en/apps/io.github.kolunmi.Bazaar) and [COSMIC Store](https://github.com/pop-os/cosmic-store), which inspired Offerings' layout and UX.
- **[Snapcraft](https://snapcraft.io/store)** — If you use a compatible distro and don't dislike snaps.
- **[Bauh](https://github.com/vinifmor/bauh)** — Covers most of what Offerings provides already.
- **[KDE Discover](https://apps.kde.org/discover/)** — Does much the same as Offerings.
- **[Autonomix](https://github.com/SgtApple/autonomix)** — Recently archived but a great concept, written in Rust.

> [!TIP]
> If Offerings isn't for you (it was designed for a single user along with a distro for the same single user), I'm sure one of the above will more than satisfy what you're looking for — and probably much better and more completely. Check them out! This was a hobby project and labor of love, and practice as I transition from pentesting to development.


## ⚠️ Known Limitations

### Packages Installed Outside of Offerings

Offerings **cannot uninstall or manage packages that were installed before Offerings, or installed via another tool** (e.g., `apt install`, COSMIC Store, or downloaded `.deb` files). If you try to remove such a package from the "Installed" tab, you may see an error like *"application cannot be removed"* even though it disappears from the list temporarily.

**Workaround:** Use COSMIC Store (pre-installed on Lilith Linux) or the terminal to manage packages installed outside Offerings:

```bash
sudo apt remove <package-name>
# or
flatpak uninstall <app-id>
```

### Installing `.deb` or `.AppImage` Files Directly

Offerings does not support installing local `.deb` or `.AppImage` files via the GUI. For that, use **COSMIC Store** (pre-installed on Lilith Linux), which supports opening local packages.

### App Applets and DE Integration

COSMIC panel applets are not currently listed in Offerings. Use COSMIC Store or install them manually — this is a planned future addition.

## 🏗️ Project Structure

| Path | Description |
|------|-------------|
| `src/` | Core application logic in Rust |
| `ui/` | User interface definitions using Slint |
| `assets/` | Icons, metadata catalogs, and desktop integration files |
| `docs/` | Technical documentation and deployment guides |

## 📄 License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request or open an issue for bug reports and feature requests.

---

<div align="center">

**Built by [BlancoBAM](https://github.com/BlancoBAM) for Lilith Linux, and Katie**

</div>
