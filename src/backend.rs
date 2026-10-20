// src/backend.rs - Core Backend Service
use crate::adapters::{
    AppImageAdapter, CustomAdapter, DbProgressCallback, FlatpakAdapter, GitHubReleaseAdapter,
    HomebrewAdapter, LilithCatalogAdapter, PackageAdapter, SnapAdapter, SoarAdapter,
};
use crate::catalog::MetadataCatalog;
use crate::db::Database;
use crate::model::{
    AppDetailInfo, HomePageContent, OperationResult, Package, PackageOperation, PackageSource,
};
use crate::notifications::{NotificationManager, NotificationType};
use crate::search::SearchEngine;
use crate::transaction::TransactionManager;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock as StdRwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// How many package operations (install/update/uninstall) may run at once.
/// Package managers like flatpak and snap keep their own internal locks, so a
/// small global pool avoids backend lock contention while still letting the
/// store do several things in parallel.
const MAX_CONCURRENT_OPERATIONS: usize = 3;

/// Cap on how many packages a single search returns to the UI. Every match is
/// cloned into the results; unbounded result sets are what froze the GUI.
pub const SEARCH_RESULT_LIMIT: usize = 200;

/// Core backend service managing all package operations
pub struct BackendService {
    adapters: Vec<Arc<dyn PackageAdapter>>,
    adapter_map: HashMap<PackageSource, usize>,
    package_cache: Arc<RwLock<HashMap<String, Package>>>,
    db: Arc<Database>,
    transaction_manager: Arc<TransactionManager>,
    notifications: Arc<NotificationManager>,
    home_content: HomePageContent,
    metadata_catalog: Arc<StdRwLock<MetadataCatalog>>,
    /// Nucleo-backed fuzzy search index over the package cache
    search: Arc<SearchEngine>,
    /// Global operation pool: at most MAX_CONCURRENT_OPERATIONS at a time.
    operation_slots: Arc<tokio::sync::Semaphore>,
    /// Package ids currently being operated on (prevents duplicate
    /// install/uninstall races on the same app from repeated clicks).
    busy_packages: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl BackendService {
    fn normalized_app_key(name: &str) -> String {
        let mut key = name.trim().to_lowercase();
        key = key.split(" - ").next().unwrap_or(&key).to_string();
        key = key.split(" (").next().unwrap_or(&key).to_string();
        key = key.replace("web browser", "");
        key = key.replace("community edition", "");
        key = key.replace("community", "");
        key = key.replace("version", "");
        key = key.replace("official", "");
        key = key.replace("alpha", "");
        key = key.replace("beta", "");
        key = key.replace("nightly", "");
        key = key.trim_end_matches(".appimage").to_string();
        key = key.trim_end_matches(".flatpak").to_string();
        key = key.trim_end_matches(".snap").to_string();
        key.trim().to_string()
    }

    fn package_group_keys(pkg: &Package) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(logical_id) = &pkg.logical_app_id {
            let logical = logical_id.trim().to_lowercase();
            if !logical.is_empty() {
                keys.push(logical);
            }
        }
        let normalized = Self::normalized_app_key(&pkg.identity.name);
        if !normalized.is_empty() && !keys.contains(&normalized) {
            keys.push(normalized);
        }
        keys
    }

    fn propagate_installed_state(cache: &mut HashMap<String, Package>) {
        let mut installed_by_key: HashMap<String, (Option<String>, PackageSource)> = HashMap::new();

        for pkg in cache.values() {
            if !pkg.is_installed {
                continue;
            }
            for key in Self::package_group_keys(pkg) {
                installed_by_key
                    .entry(key)
                    .or_insert((pkg.version.installed.clone(), pkg.identity.source.clone()));
            }
        }

        if installed_by_key.is_empty() {
            return;
        }

        for pkg in cache.values_mut() {
            for key in Self::package_group_keys(pkg) {
                if let Some((installed_version, installed_source)) = installed_by_key.get(&key) {
                    pkg.is_installed = true;
                    if pkg.version.installed.is_none() {
                        pkg.version.installed = installed_version.clone();
                    }
                    if pkg.metadata.summary.trim().is_empty()
                        && *installed_source != pkg.identity.source
                    {
                        pkg.metadata.summary =
                            format!("Installed on this system via {}", installed_source.label());
                    }
                    break;
                }
            }
        }
    }

    fn package_id_matches(pkg: &Package, package_id: &str) -> bool {
        let short_id = package_id.split(':').nth(1).unwrap_or(package_id);
        pkg.identity.id == package_id || pkg.short_id() == short_id
    }

    fn has_placeholder_description(pkg: &Package) -> bool {
        let name = pkg.identity.name.trim().to_lowercase();
        let summary = pkg.metadata.summary.trim().to_lowercase();
        let description = pkg.metadata.description.trim().to_lowercase();

        description.is_empty()
            || description == name
            || description == summary
            || description == format!("{} application", name)
            || description == format!("install {} via homebrew", name)
            || description == format!("installed via homebrew: {}", name)
            || description == format!("appimage: {}", name)
    }

    fn persist_catalog_metadata(&self, pkg: &Package) {
        let entry = crate::catalog::CatalogEntry::from_package(pkg);
        if let Err(e) = self.db.upsert_metadata_entry(&entry) {
            eprintln!(
                "Warning: Failed to persist metadata catalog entry for {}: {}",
                pkg.identity.id, e
            );
        }
        if let Ok(mut catalog) = self.metadata_catalog.write() {
            catalog.merge_entry(entry);
        }
    }

    fn merge_package_records(&self, mut base: Package, incoming: Package) -> Package {
        if base.metadata.summary.trim().is_empty() && !incoming.metadata.summary.trim().is_empty() {
            base.metadata.summary = incoming.metadata.summary.clone();
        }
        if base.metadata.description.trim().is_empty()
            && !incoming.metadata.description.trim().is_empty()
        {
            base.metadata.description = incoming.metadata.description.clone();
        }
        if base
            .metadata
            .icon_url
            .as_ref()
            .is_none_or(|url| url.trim().is_empty())
        {
            base.metadata.icon_url = incoming.metadata.icon_url.clone();
        }
        if base.metadata.screenshots.is_empty() && !incoming.metadata.screenshots.is_empty() {
            base.metadata.screenshots = incoming.metadata.screenshots.clone();
        }
        if base.metadata.homepage_url.is_none() {
            base.metadata.homepage_url = incoming.metadata.homepage_url.clone();
        }
        if base.metadata.documentation_url.is_none() {
            base.metadata.documentation_url = incoming.metadata.documentation_url.clone();
        }
        if base.metadata.categories.is_empty() && !incoming.metadata.categories.is_empty() {
            base.metadata.categories = incoming.metadata.categories.clone();
        }
        if base.metadata.rating.is_none() {
            base.metadata.rating = incoming.metadata.rating;
        }
        if base.version.latest.is_none() {
            base.version.latest = incoming.version.latest.clone();
        }
        if base.version.installed.is_none() {
            base.version.installed = incoming.version.installed.clone();
        }
        if incoming.is_installed {
            base.is_installed = true;
        }
        if base.logical_app_id.is_none() {
            base.logical_app_id = incoming.logical_app_id.clone();
        }
        if base.last_updated == 0 {
            base.last_updated = incoming.last_updated;
        }
        if base.popularity <= 0.0 {
            base.popularity = incoming.popularity;
        }
        base
    }

