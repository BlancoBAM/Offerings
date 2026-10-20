// src/adapters/github.rs - GitHub Release Adapter
///
/// RENAMED SURFACE: this source is presented in the store as **"Direct Release"**
/// (behaviour identical to installing a single project release by hand with
/// poof / stew / bin / soar — the store streams the proper asset for the
/// platform directly from the project's release page into ~/.local/bin).
use super::{emit_progress, PackageAdapter, ProgressCallback, RealProgress};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

/// A tracked GitHub repo for release-based installation
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GitHubAppEntry {
    /// GitHub owner/repo, e.g. "sharkdp/bat"
    pub repo: String,
    /// Human-readable display name
    pub name: String,
    /// One-line description
    pub description: String,
    /// Binary name to look for after extraction
    pub binary_name: String,
    /// Asset filename pattern to match (supports simple glob: `*` matches anything)
    pub asset_pattern: String,
    /// Categories for store display
    pub categories: Vec<String>,
}

/// GitHub Release adapter
/// Manages a list of GitHub repos and installs releases as local binaries
pub struct GitHubReleaseAdapter {
    /// Tracked repos loaded from manifest — shared behind a lock so the store
    /// can add new owner/repo pairs at runtime (Direct Release flow).
    entries: tokio::sync::RwLock<Vec<GitHubAppEntry>>,
    /// Where to install binaries
    install_dir: PathBuf,
    /// HTTP client
    client: reqwest::Client,
}

impl GitHubReleaseAdapter {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let manifest_path = home.join(".config/offerings/github_apps.json");

