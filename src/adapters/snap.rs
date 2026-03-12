// src/adapters/snap.rs - Snap Package Manager Adapter
use super::{command_exists, run_command, run_sudo_command, PackageAdapter};
use crate::model::{OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion};
use async_trait::async_trait;
use std::error::Error;

/// Snap package manager adapter
pub struct SnapAdapter;

impl SnapAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse snap list output
    fn parse_list_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();
        let mut first_line = true;

        for line in output.lines() {
            // Skip header line
            if first_line {
                first_line = false;
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0];
                let version = parts[1];
                let _rev = parts[2];
                let _tracking = if parts.len() > 3 { parts[3] } else { "stable" };

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("snap:{}", name),
                        name: name.to_string(),
                        source: PackageSource::Snap,
                    },
                    metadata: PackageMetadata {
                        summary: String::new(),
                        description: String::new(),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: Some(format!("https://snapcraft.io/{}", name)),
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: Some(version.to_string()),
                        latest: Some(version.to_string()),
                    },
                    is_installed: true,
                    alternatives: vec![],
                };

                packages.push(pkg);
            }
        }

        packages
    }

    /// Parse snap find output
    fn parse_find_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();
        let mut first_line = true;

        for line in output.lines() {
            // Skip header line
            if first_line {
                first_line = false;
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0];
                let version = parts[1];
                let _publisher = parts[2];
                // Rest is the summary
                let summary = parts[3..].join(" ");

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("snap:{}", name),
                        name: name.to_string(),
                        source: PackageSource::Snap,
                    },
                    metadata: PackageMetadata {
                        summary: summary.clone(),
                        description: summary,
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: Some(format!("https://snapcraft.io/{}", name)),
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: Some(version.to_string()),
                    },
                    is_installed: false,
                    alternatives: vec![],
                };

                packages.push(pkg);
            }
        }

        packages
    }

    /// Parse snap info output
    fn parse_info_output(&self, output: &str) -> Option<Package> {
        let mut name = String::new();
        let mut summary = String::new();
        let mut description = String::new();
        let mut version = String::new();
        let mut _publisher = String::new();
        let mut is_installed = false;

        let mut in_description = false;

        for line in output.lines() {
            let _line_lower = line.to_lowercase();
            
            if line.starts_with("name:") {
                name = line.trim_start_matches("name:").trim().to_string();
                in_description = false;
            } else if line.starts_with("summary:") {
                summary = line.trim_start_matches("summary:").trim().to_string();
                in_description = false;
            } else if line.starts_with("publisher:") {
                _publisher = line.trim_start_matches("publisher:").trim().to_string();
                in_description = false;
            } else if line.starts_with("description:") {
                in_description = true;
            } else if line.starts_with("installed:") {
                is_installed = true;
                version = line.trim_start_matches("installed:").trim()
                    .split_whitespace().next().unwrap_or("").to_string();
                in_description = false;
            } else if line.starts_with("snap-id:") || line.starts_with("tracking:") {
                in_description = false;
            } else if in_description && line.starts_with("  ") {
                description.push_str(line.trim());
                description.push('\n');
            } else if line.starts_with("channels:") || line.starts_with("refresh-date:") {
                in_description = false;
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(Package {
            identity: PackageIdentity {
                id: format!("snap:{}", name),
                name: name.clone(),
                source: PackageSource::Snap,
            },
            metadata: PackageMetadata {
                summary: summary.clone(),
                description: if description.is_empty() { summary } else { description },
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: Some(format!("https://snapcraft.io/{}", name)),
                categories: vec!["Application".to_string()],
                rating: None,
            },
            version: PackageVersion {
                installed: if is_installed { Some(version.clone()) } else { None },
                latest: Some(version),
            },
            is_installed,
            alternatives: vec![],
        })
    }
}

impl Default for SnapAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for SnapAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Snap
    }

    async fn is_available(&self) -> bool {
        command_exists("snap").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Snap doesn't have a way to list all available packages efficiently
        // Return featured/popular snaps instead
        let output = run_command("snap", &["find", "--section=featured"][..]).await?;
        Ok(self.parse_find_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("snap", &["list"][..]).await?;
        Ok(self.parse_list_output(&output))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let snap_name = id.strip_prefix("snap:").unwrap_or(id);
        
        let output = run_command("snap", &["info", snap_name][..]).await;
        
        match output {
            Ok(output) => Ok(self.parse_info_output(&output)),
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("snap", &["refresh", "--list"][..]).await;
        
        match output {
            Ok(output) => {
                let mut packages = Vec::new();
                for line in output.lines().skip(1) { // Skip header
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if !parts.is_empty() {
                        let name = parts[0];
                        packages.push(Package {
                            identity: PackageIdentity {
                                id: format!("snap:{}", name),
                                name: name.to_string(),
                                source: PackageSource::Snap,
                            },
                            metadata: PackageMetadata::default(),
                            version: PackageVersion::default(),
                            is_installed: true,
                            alternatives: vec![],
                        });
                    }
                }
                Ok(packages)
            }
            Err(_) => Ok(vec![]), // No updates available
        }
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);
        
        match run_sudo_command("snap", &["install", snap_name][..]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully installed {}", snap_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to install {}: {}", snap_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);
        
        match run_sudo_command("snap", &["refresh", snap_name][..]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully updated {}", snap_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to update {}: {}", snap_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);
        
        match run_sudo_command("snap", &["remove", snap_name][..]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully removed {}", snap_name),
                updated_packages: vec![package_id.to_string()],
            }),
            Err(e) => Ok(OperationResult {
                success: false,
                message: format!("Failed to remove {}: {}", snap_name, e),
                updated_packages: vec![],
            }),
        }
    }

    async fn get_dependencies(&self, _package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // Snaps are self-contained, they don't have traditional dependencies
        Ok(vec![])
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Snap automatically syncs with the store
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_snap_adapter_creation() {
        let adapter = SnapAdapter::new();
        assert_eq!(adapter.source(), PackageSource::Snap);
    }
}
