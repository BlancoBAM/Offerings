// src/adapters/soar.rs - SOAR Package Manager Adapter
use super::{
    command_exists, emit_progress, run_command, start_staged_progress, PackageAdapter,
    ProgressCallback,
};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;

/// SOAR package manager adapter
/// Provides access to pkgforge repositories via SOAR CLI
pub struct SoarAdapter {
    limit: usize,
}

impl SoarAdapter {
    pub fn new() -> Self {
        Self { limit: 50000 }
    }
    /// Parse SOAR search output
    /// Format: [○] package#repo:source | version | type - description (size)
    fn parse_soar_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let line = line.trim();

            // Skip empty lines or lines that don't look like package entries
            if !line.starts_with("[") {
                continue;
            }

            // Parse: [○] package#repo:source | version | type - description (size)
            // Example: [○] 12to11#pkgforge-dev.12to11:soarpkgs | HEAD-510a27f | appimage - Tool for running...

            // Remove the [○] or [●] prefix
            let line_without_status = line
                .trim_start_matches(|c| c == '[' || c == '○' || c == '●' || c == ']')
                .trim();

            // Split by " | " to get parts
            let parts: Vec<&str> = line_without_status.splitn(4, " | ").collect();
            if parts.len() < 3 {
                continue;
            }

            // First part: package#repo:source
            let name_repo = parts[0];
            let name_parts: Vec<&str> = name_repo.split('#').collect();
            if name_parts.is_empty() {
                continue;
            }

            let name = name_parts[0].trim();
            if name.is_empty() {
                continue;
            }

            // Version
            let version = parts[1].trim();

            // Type and description (third part may contain "type - description")
            let type_desc = parts[2];
            let type_parts: Vec<&str> = type_desc.splitn(2, " - ").collect();
            let _pkg_type = type_parts[0].trim();
            let description = type_parts
                .get(1)
                .map(|s| {
                    // Remove size from end: "description (14.09 MiB)"
                    s.rsplit(" (")
                        .next()
                        .map(|rest| s.trim_end_matches(&format!(" ({})", rest)).trim())
                        .unwrap_or(s.trim())
                })
                .unwrap_or("");

            // Determine source based on repo/type
            let pkg = Package {
                identity: PackageIdentity {
                    id: format!("soar:{}", name_repo.replace(':', "-")),
                    name: name.to_string(),
                    source: PackageSource::Soar,
                },
                metadata: PackageMetadata {
                    summary: description.to_string(),
                    description: description.to_string(),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: Some(format!("https://github.com/search?q={}", name)),
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: None,
                    latest: Some(version.to_string()),
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            };

            packages.push(pkg);
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
        // Use soar search with empty query to get all packages
        let output =
            run_command("soar", &["search", "--limit", &self.limit.to_string(), ""]).await?;
        Ok(self.parse_soar_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Use soar info to list installed packages
        let output = run_command("soar", &["info"]).await?;
        Ok(self.parse_soar_output(&output))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        // Search for specific package
        let package_name = id.strip_prefix("soar:").unwrap_or(id);
        let output = run_command("soar", &["search", "--limit", "1", package_name]).await?;
        let packages = self.parse_soar_output(&output);
        Ok(packages.into_iter().next())
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // SOAR doesn't have a direct update check, sync and compare
        let _ = run_command("soar", &["sync"]).await;
        Ok(vec![])
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);

        // Extract just the package name (before #)
        let name = package_name.split('#').next().unwrap_or(package_name);

        match run_command("soar", &["install", name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully installed {}", name),
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
            0.92,
            0.08,
            std::time::Duration::from_millis(850),
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
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        let name = package_name.split('#').next().unwrap_or(package_name);

        match run_command("soar", &["update", name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully updated {}", name),
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
            0.94,
            0.07,
            std::time::Duration::from_millis(800),
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
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        let name = package_name.split('#').next().unwrap_or(package_name);

        match run_command("soar", &["remove", name]).await {
            Ok(_) => Ok(OperationResult {
                success: true,
                message: format!("Successfully removed {}", name),
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
            0.08,
            std::time::Duration::from_millis(750),
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
        // SOAR packages are typically self-contained
        Ok(vec![])
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Sync SOAR repositories
        let _ = run_command("soar", &["sync"]).await;
        Ok(())
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