    async fn reconcile_package_state_once(
        &self,
        package_id: &str,
        expected_installed: bool,
    ) -> Option<Package> {
        let source = PackageSource::from_id(package_id);
        let adapter_idx = *self.adapter_map.get(&source)?;
        let adapter = &self.adapters[adapter_idx];

        let installed_packages = adapter.list_installed().await.ok()?;
        let installed_pkg = installed_packages
            .into_iter()
            .find(|pkg| Self::package_id_matches(pkg, package_id));

        let reconciled = if expected_installed {
            let installed_pkg = installed_pkg?;
            if let Ok(Some(available_pkg)) = adapter.get_package(package_id).await {
                self.merge_package_records(installed_pkg, self.enrich_metadata(available_pkg))
            } else {
                self.enrich_metadata(installed_pkg)
            }
        } else if installed_pkg.is_none() {
            if let Ok(Some(mut available_pkg)) = adapter.get_package(package_id).await {
                available_pkg = self.enrich_metadata(available_pkg);
                available_pkg.is_installed = false;
                available_pkg.version.installed = None;
                available_pkg
            } else {
                let cache = self.package_cache.read().await;
                let mut cached = cache.get(package_id).cloned()?;
                cached.is_installed = false;
                cached.version.installed = None;
                cached
            }
        } else {
            return None;
        };

        let _ = self.db.upsert_package(&reconciled);
        self.persist_catalog_metadata(&reconciled);
        let mut cache = self.package_cache.write().await;
        cache.insert(package_id.to_string(), reconciled.clone());
        Some(self.enrich_package_with_alternatives(reconciled, &cache))
    }

    pub async fn reconcile_package_state(
        &self,
        package_id: &str,
        expected_installed: bool,
    ) -> Option<Package> {
        for _ in 0..20 {
            if let Some(pkg) = self
                .reconcile_package_state_once(package_id, expected_installed)
                .await
            {
                return Some(pkg);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(750)).await;
        }
        None
    }

