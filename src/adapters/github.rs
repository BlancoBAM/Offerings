// src/adapters/github.rs - GitHub Releases Adapter
use super::PackageAdapter;
use crate::model::{
    DependencyInfo, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use tokio::fs;

/// GitHub Releases package adapter
/// Manages applications installed from GitHub releases
pub struct GitHubAdapter {
    /// Configuration file path
    config_path: PathBuf,
    /// Installation directory
    install_dir: PathBuf,
    /// Tracked repositories (owner/repo -> installed info)
    tracked_repos: HashMap<String, TrackedRepo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackedRepo {
    owner: String,
    repo: String,
    installed_version: String,
    asset_pattern: Option<String>,
    installed_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    content_type: String,
}

impl GitHubAdapter {
    pub fn new() -> Self {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("offerings");

        Self {
            config_path: data_dir.join("github_repos.json"),
            install_dir: data_dir.join("github"),
            tracked_repos: HashMap::new(),
        }
    }

    /// Load tracked repos from config file
    async fn load_config(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path).await?;
            self.tracked_repos = serde_json::from_str(&content)?;
        }
        Ok(())
    }

    /// Save tracked repos to config file
    async fn save_config(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        fs::create_dir_all(self.config_path.parent().unwrap()).await?;
        let content = serde_json::to_string_pretty(&self.tracked_repos)?;
        fs::write(&self.config_path, content).await?;
        Ok(())
    }

    /// Get latest release from GitHub API
    async fn get_latest_release(&self, owner: &str, repo: &str) -> Result<GitHubRelease, Box<dyn Error + Send + Sync>> {
        let url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, repo);
        
        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("User-Agent", "offerings-package-manager")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("GitHub API error: {}", response.status()).into());
        }

        let release: GitHubRelease = response.json().await?;
        Ok(release)
    }

    /// Find the best asset for the current platform
    fn find_best_asset<'a>(&self, assets: &'a [GitHubAsset], pattern: Option<&str>) -> Option<&'a GitHubAsset> {
        let arch = std::env::consts::ARCH;
        let os = std::env::consts::OS;

        // Platform-specific patterns
        let arch_patterns: Vec<&str> = match arch {
            "x86_64" => vec!["x86_64", "amd64", "x64"],
            "aarch64" => vec!["aarch64", "arm64"],
            _ => vec![arch],
        };

        let os_patterns: Vec<&str> = match os {
            "linux" => vec!["linux", "Linux"],
            "macos" => vec!["darwin", "macos", "Darwin"],
            "windows" => vec!["windows", "win64", "Windows"],
            _ => vec![os],
        };

        // Priority: explicit pattern > platform match > any AppImage/binary
        if let Some(pattern) = pattern {
            if let Some(asset) = assets.iter().find(|a| a.name.contains(pattern)) {
                return Some(asset);
            }
        }

        // Try to find platform-specific asset
        for asset in assets {
            let name_lower = asset.name.to_lowercase();
            
            let os_match = os_patterns.iter().any(|p| name_lower.contains(&p.to_lowercase()));
            let arch_match = arch_patterns.iter().any(|p| name_lower.contains(&p.to_lowercase()));

            if os_match && arch_match {
                // Prefer AppImage or tar.gz on Linux
                if os == "linux" {
                    if name_lower.ends_with(".appimage") || name_lower.ends_with(".tar.gz") {
                        return Some(asset);
                    }
                }
                return Some(asset);
            }
        }

        // Fallback: any AppImage
        assets.iter().find(|a| a.name.to_lowercase().ends_with(".appimage"))
    }

    /// Download and install an asset
    async fn download_and_install(
        &self,
        asset: &GitHubAsset,
        _owner: &str,
        _repo: &str,
    ) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
        fs::create_dir_all(&self.install_dir).await?;

        let install_path = self.install_dir.join(&asset.name);

        let client = reqwest::Client::new();
        let response = client
            .get(&asset.browser_download_url)
            .header("User-Agent", "offerings-package-manager")
            .send()
            .await?;

        let bytes = response.bytes().await?;
        fs::write(&install_path, &bytes).await?;

        // Make executable if it's a binary
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if asset.name.ends_with(".AppImage") || !asset.name.contains('.') {
                let mut perms = fs::metadata(&install_path).await?.permissions();
                perms.set_mode(perms.mode() | 0o755);
                fs::set_permissions(&install_path, perms).await?;
            }
        }

        Ok(install_path)
    }

    fn parse_repo_id(id: &str) -> Option<(String, String)> {
        let id = id.strip_prefix("github:").unwrap_or(id);
        let parts: Vec<&str> = id.split('/').collect();
        if parts.len() >= 2 {
            Some((parts[0].to_string(), parts[1].to_string()))
        } else {
            None
        }
    }
}

