// src/adapters/custom.rs - Offerings Custom Repository Adapter
use super::PackageAdapter;
use crate::model::{
    DependencyInfo, InstallReason, OperationResult, Package, PackageIdentity, PackageMetadata,
    PackageSource, PackageVersion,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use tokio::fs;
use tokio::process::Command;

/// Custom Offerings repository adapter
/// Supports custom package definitions with install scripts
pub struct CustomAdapter {
    /// Path to the custom packages directory
    packages_dir: PathBuf,
    /// Remote repository URL (optional)
    remote_url: Option<String>,
    /// Installed packages manifest
    manifest_path: PathBuf,
}

/// Custom package definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPackageDefinition {
    /// Unique package identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Short summary
    pub summary: String,
    /// Full description
    pub description: String,
    /// Current version
    pub version: String,
    /// Homepage URL
    pub homepage: Option<String>,
    /// Icon URL
    pub icon: Option<String>,
    /// Categories
    pub categories: Vec<String>,
    /// Download URL or local path
    pub source: PackageSourceDef,
    /// Installation script (bash)
    pub install_script: Option<String>,
    /// Uninstall script (bash)
    pub uninstall_script: Option<String>,
    /// Dependencies (package IDs)
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageSourceDef {
    Url { url: String },
    Local { path: String },
    Script { script: String },
}

/// Manifest of installed custom packages
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct InstalledManifest {
    packages: HashMap<String, InstalledPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledPackage {
    version: String,
    installed_at: i64,
    install_path: Option<PathBuf>,
}

impl CustomAdapter {
    pub fn new() -> Self {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("offerings");

        Self {
            packages_dir: data_dir.join("custom-packages"),
            remote_url: None,
            manifest_path: data_dir.join("custom-manifest.json"),
        }
    }

    pub fn with_remote(remote_url: String) -> Self {
        let mut adapter = Self::new();
        adapter.remote_url = Some(remote_url);
        adapter
    }

    /// Load the installed manifest
    async fn load_manifest(&self) -> Result<InstalledManifest, Box<dyn Error + Send + Sync>> {
        if self.manifest_path.exists() {
            let content = fs::read_to_string(&self.manifest_path).await?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(InstalledManifest::default())
        }
    }

    /// Save the installed manifest
    async fn save_manifest(&self, manifest: &InstalledManifest) -> Result<(), Box<dyn Error + Send + Sync>> {
        fs::create_dir_all(self.manifest_path.parent().unwrap()).await?;
        let content = serde_json::to_string_pretty(manifest)?;
        fs::write(&self.manifest_path, content).await?;
        Ok(())
    }

    /// Load all package definitions from the packages directory
    async fn load_definitions(&self) -> Result<Vec<CustomPackageDefinition>, Box<dyn Error + Send + Sync>> {
        let mut definitions = Vec::new();

        if !self.packages_dir.exists() {
            return Ok(definitions);
        }

        let mut entries = fs::read_dir(&self.packages_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(def) = serde_json::from_str::<CustomPackageDefinition>(&content) {
                        definitions.push(def);
                    }
                }
            }
        }

        Ok(definitions)
    }

    /// Load a specific package definition
    async fn load_definition(&self, id: &str) -> Result<Option<CustomPackageDefinition>, Box<dyn Error + Send + Sync>> {
        let path = self.packages_dir.join(format!("{}.json", id));
        
        if path.exists() {
            let content = fs::read_to_string(&path).await?;
            Ok(Some(serde_json::from_str(&content)?))
        } else {
            Ok(None)
        }
    }

    /// Execute an installation script
    async fn execute_script(&self, script: &str, pkg_id: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        // Create a temporary script file
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join(format!("offerings-{}.sh", pkg_id));
        
        fs::write(&script_path, script).await?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).await?;
        }

        // Execute
        let output = Command::new("bash")
            .arg(&script_path)
            .output()
            .await?;

        // Clean up
        fs::remove_file(&script_path).await?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(format!(
                "Script failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ).into())
        }
    }

    /// Convert definition to Package
    fn definition_to_package(&self, def: &CustomPackageDefinition, is_installed: bool, installed_version: Option<String>) -> Package {
        Package {
            identity: PackageIdentity {
                id: format!("custom:{}", def.id),
                name: def.name.clone(),
                source: PackageSource::OfferingsCustom,
            },
            metadata: PackageMetadata {
                summary: def.summary.clone(),
                description: def.description.clone(),
                icon_url: def.icon.clone(),
                screenshots: vec![],
                documentation_url: None,
                homepage_url: def.homepage.clone(),
                categories: def.categories.clone(),
                rating: None,
            },
            version: PackageVersion {
                installed: installed_version,
                latest: Some(def.version.clone()),
            },
            dependency_info: DependencyInfo {
                dependencies: def.dependencies.clone(),
                reverse_dependencies: vec![],
                install_reason: InstallReason::Explicit,
            },
            is_installed,
        }
    }

    fn now_timestamp() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }
}

