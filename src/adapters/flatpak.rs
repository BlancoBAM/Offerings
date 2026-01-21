// src/adapters/flatpak.rs - Flatpak Package Manager Adapter
use super::{command_exists, run_command, PackageAdapter};
use crate::model::{
    DependencyInfo, InstallReason, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;

/// Flatpak package manager adapter
pub struct FlatpakAdapter {
    /// Default remote (usually flathub)
    remote: String,
}

impl FlatpakAdapter {
    pub fn new() -> Self {
        Self {
            remote: "flathub".to_string(),
        }
    }

    pub fn with_remote(remote: String) -> Self {
        Self { remote }
    }

    /// Parse flatpak list output
    fn parse_list_output(&self, output: &str, is_installed: bool) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let name = parts[0].trim();
                let app_id = parts[1].trim();
                let version = parts[2].trim();

                // Skip runtimes unless they have a useful name
                if app_id.contains(".Platform") || app_id.contains(".Sdk") {
                    continue;
                }

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("flatpak:{}", app_id),
                        name: name.to_string(),
                        source: PackageSource::Flatpak,
                    },
                    metadata: PackageMetadata {
                        summary: String::new(),
                        description: String::new(),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: if is_installed { Some(version.to_string()) } else { None },
                        latest: Some(version.to_string()),
                    },
                    dependency_info: DependencyInfo {
                        dependencies: vec![],
                        reverse_dependencies: vec![],
                        install_reason: InstallReason::Explicit,
                    },
                    is_installed,
                };

                packages.push(pkg);
            }
        }

        packages
    }

    /// Parse flatpak info output for detailed package info
    fn parse_info_output(&self, output: &str) -> Option<Package> {
        let mut name = String::new();
        let mut app_id = String::new();
        let mut version = String::new();
        let mut description = String::new();

        for line in output.lines() {
            let line = line.trim();
            if line.starts_with("Name:") {
                name = line.trim_start_matches("Name:").trim().to_string();
            } else if line.starts_with("ID:") || line.starts_with("Application ID:") {
                app_id = line.split(':').nth(1).unwrap_or("").trim().to_string();
            } else if line.starts_with("Version:") {
                version = line.trim_start_matches("Version:").trim().to_string();
            } else if line.starts_with("Description:") {
                description = line.trim_start_matches("Description:").trim().to_string();
            }
        }

        if app_id.is_empty() {
            return None;
        }

        Some(Package {
            identity: PackageIdentity {
                id: format!("flatpak:{}", app_id),
                name: if name.is_empty() { app_id.clone() } else { name },
                source: PackageSource::Flatpak,
            },
            metadata: PackageMetadata {
                summary: description.clone(),
                description,
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                categories: vec!["Application".to_string()],
                rating: None,
            },
            version: PackageVersion {
                installed: Some(version.clone()),
                latest: Some(version),
            },
            dependency_info: DependencyInfo::default(),
            is_installed: true,
        })
    }

    /// Parse flatpak remote-ls output for available packages
    fn parse_remote_ls_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 1 {
                let app_id = parts[0].trim();
                
                // Skip runtimes
                if app_id.contains(".Platform") || app_id.contains(".Sdk") || app_id.contains(".Locale") {
                    continue;
                }

                let name = app_id.split('.').last().unwrap_or(app_id);

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("flatpak:{}", app_id),
                        name: name.to_string(),
                        source: PackageSource::Flatpak,
                    },
                    metadata: PackageMetadata {
                        summary: String::new(),
                        description: String::new(),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: None,
                    },
                    dependency_info: DependencyInfo::default(),
                    is_installed: false,
                };

                packages.push(pkg);
            }
        }

        packages
    }
}

impl Default for FlatpakAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for FlatpakAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Flatpak
    }

    async fn is_available(&self) -> bool {
        command_exists("flatpak").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("flatpak", &["remote-ls", "--app", &self.remote]).await?;
        Ok(self.parse_remote_ls_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("flatpak", &["list", "--app", "--columns=name,application,version"]).await?;
        Ok(self.parse_list_output(&output, true))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let app_id = id.strip_prefix("flatpak:").unwrap_or(id);
        
        let output = run_command("flatpak", &["info", app_id]).await;
        
        match output {
            Ok(output) => Ok(self.parse_info_output(&output)),
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("flatpak", &["remote-ls", "--app", "--updates", &self.remote]).await?;
        
        let mut packages = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if !parts.is_empty() {
                let app_id = parts[0].trim();
                if !app_id.is_empty() {
                    packages.push(Package {
                        identity: PackageIdentity {
                            id: format!("flatpak:{}", app_id),
                            name: app_id.split('.').last().unwrap_or(app_id).to_string(),
                            source: PackageSource::Flatpak,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion::default(),
                        dependency_info: DependencyInfo::default(),
                        is_installed: true,
                    });
                }
            }
        }

        Ok(packages)
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        
        match run_command("flatpak", &["install", "-y", &self.remote, app_id]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully installed {}", app_id),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to install {}: {}", app_id, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        
        match run_command("flatpak", &["update", "-y", app_id]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully updated {}", app_id),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to update {}: {}", app_id, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        
        match run_command("flatpak", &["uninstall", "-y", app_id]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully removed {}", app_id),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to remove {}: {}", app_id, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn get_dependencies(&self, package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        
        // Flatpak shows runtime dependencies in info
        let output = run_command("flatpak", &["info", "--show-runtime", app_id]).await?;
        
        let mut deps = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with("Runtime:") {
                deps.push(format!("flatpak:{}", line));
            }
        }
        
        Ok(deps)
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Flatpak automatically updates remote metadata
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flatpak_adapter_creation() {
        let adapter = FlatpakAdapter::new();
        assert_eq!(adapter.source(), PackageSource::Flatpak);
    }
}
