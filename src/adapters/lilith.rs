// src/adapters/lilith.rs - Lilith Curated Catalog Adapter
//
// Reads the curated package list from `lilith-curated.toml` (or `offer-cur.toml`
// as a fallback) and exposes them as `PackageSource::OfferingsLilith` packages.
//
// File format: one URL per line, optional inline `# comment` after the URL.
// Section headers in comments (e.g. `# ─── Productivity …`) set the active category.
// URL patterns are mapped to package IDs:
//   flathub.org/en/apps/<ID>  →  flatpak:<ID>
//   snapcraft.io/<name>       →  snap:<name>
//   github.com/<owner>/<repo> →  github:<owner>/<repo>
//   others                    →  skipped (no stable ID)
//
// Every entry is tagged with "Lilith" in its metadata categories so that
// `get_apps_by_category("Lilith")` finds them via the normal metadata search.

use super::PackageAdapter;
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;

/// A curated entry resolved from the TOML file
#[derive(Debug, Clone)]
struct CuratedEntry {
    /// The normalized package ID, e.g. "flatpak:org.mozilla.firefox"
    id: String,
    /// Section category parsed from the nearest comment header above this line
    section: String,
}

pub struct LilithCatalogAdapter {
    entries: Vec<CuratedEntry>,
}

impl LilithCatalogAdapter {
    pub fn new() -> Self {
        let entries = Self::load_entries();
        let count = entries.len();
        eprintln!("LilithCatalogAdapter: loaded {} curated entries", count);
        Self { entries }
    }

    /// Parse a URL → normalized package ID.
    /// Returns None for URLs that can't be mapped to a stable ID.
    fn url_to_id(url: &str) -> Option<String> {
        let url = url.trim();

        // Flathub: https://flathub.org/en/apps/<id>
        if let Some(rest) = url
            .strip_prefix("https://flathub.org/en/apps/")
            .or_else(|| url.strip_prefix("http://flathub.org/en/apps/"))
        {
            let id = rest.split_whitespace().next()?.trim_end_matches('/');
            if !id.is_empty() {
                return Some(format!("flatpak:{}", id));
            }
        }

        // Snapcraft: https://snapcraft.io/<name>
        if let Some(rest) = url
            .strip_prefix("https://snapcraft.io/")
            .or_else(|| url.strip_prefix("http://snapcraft.io/"))
        {
            let name = rest.split_whitespace().next()?.trim_end_matches('/');
            if !name.is_empty() && !name.contains('/') {
                return Some(format!("snap:{}", name));
            }
        }

        // GitHub: https://github.com/<owner>/<repo>
        if let Some(rest) = url
            .strip_prefix("https://github.com/")
            .or_else(|| url.strip_prefix("http://github.com/"))
        {
            let parts: Vec<&str> = rest.splitn(3, '/').collect();
            if parts.len() >= 2 {
                let owner = parts[0];
                let repo = parts[1].trim_end_matches('/');
                if !owner.is_empty() && !repo.is_empty() {
                    return Some(format!("github:{}/{}", owner, repo));
                }
            }
        }

        None
    }

