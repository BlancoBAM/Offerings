// src/adapters/lilith.rs - Lilith Curated Catalog Adapter
use super::PackageAdapter;
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::error::Error;

/// A curated metadata overlay entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LilithCatalogEntry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub target_package_id: String, // The actual underlying package to install
}

pub struct LilithCatalogAdapter {
    entries: Vec<LilithCatalogEntry>,
}

impl LilithCatalogAdapter {
    pub fn new() -> Self {
        // Fallback to default entries if no manifest is found
        let entries = Self::default_entries();

        Self { entries }
    }

    fn default_entries() -> Vec<LilithCatalogEntry> {
        // No curated entries yet - user will populate this later
        vec![]
    }
}

impl Default for LilithCatalogAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for LilithCatalogAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::OfferingsLilith
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        for entry in &self.entries {
            packages.push(Package {
                identity: PackageIdentity {
                    id: format!("lilith:{}", entry.id),
                    name: entry.name.clone(),
                    source: PackageSource::OfferingsLilith,
                },
                metadata: PackageMetadata {
                    summary: entry.summary.clone(),
                    description: format!("{}\n\n(Curated by Lilith)", entry.description),
                    icon_url: entry.icon.clone(),
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories: entry.categories.clone(),
                    rating: Some(5.0),
                },
                version: PackageVersion {
                    installed: None,
                    latest: Some("Curated".to_string()),
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            });
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // The LilithCatalogAdapter relies on the actual underlying source showing as installed,
        // so it doesn't directly manage installed state. The unified store will group them.
        Ok(vec![])
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let entry_id = id.strip_prefix("lilith:").unwrap_or(id);

        if let Some(entry) = self.entries.iter().find(|e| e.id == entry_id) {
            Ok(Some(Package {
                identity: PackageIdentity {
                    id: id.to_string(),
                    name: entry.name.clone(),
                    source: PackageSource::OfferingsLilith,
                },
                metadata: PackageMetadata {
                    summary: entry.summary.clone(),
                    description: format!("{}\n\n(Curated by Lilith)", entry.description),
                    icon_url: entry.icon.clone(),
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories: entry.categories.clone(),
                    rating: Some(5.0),
                },
                version: PackageVersion {
                    installed: None,
                    latest: Some("Curated".to_string()),
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        Ok(vec![])
    }

    async fn install(
        &self,
        _package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // Return failure to trigger fallback logic in backend, which will install the alternatives.
        Ok(OperationResult {
            success: false,
            message: "Curated metadata overlay. Redirecting to actual package source..."
                .to_string(),
            updated_packages: vec![],
        })
    }

    async fn update(
        &self,
        _package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        Ok(OperationResult {
            success: false,
            message: "Curated metadata overlay. Update the underlying package source.".to_string(),
            updated_packages: vec![],
        })
    }

    async fn uninstall(
        &self,
        _package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        Ok(OperationResult {
            success: false,
            message: "Curated metadata overlay. Uninstall the underlying package source."
                .to_string(),
            updated_packages: vec![],
        })
    }

    async fn get_dependencies(
        &self,
        package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let entry_id = package_id.strip_prefix("lilith:").unwrap_or(package_id);
        if let Some(entry) = self.entries.iter().find(|e| e.id == entry_id) {
            Ok(vec![entry.target_package_id.clone()])
        } else {
            Ok(vec![])
        }
    }
}