impl Default for CustomAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for CustomAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::OfferingsCustom
    }

    async fn is_available(&self) -> bool {
        // Always available
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let definitions = self.load_definitions().await?;
        let manifest = self.load_manifest().await?;

        Ok(definitions
            .iter()
            .map(|def| {
                let installed = manifest.packages.get(&def.id);
                self.definition_to_package(def, installed.is_some(), installed.map(|i| i.version.clone()))
            })
            .collect())
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let definitions = self.load_definitions().await?;
        let manifest = self.load_manifest().await?;

        let packages: Vec<Package> = manifest
            .packages
            .iter()
            .filter_map(|(id, installed)| {
                definitions
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|def| self.definition_to_package(def, true, Some(installed.version.clone())))
            })
            .collect();

        Ok(packages)
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let pkg_id = id.strip_prefix("custom:").unwrap_or(id);
        
        if let Some(def) = self.load_definition(pkg_id).await? {
            let manifest = self.load_manifest().await?;
            let installed = manifest.packages.get(pkg_id);
            Ok(Some(self.definition_to_package(&def, installed.is_some(), installed.map(|i| i.version.clone()))))
        } else {
            Ok(None)
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let definitions = self.load_definitions().await?;
        let manifest = self.load_manifest().await?;

        let mut updates = Vec::new();

        for def in &definitions {
            if let Some(installed) = manifest.packages.get(&def.id) {
                if installed.version != def.version {
                    updates.push(self.definition_to_package(def, true, Some(installed.version.clone())));
                }
            }
        }

        Ok(updates)
    }

    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_id = package_id.strip_prefix("custom:").unwrap_or(package_id);
        
        let def = match self.load_definition(pkg_id).await? {
            Some(d) => d,
            None => return Ok(OperationResult {
                success: false,
                message: format!("Package definition not found: {}", pkg_id),
                updated_packages: vec![],
            }),
        };

        // Execute install script if present
        if let Some(script) = &def.install_script {
            match self.execute_script(script, pkg_id).await {
                Ok(_) => {}
                Err(e) => return Ok(OperationResult {
                    success: false,
                    message: format!("Installation failed: {}", e),
                    updated_packages: vec![],
                }),
            }
        }

        // Update manifest
        let mut manifest = self.load_manifest().await?;
        manifest.packages.insert(
            pkg_id.to_string(),
            InstalledPackage {
                version: def.version.clone(),
                installed_at: Self::now_timestamp(),
                install_path: None,
            },
        );
        self.save_manifest(&manifest).await?;

        Ok(OperationResult {
            success: true,
            message: format!("Installed {} version {}", def.name, def.version),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // For custom packages, update is typically uninstall + install
        let uninstall_result = self.uninstall(package_id).await?;
        if !uninstall_result.success {
            return Ok(uninstall_result);
        }
        
        self.install(package_id).await
    }

    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let pkg_id = package_id.strip_prefix("custom:").unwrap_or(package_id);
        
        let def = self.load_definition(pkg_id).await?;

        // Execute uninstall script if present
        if let Some(def) = &def {
            if let Some(script) = &def.uninstall_script {
                match self.execute_script(script, pkg_id).await {
                    Ok(_) => {}
                    Err(e) => return Ok(OperationResult {
                        success: false,
                        message: format!("Uninstall failed: {}", e),
                        updated_packages: vec![],
                    }),
                }
            }
        }

        // Update manifest
        let mut manifest = self.load_manifest().await?;
        manifest.packages.remove(pkg_id);
        self.save_manifest(&manifest).await?;

        Ok(OperationResult {
            success: true,
            message: format!("Removed {}", pkg_id),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn get_dependencies(&self, package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let pkg_id = package_id.strip_prefix("custom:").unwrap_or(package_id);
        
        if let Some(def) = self.load_definition(pkg_id).await? {
            Ok(def.dependencies)
        } else {
            Ok(vec![])
        }
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // If we have a remote URL, sync definitions
        if let Some(url) = &self.remote_url {
            let client = reqwest::Client::new();
            let response = client.get(url).send().await?;
            
            if response.status().is_success() {
                let definitions: Vec<CustomPackageDefinition> = response.json().await?;
                
                fs::create_dir_all(&self.packages_dir).await?;
                
                for def in definitions {
                    let path = self.packages_dir.join(format!("{}.json", def.id));
                    let content = serde_json::to_string_pretty(&def)?;
                    fs::write(path, content).await?;
                }
            }
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_custom_adapter_creation() {
        let adapter = CustomAdapter::new();
        assert_eq!(adapter.source(), PackageSource::OfferingsCustom);
    }
}
