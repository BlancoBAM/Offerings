// src/adapters/github.rs - GitHub Release Adapter
use super::{emit_progress, start_staged_progress, PackageAdapter, ProgressCallback};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;
use tokio::fs;

/// A tracked GitHub repo for release-based installation
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GitHubAppEntry {
    /// GitHub owner/repo, e.g. "sharkdp/bat"
    pub repo: String,
    /// Human-readable display name
    pub name: String,
    /// One-line description
    pub description: String,
    /// Binary name to look for after extraction
    pub binary_name: String,
    /// Asset filename pattern to match (supports simple glob: `*` matches anything)
    pub asset_pattern: String,
    /// Categories for store display
    pub categories: Vec<String>,
}

/// GitHub Release adapter
/// Manages a curated list of GitHub repos and installs releases as local binaries
pub struct GitHubReleaseAdapter {
    /// Tracked repos loaded from manifest
    entries: Vec<GitHubAppEntry>,
    /// Where to install binaries
    install_dir: PathBuf,
    /// HTTP client
    client: reqwest::Client,
}

impl GitHubReleaseAdapter {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let manifest_path = home.join(".config/offerings/github_apps.json");

        let entries = std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<GitHubAppEntry>>(&s).ok())
            .unwrap_or_else(Self::default_entries);

        Self {
            entries,
            install_dir: home.join(".local/bin"),
            client: reqwest::Client::builder()
                .user_agent("Offerings/0.1")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Default curated entries when no manifest exists
    fn default_entries() -> Vec<GitHubAppEntry> {
        vec![
            GitHubAppEntry {
                repo: "sharkdp/bat".into(),
                name: "bat".into(),
                description: "A cat clone with syntax highlighting and Git integration".into(),
                binary_name: "bat".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
            GitHubAppEntry {
                repo: "sharkdp/fd".into(),
                name: "fd".into(),
                description: "A simple, fast and user-friendly alternative to find".into(),
                binary_name: "fd".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
            GitHubAppEntry {
                repo: "BurntSushi/ripgrep".into(),
                name: "ripgrep".into(),
                description: "Fast line-oriented search tool (rg)".into(),
                binary_name: "rg".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "Development".to_string()],
            },
            GitHubAppEntry {
                repo: "junegunn/fzf".into(),
                name: "fzf".into(),
                description: "A command-line fuzzy finder".into(),
                binary_name: "fzf".into(),
                asset_pattern: "*linux_amd64*".into(),
                categories: vec!["Utility".to_string()],
            },
            GitHubAppEntry {
                repo: "eza-community/eza".into(),
                name: "eza".into(),
                description: "A modern replacement for ls".into(),
                binary_name: "eza".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
        ]
    }

    /// Match an asset filename against a simple glob pattern
    fn matches_pattern(filename: &str, pattern: &str) -> bool {
        let parts: Vec<&str> = pattern.split('*').collect();
        let lower = filename.to_lowercase();
        let mut pos = 0;
        for part in &parts {
            if part.is_empty() {
                continue;
            }
            match lower[pos..].find(&part.to_lowercase()) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
        true
    }

    /// Query the GitHub API for the latest release of a repo
    async fn get_latest_release(
        &self,
        repo: &str,
    ) -> Result<(String, Vec<(String, String)>), Box<dyn Error + Send + Sync>> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        let tag = resp["tag_name"].as_str().unwrap_or("unknown").to_string();
        let assets: Vec<(String, String)> = resp["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|asset| {
                        let name = asset["name"].as_str()?.to_string();
                        let url = asset["browser_download_url"].as_str()?.to_string();
                        Some((name, url))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok((tag, assets))
    }

    /// Read installed version from a .version file next to the binary
    async fn read_installed_version(&self, binary_name: &str) -> Option<String> {
        let version_file = self.install_dir.join(format!(".{}.version", binary_name));
        fs::read_to_string(&version_file)
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Write installed version marker
    async fn write_installed_version(
        &self,
        binary_name: &str,
        version: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let version_file = self.install_dir.join(format!(".{}.version", binary_name));
        fs::write(&version_file, version).await?;
        Ok(())
    }
}

impl Default for GitHubReleaseAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for GitHubReleaseAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::GitHubRelease
    }

    async fn is_available(&self) -> bool {
        // Always available — uses HTTP, no CLI dependency
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        for entry in &self.entries {
            let installed_version = self.read_installed_version(&entry.binary_name).await;
            let is_installed = installed_version.is_some();

            packages.push(Package {
                identity: PackageIdentity {
                    id: format!("github:{}", entry.repo),
                    name: entry.name.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: entry.description.clone(),
                    description: format!(
                        "{}\n\nSource: https://github.com/{}",
                        entry.description, entry.repo
                    ),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                    homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                    categories: entry.categories.clone(),
                    rating: None,
                },
                version: PackageVersion {
                    installed: installed_version,
                    latest: None, // Filled by check_updates
                },
                is_installed,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            });
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        for entry in &self.entries {
            let binary_path = self.install_dir.join(&entry.binary_name);
            if binary_path.exists() {
                let version = self.read_installed_version(&entry.binary_name).await;
                packages.push(Package {
                    identity: PackageIdentity {
                        id: format!("github:{}", entry.repo),
                        name: entry.name.clone(),
                        source: PackageSource::GitHubRelease,
                    },
                    metadata: PackageMetadata {
                        summary: entry.description.clone(),
                        description: format!("Installed from GitHub: {}", entry.repo),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                        homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                        categories: entry.categories.clone(),
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: version.clone(),
                        latest: version,
                    },
                    is_installed: true,
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
                });
            }
        }

        Ok(packages)
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let repo = id.strip_prefix("github:").unwrap_or(id);
        if let Some(entry) = self.entries.iter().find(|e| e.repo == repo) {
            let version = self.read_installed_version(&entry.binary_name).await;
            let is_installed = version.is_some();

            Ok(Some(Package {
                identity: PackageIdentity {
                    id: id.to_string(),
                    name: entry.name.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: entry.description.clone(),
                    description: format!(
                        "{}\n\nSource: https://github.com/{}",
                        entry.description, entry.repo
                    ),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                    homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                    categories: entry.categories.clone(),
                    rating: None,
                },
                version: PackageVersion {
                    installed: version,
                    latest: None,
                },
                is_installed,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut updates = Vec::new();

        for entry in &self.entries {
            let installed = match self.read_installed_version(&entry.binary_name).await {
                Some(v) => v,
                None => continue, // Not installed, skip
            };

            if let Ok((latest_tag, _)) = self.get_latest_release(&entry.repo).await {
                // Strip leading 'v' for comparison
                let installed_clean = installed.strip_prefix('v').unwrap_or(&installed);
                let latest_clean = latest_tag.strip_prefix('v').unwrap_or(&latest_tag);

                if installed_clean != latest_clean {
                    updates.push(Package {
                        identity: PackageIdentity {
                            id: format!("github:{}", entry.repo),
                            name: entry.name.clone(),
                            source: PackageSource::GitHubRelease,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion {
                            installed: Some(installed),
                            latest: Some(latest_tag),
                        },
                        is_installed: true,
                        logical_app_id: None,
                        alternatives: vec![],
                        last_updated: 0,
                        popularity: 0.0,
                    });
                }
            }
        }

        Ok(updates)
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let repo = package_id.strip_prefix("github:").unwrap_or(package_id);
        let entry = self
            .entries
            .iter()
            .find(|e| e.repo == repo)
            .ok_or("Unknown GitHub app")?;

        // Get latest release
        let (tag, assets) = self.get_latest_release(repo).await?;

        // Find matching asset
        let (asset_name, download_url) = assets
            .iter()
            .find(|(name, _)| Self::matches_pattern(name, &entry.asset_pattern))
            .ok_or_else(|| {
                format!(
                    "No matching asset found for pattern: {}",
                    entry.asset_pattern
                )
            })?;

        // Download to temp
        let tmp_dir = std::env::temp_dir().join(format!("offerings-gh-{}", entry.binary_name));
        fs::create_dir_all(&tmp_dir).await?;
        let tmp_file = tmp_dir.join(asset_name);

        let resp = self.client.get(download_url).send().await?;
        let bytes = resp.bytes().await?;
        fs::write(&tmp_file, &bytes).await?;

        // Ensure install dir exists
        fs::create_dir_all(&self.install_dir).await?;
        let dest = self.install_dir.join(&entry.binary_name);

        // Handle tarball vs direct binary
        if asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz") {
            // Extract using tar
            let status = tokio::process::Command::new("tar")
                .args([
                    "xzf",
                    &tmp_file.to_string_lossy(),
                    "-C",
                    &tmp_dir.to_string_lossy(),
                ])
                .status()
                .await?;
            if !status.success() {
                return Ok(OperationResult {
                    success: false,
                    message: "Failed to extract tarball".into(),
                    updated_packages: vec![],
                });
            }
            // Find the binary in extracted files
            let found = find_binary_recursive(&tmp_dir, &entry.binary_name).await;
            if let Some(bin_path) = found {
                fs::copy(&bin_path, &dest).await?;
            } else {
                return Ok(OperationResult {
                    success: false,
                    message: format!("Binary '{}' not found in archive", entry.binary_name),
                    updated_packages: vec![],
                });
            }
        } else if asset_name.ends_with(".zip") {
            // For zip files, try unzip
            let status = tokio::process::Command::new("unzip")
                .args([
                    "-o",
                    &tmp_file.to_string_lossy().to_string(),
                    "-d",
                    &tmp_dir.to_string_lossy().to_string(),
                ])
                .status()
                .await?;
            if !status.success() {
                return Ok(OperationResult {
                    success: false,
                    message: "Failed to extract zip".into(),
                    updated_packages: vec![],
                });
            }
            let found = find_binary_recursive(&tmp_dir, &entry.binary_name).await;
            if let Some(bin_path) = found {
                fs::copy(&bin_path, &dest).await?;
            } else {
                return Ok(OperationResult {
                    success: false,
                    message: format!("Binary '{}' not found in archive", entry.binary_name),
                    updated_packages: vec![],
                });
            }
        } else {
            // Direct binary
            fs::copy(&tmp_file, &dest).await?;
        }

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest).await?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            fs::set_permissions(&dest, perms).await?;
        }

        // Write version marker
        self.write_installed_version(&entry.binary_name, &tag)
            .await?;

        // Cleanup temp
        let _ = fs::remove_dir_all(&tmp_dir).await;

        Ok(OperationResult {
            success: true,
            message: format!(
                "Installed {} {} to {}",
                entry.name,
                tag,
                self.install_dir.display()
            ),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.05,
            0.95,
            0.06,
            std::time::Duration::from_millis(900),
        );
        let result = self.install(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn update(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // Update is the same as install — it downloads the latest release
        self.install(package_id).await
    }

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.08,
            0.95,
            0.06,
            std::time::Duration::from_millis(850),
        );
        let result = self.update(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn uninstall(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let repo = package_id.strip_prefix("github:").unwrap_or(package_id);
        let entry = self
            .entries
            .iter()
            .find(|e| e.repo == repo)
            .ok_or("Unknown GitHub app")?;

        let binary_path = self.install_dir.join(&entry.binary_name);
        let version_path = self
            .install_dir
            .join(format!(".{}.version", entry.binary_name));

        if binary_path.exists() {
            fs::remove_file(&binary_path).await?;
        }
        if version_path.exists() {
            fs::remove_file(&version_path).await?;
        }

        Ok(OperationResult {
            success: true,
            message: format!("Removed {}", entry.name),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.1,
            0.9,
            0.08,
            std::time::Duration::from_millis(700),
        );
        let result = self.uninstall(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn get_dependencies(
        &self,
        _package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // GitHub release binaries are self-contained
        Ok(vec![])
    }
}

/// Recursively search for a binary in a directory
async fn find_binary_recursive(dir: &std::path::Path, binary_name: &str) -> Option<PathBuf> {
    let mut entries = fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = Box::pin(find_binary_recursive(&path, binary_name)).await {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern() {
        assert!(GitHubReleaseAdapter::matches_pattern(
            "bat-v0.24.0-x86_64-unknown-linux-musl.tar.gz",
            "*x86_64*linux*musl*"
        ));
        assert!(!GitHubReleaseAdapter::matches_pattern(
            "bat-v0.24.0-x86_64-apple-darwin.tar.gz",
            "*x86_64*linux*musl*"
        ));
        assert!(GitHubReleaseAdapter::matches_pattern(
            "fzf-0.46.0-linux_amd64.tar.gz",
            "*linux_amd64*"
        ));
    }

    #[tokio::test]
    async fn test_github_adapter_creation() {
        let adapter = GitHubReleaseAdapter::new();
        assert_eq!(adapter.source(), PackageSource::GitHubRelease);
    }
}