        let entries = std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<GitHubAppEntry>>(&s).ok())
            .unwrap_or_else(Self::default_entries);

        Self {
            entries: tokio::sync::RwLock::new(entries),
            install_dir: home.join(".local/bin"),
            client: reqwest::Client::builder()
                .user_agent("Offerings/0.1")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Default curated entries when no manifest exists
    fn default_entries() -> Vec<GitHubAppEntry> {
        vec![
            GitHubAppEntry {
                repo: "sharkdp/bat".into(),
                name: "bat".into(),
                description: "A cat clone with syntax highlighting and Git integration".into(),
                binary_name: "bat".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
            GitHubAppEntry {
                repo: "sharkdp/fd".into(),
                name: "fd".into(),
                description: "A simple, fast and user-friendly alternative to find".into(),
                binary_name: "fd".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
            GitHubAppEntry {
                repo: "BurntSushi/ripgrep".into(),
                name: "ripgrep".into(),
                description: "Fast line-oriented search tool (rg)".into(),
                binary_name: "rg".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "Development".to_string()],
            },
            GitHubAppEntry {
                repo: "junegunn/fzf".into(),
                name: "fzf".into(),
                description: "A command-line fuzzy finder".into(),
                binary_name: "fzf".into(),
                asset_pattern: "*linux_amd64*".into(),
                categories: vec!["Utility".to_string()],
            },
            GitHubAppEntry {
                repo: "eza-community/eza".into(),
                name: "eza".into(),
                description: "A modern replacement for ls".into(),
                binary_name: "eza".into(),
                asset_pattern: "*x86_64*linux*musl*".into(),
                categories: vec!["Utility".to_string(), "System".to_string()],
            },
        ]
    }

    /// Match an asset filename against a simple glob pattern
    fn matches_pattern(filename: &str, pattern: &str) -> bool {
        let parts: Vec<&str> = pattern.split('*').collect();
        let lower = filename.to_lowercase();
        let mut pos = 0;
        for part in &parts {
            if part.is_empty() {
                continue;
            }
            match lower[pos..].find(&part.to_lowercase()) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
        true
    }

    /// Query the GitHub API for the latest release of a repo
    async fn get_latest_release(
        &self,
        repo: &str,
    ) -> Result<(String, Vec<(String, String)>), Box<dyn Error + Send + Sync>> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        let tag = resp["tag_name"].as_str().unwrap_or("unknown").to_string();
        let assets: Vec<(String, String)> = resp["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|asset| {
                        let name = asset["name"].as_str()?.to_string();
                        let url = asset["browser_download_url"].as_str()?.to_string();
                        Some((name, url))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok((tag, assets))
    }

    /// Accept "owner/repo", a project homepage URL or a release URL and
    /// normalize it down to "owner/repo". Returns None when unrecognisable.
    fn normalize_repo(input: &str) -> Option<String> {
        let mut s = input.trim().trim_end_matches('/').to_string();
        for prefix in [
            "https://github.com/",
            "http://github.com/",
            "git@github.com:",
            "github.com/",
        ] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest.to_string();
                break;
            }
        }
        // Release URLs still contain owner/repo as the first two segments
        if let Some(rest) = s.strip_suffix(".git") {
            s = rest.to_string();
        }
        let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() == 2 && parts.iter().all(|p| !p.contains(' ')) {
            Some(format!("{}/{}", parts[0], parts[1]))
        } else {
            None
        }
    }

    /// Choose a sensible asset glob for the current machine from the real
    /// release asset list — the pattern is stored so later updates re-match.
    fn pick_asset_pattern(assets: &[(String, String)]) -> Option<String> {
        // Ignore checksums / signatures / docs so we never "pick" a sidecar.
        let assets: Vec<&(String, String)> = assets
            .iter()
            .filter(|(name, _)| Self::usable_asset_name(name))
            .collect();

        const CANDIDATES: [&str; 6] = [
            "*x86_64*linux*musl*",
            "*x86_64*linux*gnu*",
            "*x86_64*linux*",
            "*linux_amd64*",
            "*x86_64*",
            "*linux*",
        ];
        for pattern in CANDIDATES {
            if assets
                .iter()
                .any(|(name, _)| Self::matches_pattern(name, pattern))
            {
                return Some(pattern.to_string());
            }
        }
        // Nothing matched: fall back to matching a single (largest) asset.
        if assets.len() == 1 {
            return Some(format!("*{}*", assets[0].0));
        }
        None
    }

    /// Sidecars that must never be treated as installable binaries
    fn usable_asset_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        !lower.ends_with(".sha256")
            && !lower.ends_with(".sha256sum")
            && !lower.ends_with(".sha512")
            && !lower.ends_with(".md5")
            && !lower.ends_with(".sig")
            && !lower.ends_with(".asc")
            && !lower.ends_with(".sbom")
            && !lower.ends_with(".spdx.json")
            && !lower.ends_with(".json")
            && !lower.ends_with(".txt")
            && !lower.ends_with(".md")
            && !lower.ends_with(".yml")
            && !lower.ends_with(".yaml")
    }

    /// Persist the current entry list to the user's manifest
    fn persist_manifest(&self, entries: &[GitHubAppEntry]) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let manifest_path = home.join(".config/offerings/github_apps.json");
        if let Some(parent) = manifest_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&manifest_path, json) {
                    eprintln!("Warning: failed to persist github_apps manifest: {}", e);
                }
            }
            Err(e) => eprintln!("Warning: failed to serialize github_apps manifest: {}", e),
        }
    }

    /// Direct Release flow: accept ANY repo or URL, resolve its metadata,
    /// pick the right asset for this machine and remember it as an entry.
    pub async fn add_repo(
        &self,
        repo_or_url: &str,
    ) -> Result<Package, Box<dyn Error + Send + Sync>> {
        let repo = Self::normalize_repo(repo_or_url).ok_or_else(|| {
            "Not a repo — enter e.g. sharkdp/bat or a github.com URL".to_string()
        })?;

        // Already tracked? Return the existing package view of it.
        {
            let entries = self.entries.read().await;
            if entries.iter().any(|e| e.repo == repo) {
                drop(entries);
                return match self.get_package(&format!("github:{}", repo)).await {
                    Ok(Some(pkg)) => Ok(pkg),
                    _ => Err(format!("Known repo {} but package lookup failed", repo).into()),
                };
            }
        }

        // Fetch repo metadata (name/description) and latest release assets.
        let meta: serde_json::Value = self
            .client
            .get(format!("https://api.github.com/repos/{}", repo))
            .send()
            .await?
            .json()
            .await?;
        if meta["message"].as_str().is_some() && meta["full_name"].as_str().is_none() {
            return Err(format!(
                "GitHub says: {} — check the repo name",
                meta["message"].as_str().unwrap_or("not found")
            )
            .into());
        }

        let (tag, assets) = self.get_latest_release(&repo).await?;
        let pattern = Self::pick_asset_pattern(&assets).ok_or_else(|| {
            format!(
                "No Linux asset found in release {} — this repo may publish no binaries",
                tag
            )
        })?;

        let name = repo.split('/').next_back().unwrap_or(&repo).to_string();
        let description = meta["description"]
            .as_str()
            .unwrap_or(&format!("Installed directly from the {} release", repo))
            .to_string();
        let categories = vec!["Utility".to_string()];
        let entry = GitHubAppEntry {
            repo: repo.clone(),
            name: name.clone(),
            description,
            binary_name: name,
            asset_pattern: pattern,
            categories,
        };

        let mut entries = self.entries.write().await;
        if !entries.iter().any(|e| e.repo == entry.repo) {
            entries.push(entry.clone());
        }
        self.persist_manifest(&entries);
        drop(entries);

        self.get_package(&format!("github:{}", entry.repo))
            .await?
            .ok_or_else(|| "Failed to resolve added repo into a package".to_string().into())
    }

    /// Read installed version from a .version file next to the binary
    async fn read_installed_version(&self, binary_name: &str) -> Option<String> {
        let version_file = self.install_dir.join(format!(".{}.version", binary_name));
        fs::read_to_string(&version_file)
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Write installed version marker
    async fn write_installed_version(
        &self,
        binary_name: &str,
        version: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let version_file = self.install_dir.join(format!(".{}.version", binary_name));
        fs::write(&version_file, version).await?;
        Ok(())
    }
}