    /// Create a new backend service with all available adapters
    pub fn new(
        home_content: HomePageContent,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        eprintln!("BackendService::new: opening database");
        let db = Arc::new(Database::new()?);
        eprintln!(
            "BackendService::new: database ready at {}",
            db.path().display()
        );
        eprintln!("BackendService::new: creating transaction manager");
        let transaction_manager = Arc::new(TransactionManager::new(db.clone()));
        eprintln!("BackendService::new: creating notification manager");
        let notifications = Arc::new(NotificationManager::new(Default::default()));

        // Load custom sources from DB
        eprintln!("BackendService::new: loading custom sources");
        let custom_sources = db.get_sources().unwrap_or_default();
        let remote_urls: Vec<String> = custom_sources.iter().map(|s| s.url.clone()).collect();

        eprintln!("BackendService::new: constructing adapters");
        let adapters: Vec<Arc<dyn PackageAdapter>> = vec![
            Arc::new(FlatpakAdapter::new()),
            Arc::new(AppImageAdapter::new()), // AM
            Arc::new(SoarAdapter::new()),     // SOAR/pkgforge repositories
            Arc::new(SnapAdapter::new()),
            Arc::new(HomebrewAdapter::new()),
            Arc::new(GitHubReleaseAdapter::new()),
            Arc::new(CustomAdapter::with_remotes(remote_urls)),
            Arc::new(LilithCatalogAdapter::new()),
        ];

        let mut adapter_map = HashMap::new();
        for (i, adapter) in adapters.iter().enumerate() {
            adapter_map.insert(adapter.source(), i);
        }

        eprintln!("BackendService::new: loading metadata catalog files");
        let mut metadata_catalog = MetadataCatalog::load();
        eprintln!("BackendService::new: loading metadata catalog from database");
        for entry in db.load_metadata_catalog().unwrap_or_default() {
            metadata_catalog.merge_entry(entry);
        }
        eprintln!("BackendService::new: startup complete");

        Ok(Self {
            adapters,
            adapter_map,
            package_cache: Arc::new(RwLock::new(HashMap::new())),
            db,
            transaction_manager,
            notifications,
            home_content,
            metadata_catalog: Arc::new(StdRwLock::new(metadata_catalog)),
            search: Arc::new(SearchEngine::new(Some(4))),
            operation_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_OPERATIONS)),
            busy_packages: Arc::new(Mutex::new(std::collections::HashSet::new())),
        })
    }

    /// Create backend with custom adapters (for testing)
    pub fn with_adapters(
        adapters: Vec<Arc<dyn PackageAdapter>>,
        home_content: HomePageContent,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db = Arc::new(Database::new()?);
        let transaction_manager = Arc::new(TransactionManager::new(db.clone()));
        let notifications = Arc::new(NotificationManager::new(Default::default()));

        let mut adapter_map = HashMap::new();
        for (i, adapter) in adapters.iter().enumerate() {
            adapter_map.insert(adapter.source(), i);
        }

        let mut metadata_catalog = MetadataCatalog::load();
        for entry in db.load_metadata_catalog().unwrap_or_default() {
            metadata_catalog.merge_entry(entry);
        }

        Ok(Self {
            adapters,
            adapter_map,
            package_cache: Arc::new(RwLock::new(HashMap::new())),
            db,
            transaction_manager,
            notifications,
            home_content,
            metadata_catalog: Arc::new(StdRwLock::new(metadata_catalog)),
            search: Arc::new(SearchEngine::new(Some(2))),
            operation_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_OPERATIONS)),
            busy_packages: Arc::new(Mutex::new(std::collections::HashSet::new())),
        })
    }

    /// Get reference to the database
    pub fn database(&self) -> Arc<Database> {
        self.db.clone()
    }

    /// Get reference to the transaction manager
    pub fn transactions(&self) -> Arc<TransactionManager> {
        self.transaction_manager.clone()
    }

    /// Get reference to the notification manager
    pub fn notifications(&self) -> Arc<NotificationManager> {
        self.notifications.clone()
    }

    /// Load all packages from the SQLite database into the in-memory cache.
    /// This is instant (no network), giving the UI something to display before
    /// the full network refresh completes. Safe to call concurrently with refresh.
    pub async fn load_cache_from_db(&self) {
        match self.db.load_all_packages() {
            Ok(packages) => {
                let count = packages.len();
                if packages.is_empty() {
                    eprintln!("DB cache warm-up: no packages stored yet (first run?)");
                    return;
                }
                // Older databases may still hold one row per source; fold
                // duplicates once here so every consumer sees canonical rows.
                let deduped = self.deduplicate_packages(packages.into_iter());
                let mut cache = self.package_cache.write().await;
                for pkg in deduped {
                    let pkg = self.enrich_metadata(pkg);
                    // Don't overwrite entries that were already refreshed from adapters
                    cache.entry(pkg.identity.id.clone()).or_insert(pkg);
                }
                eprintln!("DB cache warm-up: loaded {} packages ({} unique)", count, cache.len());
            }
            Err(e) => {
                eprintln!("Warning: Failed to load packages from DB for warm-up: {}", e);
            }
        }
        // The warm-up data is what search serves until the first full refresh
        // completes; make sure the index has it too.
        self.rebuild_search_index().await;
    }

    /// Refresh cache from all available adapters
    pub async fn refresh_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.refresh_cache_with_progress(None).await
    }

    /// Refresh cache from all available adapters, reporting ACTUAL database
    /// progress: every emitted percentage corresponds to package rows really
    /// written to the SQLite cache out of the total the adapters reported.
    pub async fn refresh_cache_with_progress(
        &self,
        progress: Option<DbProgressCallback>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // ─── Discover available adapters up-front so windows are fair ───
        let mut active: Vec<(usize, crate::model::PackageSource)> = Vec::new();
        for (idx, adapter) in self.adapters.iter().enumerate() {
            if adapter.is_available().await {
                active.push((idx, adapter.source()));
            }
        }
        let adapter_count = active.len().max(1) as f32;
        eprintln!(
            "Cache refresh: {} adapters available (real DB progress tracking)",
            adapter_count
        );

        // ─── Build the ENTIRE new cache off to the side, then swap it in ───
        // The old code cleared the live cache first and slowly refilled it,
        // which meant live queries (search, browse, category counts) returned
        // nothing while a refresh was in flight — the "Video shows 1254 in
        // the sidebar but nothing in the grid" bug. Building `fresh` first
        // means browsing always sees a complete snapshot; the swap is atomic.
        let mut cache: HashMap<String, Package> = HashMap::new();

        let emit = {
            let progress = progress.clone();
            move |fraction: f32, message: &str| {
                if let Some(cb) = &progress {
                    cb(fraction.clamp(0.0, 0.98), message.to_string());
                }
            }
        };

        // ─── Stage 1/3: featured packages (small share of the first window) ───
        emit(0.01, "Fetching featured apps...");
        let mut rows_written: u64 = 0;

        // First, fetch all featured apps from home_content to ensure they're in cache
        let mut featured_ids: Vec<&String> = self.home_content.featured_apps.iter().collect();
        for apps in self.home_content.category_showcases.values() {
            featured_ids.extend(apps.iter());
        }
        featured_ids.sort();
        featured_ids.dedup();

        // Fetch featured packages first
        for id in &featured_ids {
            let source = PackageSource::from_id(id);
            if let Some(&adapter_idx) = self.adapter_map.get(&source) {
                let adapter = &self.adapters[adapter_idx];
                if adapter.is_available().await {
                    if let Ok(Some(pkg)) = adapter.get_package(id).await {
                        let pkg = self.enrich_metadata(pkg);
                        if let Err(e) = self.db.upsert_package(&pkg) {
                            eprintln!("Warning: Failed to cache featured package {}: {}", id, e);
                        }
                        self.persist_catalog_metadata(&pkg);
                        cache.insert(pkg.identity.id.clone(), pkg);
                        rows_written += 1;
                    }
                }
            }
        }
        // First adapter window: featured apps fill up to 25% of its share.
        // Adapter windows are scaled into 0.00–0.90 so the dedup + persist
        // stages after them stay strictly monotonic.
        emit(0.90 * 0.25 / adapter_count, "Featured apps cached to database");

        let mut adapter_completed: u32 = 0;
        for (idx, source) in &active {
            let adapter = &self.adapters[*idx];
            let window_start = 0.90 * (adapter_completed as f32) / adapter_count;
            let window_width = 0.90 / adapter_count;
            let label = source.label().to_string();

            // ── Adapter cache refresh (sync their own indexes) ──
            if let Err(e) = adapter.refresh_cache().await {
                eprintln!(
                    "Warning: Failed to refresh cache for {:?}: {}",
                    adapter.source(), e
                );
            }

            // ── Installed packages (all of them are DB rows too) ──
            let installed_rows =
                match adapter.list_installed().await {
                    Ok(packages) => {
                        for pkg in packages {
                            let pkg = self.enrich_metadata(pkg);
                            if let Err(e) = self.db.upsert_package(&pkg) {
                                eprintln!(
                                    "Warning: Failed to cache package {}: {}",
                                    pkg.identity.id, e
                                );
                            }
                            self.persist_catalog_metadata(&pkg);
                            cache.insert(pkg.identity.id.clone(), pkg);
                            rows_written += 1;
                        }
                        eprintln!(
                            "{}: installed packages cached",
                            label
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to list installed packages for {:?}: {}",
                            adapter.source(), e
                        );
                    }
                };
            let _ = installed_rows;
            emit(
                window_start + window_width * 0.25,
                &format!("{}: cached installed apps", label),
            );

            // ── Available packages — this is the bulk of the DB write load ──
            // Only adapters whose listing belongs in the browse catalogue
            // contribute here; runtime managers (flatpak/snap/am/brew) are
            // seeds for the offline catalogue instead.
            if adapter.contributes_browse() {
                self.browse_adapter_rows(
                    adapter,
                    &label,
                    &mut cache,
                    window_start,
                    window_width,
                    &emit,
                    &mut rows_written,
                )
                .await;
            } else {
                emit(
                    window_start + window_width * 0.25,
                    &format!("{}: browse handled by unified catalogue", label),
                );
            }

            adapter_completed += 1;
        }

        // ─── Stage 3/3: dedupe, propagate installed state, persist ───
        emit(0.92, "Merging duplicate apps into single canonical entries...");
        // One-time dedup: fold per-source rows of the same app into a single
        // canonical package with multi-source alternatives. Query paths no
        // longer re-dedup on every call, which removes the old per-category
        // O(n) group scans from UI population.
        {
            let deduped = self.deduplicate_packages(cache.values().cloned());
            cache.clear();
            for pkg in deduped {
                cache.insert(pkg.identity.id.clone(), pkg);
            }
        }
        emit(0.97, "Aggregating installed state to database...");
        Self::propagate_installed_state(&mut cache);
        let mut persisted: u64 = 0;
        let total_cache = cache.len() as u64;
        for pkg in cache.values() {
            if let Err(e) = self.db.upsert_package(pkg) {
                eprintln!(
                    "Warning: Failed to persist aggregated package state for {}: {}",
                    pkg.identity.id, e
                );
            }
            persisted += 1;
            if persisted % 500 == 0 {
                emit(
                    0.97 + (persisted as f32 / total_cache.max(1) as f32) * 0.02,
                    &format!(
                        "Persisting aggregated state: {} of {}",
                        persisted, total_cache
                    ),
                );
            }
        }

        println!("Cache refreshed: {} packages loaded", cache.len());

        // ─── Atomic swap: the live cache jumps from the previous complete
        // snapshot straight to this new one. Never an empty in-between.
        {
            let mut live = self.package_cache.write().await;
            *live = std::mem::take(&mut cache);
        }

        // ─── Stage 4: prune stale per-source duplicates from the DB ───
        {
            let favorites = self.db.get_favorites().unwrap_or_default();
            let mut keep: Vec<String> = cache.keys().cloned().collect();
            keep.extend(favorites);
            match self.db.prune_packages_except(&keep) {
                Ok(pruned) if pruned > 0 => {
                    eprintln!("DB prune: removed {} stale duplicate rows", pruned);
                    let _ = self.db.optimize();
                }
                Ok(_) => {}
                Err(e) => eprintln!("Warning: DB prune failed: {}", e),
            }
        }
        if let Some(cb) = &progress {
            cb(1.0, "Refresh complete".to_string());
        }

        // Keep the fuzzy search index in lockstep with the refreshed cache.
        self.rebuild_search_index().await;

        Ok(())
    }

    /// Persist one browse adapter's `list_available()` rows into the cache
    /// and database, emitting real per-adapter DB write progress. Extracted
    /// from the refresh loop so non-browse adapters (runtime managers)
    /// can skip the whole block.
    #[allow(clippy::too_many_arguments)]
    async fn browse_adapter_rows(
        &self,
        adapter: &std::sync::Arc<dyn crate::adapters::PackageAdapter>,
        label: &str,
        cache: &mut HashMap<String, Package>,
        window_start: f32,
        window_width: f32,
        emit: &impl Fn(f32, &str),
        rows_written: &mut u64,
    ) {
        let packages = match adapter.list_available().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to list available packages for {:?}: {}",
                    adapter.source(),
                    e
                );
                return;
            }
        };

        let limit = 50000;
        let total = packages.len().min(limit) as f32;
        let mut processed: usize = 0;
        for pkg in packages.into_iter().take(limit) {
            let pkg = self.enrich_metadata(pkg);
            let merged = if let Some(existing) = cache.get(&pkg.identity.id).cloned() {
                self.merge_package_records(existing, pkg)
            } else {
                pkg
            };
            if let Err(e) = self.db.upsert_package(&merged) {
                eprintln!("Warning: Failed to cache package {}: {}", merged.identity.id, e);
            }
            self.persist_catalog_metadata(&merged);
            cache.insert(merged.identity.id.clone(), merged);
            *rows_written += 1;
            processed += 1;
        }
        let _ = total;

        emit(
            window_start + window_width * (0.25 + 0.75),
            &format!("{}: {} browse rows written", label, processed),
        );
    }

    /// Helper to deduplicate packages based on priority order:
    /// 1. Flatpak, 2. AppImage (AM), 3. Snap
    fn deduplicate_packages(&self, packages: impl Iterator<Item = Package>) -> Vec<Package> {
        let mut by_name: HashMap<String, Vec<Package>> = HashMap::new();

        for pkg in packages {
            let mut name = Self::normalized_app_key(&pkg.identity.name);
            if name.is_empty() {
                name = pkg.identity.name.to_lowercase();
            }

            // Use logical_app_id if present, otherwise fallback to cleaned name
            let group_key = pkg.logical_app_id.clone().unwrap_or(name);
            by_name.entry(group_key).or_default().push(pkg);
        }

        let mut result = Vec::new();
        for (_, mut group) in by_name {
            // Check if ANY package in the group is installed
            let any_installed = group.iter().any(|p| p.is_installed);

            // Canonical choice: a row with a real name beats a curational stub,
            // then source priority (Lilith > Flatpak > AM > Soar > Snap), then
            // the row carrying the richest summary.
            group.sort_by_key(|pkg| (
                pkg.identity.name.trim().is_empty(),
                match pkg.identity.source {
                    PackageSource::OfferingsLilith => 0,
                    PackageSource::Flatpak => 1,
                    PackageSource::AppImage => 2,
                    PackageSource::Soar => 3,
                    PackageSource::Snap => 4,
                    PackageSource::Homebrew => 5,
                    PackageSource::GitHubRelease => 6,
                    PackageSource::OfferingsCustom => 7,
                },
                std::cmp::Reverse(pkg.metadata.summary.len()),
            ));

            if !group.is_empty() {
                let mut best = group.remove(0);
                best.is_installed = any_installed; // Aggregate state

                // Wave 15.0: Metadata Aggregation
                // Even if source priority is lower, we want to keep the RICHEST metadata
                for other in &group {
                    // Keep the longest description
                    if other.metadata.description.len() > best.metadata.description.len() {
                        best.metadata.description = other.metadata.description.clone();
                    }
                    // A stub (e.g. a Lilith curated anchor) can win the group
                    // while carrying no displayable name — inherit it.
                    if best.identity.name.trim().is_empty() && !other.identity.name.is_empty() {
                        best.identity.name = other.identity.name.clone();
                    }
                    // Merge categories so a dedup row appears in every menu it
                    // qualifies for across its sources.
                    for cat in &other.metadata.categories {
                        if !best.metadata.categories.contains(cat) {
                            best.metadata.categories.push(cat.clone());
                        }
                    }
                    // Keep screenshots if the "best" source lacks them
                    if best.metadata.screenshots.is_empty()
                        && !other.metadata.screenshots.is_empty()
                    {
                        best.metadata.screenshots = other.metadata.screenshots.clone();
                    }
                    // Keep the most recent update timestamp
                    if other.last_updated > best.last_updated {
                        best.last_updated = other.last_updated;
                    }
                    // Aggregate popularity
                    if other.popularity > best.popularity {
                        best.popularity = other.popularity;
                    }
                    // Use a more descriptive summary if available
                    if best.metadata.summary.len() < 10
                        && other.metadata.summary.len() > best.metadata.summary.len()
                    {
                        best.metadata.summary = other.metadata.summary.clone();
                    }
                }

                // The remaining items in the group are alternatives
                best.alternatives = group.into_iter().map(|p| p.identity).collect();
                result.push(best);
            }
        }

        result.sort_by(|a, b| a.identity.name.cmp(&b.identity.name));
        result
    }

    /// Rebuild the search index from the current package cache. Runs as a
    /// backgroundable sub-task; Nucleo ingests lock-free on its worker pool.
    pub async fn rebuild_search_index(&self) {
        let cache = self.package_cache.read().await;
        let apps: Vec<Package> = cache
            .values()
            .filter(|pkg| pkg.is_app())
            .cloned()
            .collect();
        drop(cache);
        self.search.rebuild(apps);
        eprintln!(
            "Search index rebuilt: {} apps indexed",
            self.search.indexed_count()
        );
    }

    /// Search apps by name or description
    pub async fn search_apps(&self, query: &str) -> Vec<Package> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        // The Nucleo index should always lag at most one refresh behind the
        // cache; if it is missing entirely (never built), build it once, then
        // answer from it.
        if self.search.indexed_count() == 0 {
            self.rebuild_search_index().await;
        }

        self.search.search(query, SEARCH_RESULT_LIMIT)
    }

    /// Get apps by category
    pub async fn get_apps_by_category(&self, category: &str) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        let matches = cache
            .values()
            .filter(|pkg| {
                if !pkg.is_app() {
                    return false;
                }

                // staleness filter: hide truly unmaintained apps from generic browsing
                // especially in large categories, but keep them in search.
                let cat_lower = category.to_lowercase();
                if pkg.is_stale()
                    && (cat_lower == "miscellaneous"
                        || cat_lower == "utilities"
                        || cat_lower == "system")
                {
                    return false;
                }

                if cat_lower == "miscellaneous" {
                    // Return apps with no categories OR categories not in the main sidebar list
                    if pkg.metadata.categories.is_empty() {
                        return true;
                    }

                    let known_categories = vec![
                        "audio",
                        "video",
                        "development",
                        "education",
                        "game",
                        "graphics",
                        "network",
                        "office",
                        "science",
                        "settings",
                        "system",
                        "utilit",
                        "ai",
                        "productivity",
                        "desktop",
                        "security",
                        "lifestyle",
                        "lilith",
                        "communication",
                        "chat",
                        "browser",
                        "monitor",
                        "finance",
                        "android",
                        "comic",
                        "wine",
                        "gnome",
                        "kde",
                        "disk",
                        "file",
                        "email",
                        "internet",
                    ];

                    !pkg.metadata.categories.iter().any(|c| {
                        let c_lower = c.to_lowercase();
                        known_categories
                            .iter()
                            .any(|&known| c_lower.contains(known))
                    })
                } else {
                    let search_terms = match category.to_lowercase().as_str() {
                        "ai" | "ai / machine learning" => {
                            vec!["ai", "artificialintelligence", "machinelearning", "neural"]
                        }
                        "game" | "games" => vec!["game", "gaming", "steam"],
                        "audio" => vec!["audio", "music", "sound", "player", "recorder"],
                        "video" => vec!["video", "movie", "mediaplayer", "stream", "multimedia"],
                        "development" => vec![
                            "development",
                            "coding",
                            "ide",
                            "programming",
                            "git",
                            "compiler",
                            "debugger",
                            "versioncontrol",
                        ],
                        "graphics" => vec![
                            "graphics", "image", "photo", "drawing", "design", "paint",
                            "illustration", "raster", "vector",
                        ],
                        "network" => {
                            // Intentionally exclude 'browser' and 'internet' here - those have their own categories
                            // and would incorrectly pull email/browser apps into Network
                            vec!["network", "remote", "ftp", "ssh", "vpn", "filesharing", "torrent"]
                        }
                        "communication" | "chat" | "social" => vec![
                            "communication",
                            "chat",
                            "messenger",
                            "instantmessaging",
                            "social",
                            "telegram",
                            "discord",
                            "whatsapp",
                            "signal",
                            "matrix",
                            "email",
                            "mail",
                        ],
                        "web-browser" | "browser" | "browsers" => vec![
                            "browser", "webbrowser", "firefox", "chrome", "chromium", "opera",
                            "brave", "vivaldi", "epiphany",
                        ],
                        "system-monitor" | "monitor" => vec![
                            "monitor",
                            "systemmonitor",
                            "performance",
                            "taskmanager",
                            "cpu",
                            "memory",
                            "usage",
                            "htop",
                            "btop",
                        ],
                        "password" | "security" | "security & privacy" => vec![
                            "security",
                            "privacy",
                            "password",
                            "encryption",
                            "firewall",
                            "vpn",
                            "bitwarden",
                            "keepass",
                            "auth",
                        ],
                        "utilities" | "utility" | "tools" => vec![
                            "utility",
                            "utilities",
                            "tool",
                            "accessory",
                            "calculator",
                            "archive",
                        ],
                        "productivity" => vec![
                            "productivity",
                            "task",
                            "notes",
                            "calendar",
                            "time",
                            "todo",
                            "kanban",
                            "planner",
                        ],
                        "office" => vec![
                            "office",
                            "document",
                            "spreadsheet",
                            "wordprocessor",
                            "writer",
                            "pdf",
                        ],
                        "science" => vec![
                            "science",
                            "math",
                            "laboratory",
                            "research",
                            "physics",
                            "biology",
                            "chemistry",
                            "astronomy",
                        ],
                        "education" => vec![
                            "education",
                            "learning",
                            "student",
                            "teacher",
                            "school",
                            "quiz",
                            "tutor",
                        ],
                        "lifestyle" => vec![
                            "lifestyle",
                            "personal",
                            "cooking",
                            "travel",
                            "health",
                            "fitness",
                        ],
                        "system" => vec![
                            "system", "settings", "administration", "admin", "config",
                        ],
                        "desktop-customization" | "desktop" => vec![
                            "desktop",
                            "theme",
                            "icon",
                            "wallpaper",
                            "shell",
                        ],
                        "finance" => vec![
                            "finance",
                            "money",
                            "accounting",
                            "crypto",
                            "wallet",
                            "budget",
                        ],
                        "android" => vec!["android", "mobile", "adb", "scrcpy"],
                        "lilith" => {
                            // Packages tagged "Lilith" in their metadata categories
                            // (set by LilithCatalogAdapter on packages it lists)
                            vec!["lilith"]
                        }
                        _ => vec![category],
                    };

                    // Match ONLY on metadata categories — never on name/summary.
                    // Matching on name/summary caused severe miscategorization:
                    // "ai" matched "FastMAIL", "GAIa Sky", "KAIdan", "IaIto", etc.
                    // Category membership is determined solely by FreeDesktop categories metadata.
                    pkg.metadata.categories.iter().any(|c| {
                        let c_lower = c.to_lowercase();
                        search_terms
                            .iter()
                            .any(|&term| c_lower.contains(&term.to_lowercase()))
                    })
                }
            })
            .cloned();

        let mut result: Vec<Package> = matches.collect();
        // The cache itself is already deduplicated (one canonical row per app
        // with multi-source alternatives), so no per-query group scan.

        let cat_lower_fallback = category.to_lowercase();

        // ── Lilith: also look up curated showcase IDs directly from the cache ──
        // The metadata-tag search above finds packages only once the Lilith adapter
        // has populated the cache with them. On first run or after a timeout the
        // in-memory cache might hold the same packages under their primary source ID
        // (e.g. "flatpak:io.unobserved.espansoGUI") without the "Lilith" tag.
        // We look them up by ID directly to guarantee results in all cases.
        if cat_lower_fallback == "lilith" {
            let cache = self.package_cache.read().await;
            let showcase_ids: Vec<String> = self
                .home_content
                .category_showcases
                .get("Lilith")
                .cloned()
                .unwrap_or_default();
            for id in &showcase_ids {
                // Try exact ID match first
                if let Some(pkg) = cache.get(id) {
                    if !result.iter().any(|p| p.identity.id == pkg.identity.id) {
                        result.push(pkg.clone());
                    }
                    continue;
                }
                // Also try the short ID (the part after the colon prefix)
                let short = id.split(':').nth(1).unwrap_or(id.as_str());
                if let Some(pkg) = cache.values().find(|p| {
                    p.identity.id == *id
                        || p.identity.id.ends_with(short)
                        || p.short_id() == short
                }) {
                    if !result.iter().any(|p| p.identity.id == pkg.identity.id) {
                        result.push(pkg.clone());
                    }
                }
            }
        }

        // ── Generic fallback: if still under 10 results, add showcase packages ──
        if result.len() < 10 && cat_lower_fallback != "miscellaneous" && cat_lower_fallback != "lilith" {
            let showcase_key = if cat_lower_fallback == "game" || cat_lower_fallback == "games" {
                "Games"
            } else {
                category
            };
            if let Some(featured_ids) = self.home_content.category_showcases.get(showcase_key) {
                let cache = self.package_cache.read().await;
                for id in featured_ids {
                    if let Some(pkg) = cache.get(id) {
                        if !result.iter().any(|p| p.identity.id == pkg.identity.id) {
                            result.push(pkg.clone());
                        }
                    }
                }
            }
        }

        // Wave 15.0: Sort by discovery score (Popularity + Recency + Quality)
        result.sort_by(|a, b| {
            let score_a = self.calculate_discovery_score(a);
            let score_b = self.calculate_discovery_score(b);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        eprintln!(
            "Category '{}': {} packages found (sorted by discovery score)",
            category,
            result.len()
        );

        result
    }

    // ==================== Source Management ====================

    /// Add a custom source
    pub async fn add_source(
        &self,
        name: String,
        url: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Save to DB
        self.db.add_source(&name, &url)?;

        // Update CustomAdapter remotes
        for adapter in &self.adapters {
            // Find the CustomAdapter and cast it (or just rely on the next refresh)
            // For now, simpler to just trigger a refresh which will reload from DB if I change the constructor
            // But I'll just find it and add it
            if adapter.source() == PackageSource::OfferingsCustom {
                // This is a bit hacky but works since we only have one CustomAdapter
                // Actually, I can't easily downcast Arc<dyn PackageAdapter> to CustomAdapter here without more traits
            }
        }

        // Best way: just trigger a partial refresh or wait for the next full refresh
        // For immediate feedback, we should at least trigger a background sync for this new source
        self.refresh_cache().await?;

        Ok(())
    }

    /// Remove a custom source
    pub async fn remove_source(
        &self,
        url: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db.remove_source(&url)?;
        self.refresh_cache().await?;
        Ok(())
    }

    /// Get all custom sources
    pub fn get_sources(&self) -> Vec<crate::model::SourceItem> {
        self.db.get_sources().unwrap_or_default()
    }

    // ==================== Favorites / Saved for Later ====================

    /// Check if a package is in the user's favorites (Saved section)
    pub fn is_favorite(&self, package_id: &str) -> bool {
        self.db.is_favorite(package_id)
    }

    /// Get all favorited package ids (most recently saved first)
    pub fn get_favorite_ids(&self) -> Vec<String> {
        self.db.get_favorites().unwrap_or_default()
    }

    /// Set favorite state for a package.
    pub fn set_favorite(
        &self,
        package_id: &str,
        name: &str,
        favorite: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if favorite {
            self.db.add_favorite(package_id, name)?;
            Ok(true)
        } else {
            self.db.remove_favorite(package_id)?;
            Ok(false)
        }
    }

    /// Direct Release: accept ANY GitHub repo or URL, resolve its latest
    /// release, pick the right asset for this machine, remember the entry
    /// (manifest), and install through the normal operation pipeline (so
    /// real byte-level download progress and transaction logging still run).
    pub async fn install_direct_release(
        &self,
        repo_or_url: &str,
    ) -> Result<OperationResult, Box<dyn std::error::Error + Send + Sync>> {
        let github = self
            .adapters
            .iter()
            .find_map(|a| a.as_any().downcast_ref::<crate::adapters::GitHubReleaseAdapter>())
            .ok_or_else(|| "Direct Release source unavailable".to_string())?;

        let pkg = github.add_repo(repo_or_url).await?;
        let pkg_id = pkg.identity.id.clone();

        self.execute_operation(PackageOperation::Install(pkg_id), None)
            .await
    }

    /// Get full Package objects for every favorited id
    pub async fn get_favorite_packages(
        &self,
    ) -> Vec<Package> {
        let favorites = self.db.get_favorites().unwrap_or_default();
        let mut result = Vec::new();
        let mut missing: Vec<String> = Vec::new();

        {
            let cache = self.package_cache.read().await;
            for id in &favorites {
                if let Some(pkg) = cache.get(id) {
                    result.push(pkg.clone());
                } else {
                    missing.push(id.clone());
                }
            }
        }

        // Not in cache (e.g. first run): pull from DB directly.
        for id in missing {
            if let Ok(Some(pkg)) = self.db.get_package(&id) {
                result.push(self.enrich_metadata(pkg));
            }
        }

        result
    }

    /// Get all installed packages
    pub async fn get_installed_packages(&self) -> Vec<Package> {
        let cache = self.package_cache.read().await;
        // Cache rows are canonical (deduped at refresh); installed state has
        // already been merged across sources, so a plain filter suffices.
        cache.values().filter(|pkg| pkg.is_installed).cloned().collect()
    }

    /// Get packages with available updates
    pub async fn get_updates(&self) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        cache
            .values()
            .filter(|pkg| pkg.is_installed && pkg.version.has_update())
            .cloned()
            .collect()
    }

    /// Get home page content
    pub async fn get_home_content(&self) -> HomePageContent {
        self.home_content.clone()
    }

    /// Check for updates in the background and persist to DB
    pub async fn check_for_updates_background(&self) {
        let adapters = self.adapters.clone();
        let db = self.db.clone();
        let cache_lock = self.package_cache.clone();

        tokio::spawn(async move {
            eprintln!("Wave 12.0: Starting background update check...");
            for adapter in adapters {
                if !adapter.is_available().await {
                    continue;
                }

                match adapter.check_updates().await {
                    Ok(updates) => {
                        let mut cache = cache_lock.write().await;
                        for update in updates {
                            // Update both DB and in-memory cache
                            if let Err(e) = db.upsert_package(&update) {
                                eprintln!(
                                    "Error persisting update for {}: {}",
                                    update.identity.id, e
                                );
                            }
                            let _ = db.upsert_metadata_entry(
                                &crate::catalog::CatalogEntry::from_package(&update),
                            );
                            cache.insert(update.identity.id.clone(), update);
                        }
                    }
                    Err(e) => eprintln!("Update check failed for {:?}: {}", adapter.source(), e),
                }
            }
            eprintln!("Wave 12.0: Background update check complete.");
        });
    }

    /// Get all variants of a logical app
    pub async fn get_package_variants(&self, logical_id: &str) -> Vec<Package> {
        let cache = self.package_cache.read().await;
        cache
            .values()
            .filter(|p| {
                p.logical_app_id.as_ref().is_some_and(|id| id == logical_id) ||
                // Fallback: if logical_id is just a cleaned name, match on identity
                p.identity.name.to_lowercase().contains(logical_id)
            })
            .cloned()
            .collect()
    }

    /// Internal helper to enrich package metadata with fallbacks
    fn enrich_metadata(&self, mut pkg: Package) -> Package {
        if let Some(entry) = self
            .metadata_catalog
            .read()
            .ok()
            .and_then(|catalog| catalog.find_for_package(&pkg).cloned())
        {
            if pkg.metadata.summary.trim().is_empty() {
                if let Some(summary) = &entry.summary {
                    pkg.metadata.summary = summary.clone();
                }
            }
            if Self::has_placeholder_description(&pkg) {
                if let Some(description) = &entry.description {
                    if !description.trim().is_empty() {
                        pkg.metadata.description = description.clone();
                    }
                }
            }
            if pkg
                .metadata
                .icon_url
                .as_ref()
                .is_none_or(|url| url.trim().is_empty())
            {
                if let Some(icon_url) = &entry.icon_url {
                    pkg.metadata.icon_url = Some(icon_url.clone());
                }
            }
            if pkg.metadata.screenshots.is_empty() && !entry.screenshots.is_empty() {
                pkg.metadata.screenshots = entry.screenshots.clone();
            }
            if pkg.metadata.homepage_url.is_none() {
                pkg.metadata.homepage_url = entry.homepage_url.clone();
            }
            if pkg.metadata.documentation_url.is_none() {
                pkg.metadata.documentation_url = entry.documentation_url.clone();
            }
            if pkg.metadata.categories.is_empty() && !entry.categories.is_empty() {
                pkg.metadata.categories = entry.categories.clone();
            }
            if pkg.metadata.rating.is_none() {
                pkg.metadata.rating = entry.rating;
            }
            if pkg.popularity <= 0.0 {
                if let Some(popularity) = entry.popularity {
                    pkg.popularity = popularity;
                }
            }
            if pkg.last_updated == 0 {
                if let Some(last_updated) = entry.last_updated {
                    pkg.last_updated = last_updated;
                }
            }
            if pkg.logical_app_id.is_none() {
                pkg.logical_app_id = entry.logical_id.clone();
            }
        }

        // Flatpak metadata enrichment
        if pkg.identity.source == PackageSource::Flatpak {
            let app_id = pkg
                .identity
                .id
                .strip_prefix("flatpak:")
                .unwrap_or(&pkg.identity.id);

            // Fallback icon (Flathub 128x128)
            if pkg.metadata.icon_url.is_none()
                || pkg.metadata.icon_url.as_ref().is_none_or(|u| u.is_empty())
            {
                // Try common icon locations
                pkg.metadata.icon_url = Some(format!(
                    "https://dl.flathub.org/repo/appstream/x86_64/icons/128x128/{}.png",
                    app_id
                ));
            }

            // Fallback screenshots (Standard AppStream naming)
            if pkg.metadata.screenshots.is_empty() {
                pkg.metadata.screenshots = vec![
                    format!(
                        "https://dl.flathub.org/repo/appstream/x86_64/screenshots/{}-1.png",
                        app_id
                    ),
                    format!(
                        "https://dl.flathub.org/repo/appstream/x86_64/screenshots/{}-2.png",
                        app_id
                    ),
                    format!(
                        "https://dl.flathub.org/repo/appstream/x86_64/screenshots/{}-3.png",
                        app_id
                    ),
                ];
            }

            if pkg
                .metadata
                .homepage_url
                .as_ref()
                .is_none_or(|url| url.trim().is_empty())
            {
                pkg.metadata.homepage_url = Some(format!("https://flathub.org/apps/{}", app_id));
            }
        }

        // Clean up descriptions
        if pkg
            .metadata
            .summary
            .to_lowercase()
            .ends_with(" application")
        {
            pkg.metadata.summary =
                pkg.metadata.summary[..pkg.metadata.summary.len() - 12].to_string();
        }

        if Self::has_placeholder_description(&pkg)
            && !pkg.metadata.summary.trim().is_empty()
            && pkg.metadata.summary.trim().to_lowercase() != pkg.identity.name.trim().to_lowercase()
        {
            pkg.metadata.description = pkg.metadata.summary.clone();
        }

        // Wave 12.0: Assign logical_app_id if not present
        if pkg.logical_app_id.is_none() {
            let mut name = pkg.identity.name.to_lowercase();
            name = name.split(" - ").next().unwrap_or(&name).to_string();
            name = name.split(" (").next().unwrap_or(&name).to_string();
            name = name.replace("web browser", "").trim().to_string();
            pkg.logical_app_id = Some(name);
        }

        pkg
    }

    /// Wave 15.0: Calculate a discovery score to prioritize high-quality/recent apps
    fn calculate_discovery_score(&self, pkg: &Package) -> f32 {
        let mut score = 0.0;

        // 1. Popularity (0.0 to 1.0) -> Weighted 50%
        score += pkg.popularity * 0.5;

        // 2. Recency -> Weighted 30%
        if pkg.last_updated > 0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            let age_days = (now - pkg.last_updated) as f32 / (24.0 * 3600.0);

            if age_days < 90.0 {
                // Last 3 months
                score += 0.3;
            } else if age_days < 365.0 {
                // Last year
                score += 0.2;
            } else if age_days < 730.0 {
                // Last 2 years
                score += 0.1;
            }
        }

        // 3. Metadata Quality -> Weighted 20%
        if !pkg.metadata.screenshots.is_empty() {
            score += 0.1;
        }
        if pkg.metadata.description.len() > 300 {
            score += 0.05;
        }
        if pkg.metadata.icon_url.is_some() {
            score += 0.05;
        }

        // 4. Source Priority
        match pkg.identity.source {
            PackageSource::OfferingsLilith => score += 0.2, // Curated apps get a boost
            PackageSource::Flatpak => score += 0.05,
            _ => {}
        }

        score
    }

    /// Get detailed package info
    pub async fn get_package_detail(&self, package_id: &str) -> Option<AppDetailInfo> {
        let cache = self.package_cache.read().await;

        if let Some(pkg) = cache.get(package_id) {
            let mut info = AppDetailInfo::from(pkg.clone());

            // Find alternatives by logical_app_id or name
            let variants = if let Some(logical_id) = &pkg.logical_app_id {
                self.get_package_variants(logical_id).await
            } else {
                let name_lower = pkg.identity.name.to_lowercase();
                cache
                    .values()
                    .filter(|p| {
                        p.identity.name.to_lowercase() == name_lower
                            && p.identity.id != pkg.identity.id
                    })
                    .cloned()
                    .collect()
            };

            let mut alternatives = Vec::new();
            for alt_pkg in variants {
                if alt_pkg.identity.id != pkg.identity.id {
                    alternatives.push(alt_pkg.identity.clone());
                }
            }

            alternatives.sort_by_key(|id| match id.source {
                PackageSource::OfferingsLilith => 0,
                PackageSource::Flatpak => 1,
                PackageSource::AppImage => 2,
                PackageSource::Soar => 3,
                PackageSource::Snap => 4,
                PackageSource::Homebrew => 5,
                PackageSource::GitHubRelease => 6,
                PackageSource::OfferingsCustom => 7,
            });

            info.alternatives = alternatives;
            Some(info)
        } else {
            None
        }
    }

    pub async fn get_package(&self, package_id: &str) -> Option<Package> {
        // 1. Check cache first
        let mut needs_refresh = false;
        {
            let cache = self.package_cache.read().await;
            if let Some(pkg) = cache.get(package_id).cloned() {
                // Return immediately if it has rich metadata or isn't Flatpak
                // Wait, actually snap also might have empty description in list_available
                if !pkg.metadata.description.is_empty() {
                    return Some(self.enrich_package_with_alternatives(pkg, &cache));
                }
                needs_refresh = true;
            }
        }

        // 2. Not in cache or needs refresh? Try identifying source and fetching from adapter
        let source = PackageSource::from_id(package_id);
        if let Some(&adapter_idx) = self.adapter_map.get(&source) {
            if let Ok(Some(pkg)) = self.adapters[adapter_idx].get_package(package_id).await {
                let pkg = self.enrich_metadata(pkg);
                // Upsert to DB and cache
                let _ = self.db.upsert_package(&pkg);
                self.persist_catalog_metadata(&pkg);
                let mut cache = self.package_cache.write().await;
                cache.insert(package_id.to_string(), pkg.clone());
                return Some(self.enrich_package_with_alternatives(pkg, &cache));
            }
        }

        // 3. If refresh failed but we had a cached version, return the cached one
        if needs_refresh {
            let cache = self.package_cache.read().await;
            if let Some(pkg) = cache.get(package_id).cloned() {
                return Some(self.enrich_package_with_alternatives(pkg, &cache));
            }
        }

        None
    }

    /// Helper to find and add alternatives to a package
    fn enrich_package_with_alternatives(
        &self,
        mut pkg: Package,
        cache: &HashMap<String, Package>,
    ) -> Package {
        let name_lower = pkg.identity.name.to_lowercase();
        let mut alternatives = Vec::new();
        let mut installed_variant: Option<Package> = None;
        let package_keys = Self::package_group_keys(&pkg);
        for alt_pkg in cache.values() {
            let same_group = alt_pkg.identity.id != pkg.identity.id
                && (alt_pkg.identity.name.to_lowercase() == name_lower
                    || Self::package_group_keys(alt_pkg)
                        .iter()
                        .any(|key| package_keys.contains(key)));

            if same_group {
                alternatives.push(alt_pkg.identity.clone());
                if alt_pkg.is_installed && installed_variant.is_none() {
                    installed_variant = Some(alt_pkg.clone());
                }
            }
        }

        // Sort alternatives by priority
        alternatives.sort_by_key(|id| match id.source {
            PackageSource::Flatpak => 1,
            PackageSource::AppImage => 2,
            PackageSource::Soar => 3,
            PackageSource::Snap => 4,
            PackageSource::Homebrew => 5,
            PackageSource::GitHubRelease => 6,
            PackageSource::OfferingsCustom => 7,
            PackageSource::OfferingsLilith => 0, // Lilith is usually highest
        });

        pkg.alternatives = alternatives;
        if let Some(installed_variant) = installed_variant {
            pkg.is_installed = true;
            if pkg.version.installed.is_none() {
                pkg.version.installed = installed_variant.version.installed.clone();
            }
        }
        pkg
    }

    /// Get packages by source
    pub async fn get_packages_by_source(&self, source: PackageSource) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        cache
            .values()
            .filter(|pkg| pkg.identity.source == source)
            .cloned()
            .collect()
    }

    /// Get dependency identifiers for a package from its source adapter.
    pub async fn get_dependencies(
        &self,
        package_id: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let adapter = self.find_adapter_for_package(package_id).await?;
        adapter.get_dependencies(package_id).await
    }

    /// Execute a package operation
    pub async fn execute_operation(
        &self,
        operation: PackageOperation,
        progress_callback: Option<crate::adapters::ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn std::error::Error + Send + Sync>> {
        // Bound the operation pool: at most MAX_CONCURRENT_OPERATIONS package
        // operations run at once, the rest queue up (UI shows them as rows).
        let _permit = self
            .operation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;

        // One operation per package at any time — double-clicks must not start
        // two competing installs/uninstalls of the same app.
        let busy_key = match &operation {
            PackageOperation::UpdateAll => "update-all".to_string(),
            other => other.package_id().unwrap_or_default().to_string(),
        };        {
            let mut busy = self.busy_packages.lock().unwrap();
            if !busy.insert(busy_key.clone()) {
                return Err(format!(
                    "'{}' is already being processed; wait for it to finish first.",
                    busy_key
                )
                .into());
            }
        }
        // RAII safety: always release the package slot, whatever happens.
        let busy_slot = BusyRelease {
            set: self.busy_packages.clone(),
            key: busy_key,
        };

        // Get the package for transaction logging
        let package = if let Some(pkg_id) = operation.package_id() {
            self.get_package(pkg_id).await
        } else {
            None
        };

        // Execute within transaction
        let result = self
            .transaction_manager
            .execute(&operation, package.as_ref(), || async {
                match &operation {
                    PackageOperation::Install(pkg_id) => {
                        let adapter = self.find_adapter_for_package(pkg_id).await?;
                        let original_source = adapter.source().label().to_string();
                        let mut result =
                            adapter.install_with_progress(pkg_id, progress_callback).await?;
                        let mut used_fallback = false;
                        let mut fallback_source = String::new();

                        if !result.success {
                            // Fallback logic: try alternatives
                            if let Some(pkg) = &package {
                                for alt in &pkg.alternatives {
                                    if let Ok(alt_adapter) = self.find_adapter_for_package(&alt.id).await {
                                        if let Ok(alt_result) = alt_adapter.install(&alt.id).await {
                                            if alt_result.success {
                                                result = alt_result;
                                                used_fallback = true;
                                                fallback_source = alt.source.label().to_string();
                                                result.message = format!("Primary installation failed, but fallback to {} succeeded: {}", alt.id, result.message);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if result.success {
                            let confirmed = self.reconcile_package_state(pkg_id, true).await;
                            if used_fallback {
                                if confirmed.is_some() {
                                    self.notifications
                                        .notify(NotificationType::FallbackSuccess {
                                            package_name: pkg_id.clone(),
                                            source: fallback_source,
                                            original_source,
                                        })
                                        .map_err(
                                            |e| -> Box<dyn std::error::Error + Send + Sync> {
                                                e.to_string().into()
                                            },
                                        )?;
                                }
                            } else if confirmed.is_some() {
                                self.notifications.notify_install(pkg_id, true);
                            }

                            if confirmed.is_none() {
                                result.message.push_str(
                                    " Install completed, but package state could not be confirmed yet.",
                                );
                            }
                        } else {
                            self.notifications.notify_error("install", pkg_id, &result.message);
                        }

                        Ok(result)
                    }
                    PackageOperation::Update(pkg_id) => {
                        let adapter = self.find_adapter_for_package(pkg_id).await?;
                        let result = adapter.update_with_progress(pkg_id, progress_callback).await?;

                        if result.success {
                            let _ = self.reconcile_package_state(pkg_id, true).await;
                            if let Some(pkg) = &package {
                                let old = pkg.version.installed.as_deref().unwrap_or("unknown");
                                let new = pkg.version.latest.as_deref().unwrap_or("unknown");
                                self.notifications.notify_update(&pkg.identity.name, old, new);
                            }
                        }

                        Ok(result)
                    }
                    PackageOperation::Uninstall(pkg_id) => {
                        let adapter = self.find_adapter_for_package(pkg_id).await?;
                        let result = adapter
                            .uninstall_with_progress(pkg_id, progress_callback)
                            .await?;

                        if result.success {
                            if self.reconcile_package_state(pkg_id, false).await.is_some() {
                                self.notifications.notify_uninstall(pkg_id);
                            }
                        } else {
                            self.notifications
                                .notify_error("uninstall", pkg_id, &result.message);
                        }

                        Ok(result)
                    }
                    PackageOperation::UpdateAll => {
                        let mut total_updated = Vec::new();
                        let mut failed = 0;

                        for adapter in &self.adapters {
                            if !adapter.is_available().await {
                                continue;
                            }

                            let updates = adapter.check_updates().await?;
                            for pkg in updates {
                                let result = adapter.update(&pkg.identity.id).await?;
                                if result.success {
                                    total_updated.extend(result.updated_packages);
                                } else {
                                    failed += 1;
                                }
                            }
                        }

                        self.notifications.notify(NotificationType::BatchComplete {
                            operation: "updated".to_string(),
                            count: total_updated.len(),
                            failed,
                        }).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;

                        Ok(OperationResult {
                            success: failed == 0,
                            message: format!("Updated {} packages, {} failed", total_updated.len(), failed),
                            updated_packages: total_updated,
                        })
                    }
                }
            })
            .await?;

        // NOTE: no cache-wide refresh here — reconcile_package_state already
        // updated the package row in the DB and in-memory cache, and doing a
        // full catalogue rescan after every op kept the progress overlay on
        // screen for ages after completion. The UI layer schedules a
        // background refresh from its Arc<BackendService> instead.
        drop(busy_slot);
        Ok(result)
    }

    /// Find the appropriate adapter for a package
    async fn find_adapter_for_package(
        &self,
        pkg_id: &str,
    ) -> Result<Arc<dyn PackageAdapter>, Box<dyn std::error::Error + Send + Sync>> {
        // Try to get from cache first
        let cache = self.package_cache.read().await;

        if let Some(pkg) = cache.get(pkg_id) {
            if let Some(&idx) = self.adapter_map.get(&pkg.identity.source) {
                return Ok(self.adapters[idx].clone());
            }
        }

        // Parse package ID to determine source
        let source = if pkg_id.starts_with("flatpak:") {
            PackageSource::Flatpak
        } else if pkg_id.starts_with("appimage:") || pkg_id.starts_with("am:") {
            PackageSource::AppImage
        } else if pkg_id.starts_with("soar:") {
            PackageSource::Soar
        } else if pkg_id.starts_with("snap:") {
            PackageSource::Snap
        } else if pkg_id.starts_with("brew:") {
            PackageSource::Homebrew
        } else if pkg_id.starts_with("github:") {
            PackageSource::GitHubRelease
        } else if pkg_id.starts_with("custom:") {
            PackageSource::OfferingsCustom
        } else {
            // Default to Flathub if not specified
            PackageSource::Flatpak
        };

        if let Some(&idx) = self.adapter_map.get(&source) {
            Ok(self.adapters[idx].clone())
        } else {
            Err("Adapter not found".into())
        }
    }

    /// Check for updates across all sources
    pub async fn check_all_updates(&self) -> Vec<Package> {
        let mut all_updates = Vec::new();

        for adapter in &self.adapters {
            if !adapter.is_available().await {
                continue;
            }

            if let Ok(updates) = adapter.check_updates().await {
                all_updates.extend(updates);
            }
        }

        // Notify if there are updates
        if !all_updates.is_empty() {
            self.notifications
                .notify_updates_available(all_updates.len());
        }

        all_updates
    }

    /// Get available sources on this system
    pub async fn get_available_sources(&self) -> Vec<PackageSource> {
        let mut sources = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for adapter in &self.adapters {
            if adapter.is_available().await {
                let source = adapter.source();
                if seen.insert(source.clone()) {
                    sources.push(source);
                }
            }
        }

        sources
    }

    /// Start a background task that periodically refreshes the package cache
    /// This checks for new apps, removed apps, and version updates
    pub fn start_background_refresh(
        self: &Arc<Self>,
        interval_secs: u64,
    ) -> Arc<tokio::task::JoinHandle<()>> {
        let backend = self.clone();

        let handle = tokio::spawn(async move {
            let duration = tokio::time::Duration::from_secs(interval_secs);

            loop {
                tokio::time::sleep(duration).await;

                // Perform a full cache refresh
                if let Err(e) = backend.refresh_cache().await {
                    eprintln!("Background refresh failed: {}", e);
                } else {
                    // Check for updates after refresh
                    let updates = backend.check_all_updates().await;
                    if !updates.is_empty() {
                        backend
                            .notifications
                            .notify_updates_available(updates.len());
                    }
                }
            }
        });

        Arc::new(handle)
    }

    pub fn export_metadata_catalog(
        &self,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut entries = self.db.load_metadata_catalog()?;
        entries.sort_by(|a, b| {
            a.package_id
                .as_deref()
                .unwrap_or_default()
                .cmp(b.package_id.as_deref().unwrap_or_default())
        });
        let json = serde_json::to_string_pretty(&crate::catalog::UnifiedCatalog { entries })?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn import_metadata_catalog(
        &self,
        path: &Path,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let catalog: crate::catalog::UnifiedCatalog = serde_json::from_str(&content)?;
        let count = catalog.entries.len();
        for entry in catalog.entries {
            self.db.upsert_metadata_entry(&entry)?;
            if let Ok(mut loaded) = self.metadata_catalog.write() {
                loaded.merge_entry(entry);
            }
        }
        Ok(count)
    }

    /// Get current package count for change detection
    pub async fn get_package_count(&self) -> usize {
        let cache = self.package_cache.read().await;
        cache.len()
    }

    /// Launch a package using its adapter
    pub async fn launch_package(
        &self,
        package_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let adapter = self.find_adapter_for_package(package_id).await?;
        adapter.launch(package_id).await
    }
}

/// RAII guard that removes a package from the busy-set when its operation ends
/// (success, failure or error path). Guarantees a later click can retry.
struct BusyRelease {
    set: Arc<Mutex<std::collections::HashSet<String>>>,
    key: String,
}

impl Drop for BusyRelease {
    fn drop(&mut self) {
        self.set.lock().unwrap().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_backend_creation() {
        let backend = BackendService::new(HomePageContent::default());
        assert!(backend.is_ok(), "backend init failed: {:?}", backend.err());
    }
}
