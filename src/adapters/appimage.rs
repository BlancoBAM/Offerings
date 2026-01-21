// src/adapters/appimage.rs - AppImage Management Adapter
use super::PackageAdapter;
// use super::command_exists;
use crate::model::{
    DependencyInfo, InstallReason, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;
use tokio::fs;

/// AppImage management adapter
/// Scans standard directories for .AppImage files
pub struct AppImageAdapter {
    /// Directories to scan for AppImages
    scan_dirs: Vec<PathBuf>,
    /// Directory to install AppImages to
    install_dir: PathBuf,
}

impl AppImageAdapter {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        
        Self {
            scan_dirs: vec![
                home.join("Applications"),
                home.join(".local/share/applications"),
                home.join("Downloads"),
                PathBuf::from("/opt"),
                PathBuf::from("/usr/local/bin"),
            ],
            install_dir: home.join("Applications"),
        }
    }

    pub fn with_dirs(scan_dirs: Vec<PathBuf>, install_dir: PathBuf) -> Self {
        Self { scan_dirs, install_dir }
    }

    /// Extract app info from an AppImage filename
    fn parse_appimage_name(&self, filename: &str) -> (String, Option<String>) {
        // Common patterns: AppName-version-arch.AppImage, AppName_version.AppImage
        let name = filename
            .trim_end_matches(".AppImage")
            .trim_end_matches(".appimage");

        // Try to extract version
        let parts: Vec<&str> = name.split(|c| c == '-' || c == '_').collect();
        
        if parts.len() >= 2 {
            // Check if second part looks like a version
            let potential_version = parts[1];
            if potential_version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return (parts[0].to_string(), Some(potential_version.to_string()));
            }
        }

        (name.to_string(), None)
    }

    /// Create desktop entry for an AppImage
    async fn create_desktop_entry(&self, path: &PathBuf, name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let desktop_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("applications");

        fs::create_dir_all(&desktop_dir).await?;

        let desktop_file = desktop_dir.join(format!("{}.desktop", name.to_lowercase().replace(' ', "-")));
        
        let content = format!(
            r#"[Desktop Entry]
Type=Application
Name={}
Exec="{}"
Icon=application-x-executable
Categories=Application;
Terminal=false
"#,
            name,
            path.display()
        );

        fs::write(&desktop_file, content).await?;

        Ok(())
    }

    /// Remove desktop entry for an AppImage
    async fn remove_desktop_entry(&self, name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let desktop_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("applications");

        let desktop_file = desktop_dir.join(format!("{}.desktop", name.to_lowercase().replace(' ', "-")));
        
        if desktop_file.exists() {
            fs::remove_file(&desktop_file).await?;
        }

        Ok(())
    }
}

impl Default for AppImageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for AppImageAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::AppImage
    }

    async fn is_available(&self) -> bool {
        // AppImages are always "available" on Linux
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // AppImages don't have a central repository
        // This could be extended to integrate with AppImageHub
        Ok(vec![])
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        for dir in &self.scan_dirs {
            if !dir.exists() {
                continue;
            }

            let mut entries = match fs::read_dir(dir).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let filename = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                if filename.to_lowercase().ends_with(".appimage") {
                    let (name, version) = self.parse_appimage_name(filename);
                    
                    // Check if executable
                    let metadata = fs::metadata(&path).await;
                    let is_executable = metadata
                        .map(|m| {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                m.permissions().mode() & 0o111 != 0
                            }
                            #[cfg(not(unix))]
                            {
                                true
                            }
                        })
                        .unwrap_or(false);

                    if !is_executable {
                        continue;
                    }

                    let pkg = Package {
                        identity: PackageIdentity {
                            id: format!("appimage:{}", path.display()),
                            name: name.clone(),
                            source: PackageSource::AppImage,
                        },
                        metadata: PackageMetadata {
                            summary: format!("AppImage: {}", filename),
                            description: format!("Local AppImage at {}", path.display()),
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
        }

        Ok(packages)
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let path_str = id.strip_prefix("appimage:").unwrap_or(id);
        let path = PathBuf::from(path_str);
        
        if !path.exists() {
            return Ok(None);
        }

        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let (name, version) = self.parse_appimage_name(filename);

        Ok(Some(Package {
            identity: PackageIdentity {
                id: id.to_string(),
                name,
                source: PackageSource::AppImage,
            },
            metadata: PackageMetadata {
                summary: format!("AppImage: {}", filename),
                description: format!("Local AppImage at {}", path.display()),
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
            dependency_info: DependencyInfo::default(),
            is_installed: true,
        }))
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // AppImage update checking would require zsync or similar
        // For now, return empty list
        Ok(vec![])
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // For AppImages, "install" means:
        // 1. Move to install directory
        // 2. Make executable
        // 3. Create desktop entry
        
        let source_path = PathBuf::from(package_id.strip_prefix("appimage:").unwrap_or(package_id));
        
        if !source_path.exists() {
            return Ok(OperationResult {
                success: false,
                message: "AppImage file not found".to_string(),
                updated_packages: vec![],
            });
        }

        // Ensure install directory exists
        fs::create_dir_all(&self.install_dir).await?;

        let filename = source_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app.AppImage");

        let dest_path = self.install_dir.join(filename);

        // Copy if not already in install dir
        if source_path != dest_path {
            fs::copy(&source_path, &dest_path).await?;
        }

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest_path).await?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            fs::set_permissions(&dest_path, perms).await?;
        }

        // Create desktop entry
        let (name, _) = self.parse_appimage_name(filename);
        self.create_desktop_entry(&dest_path, &name).await?;

        Ok(OperationResult {
            success: true,
            message: format!("Installed {} to {}", filename, self.install_dir.display()),
            updated_packages: vec![format!("appimage:{}", dest_path.display())],
        })
    }

    async fn update(&self, _package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // AppImage updates would require downloading a new version
        Ok(OperationResult {
            success: false,
            message: "AppImage updates not yet implemented".to_string(),
            updated_packages: vec![],
        })
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let path_str = package_id.strip_prefix("appimage:").unwrap_or(package_id);
        let path = PathBuf::from(path_str);
        
        if !path.exists() {
            return Ok(OperationResult {
                success: false,
                message: "AppImage not found".to_string(),
                updated_packages: vec![],
            });
        }

        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let (name, _) = self.parse_appimage_name(filename);

        // Remove desktop entry
        self.remove_desktop_entry(&name).await?;

        // Remove the AppImage file
        fs::remove_file(&path).await?;

        Ok(OperationResult {
            success: true,
            message: format!("Removed {}", filename),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn get_dependencies(&self, _package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // AppImages are self-contained
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_appimage_name() {
        let adapter = AppImageAdapter::new();
        
        let (name, version) = adapter.parse_appimage_name("Firefox-102.0.AppImage");
        assert_eq!(name, "Firefox");
        assert_eq!(version, Some("102.0".to_string()));

        let (name, version) = adapter.parse_appimage_name("MyApp.AppImage");
        assert_eq!(name, "MyApp");
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn test_appimage_adapter_creation() {
        let adapter = AppImageAdapter::new();
        assert_eq!(adapter.source(), PackageSource::AppImage);
    }
}
