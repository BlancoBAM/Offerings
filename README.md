# Offerings

**Offerings** is a modern, unified package manager GUI for Linux, designed to simplify application management across multiple package formats. Built with **Rust** and **Slint**, it provides a high-performance, aesthetically pleasing, and responsive interface for discovering, installing, and managing software.

## 🚀 Features

*   **Unified Package Management**: Seamlessly manage packages from multiple sources in one interface:
    *   📦 **APT** (Debian/Ubuntu native packages)
    *   🥡 **Flatpak**
    *   🍱 **Snap**
    *   💿 **AppImage**
    *   🦅 **Soar**
    *   🐙 **GitHub Releases**
    *   🛠️ **Custom Offerings Packages**
*   **Modern User Interface**: A sleek, "Dark Black" themed UI built with Slint.
    *   Sidebar navigation with categorized views (Audio, Video, Development, etc.).
    *   Responsive and fluid animations.
    *   Optimized for readability with alternating row colors.
*   **Advanced Dependency Management**:
    *   Dedicated **Dependencies View** to explore installed dependencies.
    *   **Dependency Graph**: Visualise and trace dependency trees for any installed package.
    *   Orphan detection and circular dependency analysis.
*   **Safe Transactions**: 
    *   Transactional package operations with full rollback support.
    *   Queue management for batch operations.
*   **Accessibility First**:
    *   Adjustable font sizes and contrast settings.
    *   System font integration.
*   **Performance**: 
    *   Fast SQLite-based caching for instant search results.
    *   Asynchronous backend powered by Tokio.

## 🛠️ Installation

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

## 📖 Usage

### Navigation
The sidebar allows you to filter applications by category. The **Featured** section highlights popular apps, while various categories organize your software library effectively.

### Installing a Package
1.  Use the **Search Bar** at the top to find a package by name or description.
2.  Click "Install" on the package card.
3.  Monitor progress via system notifications.

### Managing Dependencies
Navigate to the **Dependencies** tab to view all installed artifacts. Click on any item to view:
*   Installation date.
*   Full dependency tree.
*   Reverse dependencies (what apps rely on this package).

### Settings
Access the **App Settings** from the sidebar to customize:
*   Font Family (System defaults).
*   Font Size (10px - 24px).
*   Colors & Contrast.

## 🏗️ Architecture

Offerings is built using a clean, modular architecture:

*   **Frontend**: [Slint UI](https://slint.dev) (`ui/main.slint`) - A lightweight, native UI toolkit.
*   **Backend**: Rust (`src/backend.rs`) - Handles business logic, state, and orchestration.
*   **Adapters**: (`src/adapters/`) - Modular traits for connecting to different package managers (APT, Flatpak, etc.).
*   **Database**: SQLite (`src/db.rs`) - Caches package metadata and logs transactions.
*   **IPC**: Unix domain sockets for external control and single-instance locking.

## 🤝 Contributing

Contributions are welcome!

1.  Fork the repository.
2.  Create a feature branch: `git checkout -b feature/amazing-feature`.
3.  Commit your changes: `git commit -m 'Add some amazing feature'`.
4.  Push to the branch: `git push origin feature/amazing-feature`.
5.  Open a Pull Request.

## 📄 License

This project is open-source and available under the [MIT License](LICENSE).
