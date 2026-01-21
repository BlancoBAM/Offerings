// src/adapters/apt.rs - APT Package Manager Adapter
use super::{command_exists, run_command, run_sudo_command, PackageAdapter};
use crate::model::{
    DependencyInfo, InstallReason, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
// use std::collections::HashMap;
use std::error::Error;

/// APT package manager adapter for Debian/Ubuntu systems
pub struct AptAdapter {
    cache_path: String,
}

impl AptAdapter {
    pub fn new() -> Self {
        Self {
            cache_path: "/var/cache/apt".to_string(),
        }
    }

    pub fn with_cache_path(cache_path: String) -> Self {
        Self { cache_path }
    }

    /// Parse dpkg-query output into packages
    fn parse_dpkg_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                let name = parts[0].trim();
                let version = parts[1].trim();
                let arch = parts[2].trim();
                let description = parts[3].trim();

                // Skip architecture-specific duplicates
                if arch != "all" && arch != std::env::consts::ARCH {
                    continue;
                }

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("apt:{}", name),
                        name: name.to_string(),
                        source: PackageSource::APT,
                    },
                    metadata: PackageMetadata {
                        summary: description.to_string(),
                        description: description.to_string(),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: self.guess_categories(name, description),
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: Some(version.to_string()),
                        latest: Some(version.to_string()),
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

    /// Parse apt-cache show output for a single package
    fn parse_apt_cache_show(&self, output: &str) -> Option<Package> {
        let mut name = String::new();
        let mut version = String::new();
        let mut summary = String::new();
        let mut description = String::new();
        let mut homepage = None;
        let mut deps = Vec::new();

        let mut in_description = false;

        for line in output.lines() {
            if line.starts_with("Package:") {
                name = line.trim_start_matches("Package:").trim().to_string();
                in_description = false;
            } else if line.starts_with("Version:") {
                version = line.trim_start_matches("Version:").trim().to_string();
            } else if line.starts_with("Description:") {
                summary = line.trim_start_matches("Description:").trim().to_string();
                in_description = true;
            } else if line.starts_with("Homepage:") {
                homepage = Some(line.trim_start_matches("Homepage:").trim().to_string());
            } else if line.starts_with("Depends:") {
                let deps_str = line.trim_start_matches("Depends:").trim();
                deps = self.parse_depends_line(deps_str);
            } else if in_description && line.starts_with(' ') {
                description.push_str(line.trim());
                description.push('\n');
            } else if !line.starts_with(' ') && !line.is_empty() {
                in_description = false;
            }
        }

        if name.is_empty() {
            return None;
        }

        let categories = self.guess_categories(&name, &summary);
        
        Some(Package {
            identity: PackageIdentity {
                id: format!("apt:{}", name),
                name: name.clone(),
                source: PackageSource::APT,
            },
            metadata: PackageMetadata {
                summary: summary.clone(),
                description: if description.is_empty() { summary } else { description },
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: homepage,
                categories,
                rating: None,
            },
            version: PackageVersion {
                installed: None,
                latest: Some(version),
            },
            dependency_info: DependencyInfo {
                dependencies: deps,
                reverse_dependencies: vec![],
                install_reason: InstallReason::Explicit,
            },
            is_installed: false,
        })
    }

    /// Parse a Depends line into package names
    fn parse_depends_line(&self, line: &str) -> Vec<String> {
        line.split(',')
            .map(|dep| {
                // Handle alternatives (pkg1 | pkg2) - take the first one
                let dep = dep.split('|').next().unwrap_or(dep);
                // Remove version constraints
                dep.split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .map(|s| format!("apt:{}", s))
            .collect()
    }

    /// Parse apt list --upgradable output
    fn parse_upgradable(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            // Format: package/release version architecture [upgradable from: old_version]
            if let Some(idx) = line.find('/') {
                let name = &line[..idx];
                
                // Extract versions
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let new_version = parts[1].to_string();
                    let old_version = if let Some(from_idx) = line.find("from:") {
                        line[from_idx + 6..].trim_end_matches(']').trim().to_string()
                    } else {
                        new_version.clone()
                    };

                    let pkg = Package {
                        identity: PackageIdentity {
                            id: format!("apt:{}", name),
                            name: name.to_string(),
                            source: PackageSource::APT,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion {
                            installed: Some(old_version),
                            latest: Some(new_version),
                        },
                        dependency_info: DependencyInfo::default(),
                        is_installed: true,
                    };

                    packages.push(pkg);
                }
            }
        }

        packages
    }

    /// Guess categories based on package name and description
    fn guess_categories(&self, name: &str, description: &str) -> Vec<String> {
        let combined = format!("{} {}", name, description).to_lowercase();
        let mut categories = Vec::new();

        if combined.contains("editor") || combined.contains("vim") || combined.contains("emacs") {
            categories.push("Development".to_string());
        }
        if combined.contains("game") || combined.contains("gaming") {
            categories.push("Game".to_string());
        }
        if combined.contains("audio") || combined.contains("music") || combined.contains("sound") {
            categories.push("Audio".to_string());
        }
        if combined.contains("video") || combined.contains("player") {
            categories.push("Video".to_string());
        }
        if combined.contains("graphic") || combined.contains("image") || combined.contains("photo") {
            categories.push("Graphics".to_string());
        }
        if combined.contains("network") || combined.contains("internet") || combined.contains("browser") {
            categories.push("Network".to_string());
        }
        if combined.contains("office") || combined.contains("document") {
            categories.push("Office".to_string());
        }
        if combined.contains("system") || combined.contains("utility") {
            categories.push("System".to_string());
        }

        if categories.is_empty() {
            categories.push("Other".to_string());
        }

        categories
    }
}

impl Default for AptAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for AptAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::APT
    }

    async fn is_available(&self) -> bool {
        command_exists("apt-cache").await && command_exists("dpkg").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Get list of all packages (names only for performance)
        let output = run_command("apt-cache", &["pkgnames"]).await?;
        
        let mut packages = Vec::new();
        for name in output.lines().take(1000) { // Limit to first 1000 for performance
            let name = name.trim();
            if !name.is_empty() {
                packages.push(Package {
                    identity: PackageIdentity {
                        id: format!("apt:{}", name),
                        name: name.to_string(),
                        source: PackageSource::APT,
                    },
                    metadata: PackageMetadata::default(),
                    version: PackageVersion::default(),
                    dependency_info: DependencyInfo::default(),
                    is_installed: false,
                });
            }
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command(
            "dpkg-query",
            &["-W", "-f=${Package}\t${Version}\t${Architecture}\t${binary:Summary}\n"],
        )
        .await?;

        Ok(self.parse_dpkg_output(&output))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let pkg_name = id.strip_prefix("apt:").unwrap_or(id);
        
        let output = run_command("apt-cache", &["show", pkg_name]).await;
        
        match output {
            Ok(output) => {
                let mut pkg = self.parse_apt_cache_show(&output);
                
                // Check if installed
                if let Some(ref mut pkg) = pkg {
                    let status = run_command("dpkg-query", &["-W", "-f=${Status}", pkg_name]).await;
                    if let Ok(status) = status {
                        pkg.is_installed = status.contains("installed");
                        if pkg.is_installed {
                            let version = run_command("dpkg-query", &["-W", "-f=${Version}", pkg_name]).await;
                            if let Ok(v) = version {
                                pkg.version.installed = Some(v.trim().to_string());
                            }
                        }
                    }
                }
                
                Ok(pkg)
            }
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // First update the package lists
        let _ = run_sudo_command("apt-get", &["update", "-qq"]).await;
        
        // Then get upgradable packages
        let output = run_command("apt", &["list", "--upgradable"]).await?;
        
        Ok(self.parse_upgradable(&output))
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_name = package_id.strip_prefix("apt:").unwrap_or(package_id);
        
        match run_sudo_command("apt-get", &["install", "-y", pkg_name]).await {
            Ok(output) => Ok(OperationResult {
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
        let pkg_name = package_id.strip_prefix("apt:").unwrap_or(package_id);
        
        match run_sudo_command("apt-get", &["install", "--only-upgrade", "-y", pkg_name]).await {
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
        let pkg_name = package_id.strip_prefix("apt:").unwrap_or(package_id);
        
        match run_sudo_command("apt-get", &["remove", "-y", pkg_name]).await {
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

    async fn get_dependencies(&self, package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let pkg_name = package_id.strip_prefix("apt:").unwrap_or(package_id);
        
        let output = run_command("apt-cache", &["depends", "--installed", pkg_name]).await?;
        
        let mut deps = Vec::new();
        for line in output.lines() {
            if line.trim_start().starts_with("Depends:") {
                let dep = line.trim_start().trim_start_matches("Depends:").trim();
                if !dep.is_empty() && !dep.starts_with('<') {
                    deps.push(format!("apt:{}", dep));
                }
            }
        }
        
        Ok(deps)
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        run_sudo_command("apt-get", &["update", "-qq"]).await?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_apt_adapter_creation() {
        let adapter = AptAdapter::new();
        assert_eq!(adapter.source(), PackageSource::APT);
    }

    #[test]
    fn test_parse_depends_line() {
        let adapter = AptAdapter::new();
        let deps = adapter.parse_depends_line("libc6 (>= 2.17), libgcc1 | libgcc-s1");
        assert!(deps.contains(&"apt:libc6".to_string()));
        assert!(deps.contains(&"apt:libgcc1".to_string()));
    }

    #[test]
    fn test_guess_categories() {
        let adapter = AptAdapter::new();
        let cats = adapter.guess_categories("firefox", "web browser");
        assert!(cats.contains(&"Network".to_string()));
    }
}
