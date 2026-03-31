// src/model.rs - Canonical Data Model
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Package source types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PackageSource {
    Flatpak,
    AppImage,
    Soar,
    Snap,
    Homebrew,
    GitHubRelease,
    OfferingsCustom,
    OfferingsLilith,
}

impl PackageSource {
    pub fn from_id(id: &str) -> Self {
        if id.starts_with("flatpak:") {
            Self::Flatpak
        } else if id.starts_with("appimage:") {
            Self::AppImage
        } else if id.starts_with("soar:") {
            Self::Soar
        } else if id.starts_with("snap:") {
            Self::Snap
        } else if id.starts_with("homebrew:") || id.starts_with("brew:") {
            Self::Homebrew
        } else if id.starts_with("github:") {
            Self::GitHubRelease
        } else if id.starts_with("lilith:") {
            Self::OfferingsLilith
        } else {
            Self::OfferingsCustom
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::AppImage => "AM / AppImage",
            Self::Soar => "SOAR / PkgForge",
            Self::Snap => "Snap",
            Self::Homebrew => "Homebrew",
            Self::GitHubRelease => "GitHub Release",
            Self::OfferingsCustom => "Custom",
            Self::OfferingsLilith => "Lilith",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Flatpak => "application-x-flatpak",
            Self::AppImage => "application-x-executable",
            Self::Soar => "package-x-generic",
            Self::Snap => "snap-symbolic",
            Self::Homebrew => "package-x-generic",
            Self::GitHubRelease => "system-software-install",
            Self::OfferingsCustom => "emblem-package",
            Self::OfferingsLilith => "emblem-favorite",
        }
    }

