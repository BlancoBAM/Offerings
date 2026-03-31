use crate::model::Package;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub package_id: Option<String>,
    pub source_ids: Vec<String>,
    pub logical_id: Option<String>,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub screenshots: Vec<String>,
    pub homepage_url: Option<String>,
    pub documentation_url: Option<String>,
    pub categories: Vec<String>,
    pub rating: Option<f32>,
    pub popularity: Option<f32>,
    pub last_updated: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedCatalog {
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataCatalog {
    by_package_id: HashMap<String, CatalogEntry>,
    by_source_id: HashMap<String, CatalogEntry>,
    by_logical_id: HashMap<String, CatalogEntry>,
    by_name: HashMap<String, CatalogEntry>,
}

impl MetadataCatalog {
    pub fn load() -> Self {
        let mut merged = UnifiedCatalog::default();

        for path in candidate_catalog_paths() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(catalog) = serde_json::from_str::<UnifiedCatalog>(&content) {
                    merged.entries.extend(catalog.entries);
                }
            }
        }

        let mut by_package_id = HashMap::new();
        let mut by_source_id = HashMap::new();
        let mut by_logical_id = HashMap::new();
        let mut by_name = HashMap::new();

        for entry in merged.entries {
            if let Some(package_id) = entry.package_id.clone() {
                by_package_id.insert(normalize_key(&package_id), entry.clone());
            }
            for source_id in &entry.source_ids {
                by_source_id.insert(normalize_key(source_id), entry.clone());
            }
            if let Some(logical_id) = entry.logical_id.clone() {
                by_logical_id.insert(normalize_key(&logical_id), entry.clone());
            }
            if let Some(name) = entry.name.clone() {
                by_name.insert(normalize_key(&name), entry.clone());
            }
        }

        Self {
            by_package_id,
            by_source_id,
            by_logical_id,
            by_name,
        }
    }

    pub fn merge_entry(&mut self, entry: CatalogEntry) {
        if let Some(package_id) = entry.package_id.clone() {
            self.by_package_id
                .insert(normalize_key(&package_id), entry.clone());
        }
        for source_id in &entry.source_ids {
            self.by_source_id
                .insert(normalize_key(source_id), entry.clone());
        }
        if let Some(logical_id) = entry.logical_id.clone() {
            self.by_logical_id
                .insert(normalize_key(&logical_id), entry.clone());
        }
        if let Some(name) = entry.name.clone() {
            self.by_name.insert(normalize_key(&name), entry);
        }
    }

    pub fn find_for_package(&self, pkg: &Package) -> Option<&CatalogEntry> {
        self.by_package_id
            .get(&normalize_key(&pkg.identity.id))
            .or_else(|| self.by_source_id.get(&normalize_key(pkg.short_id())))
            .or_else(|| {
                pkg.logical_app_id
                    .as_ref()
                    .and_then(|id| self.by_logical_id.get(&normalize_key(id)))
            })
            .or_else(|| self.by_name.get(&normalize_key(&pkg.identity.name)))
    }
}

impl CatalogEntry {
    pub fn from_package(pkg: &Package) -> Self {
        Self {
            package_id: Some(pkg.identity.id.clone()),
            source_ids: vec![pkg.short_id().to_string()],
            logical_id: pkg.logical_app_id.clone(),
            name: Some(pkg.identity.name.clone()),
            summary: non_empty(&pkg.metadata.summary),
            description: non_empty(&pkg.metadata.description),
            icon_url: pkg
                .metadata
                .icon_url
                .clone()
                .filter(|v| !v.trim().is_empty()),
            screenshots: pkg.metadata.screenshots.clone(),
            homepage_url: pkg
                .metadata
                .homepage_url
                .clone()
                .filter(|v| !v.trim().is_empty()),
            documentation_url: pkg
                .metadata
                .documentation_url
                .clone()
                .filter(|v| !v.trim().is_empty()),
            categories: pkg.metadata.categories.clone(),
            rating: pkg.metadata.rating,
            popularity: (pkg.popularity > 0.0).then_some(pkg.popularity),
            last_updated: (pkg.last_updated > 0).then_some(pkg.last_updated),
        }
    }
}

fn normalize_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

fn candidate_catalog_paths() -> Vec<PathBuf> {
    let mut paths =
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/metadata-catalog.json")];

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("offerings/metadata-catalog.json"));
    }
    if let Some(data_dir) = dirs::data_local_dir() {
        paths.push(data_dir.join("offerings/metadata-catalog.json"));
    }

    paths
}
