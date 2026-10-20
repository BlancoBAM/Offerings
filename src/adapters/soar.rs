// src/adapters/soar.rs - SOAR Package Manager Adapter
use super::{
    command_exists, emit_progress, run_command, run_command_with_stream, PackageAdapter,
    ProgressCallback, RealProgress,
};
use crate::catalog::{load_raw_entries, CatalogEntry};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;


/// SOAR package manager adapter
/// Provides access to pkgforge repositories via SOAR CLI, with the offline
/// unified catalogue (`assets/metadata-catalog.json`) as the primary listing
/// source. The catalogue is pre-deduplicated and desktop-only, so browsing /
/// searching never has to shell out to a slow network-backed CLI.
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
            let line_without_status = line.trim_start_matches(['[', '○', '●', ']']).trim();

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

impl SoarAdapter {
    /// Build the browsable package list straight from the offline unified
    /// catalogue. Ids come from `package_id` (canonical `soar:<name>`) so the
    /// metadata overlay and install routing keep matching exactly.
    fn packages_from_catalog(&self, entries: Vec<CatalogEntry>) -> Vec<Package> {
        entries
            .into_iter()
            .filter_map(|entry| {
                let name = entry.name.clone().unwrap_or_default();
                if name.trim().is_empty() {
                    return None;
                }
                let id = entry
                    .package_id
                    .clone()
                    .unwrap_or_else(|| format!("soar:{}", name.to_lowercase()));
                let summary = entry.summary.unwrap_or_default();
                let description = if entry.description.as_deref().unwrap_or("").is_empty() {
                    summary.clone()
                } else {
                    entry.description.unwrap()
                };
                let categories = if entry.categories.is_empty() {
                    vec!["Application".to_string()]
                } else {
                    entry.categories
                };
                // Multi-source install fallback: catalogue entries carry the
                // ids of every source offering this app (flatpak org ids, AM
                // appimage names…). They become this row's alternatives so a
                // failed soar install can fall back in preference order
                // (flathub first — auto-updates + richest metadata).
                let own_id = id.clone();
                let alternatives = entry
                    .source_ids
                    .iter()
                    .filter(|sid| {
                        sid.as_str() != own_id.as_str()
                            && (sid.starts_with("flatpak:")
                                || sid.starts_with("appimage:")
                                || sid.starts_with("snap:"))
                    })
                    .map(|sid| {
                        let source = PackageSource::from_id(sid);
                        PackageIdentity {
                            id: sid.clone(),
                            name: name.clone(),
                            source,
                        }
                    })
                    .collect();
                Some(Package {
                    identity: PackageIdentity {
                        id,
                        name,
                        source: PackageSource::Soar,
                    },
                    metadata: PackageMetadata {
                        summary,
                        description,
                        icon_url: entry.icon_url,
                        screenshots: entry.screenshots,
                        documentation_url: entry.documentation_url,
                        homepage_url: entry.homepage_url,
                        categories,
                        rating: entry.rating,
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: None,
                    },
                    is_installed: false,
                    logical_app_id: None,
                    alternatives,
                    last_updated: entry.last_updated.unwrap_or(0),
                    popularity: entry.popularity.unwrap_or(0.0),
                })
            })
            .collect()
    }

    /// Offline catalogue rows, parsed once per process and automatically
    /// re-loaded when an external catalogue updater touches the JSON files.
    fn catalog_packages(&self) -> Option<std::sync::Arc<Vec<Package>>> {
        static CACHE: std::sync::Mutex<Option<(Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>, Option<std::sync::Arc<Vec<Package>>>)>> =
            std::sync::Mutex::new(None);
        let fingerprint = crate::catalog::catalog_fingerprint();
        let mut guard = CACHE.lock().unwrap();
        let unchanged = guard
            .as_ref()
            .map(|(cached_fp, _)| *cached_fp == fingerprint)
            .unwrap_or(false);
        if !unchanged {
            let entries = load_raw_entries();
            let packages = if entries.is_empty() {
                None
            } else {
                Some(std::sync::Arc::new(self.packages_from_catalog(entries)))
            };
            *guard = Some((fingerprint, packages.clone()));
        }
        guard.as_ref().and_then(|(_, pkgs)| pkgs.clone())
    }
}

#[async_trait]
impl PackageAdapter for SoarAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Soar
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn is_available(&self) -> bool {
        // The adapter serves the offline catalogue even without the CLI,
        // so it participates in cache refreshes whenever either is present.
        self.catalog_packages().is_some() || command_exists("soar").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Primary: the offline unified catalogue (already deduplicated to one
        // desktop entry per app, with multi-source alternatives attached later
        // by the cache dedup pass). No network, no CLI call, instant.
        if let Some(pkgs) = self.catalog_packages() {
            return Ok((*pkgs).clone());
        }
        // Fallback: live soar CLI search (network-backed, slow) when no
        // catalogue JSON shipped with the app.
        let output =
            run_command("soar", &["search", "--limit", &self.limit.to_string(), ""]).await?;
        Ok(self.parse_soar_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // Only meaningful with the CLI present; otherwise report nothing so
        // cache refreshes don't spam an error for a tool that isn't installed.
        if !command_exists("soar").await {
            return Ok(vec![]);
        }
        let output = run_command("soar", &["info"]).await?;
        Ok(self.parse_soar_output(&output))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        // Catalogue lookup first (offline + instant); fall back to a live CLI
        // search when the app ships without a catalogue JSON or the id refers
        // to a repo-specific soar name (name#repo:source).
        let package_name = id.strip_prefix("soar:").unwrap_or(id);
        if let Some(pkgs) = self.catalog_packages() {
            let wanted = package_name.split('#').next().unwrap_or(package_name);
            let wanted_lower = wanted.to_lowercase();
            if let Some(found) = pkgs
                .iter()
                .find(|p| p.identity.name.to_lowercase() == wanted_lower)
            {
                return Ok(Some(found.clone()));
            }
            if !wanted.contains('#') && !wanted.contains(':') {
                // A well-formed catalogue id covers the whole namespace;
                // missing here means the CLI may carry a newer name.
                return Ok(None);
            }
        }
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
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        let name = package_name.split('#').next().unwrap_or(package_name);

        // Real progress: soar publishes its own download/install percentages on
        // stdout; every observed % is lifted into the 0.02..0.9 window.
        let rp = RealProgress::new(callback.clone(), 0.02, 0.92);
        let mapper = rp.percent_mapper();

        let result = match run_command_with_stream("soar", &["install", name], mapper).await {
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
        };
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            rp.finish();
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
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        let name = package_name.split('#').next().unwrap_or(package_name);

        let rp = RealProgress::new(callback.clone(), 0.03, 0.93);
        let mapper = rp.percent_mapper();
        let result = match run_command_with_stream("soar", &["update", name], mapper).await {
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
        };
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            rp.finish();
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
        let package_name = package_id.strip_prefix("soar:").unwrap_or(package_id);
        let name = package_name.split('#').next().unwrap_or(package_name);

        // Real uninstall stages: files removed → index/database updated → cache sync.
        let rp = RealProgress::new(callback.clone(), 0.05, 0.95);
        let result = match run_command("soar", &["remove", name]).await {
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
        };
        // Files removed (actual sub-step complete) → index write → done.
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            rp.milestone(0.8);
            let _ = run_command("soar", &["sync"]).await; // refresh index after removal
            rp.finish();
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
