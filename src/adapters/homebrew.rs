// src/adapters/homebrew.rs - Homebrew Cask Adapter
use super::{
    command_exists, emit_progress, run_command, start_staged_progress, PackageAdapter,
    ProgressCallback,
};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;

/// Homebrew adapter focusing on casks (GUI apps)
pub struct HomebrewAdapter;

impl HomebrewAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse `brew list --cask --versions` output
    fn parse_installed_line(line: &str) -> Option<(String, Option<String>)> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        let name = parts[0].to_string();
        let version = parts.get(1).map(|v| v.to_string());
        Some((name, version))
    }
}

impl Default for HomebrewAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for HomebrewAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Homebrew
    }

    async fn is_available(&self) -> bool {
        if !command_exists("brew").await {
            return false;
        }

        let prefix = match run_command("brew", &["--prefix"]).await {
            Ok(prefix) => prefix,
            Err(_) => return false,
        };

        std::path::Path::new(prefix.trim())
            .join("Caskroom")
            .is_dir()
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // `brew search --casks` returns cask names, one per line
        let output = run_command("brew", &["search", "--casks"]).await?;
        let mut packages = Vec::new();

        for line in output.lines() {
            let name = line.trim().to_string();
            if name.is_empty() || name.contains("==>") {
                continue;
            }

            packages.push(Package {
                identity: PackageIdentity {
                    id: format!("brew:{}", name),
                    name: name.clone(),
                    source: PackageSource::Homebrew,
                },
                metadata: PackageMetadata {
                    summary: format!("Homebrew cask: {}", name),
                    description: format!("Install {} via Homebrew", name),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: None,
                    latest: None,
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            });
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("brew", &["list", "--cask", "--versions"]).await?;
        let mut packages = Vec::new();

        for line in output.lines() {
            if let Some((name, version)) = Self::parse_installed_line(line.trim()) {
                packages.push(Package {
                    identity: PackageIdentity {
                        id: format!("brew:{}", name),
                        name: name.clone(),
                        source: PackageSource::Homebrew,
                    },
                    metadata: PackageMetadata {
                        summary: format!("Homebrew cask: {}", name),
                        description: format!("Installed via Homebrew: {}", name),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
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
        let name = id.strip_prefix("brew:").unwrap_or(id);
        let output = run_command("brew", &["info", "--cask", name]).await;

        match output {
            Ok(info) => {
                let first_line = info.lines().next().unwrap_or(name);
                let version = first_line.split_whitespace().nth(1).map(|v| v.to_string());

                Ok(Some(Package {
                    identity: PackageIdentity {
                        id: id.to_string(),
                        name: name.to_string(),
                        source: PackageSource::Homebrew,
                    },
                    metadata: PackageMetadata {
                        summary: info.lines().nth(1).unwrap_or("").to_string(),
                        description: info.lines().take(5).collect::<Vec<_>>().join("\n"),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: version,
                    },
                    is_installed: false,
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("brew", &["outdated", "--cask", "--greedy"]).await?;
        let mut updates = Vec::new();

        for line in output.lines() {
            let name = line.split_whitespace().next().unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }
            updates.push(Package {
                identity: PackageIdentity {
                    id: format!("brew:{}", name),
                    name: name.to_string(),
                    source: PackageSource::Homebrew,
                },
                metadata: PackageMetadata::default(),
                version: PackageVersion::default(),
                is_installed: true,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            });
        }

        Ok(updates)
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let name = package_id.strip_prefix("brew:").unwrap_or(package_id);
        match run_command("brew", &["install", "--cask", name]).await {
            Ok(msg) => Ok(OperationResult {
                success: true,
                message: format!(
                    "Installed {} via Homebrew: {}",
                    name,
                    msg.lines().last().unwrap_or("")
                ),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to install {}: {}", name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.05,
            0.9,
            0.08,
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
        let name = package_id.strip_prefix("brew:").unwrap_or(package_id);
        match run_command("brew", &["upgrade", "--cask", name]).await {
            Ok(msg) => Ok(OperationResult {
                success: true,
                message: format!("Updated {}: {}", name, msg.lines().last().unwrap_or("")),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to update {}: {}", name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.08,
            0.92,
            0.07,
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
        let name = package_id.strip_prefix("brew:").unwrap_or(package_id);
        match run_command("brew", &["uninstall", "--cask", name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Removed {}", name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to remove {}: {}", name, e),
                updated_packages: vec![],
            }),
        }
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
            0.09,
            std::time::Duration::from_millis(800),
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
        // Homebrew casks are self-contained
        Ok(vec![])
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        run_command("brew", &["update"]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_installed_line() {
        let (name, version) = HomebrewAdapter::parse_installed_line("firefox 120.0").unwrap();
        assert_eq!(name, "firefox");
        assert_eq!(version, Some("120.0".to_string()));

        let (name, version) = HomebrewAdapter::parse_installed_line("visual-studio-code").unwrap();
        assert_eq!(name, "visual-studio-code");
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn test_homebrew_adapter_creation() {
        let adapter = HomebrewAdapter::new();
        assert_eq!(adapter.source(), PackageSource::Homebrew);
    }
}
