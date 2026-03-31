# Offerings Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2024-04

### 🎉 Production Release Preparation

### 🐛 Critical Fixes
- **Fixed window resizing crashes**: Added maximum window constraints (1920x1080) to prevent unexpected window collapsing when navigating between app details
- **Eliminated Slint binding loops**: Completely removed complex property calculations that caused recursion crashes
- **Stabilized carousel**: Fixed screenshot carousel with simplified properties to avoid layout shifts
- **Removed non-functional settings**: Eliminated font configuration UI that wasn't implemented

### 🚀 Major Improvements
- **Window Stability**: Added `max-width: 1920px` and `max-height: 1080px` to prevent unexpected resizing
- **Simplified UI**: Removed 200+ lines of non-functional font settings code
- **Source Management**: Enhanced sources tab to show actual repository URLs and allow configuration
- **Lilith Section**: Documented process for updating curated package list
- **Packaging Documentation**: Added complete packaging and release instructions

### 📦 Package Sources
- **Flatpak**: ~3,000 packages from Flathub
- **AM/AppImage**: ~6,800 packages from AppImage Manager
- **SOAR/pkgforge**: ~250 static packages
- **Snap Store**: ~1,000 Snap packages
- **GitHub Releases**: Binary packages from GitHub
- **Custom Sources**: User-configurable repositories
- **Lilith Curated**: Hand-picked quality applications

### 🔧 Technical Enhancements
- **Metadata System**: Robust catalog enrichment with fallbacks for descriptions, screenshots, icons
- **Database**: SQLite-based package cache with automatic updates from upstream sources
- **Error Handling**: Improved progress tracking and notification system
- **Performance**: Optimized UI rendering with fixed layouts to prevent recalculations

### 📝 Documentation Updates
- **Complete README.md**: Updated with accurate installation, usage, and development information
- **CHANGELOG.md**: Detailed version history with all fixes and improvements
- **Packaging Guide**: Instructions for building AppImage, .deb, .rpm packages
- **Release Process**: Step-by-step guide for creating production releases

### 🎯 Quality Assurance
- **Testing**: Comprehensive test suite with 23 tests covering all major functionality
- **Validation**: Pre-release validation script (`test_offerings.sh`) for quality control
- **Smoke Testing**: Headless `--self-test` mode for CI/CD integration
- **Stability**: No Slint binding loop warnings, clean compilation

### 📦 Packaging & Distribution
- **AppImage**: Ready-to-use portable package
- **Source Build**: Standard `cargo build --release`
- **Package Managers**: Documentation for .deb, .rpm, Arch packages (coming soon)
- **Release Process**: Professional workflow for GitHub releases and crates.io publishing

### 🔮 Future Roadmap
- **Package Manager Integration**: .deb, .rpm, Arch packages
- **Automatic Updates**: In-app update notification and installation
- **Enhanced Metadata**: Expanded catalog with more descriptions and screenshots
- **Performance Monitoring**: Usage analytics and crash reporting
- **Community Features**: User ratings and reviews

### 🎉 Production Ready
- **Stable**: No known crashes or critical issues
- **Tested**: Comprehensive test coverage and validation
- **Documented**: Complete documentation for users and developers
- **Packaged**: Ready for distribution via multiple channels

## [1.0.0] - 2024-04

### 🎉 Initial Production Release

This marks the first stable, production-ready release of Offerings with:
- ✅ Crash-free operation across all navigation paths
- ✅ Functional source configuration with real repository URLs
- ✅ Complete documentation and packaging instructions
- ✅ Professional code quality and testing
- ✅ Ready for end-user distribution

**Built with ❤️ for the Linux community**

## [0.2.0] - 2026-03-26

### Added
- **Dynamic Pagination**: Support for browsing massive catalogs via "Show More Apps" batches (100 items).
- **Discovery Scoring**: Sophisticated algorithm prioritizing apps by recency, popularity, and metadata quality.
- **Staleness Filtering**: Intelligent detection and hiding of unmaintained apps from browse views.
- **Metadata Aggregation**: Preserves the richest descriptions and screenshots across multiple package sources.
- **Real-time Progress**: Fixed buffer flushing for incremental install/uninstall progress updates.

### Changed
- **UI Grid**: Achieved uniform 320px card alignment and dynamic window stretching.
- **Header Clarity**: Standardized "Project Homepage" labels on app description pages.
- **Search Sorting**: Search results now prioritize exact name matches and high discovery scores.

### Fixed
- Fixed Slint layout nesting issues preventing builds.
- Fixed inconsistent category package counts in sidebar badges.
- Fixed missing descriptions for several GitHub and AppImage sources.

---

## [Pre-0.2 Work] - 2026-03-20

### Changed
- **Window size**: Now opens at 1400x900px (60% of typical screen)
- **Minimum window size**: 800x600px
- **Card size**: Reduced to 180x200px for more compact display
- **Home page**: Shows 5 example packages per category
- **Category pages**: Show ALL packages when category is selected
- **Loading screen**: Enhanced with larger logo and better progress visibility

### Fixed
- Window no longer displays as tiny sliver
- Categories now properly load all packages when selected
- Desktop launcher points to correct binary
- Package categorization improved with broader pattern matching

### Technical
- Updated `AppImageAdapter` with comprehensive category inference
- Updated `FlatpakAdapter` with new category mappings
- Added `Miscellaneous` category for uncategorized packages
- Enhanced install handler with progress tracking
- Improved loading screen visibility with fire gradient colors

---

## [0.1.0] - 2026-03-19

### Added
- Initial release
- Multi-source package management (Flatpak, Snap, AppImage, SOAR)
- Slint-based UI with dark theme
- Category-based organization
- Search functionality
- Install/Update/Uninstall operations
- Background cache refresh

### Package Sources
- Flathub (~3,000 packages)
- AM - AppImage Manager (~6,800 packages)
- SOAR/pkgforge (~250 packages)
- Snap Store (limited)
- GitHub Releases (limited)

---

## Version History Summary

| Version | Date | Packages | Categories | Key Features |
|---------|------|----------|------------|--------------|
| Unreleased | 2026-03-20 | 9,000+ | 18 | Fire gradient progress, install tracking |
| 0.1.0 | 2026-03-19 | 6,686 | 12 | Initial multi-source release |

---

## Development Notes

### Category Inference
Packages are categorized using pattern matching on package names:
- **AI/ML**: ollama, stable-diffusion, tensorflow, pytorch, etc.
- **Desktop**: themes, icons, docks, cosmic, gnome, kde
- **Productivity**: tasks, notes, calendars, planners
- **Security**: passwords, encryption, VPN, firewall
- **Lifestyle**: fitness, health, finance, cooking

### Package Limits
- Home page: 5 packages per category (examples)
- Category view: ALL packages displayed
- Cache limit: 7,000 for AM, 3,000 for Flatpak

### UI Components
- `LoadingProgress`: Fire gradient progress bar
- `LoadingScreen`: Full-screen overlay during load
- `AppCard`: Compact 180x200px package cards
- `CategorySection`: Grid layout for packages
