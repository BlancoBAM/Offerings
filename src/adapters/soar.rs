// src/adapters/soar.rs - Soar Package Manager Adapter
use super::{command_exists, run_command, PackageAdapter};
use crate::model::{
    DependencyInfo, InstallReason, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;

/// Soar package manager adapter
/// Soar is a modern package manager for downloading and managing portable packages
pub struct SoarAdapter;

impl SoarAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse soar list output
    fn parse_list_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Soar output format: name version
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let name = parts[0];
                let version = parts.get(1).map(|v| v.to_string());

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("soar:{}", name),
                        name: name.to_string(),
                        source: PackageSource::Soar,
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
                        installed: version.clone(),
                        latest: version,
                    },
                    dependency_info: DependencyInfo {
                        dependencies: vec![],
                        reverse_dependencies: vec![],
                        install_reason: InstallReason::Explicit,
                    },
                    is_installed: true,
                };

                packages.push(pkg);
            }
        }

        packages
    }

    /// Parse soar search output
    fn parse_search_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let name = parts[0];
                let description = if parts.len() > 1 {
                    parts[1..].join(" ")
                } else {
                    String::new()
                };

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("soar:{}", name),
                        name: name.to_string(),
                        source: PackageSource::Soar,
                    },
                    metadata: PackageMetadata {
                        summary: description.clone(),
                        description,
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion::default(),
                    dependency_info: DependencyInfo::default(),
                    is_installed: false,
                };

                packages.push(pkg);
            }
        }

        packages
    }
}

impl Default for SoarAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for SoarAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Soar
    }

    async fn is_available(&self) -> bool {
        command_exists("soar").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("soar", &["search", ""]).await?;
        Ok(self.parse_search_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("soar", &["list"]).await?;
        Ok(self.parse_list_output(&output))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let pkg_name = id.strip_prefix("soar:").unwrap_or(id);
        
        let output = run_command("soar", &["info", pkg_name]).await;
        
        match output {
            Ok(output) => {
                if output.is_empty() {
                    return Ok(None);
                }
                
                // Parse info output - format varies
                let mut name = pkg_name.to_string();
                let mut description = String::new();
                let mut version = String::new();

                for line in output.lines() {
                    if line.starts_with("Name:") {
                        name = line.trim_start_matches("Name:").trim().to_string();
                    } else if line.starts_with("Description:") {
                        description = line.trim_start_matches("Description:").trim().to_string();
                    } else if line.starts_with("Version:") {
                        version = line.trim_start_matches("Version:").trim().to_string();
                    }
                }

                Ok(Some(Package {
                    identity: PackageIdentity {
                        id: format!("soar:{}", name),
                        name,
                        source: PackageSource::Soar,
                    },
                    metadata: PackageMetadata {
                        summary: description.clone(),
                        description,
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: Some(version.clone()),
                        latest: Some(version),
                    },
                    dependency_info: DependencyInfo::default(),
                    is_installed: true,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Soar update check
        let output = run_command("soar", &["update", "--check"]).await;
        
        match output {
            Ok(output) => Ok(self.parse_list_output(&output)),
            Err(_) => Ok(vec![]),
        }
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        
        match run_command("soar", &["install", pkg_name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully installed {}", pkg_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to install {}: {}", pkg_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        
        match run_command("soar", &["update", pkg_name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully updated {}", pkg_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to update {}: {}", pkg_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        
        match run_command("soar", &["uninstall", pkg_name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully removed {}", pkg_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to remove {}: {}", pkg_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn get_dependencies(&self, _package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // Soar packages are typically self-contained
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_soar_adapter_creation() {
        let adapter = SoarAdapter::new();
        assert_eq!(adapter.source(), PackageSource::Soar);
    }
}
