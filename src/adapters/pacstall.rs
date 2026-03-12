// src/adapters/pacstall.rs - Backend adapter for Pacstall
use super::{command_exists, run_command, run_sudo_command, PackageAdapter};
use crate::model::{OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion};
use async_trait::async_trait;
use std::error::Error;

pub struct PacstallAdapter;

impl PacstallAdapter {
    pub fn new() -> Self {
        Self
    }

    fn parse_pacstall_list(&self, output: &str, is_installed: bool) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split(':').collect();
            if parts.is_empty() {
                continue;
            }

            let name = parts[0].trim().to_string();
            if name.is_empty() {
                continue;
            }

            let version = if parts.len() > 1 {
                Some(parts[1].trim().to_string())
            } else {
                None
            };

            let pkg = Package {
                identity: PackageIdentity {
                    id: format!("pacstall:{}", name),
                    name: name.clone(),
                    source: PackageSource::Pacstall,
                },
                metadata: PackageMetadata {
                    summary: format!("Pacstall package: {}", name),
                    description: format!("Package from Pacstall repository: {}", name),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: if is_installed { version.clone() } else { None },
                    latest: version.or(Some("latest".to_string())),
                },
                is_installed,
                alternatives: vec![],
            };

            packages.push(pkg);
        }

        packages
    }

    async fn get_package_info(&self, name: &str) -> Option<Package> {
        let output = match run_command("pacstall", &["-D", name]).await {
            Ok(o) => o,
            Err(_) => return None,
        };

        let mut pkg_name = name.to_string();
        let mut version = None;
        let mut description = String::new();

        for line in output.lines() {
            let line = line.trim();
            if line.starts_with("Package:") {
                pkg_name = line.trim_start_matches("Package:").trim().to_string();
            } else if line.starts_with("Version:") {
                version = Some(line.trim_start_matches("Version:").trim().to_string());
            } else if line.starts_with("Description:") {
                description = line.trim_start_matches("Description:").trim().to_string();
            }
        }

        Some(Package {
            identity: PackageIdentity {
                id: format!("pacstall:{}", pkg_name),
                name: pkg_name,
                source: PackageSource::Pacstall,
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
                installed: None,
                latest: version,
            },
            is_installed: false,
            alternatives: vec![],
        })
    }
}

impl Default for PacstallAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for PacstallAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Pacstall
    }

    async fn is_available(&self) -> bool {
        command_exists("pacstall").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("pacstall", &["-L"]).await?;
        Ok(self.parse_pacstall_list(&output, false))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command("pacstall", &["-Q"]).await?;
        Ok(self.parse_pacstall_list(&output, true))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let name = id.strip_prefix("pacstall:").unwrap_or(id);
        
        let mut pkg = match self.get_package_info(name).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let installed_output = run_command("pacstall", &["-Q", name]).await;
        if installed_output.is_ok() {
            pkg.is_installed = true;
            if let Ok(output) = run_command("pacstall", &["-S", name]).await {
                for line in output.lines() {
                    if line.trim().starts_with("Version:") {
                        pkg.version.installed = Some(line.trim_start_matches("Version:").trim().to_string());
                        break;
                    }
                }
            }
        }

        Ok(Some(pkg))
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let installed = self.list_installed().await?;
        let mut updates = Vec::new();

        for pkg in installed {
            if let Some(available) = self.get_package_info(&pkg.identity.name).await {
                if let (Some(installed_ver), Some(available_ver)) = (&pkg.version.installed, &available.version.latest) {
                    if installed_ver != available_ver {
                        let mut update_pkg = pkg;
                        update_pkg.version.latest = Some(available_ver.clone());
                        updates.push(update_pkg);
                    }
                }
            }
        }

        Ok(updates)
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let name = package_id.strip_prefix("pacstall:").unwrap_or(package_id);
        
        let output = run_sudo_command("pacstall", &["-I", name]).await
            .map_err(|e| format!("Failed to install package: {}", e))?;

        Ok(OperationResult {
            success: true,
            message: format!("Package {} installed successfully", name),
            updated_packages: vec![],
        })
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let name = package_id.strip_prefix("pacstall:").unwrap_or(package_id);
        
        let output = run_sudo_command("pacstall", &["-U", name]).await
            .map_err(|e| format!("Failed to update package: {}", e))?;

        Ok(OperationResult {
            success: true,
            message: format!("Package {} updated successfully", name),
            updated_packages: vec![],
        })
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let name = package_id.strip_prefix("pacstall:").unwrap_or(package_id);
        
        let output = run_sudo_command("pacstall", &["-R", name]).await
            .map_err(|e| format!("Failed to remove package: {}", e))?;

        Ok(OperationResult {
            success: true,
            message: format!("Package {} removed successfully", name),
            updated_packages: vec![],
        })
    }

    async fn get_dependencies(&self, _package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        Ok(vec![])
    }
}