    /// Get all available sources
    pub fn all() -> Vec<Self> {
        vec![
            Self::Flatpak,
            Self::AppImage,
            Self::Soar,
            Self::Snap,
            Self::Homebrew,
            Self::GitHubRelease,
            Self::OfferingsCustom,
            Self::OfferingsLilith,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub id: String,   // Unique ID across all sources (source:name format)
    pub name: String, // Display name
    pub source: PackageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageMetadata {
    pub summary: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub screenshots: Vec<String>,
    pub documentation_url: Option<String>,
    pub homepage_url: Option<String>,
    pub categories: Vec<String>, // freedesktop.org categories
    pub rating: Option<f32>,     // Display only, never affects sorting
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageVersion {
    pub installed: Option<String>,
    pub latest: Option<String>,
}

impl PackageVersion {
    pub fn has_update(&self) -> bool {
        match (&self.installed, &self.latest) {
            (Some(installed), Some(latest)) => installed != latest,
            _ => false,
        }
    }

    pub fn display_installed(&self) -> String {
        self.installed
            .clone()
            .unwrap_or_else(|| "Not installed".to_string())
    }

    pub fn display_latest(&self) -> String {
        self.latest.clone().unwrap_or_else(|| "Unknown".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub identity: PackageIdentity,
    pub metadata: PackageMetadata,
    pub version: PackageVersion,
    pub is_installed: bool,
    pub logical_app_id: Option<String>,
    pub alternatives: Vec<PackageIdentity>,
    pub last_updated: i64, // Unix timestamp
    pub popularity: f32,   // Normalized value (e.g. 0.0 - 1.0)
}

impl Package {
    pub fn is_app(&self) -> bool {
        // Consider it an app if it has categories OR if it's installed OR if it has a meaningful name
        !self.metadata.categories.is_empty()
            || self.is_installed
            || (!self.identity.name.is_empty()
                && !self.identity.name.contains(".Platform")
                && !self.identity.name.contains(".Sdk"))
    }

    /// Get the short ID without source prefix
    pub fn short_id(&self) -> &str {
        self.identity
            .id
            .split(':')
            .nth(1)
            .unwrap_or(&self.identity.id)
    }

    /// Wave 15.0: Check if an app is "stale" (unmaintained)
    pub fn is_stale(&self) -> bool {
        if self.last_updated == 0 {
            return false;
        } // Unknown is not necessarily stale

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let age_days = (now - self.last_updated) as f32 / (24.0 * 3600.0);

        // Stale if > 2 years old AND low popularity
        age_days > 730.0 && self.popularity < 0.2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceItem {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomePageContent {
    pub featured_apps: Vec<String>,                       // Package IDs
    pub category_showcases: HashMap<String, Vec<String>>, // Category -> Package IDs
}

impl Default for HomePageContent {
    fn default() -> Self {
        let mut category_showcases = HashMap::new();
        category_showcases.insert("Graphics".to_string(), vec![]);
        category_showcases.insert("Development".to_string(), vec![]);
        category_showcases.insert("Audio".to_string(), vec![]);
        category_showcases.insert("Video".to_string(), vec![]);

        Self {
            featured_apps: vec![],
            category_showcases,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageOperation {
    Install(String),
    Update(String),
    Uninstall(String),
    UpdateAll,
}

impl PackageOperation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Install(_) => "install",
            Self::Update(_) => "update",
            Self::Uninstall(_) => "uninstall",
            Self::UpdateAll => "update_all",
        }
    }

    pub fn package_id(&self) -> Option<&str> {
        match self {
            Self::Install(id) | Self::Update(id) | Self::Uninstall(id) => Some(id),
            Self::UpdateAll => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
    pub updated_packages: Vec<String>,
}

// ==================== Extended Types for Production ====================

/// Detailed app information for the detail page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDetailInfo {
    pub package: Package,
    pub changelog: Option<String>,
    pub release_notes: Option<String>,
    pub permissions: Vec<PackagePermission>,
    pub size_installed: Option<u64>,
    pub size_download: Option<u64>,
    pub license: Option<String>,
    pub publisher: Option<String>,
    pub verified: bool,
    pub alternatives: Vec<PackageIdentity>,
}

impl From<Package> for AppDetailInfo {
    fn from(package: Package) -> Self {
        Self {
            package,
            changelog: None,
            release_notes: None,
            permissions: vec![],
            size_installed: None,
            size_download: None,
            license: None,
            publisher: None,
            verified: false,
            alternatives: vec![],
        }
    }
}

/// Package permissions (primarily for Flatpak/Snap)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagePermission {
    pub name: String,
    pub description: String,
    pub level: PermissionLevel,
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionLevel {
    Safe,      // Normal/expected permissions
    Moderate,  // Worth noting
    Dangerous, // Security-sensitive
}

/// Transaction log entry for rollback support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLog {
    pub id: i64,
    pub operation: String,
    pub package_id: String,
    pub package_source: String,
    pub previous_state: Option<String>,
    pub new_state: Option<String>,
    pub status: TransactionStatus,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransactionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    RolledBack,
}

/// Operation progress for UI updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationProgress {
    pub operation: PackageOperation,
    pub percent: f32,
    pub message: String,
    pub stage: ProgressStage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressStage {
    Preparing,
    Downloading,
    Installing,
    Configuring,
    Cleaning,
    Complete,
    Failed,
}

/// Category definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
}

impl Category {
    /// Get standard freedesktop.org categories
    pub fn standard_categories() -> Vec<Self> {
        vec![
            Self {
                id: "AudioVideo".to_string(),
                name: "Audio & Video".to_string(),
                icon: "multimedia".to_string(),
                description: "Media players and editors".to_string(),
            },
            Self {
                id: "Development".to_string(),
                name: "Development".to_string(),
                icon: "applications-development".to_string(),
                description: "IDEs, compilers, and tools".to_string(),
            },
            Self {
                id: "Education".to_string(),
                name: "Education".to_string(),
                icon: "applications-education".to_string(),
                description: "Learning and teaching tools".to_string(),
            },
            Self {
                id: "Game".to_string(),
                name: "Games".to_string(),
                icon: "applications-games".to_string(),
                description: "Games and entertainment".to_string(),
            },
            Self {
                id: "Graphics".to_string(),
                name: "Graphics".to_string(),
                icon: "applications-graphics".to_string(),
                description: "Image and design tools".to_string(),
            },
            Self {
                id: "Network".to_string(),
                name: "Internet".to_string(),
                icon: "applications-internet".to_string(),
                description: "Browsers, email, and networking".to_string(),
            },
            Self {
                id: "Office".to_string(),
                name: "Office".to_string(),
                icon: "applications-office".to_string(),
                description: "Productivity and office suites".to_string(),
            },
            Self {
                id: "Science".to_string(),
                name: "Science".to_string(),
                icon: "applications-science".to_string(),
                description: "Scientific and math tools".to_string(),
            },
            Self {
                id: "Settings".to_string(),
                name: "Settings".to_string(),
                icon: "preferences-system".to_string(),
                description: "System configuration".to_string(),
            },
            Self {
                id: "System".to_string(),
                name: "System".to_string(),
                icon: "applications-system".to_string(),
                description: "System utilities".to_string(),
            },
            Self {
                id: "Utility".to_string(),
                name: "Utilities".to_string(),
                icon: "applications-utilities".to_string(),
                description: "General purpose tools".to_string(),
            },
        ]
    }
}

/// Application state for UI
#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub current_view: ViewState,
    pub selected_package: Option<String>,
    pub search_query: String,
    pub active_operations: Vec<OperationProgress>,
    pub notification_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ViewState {
    #[default]
    Home,
    Categories,
    Category(String),
    Installed,
    Dependencies,
    Updates,
    Search,
    PackageDetail(String),
    Settings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_source_label() {
        assert_eq!(PackageSource::Flatpak.label(), "Flatpak");
    }

    #[test]
    fn test_package_version_has_update() {
        let version = PackageVersion {
            installed: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
        };
        assert!(version.has_update());

        let version = PackageVersion {
            installed: Some("1.0.0".to_string()),
            latest: Some("1.0.0".to_string()),
        };
        assert!(!version.has_update());
    }
}