impl Default for GitHubReleaseAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for GitHubReleaseAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::GitHubRelease
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn is_available(&self) -> bool {
        // Always available — uses HTTP, no CLI dependency
        true
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();
        let entries = self.entries.read().await;

        for entry in entries.iter() {
            let installed_version = self.read_installed_version(&entry.binary_name).await;
            let is_installed = installed_version.is_some();

            packages.push(Package {
                identity: PackageIdentity {
                    id: format!("github:{}", entry.repo),
                    name: entry.name.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: entry.description.clone(),
                    description: format!(
                        "{}\n\nSource: https://github.com/{}",
                        entry.description, entry.repo
                    ),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                    homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                    categories: entry.categories.clone(),
                    rating: None,
                },
                version: PackageVersion {
                    installed: installed_version,
                    latest: None, // Filled by check_updates
                },
                is_installed,
                logical_app_id: None,
                alternatives: vec![],
                last_updated: 0,
                popularity: 0.0,
            });
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();
        let entries = self.entries.read().await;

        for entry in entries.iter() {
            let binary_path = self.install_dir.join(&entry.binary_name);
            if binary_path.exists() {
                let version = self.read_installed_version(&entry.binary_name).await;
                packages.push(Package {
                    identity: PackageIdentity {
                        id: format!("github:{}", entry.repo),
                        name: entry.name.clone(),
                        source: PackageSource::GitHubRelease,
                    },
                    metadata: PackageMetadata {
                        summary: entry.description.clone(),
                        description: format!("Installed from GitHub: {}", entry.repo),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                        homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                        categories: entry.categories.clone(),
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: version.clone(),
                        latest: version,
                    },
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

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let repo = id.strip_prefix("github:").unwrap_or(id);
        let entries = self.entries.read().await;
        if let Some(entry) = entries.iter().find(|e| e.repo == repo) {
            let version = self.read_installed_version(&entry.binary_name).await;
            let is_installed = version.is_some();

            Ok(Some(Package {
                identity: PackageIdentity {
                    id: id.to_string(),
                    name: entry.name.clone(),
                    source: PackageSource::GitHubRelease,
                },
                metadata: PackageMetadata {
                    summary: entry.description.clone(),
                    description: format!(
                        "{}\n\nSource: https://github.com/{}",
                        entry.description, entry.repo
                    ),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: Some(format!("https://github.com/{}", entry.repo)),
                    homepage_url: Some(format!("https://github.com/{}", entry.repo)),
                    categories: entry.categories.clone(),
                    rating: None,
                },
                version: PackageVersion {
                    installed: version,
                    latest: None,
                },
                is_installed,
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
        let mut updates = Vec::new();
        let entries = self.entries.read().await;

        for entry in entries.iter() {
            let installed = match self.read_installed_version(&entry.binary_name).await {
                Some(v) => v,
                None => continue, // Not installed, skip
            };

            if let Ok((latest_tag, _)) = self.get_latest_release(&entry.repo).await {
                // Strip leading 'v' for comparison
                let installed_clean = installed.strip_prefix('v').unwrap_or(&installed);
                let latest_clean = latest_tag.strip_prefix('v').unwrap_or(&latest_tag);

                if installed_clean != latest_clean {
                    updates.push(Package {
                        identity: PackageIdentity {
                            id: format!("github:{}", entry.repo),
                            name: entry.name.clone(),
                            source: PackageSource::GitHubRelease,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion {
                            installed: Some(installed),
                            latest: Some(latest_tag),
                        },
                        is_installed: true,
                        logical_app_id: None,
                        alternatives: vec![],
                        last_updated: 0,
                        popularity: 0.0,
                    });
                }
            }
        }

        Ok(updates)
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.install_with_callback(package_id, None, None).await
    }


    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        use std::sync::Arc;

        // Real progress: stage_cb tracks download/extraction/install milestones,
        // download_cb carries the actual byte-level transfer fraction.
        let rp = Arc::new(RealProgress::new(callback.clone(), 0.02, 0.98));
        let download_cb: ProgressCallback = {
            let rp = rp.clone();
            Arc::new(move |p: f32| rp.set(0.02 + 0.6 * p.clamp(0.0, 1.0)))
        };
        let result = self
            .install_with_callback(package_id, Some(download_cb), Some(rp.clone()))
            .await;
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
        // Update is the same as install — it downloads the latest release
        self.install(package_id).await
    }

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        use std::sync::Arc;

        // Real progress: identical to install (fresh download of the release).
        let rp = Arc::new(RealProgress::new(callback.clone(), 0.02, 0.98));
        let download_cb: ProgressCallback = {
            let rp = rp.clone();
            Arc::new(move |p: f32| rp.set(0.02 + 0.6 * p.clamp(0.0, 1.0)))
        };
        let result = self
            .install_with_callback(package_id, Some(download_cb), Some(rp.clone()))
            .await;
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
        let repo = package_id.strip_prefix("github:").unwrap_or(package_id);
        let entries = self.entries.read().await;
        let entry = entries
            .iter()
            .find(|e| e.repo == repo)
            .ok_or("Unknown GitHub app")?;

        let binary_path = self.install_dir.join(&entry.binary_name);
        let version_path = self
            .install_dir
            .join(format!(".{}.version", entry.binary_name));

        if binary_path.exists() {
            fs::remove_file(&binary_path).await?;
        }
        if version_path.exists() {
            fs::remove_file(&version_path).await?;
        }

        Ok(OperationResult {
            success: true,
            message: format!("Removed {}", entry.name),
            updated_packages: vec![package_id.to_string()],
        })
    }

    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        // Real uninstall stages: files removed on return, version marker next.
        let rp = RealProgress::new(callback.clone(), 0.05, 0.98);
        let result = self.uninstall(package_id).await;
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            rp.milestone(0.85);
            rp.finish();
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn get_dependencies(
        &self,
        _package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // GitHub release binaries are self-contained
        Ok(vec![])
    }
}

impl GitHubReleaseAdapter {
    /// Full install flow with optional real-time callbacks:
    /// `download_cb` receives actual byte-level download fractions,
    /// `stage_cb` receives milestones for extraction/install/db steps.
    async fn install_with_callback(
        &self,
        package_id: &str,
        download_cb: Option<ProgressCallback>,
        stage_cb: Option<Arc<RealProgress>>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let repo = package_id.strip_prefix("github:").unwrap_or(package_id);
        let entries = self.entries.read().await;
        let entry = entries
            .iter()
            .find(|e| e.repo == repo)
            .ok_or("Unknown GitHub app")?;

        // Get latest release
        let (tag, assets) = self.get_latest_release(repo).await?;

        // Find matching asset
        let (asset_name, download_url) = assets
            .iter()
            .find(|(name, _)| Self::matches_pattern(name, &entry.asset_pattern))
            .ok_or_else(|| {
                format!(
                    "No matching asset found for pattern: {}",
                    entry.asset_pattern
                )
            })?;

        // Download to temp — with REAL byte-level download progress
        let tmp_dir = std::env::temp_dir().join(format!("offerings-gh-{}", entry.binary_name));
        fs::create_dir_all(&tmp_dir).await?;
        let tmp_file = tmp_dir.join(asset_name);

        let resp = self.client.get(download_url).send().await?;
        let total_len = resp.content_length();
        let mut file = tokio::io::BufWriter::new(fs::File::create(&tmp_file).await?);
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut received: u64 = 0;
        let mut last_report: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            received += chunk.len() as u64;
            file.write_all(&chunk).await?;
            if let Some(cb) = &download_cb {
                if received - last_report > 256 * 1024 {
                    last_report = received;
                    if let Some(total) = total_len {
                        if total > 0 {
                            cb(received as f32 / total as f32);
                        } else {
                            cb(0.05);
                        }
                    } else {
                        // Unknown content length: idle-ish at 5% until done.
                        cb(0.05);
                    }
                }
            }
        }
        file.flush().await?;
        if total_len.is_none() {
            if let Some(cb) = &download_cb {
                cb(1.0);
            }
        }
        emit_progress(&download_cb, 1.0);
        // Download 100% complete — real milestone.
        if let Some(rp) = &stage_cb {
            rp.milestone(0.65);
        }

        // Ensure install dir exists
        fs::create_dir_all(&self.install_dir).await?;
        let dest = self.install_dir.join(&entry.binary_name);

        // Handle tarball vs direct binary
        if asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz") {
            // Extract using tar
            let status = tokio::process::Command::new("tar")
                .args([
                    "xzf",
                    &tmp_file.to_string_lossy(),
                    "-C",
                    &tmp_dir.to_string_lossy(),
                ])
                .status()
                .await?;
            if !status.success() {
                return Ok(OperationResult {
                    success: false,
                    message: "Failed to extract tarball".into(),
                    updated_packages: vec![],
                });
            }
            // Find the binary in extracted files; fall back to the archive's
            // only executable when it ships under an unexpected name.
            let found = match find_binary_recursive(&tmp_dir, &entry.binary_name).await {
                Some(path) => Some(path),
                None => find_any_executable_recursive(&tmp_dir).await,
            };
            if let Some(bin_path) = found {
                fs::copy(&bin_path, &dest).await?;
            } else {
                return Ok(OperationResult {
                    success: false,
                    message: format!("Binary '{}' not found in archive", entry.binary_name),
                    updated_packages: vec![],
                });
            }
        } else if asset_name.ends_with(".zip") {
            // For zip files, try unzip
            let status = tokio::process::Command::new("unzip")
                .args([
                    "-o",
                    tmp_file.to_string_lossy().as_ref(),
                    "-d",
                    tmp_dir.to_string_lossy().as_ref(),
                ])
                .status()
                .await?;
            if !status.success() {
                return Ok(OperationResult {
                    success: false,
                    message: "Failed to extract zip".into(),
                    updated_packages: vec![],
                });
            }
            let found = match find_binary_recursive(&tmp_dir, &entry.binary_name).await {
                Some(path) => Some(path),
                None => find_any_executable_recursive(&tmp_dir).await,
            };
            if let Some(bin_path) = found {
                fs::copy(&bin_path, &dest).await?;
            } else {
                return Ok(OperationResult {
                    success: false,
                    message: format!("Binary '{}' not found in archive", entry.binary_name),
                    updated_packages: vec![],
                });
            }
        } else {
            // Direct binary
            fs::copy(&tmp_file, &dest).await?;
        }

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest).await?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            fs::set_permissions(&dest, perms).await?;
        }
        // Binary placed & chmodded — real milestone.
        if let Some(rp) = &stage_cb {
            rp.milestone(0.9);
        }

        // Write version marker
        self.write_installed_version(&entry.binary_name, &tag)
            .await?;

        // Cleanup temp
        let _ = fs::remove_dir_all(&tmp_dir).await;

        Ok(OperationResult {
            success: true,
            message: format!(
                "Installed {} {} to {}",
                entry.name,
                tag,
                self.install_dir.display()
            ),
            updated_packages: vec![package_id.to_string()],
        })
    }
}

/// Recursively search for a binary in a directory
async fn find_binary_recursive(dir: &std::path::Path, binary_name: &str) -> Option<PathBuf> {
    let mut entries = fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = Box::pin(find_binary_recursive(&path, binary_name)).await {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
            return Some(path);
        }
    }
    None
}

/// Fallback: when an archive doesn't ship a binary with the expected name,
/// grab the (single) executable it does contain.
async fn find_any_executable_recursive(dir: &std::path::Path) -> Option<PathBuf> {
    let mut entries = fs::read_dir(dir).await.ok()?;
    let mut first_exec: Option<PathBuf> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = Box::pin(find_any_executable_recursive(&path)).await {
                return Some(found);
            }
        } else if let Some(found) = is_executable_file(&path) {
            if first_exec.is_none() {
                first_exec = Some(found);
            }
        }
    }
    first_exec
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).ok()?;
    if meta.permissions().mode() & 0o111 != 0 {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(not(unix))]
fn is_executable_file(_path: &std::path::Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern() {
        assert!(GitHubReleaseAdapter::matches_pattern(
            "bat-v0.24.0-x86_64-unknown-linux-musl.tar.gz",
            "*x86_64*linux*musl*"
        ));
        assert!(!GitHubReleaseAdapter::matches_pattern(
            "bat-v0.24.0-x86_64-apple-darwin.tar.gz",
            "*x86_64*linux*musl*"
        ));
        assert!(GitHubReleaseAdapter::matches_pattern(
            "fzf-0.46.0-linux_amd64.tar.gz",
            "*linux_amd64*"
        ));
    }

    #[tokio::test]
    async fn test_github_adapter_creation() {
        let adapter = GitHubReleaseAdapter::new();
        assert_eq!(adapter.source(), PackageSource::GitHubRelease);
    }
}
