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

**Offerings** is a unified GUI interface for easy desktop app management designed for Lilith Linux. It combines multiple package sources into a single, beautiful interface, allowing users to discover, install, and manage applications from Flatpak, Snap, AppImage, SOAR/PkgForge, and Direct Releases (GitHub binaries). Catalogue of over 11,000 desktop apps!

## ✨ Features

- 🗃️ **One Catalogue, Many Sources**: the store serves a single deduplicated
  desktop-app catalogue (Flathub AppStream, AM/AppImage databases,
  pkgforge/soarpkgs GUI formats, spm feed, appbundle) rebuilt automatically by
  `tools/update_catalogue.py` (systemd user timer included). Package managers
  are only used for installing, uninstalling and tracking what's actually on
  your system — never for re-flooding the browse grid.
- 📦 **Multi-Source Support**: Seamless integration with Flatpak, Snap, AppImage (AM), SOAR/pkgforge, Homebrew, and Direct Release (GitHub binaries).
- 🎨 **Modern Interface**: Built with Slint for a fast, responsive, and native-feeling experience.
- 🛡️ **Lilith Curated Section**: Hand-picked applications specifically chosen for Lilith Linux users.
- 🔄 **Automated Updates**: Background synchronization ensures you always have the latest versions and package metadata.
- 🔍 **Powerful Search**: Fuzzy matching across all enabled sources to find exactly what you need.
- 🗂️ **Smart Categorization**: Automated classification of thousands of apps into intuitive categories following the freedesktop Main Categories spec.
- ❤️ **Saved for Later**: Infernal heart toggle on any app saves it to your personal Saved shelf.
- ⚡ **Direct Release**: enter any `owner/repo` or github.com URL and Offerings picks and installs the right release asset for your machine.
- 📥 **Local Packages**: double-click any `.deb`, `.AppImage`, `.flatpak`, or `.snap` file (or run `offerings <file>`) to inspect its metadata in Offerings before installing. MIME associations are registered by `install.sh`.
- 📊 **Honest Progress**: install/uninstall/refresh bars show real percentages — byte counts for downloads, actual task completion for snaps, and real database rows written during refreshes.
- 🪟 **Tiling-Friendly Layout**: the app grid reflows dynamically as the window resizes (cosmic tiling ready).

>[!NOTE]
>Flathub apps (≈3,300) now carry their real AppStream categories. The portable
>(AM/pkgforge/appbundle) rows are still auto-categorized, so the odd misfile
>remains — help is more than welcome!

- ⚡ **Performance First**: Efficient SQLite-backed caching and incremental loading for smooth browsing of massive catalogs.

## 🚀 Installation

### AppImage (Recommended)

Works on any Linux distribution, no install needed:

```bash
# Grab the latest AppImage from the Releases page
# (asset name follows Offerings-<version>-<arch>.AppImage, e.g. Offerings-1.4.0-x86_64.AppImage)
wget https://github.com/BlancoBAM/Offerings/releases/latest/download/Offerings-1.4.0-x86_64.AppImage
chmod +x Offerings-*.AppImage
./Offerings-*.AppImage
```

### Debian/Ubuntu .deb

Pre-built `.deb` packages are also published on the [Releases page](https://github.com/BlancoBAM/Offerings/releases/latest):

```bash
wget https://github.com/BlancoBAM/Offerings/releases/latest/download/offerings_1.4.0-1_amd64.deb
sudo dpkg -i offerings_*.deb
```

### Install Script (recommended for Lilith Linux)

Clones the repo, installs dependencies, builds in release mode, and registers desktop + MIME integration (double-click `.deb`/`.AppImage`/`.flatpak`/`.snap` to open in Offerings):

```bash
curl -fsSL https://raw.githubusercontent.com/BlancoBAM/Offerings/main/install.sh | bash
```

### From Source

```bash
git clone https://github.com/BlancoBAM/Offerings.git
cd Offerings
cargo install --path .
```

> **Note:** `cargo install --git https://github.com/BlancoBAM/Offerings` works too. Releases are produced automatically by CI on version tags (`vX.Y.Z`), publishing the AppImage, `.deb`, and generated release notes.

## 🛠️ Configuration for Developers

### Lilith Curated Section

The Lilith section is populated from a small TOML file of hand-picked URLs, not
the generated metadata catalogue. Offerings looks for `lilith-curated.toml` (or
`offer-cur.toml`) in this order:

1. Next to the binary (AppImage / local install)
2. The current working directory (running from source)
3. `~/.local/share/offerings/lilith-curated.toml`
4. `/usr/share/offerings/lilith-curated.toml` (where the `.deb` ships it)

To curate apps:

1. Open `lilith-curated.toml` and add a URL on its own line under a section
   header (`# ─── Category ───`), with an optional comment after it:
   ```toml
   # ─── Productivity & Note-taking ───
   https://flathub.org/en/apps/io.github.alainm23.planify     # Planify — task management
   ```
2. The source is inferred from the URL pattern: `flathub.org` → Flatpak,
   `snapcraft.io` → Snap, `github.com` → Direct Release, direct file links →
   AppImage.
3. Restart Offerings and the app appears in the Lilith section —
   every curated entry is implicitly tagged `"Lilith"`.

### Package Sources

Offerings supports the following sources out of the box:
- **Flathub** (Flatpak)
- **Snap Store** (Snap)
- **AM / AppImage** (AppImage)
- **PkgForge** (SOAR)
- **Homebrew**
- **Direct Release** (GitHub binaries — enter any repo or URL)

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

### App Applets and DE Integration

COSMIC panel applets are not currently listed in Offerings. Use COSMIC Store or install them manually — this is a planned future addition.

## 🏗️ Project Structure

| Path | Description |
|------|-------------|
| `src/` | Core application logic in Rust |
| `ui/` | User interface definitions using Slint |
| `assets/` | Icons, the generated metadata catalogue, and desktop integration files |
| `tools/` | Catalogue builder/updater scripts and systemd timer units |
| `docs/` | Technical documentation and deployment guides |

## 📄 License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request or open an issue for bug reports and feature requests.

---

<div align="center">

**Built by [BlancoBAM](https://github.com/BlancoBAM) for Lilith Linux, and Katie**

</div>
