# Offerings

**Offerings** is a modern, unified package manager GUI for Linux, designed to simplify application management across multiple package formats. Built with **Rust** and **Slint**, it provides a high-performance, aesthetically pleasing, and responsive interface for discovering, installing, and managing software.

## Features

*   **Unified Package Management**: Seamlessly manage packages from multiple sources in one interface:
    *   **Flatpak** - Sandboxed applications from Flathub
    *   **AppImage** - Portable applications via AM (AppImage Manager)
    *   **Pacstall** - Ubuntu/Debian packages from Pacstall
    *   **Snap** - Canonical's Snap packages
    *   **Custom** - Developer-curated packages
*   **Smart Source Selection**: When an app is available from multiple sources, easily switch between them via dropdown menu
*   **Automatic Fallback**: If installation fails from one source, automatically tries alternative sources and notifies you of success
*   **Auto-Updating App List**: Background refresh automatically detects new apps, removed apps, and version updates from all sources
*   **Modern User Interface**: A sleek, "Dark Black" themed UI built with Slint
    *   Sidebar navigation with categorized views
    *   Responsive and fluid animations
    *   Optimized for readability with alternating row colors
*   **Developer Curated**: Featured "Lilith" category for developer-curated applications

## Installation

### Prerequisites

Ensure you have the following installed on your system:
*   **Rust & Cargo**: [Install Rust](https://www.rust-lang.org/tools/install)
*   **System Dependencies** (Debian/Ubuntu):
    ```bash
    sudo apt install build-essential libfontconfig1-dev libxcb-xfixes0-dev libxcb-shape0-dev libxcb1-dev libxkbcommon-dev
    ```
    *Note: Additional dependencies for specific adapters (like Flatpak or Snap) may be required.*

### Building from Source

1.  **Clone the repository**:
    ```bash
    git clone https://github.com/BlancoBAM/Offerings.git
    cd Offerings
    ```

2.  **Build the application**:
    ```bash
    cargo build --release
    ```

3.  **Run**:
    ```bash
    ./target/release/offerings
    ```

## Usage

### Navigation
The sidebar allows you to filter applications by category. The **Featured** section highlights popular apps, while the **Lilith** category showcases developer-curated applications.

### Installing a Package
1.  Use the **Search Bar** at the top to find a package by name or description
2.  Click on a package to view its details
3.  If the package is available from multiple sources, use the **Source** dropdown to select your preferred source
4.  Click "Install" to install the package

### Automatic Fallback
If installation fails from the primary source, Offerings will automatically attempt to install from alternative sources. You'll receive a notification when a fallback installation succeeds.

### Updates
Offerings automatically checks for updates in the background. You can also manually check for updates. The app list refreshes every 30 minutes to detect new apps, removed apps, and version changes.

## Architecture

Offerings is built using a clean, modular architecture:

*   **Frontend**: [Slint UI](https://slint.dev) (`ui/main.slint`) - A lightweight, native UI toolkit
*   **Backend**: Rust (`src/backend.rs`) - Handles business logic, state, and orchestration
*   **Adapters**: (`src/adapters/`) - Modular traits for connecting to different package managers
*   **Database**: SQLite (`src/db.rs`) - Caches package metadata
*   **IPC**: Unix domain sockets for external control and single-instance locking

### Priority System
When duplicates exist across sources, packages are prioritized in this order:
1.  Flatpak (highest)
2.  AppImage
3.  Pacstall
4.  Snap
5.  Custom (lowest)

## License

This project is open-source and available under the [MIT License](LICENSE).
