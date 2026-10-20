// src/adapters/snap.rs - Snap Package Manager Adapter
use super::{
    command_exists, run_command, run_sudo_command, PackageAdapter, ProgressCallback, RealProgress,
};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::error::Error;

/// REAL Snap progress: snapd persists every change with a task list in its
/// database. We poll `snap changes` / `snap change <id>` and compute the ratio
/// of finished tasks — actual progress, straight from snapd's DB, not a timer.
async fn snap_change_progress(snap_name: &str) -> Option<f32> {
    let changes = run_command("snap", &["changes"]).await.ok()?;
    let needle = format!("\"{}\"", snap_name.to_lowercase());

    let mut best: Option<f32> = None;
    for line in changes.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        // Summary column mentions `Install "name" snap`, `Refresh "name" snap`, ...
        if !line.to_lowercase().contains(&needle) {
            continue;
        }
        let change_id = cols[0];
        // A running change shows '∅' in its Ready column.
        let running = line.contains('∅') || line.contains('−');
        let p: f32 = if running {
            change_task_progress(change_id).await.unwrap_or(0.05)
        } else {
            1.0
        };
        best = Some(best.map_or(p, |prev| p.max(prev)));
    }
    best
}

/// Compute actual progress of one snapd change: finished tasks / total tasks.
async fn change_task_progress(change_id: &str) -> Option<f32> {
    let out = run_command("snap", &["change", change_id]).await.ok()?;
    let mut total = 0usize;
    let mut done_units = 0.0f32;
    for line in out.lines().skip(1) {
        let status = line.trim_start().split_whitespace().next().unwrap_or("");
        match status {
            "Done" | "Done." => {
                total += 1;
                done_units += 1.0;
            }
            "Doing" | "Doing." | "Running" | "Run" | "Wait" => {
                total += 1;
                done_units += 0.25;
            }
            "Error" | "Undone" | "Abort" | "Hold" => {
                total += 1;
            }
            _ => {}
        }
    }
    (total > 0).then(|| done_units / total as f32)
}

/// Poll snapd in the background while the blocking pkexec command runs,
/// feeding each observed real progress value into the window 0.08..0.88.
async fn poll_snap_progress(snap_name: String, callback: Option<ProgressCallback>) {
    let rp = std::sync::Arc::new(RealProgress::new(callback, 0.08, 0.9));
    for _ in 0..2400 {
        // up to ~30 minutes at 750ms
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        match snap_change_progress(&snap_name).await {
            Some(p) if p >= 0.999 => {
                rp.set_percent(1.0);
                return;
            }
            Some(p) => rp.set_percent(p),
            None => {}
        }
    }
}