impl Default for GitHubAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for GitHubAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::GitHubRelease
    }

    async fn is_available(&self) -> bool {
        // GitHub adapter is always available (needs network)
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // GitHub doesn't have a browsable list
        // Users need to add repos explicitly
        Ok(vec![])
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut adapter = self.clone();
        adapter.load_config().await?;

        let packages: Vec<Package> = adapter
            .tracked_repos
            .iter()
            .map(|(key, tracked)| Package {
                identity: PackageIdentity {
                    id: format!("github:{}", key),
                    name: tracked.repo.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: format!("{}/{}", tracked.owner, tracked.repo),
                    description: format!("Installed from GitHub: {}/{}", tracked.owner, tracked.repo),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: Some(format!("https://github.com/{}/{}", tracked.owner, tracked.repo)),
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: Some(tracked.installed_version.clone()),
                    latest: None,
                },
                dependency_info: DependencyInfo::default(),
                is_installed: true,
            })
            .collect();

        Ok(packages)
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let (owner, repo) = match Self::parse_repo_id(id) {
            Some(parts) => parts,
            None => return Ok(None),
        };

        match self.get_latest_release(&owner, &repo).await {
            Ok(release) => Ok(Some(Package {
                identity: PackageIdentity {
                    id: format!("github:{}/{}", owner, repo),
                    name: repo.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: release.name.unwrap_or_else(|| format!("{}/{}", owner, repo)),
                    description: release.body.unwrap_or_default(),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: Some(release.html_url),
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: None,
                    latest: Some(release.tag_name),
                },
                dependency_info: DependencyInfo::default(),
                is_installed: false,
            })),
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut adapter = self.clone();
        adapter.load_config().await?;

        let mut updates = Vec::new();

        for (key, tracked) in &adapter.tracked_repos {
            if let Ok(release) = adapter.get_latest_release(&tracked.owner, &tracked.repo).await {
                let latest_version = release.tag_name.trim_start_matches('v');
                let installed_version = tracked.installed_version.trim_start_matches('v');

                if latest_version != installed_version {
                    updates.push(Package {
                        identity: PackageIdentity {
                            id: format!("github:{}", key),
                            name: tracked.repo.clone(),
                            source: PackageSource::GitHubRelease,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion {
                            installed: Some(tracked.installed_version.clone()),
                            latest: Some(release.tag_name),
                        },
                        dependency_info: DependencyInfo::default(),
                        is_installed: true,
                    });
                }
            }
        }

        Ok(updates)
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let (owner, repo) = match Self::parse_repo_id(package_id) {
            Some(parts) => parts,
            None => return Ok(OperationResult {
                success: false,
                message: "Invalid package ID format. Use: github:owner/repo".to_string(),
                updated_packages: vec![],
            }),
        };

        let release = self.get_latest_release(&owner, &repo).await?;
        
        let asset = match self.find_best_asset(&release.assets, None) {
            Some(a) => a,
            None => return Ok(OperationResult {
                success: false,
                message: "No suitable asset found for this platform".to_string(),
                updated_packages: vec![],
            }),
        };

        let install_path = self.download_and_install(asset, &owner, &repo).await?;

        // Track the installation
        let mut adapter = self.clone();
        adapter.load_config().await?;
        
        adapter.tracked_repos.insert(
            format!("{}/{}", owner, repo),
            TrackedRepo {
                owner: owner.clone(),
                repo: repo.clone(),
                installed_version: release.tag_name.clone(),
                asset_pattern: None,
                installed_path: install_path,
            },
        );
        
        adapter.save_config().await?;

        Ok(OperationResult {
            success: true,
            message: format!("Installed {}/{} version {}", owner, repo, release.tag_name),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // Update is essentially reinstall
        self.install(package_id).await
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let (owner, repo) = match Self::parse_repo_id(package_id) {
            Some(parts) => parts,
            None => return Ok(OperationResult {
                success: false,
                message: "Invalid package ID".to_string(),
                updated_packages: vec![],
            }),
        };

        let key = format!("{}/{}", owner, repo);
        
        let mut adapter = self.clone();
        adapter.load_config().await?;

        if let Some(tracked) = adapter.tracked_repos.remove(&key) {
            // Remove the installed file
            if tracked.installed_path.exists() {
                fs::remove_file(&tracked.installed_path).await?;
            }
            
            adapter.save_config().await?;

            Ok(OperationResult {
                success: true,
                message: format!("Removed {}/{}", owner, repo),
                updated_packages: vec![package_id.to_string()],
            })
        } else {
            Ok(OperationResult {
                success: false,
                message: "Package not installed".to_string(),
                updated_packages: vec![],
            })
        }
    }

    async fn get_dependencies(&self, _package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // GitHub releases are typically self-contained
        Ok(vec![])
    }
}

impl Clone for GitHubAdapter {
    fn clone(&self) -> Self {
        Self {
            config_path: self.config_path.clone(),
            install_dir: self.install_dir.clone(),
            tracked_repos: self.tracked_repos.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_github_adapter_creation() {
        let adapter = GitHubAdapter::new();
        assert_eq!(adapter.source(), PackageSource::GitHubRelease);
    }

    #[test]
    fn test_parse_repo_id() {
        let (owner, repo) = GitHubAdapter::parse_repo_id("github:owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");

        let (owner, repo) = GitHubAdapter::parse_repo_id("owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }
}