    /// Parse a comment header line into a human-readable section name.
    /// e.g. "# ─── Productivity & Note-taking ───" → "Productivity"
    fn parse_section_header(comment: &str) -> Option<String> {
        let inner = comment.trim_start_matches('#').trim();
        // Strip unicode box-drawing characters used as decorative borders
        let cleaned: String = inner
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '&' || *c == '-')
            .collect();
        let cleaned = cleaned.trim().to_string();
        if cleaned.is_empty() {
            return None;
        }
        // Take only the first word-group before ' & ' or ' - '
        let first_word = cleaned
            .split(" & ")
            .next()
            .unwrap_or(&cleaned)
            .split(" - ")
            .next()
            .unwrap_or(&cleaned)
            .trim()
            .to_string();
        if first_word.len() < 3 {
            return None;
        }
        Some(first_word)
    }

    /// Candidate paths to search for the curated TOML, in priority order.
    fn catalog_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Alongside the binary (AppImage / installed)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                paths.push(dir.join("lilith-curated.toml"));
                paths.push(dir.join("offer-cur.toml"));
            }
        }

        // 2. Current working directory (running from source)
        if let Ok(cwd) = std::env::current_dir() {
            paths.push(cwd.join("lilith-curated.toml"));
            paths.push(cwd.join("offer-cur.toml"));
        }

        // 3. Data directory
        if let Some(data_dir) = dirs::data_local_dir() {
            paths.push(data_dir.join("offerings").join("lilith-curated.toml"));
        }

        // 4. /usr/share (system install)
        paths.push(PathBuf::from("/usr/share/offerings/lilith-curated.toml"));

        paths
    }

    fn load_entries() -> Vec<CuratedEntry> {
        // Find the first readable catalog file
        let content = Self::catalog_search_paths()
            .into_iter()
            .find_map(|p| {
                if p.exists() {
                    eprintln!("LilithCatalogAdapter: reading {}", p.display());
                    std::fs::read_to_string(&p).ok()
                } else {
                    None
                }
            });

        let content = match content {
            Some(c) => c,
            None => {
                eprintln!("LilithCatalogAdapter: no curated catalog file found — Lilith section will be empty");
                return vec![];
            }
        };

        let mut entries = Vec::new();
        let mut current_section = "Curated".to_string();

        for raw_line in content.lines() {
            let line = raw_line.trim();

            // Skip pure comment lines, but use them to detect section headers
            if line.starts_with('#') {
                if let Some(section) = Self::parse_section_header(line) {
                    current_section = section;
                }
                continue;
            }

            // Strip inline comment
            let url_part = line.split('#').next().unwrap_or(line).trim();
            if url_part.is_empty() {
                continue;
            }

            if let Some(id) = Self::url_to_id(url_part) {
                entries.push(CuratedEntry {
                    id,
                    section: current_section.clone(),
                });
            }
        }

        entries
    }

    /// Return all curated package IDs (used by backend to look them up in cache)
    pub fn curated_ids(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.id.clone()).collect()
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
        // Each curated entry is represented as a stub package tagged "Lilith".
        // The real package data is fetched from the underlying adapter (flatpak/snap/github).
        // We set the ID to the underlying source ID so the backend's merge logic
        // will enrich it with metadata from the appropriate adapter.
        let packages = self
            .entries
            .iter()
            .map(|entry| {
                let categories = vec!["Lilith".to_string(), entry.section.clone()];
                Package {
                    identity: PackageIdentity {
                        id: entry.id.clone(),
                        // Name is left empty — the underlying adapter will fill it in
                        name: entry.id.split(':').nth(1).unwrap_or(&entry.id).to_string(),
                        source: PackageSource::OfferingsLilith,
                    },
                    metadata: PackageMetadata {
                        summary: format!("Curated for Lilith — {}", entry.section),
                        description: String::new(),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories,
                        rating: Some(5.0),
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: None,
                    },
                    is_installed: false,
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 10.0, // Give curated apps a boost in discovery score
                }
            })
            .collect();

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        Ok(vec![])
    }

    async fn get_package(
        &self,
        id: &str,
    ) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let entry = self.entries.iter().find(|e| e.id == id);
        if let Some(entry) = entry {
            let categories = vec!["Lilith".to_string(), entry.section.clone()];
            Ok(Some(Package {
                identity: PackageIdentity {
                    id: entry.id.clone(),
                    name: entry.id.split(':').nth(1).unwrap_or(&entry.id).to_string(),
                    source: PackageSource::OfferingsLilith,
                },
                metadata: PackageMetadata {
                    summary: format!("Curated for Lilith — {}", entry.section),
                    description: String::new(),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories,
                    rating: Some(5.0),
                },
                version: PackageVersion {
                    installed: None,
                    latest: None,
                },
                is_installed: false,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 10.0,
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
        Ok(OperationResult {
            success: false,
            message: "Curated entry — install via the underlying package source.".to_string(),
            updated_packages: vec![],
        })
    }

    async fn update(
        &self,
        _package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        Ok(OperationResult {
            success: false,
            message: "Curated entry — update via the underlying package source.".to_string(),
            updated_packages: vec![],
        })
    }

    async fn uninstall(
        &self,
        _package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        Ok(OperationResult {
            success: false,
            message: "Curated entry — uninstall via the underlying package source.".to_string(),
            updated_packages: vec![],
        })
    }

    async fn get_dependencies(
        &self,
        _package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        Ok(vec![])
    }
}