#[derive(Debug, Deserialize)]
struct SnapSearchResult {
    #[serde(rename = "icon_url")]
    icon_url: Option<String>,
    #[serde(rename = "screenshot_urls")]
    screenshot_urls: Option<Vec<String>>,
    description: Option<String>,
    summary: Option<String>,
    website: Option<String>,
    #[serde(rename = "ratings_average")]
    ratings_average: Option<f64>,
    #[serde(rename = "package_name")]
    package_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnapSearchResponse {
    #[serde(rename = "_embedded")]
    embedded: Option<SnapSearchEmbedded>,
}

#[derive(Debug, Deserialize)]
struct SnapSearchEmbedded {
    #[serde(rename = "clickindex:package")]
    packages: Option<Vec<SnapSearchResult>>,
}

/// Fetch metadata from the Snap Store API for a given snap name
fn fetch_snap_metadata(name: &str) -> Option<PackageMetadata> {
    let url = format!("https://api.snapcraft.io/api/v1/snaps/search?q={}", name);
    let resp = ureq::get(&url)
        .set("Accept", "application/json")
        .call()
        .ok()?;

    let body: SnapSearchResponse = resp.into_json().ok()?;
    let pkg = body
        .embedded
        .as_ref()?
        .packages
        .as_ref()?
        .iter()
        .find(|p| {
            p.package_name
                .as_ref()
                .map(|pn| pn == name)
                .unwrap_or(false)
        })
        .or_else(|| body.embedded.as_ref()?.packages.as_ref()?.iter().next())?;

    let icon_url = pkg.icon_url.clone().filter(|u| !u.is_empty());
    let screenshots = pkg.screenshot_urls.clone().unwrap_or_default();
    let description = pkg.description.clone().unwrap_or_default();
    let summary = pkg.summary.clone().unwrap_or_default();
    let homepage_url = pkg.website.clone().filter(|u| !u.is_empty());
    let rating = pkg.ratings_average.filter(|r| *r > 0.0).map(|r| r as f32);

    Some(PackageMetadata {
        summary: if summary.is_empty() {
            String::new()
        } else {
            summary
        },
        description: if description.is_empty() {
            String::new()
        } else {
            description
        },
        icon_url,
        screenshots,
        documentation_url: None,
        homepage_url,
        categories: vec!["Application".to_string()],
        rating,
    })
}

/// Enrich a package with Snap Store API metadata if fields are missing
fn enrich_from_api(pkg: &mut Package) {
    if pkg.metadata.icon_url.is_some()
        && !pkg.metadata.description.is_empty()
        && pkg.metadata.homepage_url.is_some()
    {
        return;
    }

    let snap_name = pkg
        .identity
        .id
        .strip_prefix("snap:")
        .unwrap_or(&pkg.identity.id);
    if let Some(api_meta) = fetch_snap_metadata(snap_name) {
        if pkg.metadata.icon_url.is_none() {
            pkg.metadata.icon_url = api_meta.icon_url;
        }
        if pkg.metadata.screenshots.is_empty() {
            pkg.metadata.screenshots = api_meta.screenshots;
        }
        if pkg.metadata.description.is_empty() {
            pkg.metadata.description = api_meta.description;
        }
        if pkg.metadata.summary.is_empty() && !api_meta.summary.is_empty() {
            pkg.metadata.summary = api_meta.summary;
        }
        if pkg.metadata.homepage_url.is_none() {
            pkg.metadata.homepage_url = api_meta.homepage_url;
        }
        if pkg.metadata.rating.is_none() {
            pkg.metadata.rating = api_meta.rating;
        }
        if pkg.metadata.categories == vec!["Application".to_string()]
            && api_meta.categories != vec!["Application".to_string()]
        {
            pkg.metadata.categories = api_meta.categories;
        }
    }
}

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
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
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
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
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
                version = line
                    .trim_start_matches("installed:")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
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
                description: if description.is_empty() {
                    summary
                } else {
                    description
                },
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: Some(format!("https://snapcraft.io/{}", name)),
                categories: vec!["Application".to_string()],
                rating: None,
            },
            version: PackageVersion {
                installed: if is_installed {
                    Some(version.clone())
                } else {
                    None
                },
                latest: Some(version),
            },
            is_installed,
            logical_app_id: None,
            alternatives: vec![],
            last_updated: 0,
            popularity: 0.0,
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

    fn contributes_browse(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
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

        // Try Snap Store API first for rich metadata
        if let Some(mut meta) = fetch_snap_metadata(snap_name) {
            if meta.summary.is_empty() {
                meta.summary = snap_name.to_string();
            }
            let pkg = Package {
                identity: PackageIdentity {
                    id: format!("snap:{}", snap_name),
                    name: snap_name.to_string(),
                    source: PackageSource::Snap,
                },
                metadata: meta,
                version: PackageVersion {
                    installed: None,
                    latest: None,
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            };
            return Ok(Some(pkg));
        }

        // Fallback to CLI
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
                for line in output.lines().skip(1) {
                    // Skip header
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
                            logical_app_id: None,
                            alternatives: vec![],
                            last_updated: 0,
                            popularity: 0.0,
                        });
                    }
                }
                Ok(packages)
            }
            Err(_) => Ok(vec![]), // No updates available
        }
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
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

    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);

        // Real progress: poller reads snapd's change DB while install runs.
        let poller = tokio::spawn(poll_snap_progress(snap_name.to_string(), callback.clone()));
        let result = match run_sudo_command("snap", &["install", snap_name][..]).await {
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
        };
        poller.abort();
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            if let Some(cb) = callback.as_ref() {
                cb(1.0);
            }
        }
        result
    }

    async fn update(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
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

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);

        let poller = tokio::spawn(poll_snap_progress(snap_name.to_string(), callback.clone()));
        let result = match run_sudo_command("snap", &["refresh", snap_name][..]).await {
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
        };
        poller.abort();
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            if let Some(cb) = callback.as_ref() {
                cb(1.0);
            }
        }
        result
    }

    async fn uninstall(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
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

    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let snap_name = package_id.strip_prefix("snap:").unwrap_or(package_id);

        let poller = tokio::spawn(poll_snap_progress(snap_name.to_string(), callback.clone()));
        let result = match run_sudo_command("snap", &["remove", snap_name][..]).await {
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
        };
        poller.abort();
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            if let Some(cb) = callback.as_ref() {
                cb(1.0);
            }
        }
        result
    }

    async fn get_dependencies(
        &self,
        _package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // Snaps are self-contained, they don't have traditional dependencies
        Ok(vec![])
    }

    async fn launch(
        &self,
        package_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let id = package_id.strip_prefix("snap:").unwrap_or(package_id);
        tokio::process::Command::new("snap")
            .arg("run")
            .arg(id)
            .spawn()?;
        Ok(())
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
