// src/backend.rs - Core Backend Service
use crate::adapters::{
    AptAdapter, AppImageAdapter, CustomAdapter, FlatpakAdapter, GitHubAdapter, PackageAdapter,
    SnapAdapter, SoarAdapter,
};
use crate::db::Database;
use crate::depgraph::DependencyGraph;
use crate::model::{
    AppDetailInfo, HomePageContent, OperationResult, Package, PackageOperation, PackageSource,
};
use crate::notifications::{NotificationManager, NotificationType};
use crate::transaction::TransactionManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Core backend service managing all package operations
pub struct BackendService {
    adapters: Vec<Arc<dyn PackageAdapter>>,
    adapter_map: HashMap<PackageSource, usize>,
    package_cache: Arc<RwLock<HashMap<String, Package>>>,
    db: Arc<Database>,
    transaction_manager: Arc<TransactionManager>,
    dep_graph: Arc<RwLock<DependencyGraph>>,
    notifications: Arc<NotificationManager>,
    home_content: HomePageContent,
}

impl BackendService {
    /// Create a new backend service with all available adapters
    pub fn new(home_content: HomePageContent) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Arc::new(Database::new()?);
        let transaction_manager = Arc::new(TransactionManager::new(db.clone()));
        let dep_graph = Arc::new(RwLock::new(DependencyGraph::new(db.clone())));
        let notifications = Arc::new(NotificationManager::new(Default::default()));

        let adapters: Vec<Arc<dyn PackageAdapter>> = vec![
            Arc::new(AptAdapter::new()),
            Arc::new(FlatpakAdapter::new()),
            Arc::new(SnapAdapter::new()),
            Arc::new(AppImageAdapter::new()),
            Arc::new(SoarAdapter::new()),
            Arc::new(GitHubAdapter::new()),
            Arc::new(CustomAdapter::new()),
        ];

        let mut adapter_map = HashMap::new();
        for (i, adapter) in adapters.iter().enumerate() {
            adapter_map.insert(adapter.source(), i);
        }

        Ok(Self {
            adapters,
            adapter_map,
            package_cache: Arc::new(RwLock::new(HashMap::new())),
            db,
            transaction_manager,
            dep_graph,
            notifications,
            home_content,
        })
    }

    /// Create backend with custom adapters (for testing)
    pub fn with_adapters(
        adapters: Vec<Arc<dyn PackageAdapter>>,
        home_content: HomePageContent,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Arc::new(Database::new()?);
        let transaction_manager = Arc::new(TransactionManager::new(db.clone()));
        let dep_graph = Arc::new(RwLock::new(DependencyGraph::new(db.clone())));
        let notifications = Arc::new(NotificationManager::new(Default::default()));

        let mut adapter_map = HashMap::new();
        for (i, adapter) in adapters.iter().enumerate() {
            adapter_map.insert(adapter.source(), i);
        }

        Ok(Self {
            adapters,
            adapter_map,
            package_cache: Arc::new(RwLock::new(HashMap::new())),
            db,
            transaction_manager,
            dep_graph,
            notifications,
            home_content,
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

    /// Refresh cache from all available adapters
    pub async fn refresh_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut cache = self.package_cache.write().await;
        cache.clear();

        for adapter in &self.adapters {
            // Check if adapter is available on this system
            if !adapter.is_available().await {
                continue;
            }

            // Refresh adapter's cache
            if let Err(e) = adapter.refresh_cache().await {
                eprintln!("Warning: Failed to refresh cache for {:?}: {}", adapter.source(), e);
            }

            // Get installed packages
            match adapter.list_installed().await {
                Ok(packages) => {
                    for pkg in packages {
                        // Store in database
                        if let Err(e) = self.db.upsert_package(&pkg) {
                            eprintln!("Warning: Failed to cache package {}: {}", pkg.identity.id, e);
                        }
                        cache.insert(pkg.identity.id.clone(), pkg);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to list installed packages for {:?}: {}", adapter.source(), e);
                }
            }

            // Get available packages (limited for performance)
            match adapter.list_available().await {
                Ok(packages) => {
                    for pkg in packages.into_iter().take(500) {
                        if !cache.contains_key(&pkg.identity.id) {
                            if let Err(e) = self.db.upsert_package(&pkg) {
                                eprintln!("Warning: Failed to cache package {}: {}", pkg.identity.id, e);
                            }
                            cache.insert(pkg.identity.id.clone(), pkg);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to list available packages for {:?}: {}", adapter.source(), e);
                }
            }
        }

        // Rebuild dependency graph
        let mut dep_graph = self.dep_graph.write().await;
        if let Err(e) = dep_graph.build() {
            eprintln!("Warning: Failed to build dependency graph: {}", e);
        }

        Ok(())
    }

    /// Search apps by name or description
    pub async fn search_apps(&self, query: &str) -> Vec<Package> {
        let cache = self.package_cache.read().await;
        let query_lower = query.to_lowercase();

        cache
            .values()
            .filter(|pkg| {
                pkg.is_app()
                    && (pkg.identity.name.to_lowercase().contains(&query_lower)
                        || pkg.metadata.summary.to_lowercase().contains(&query_lower))
            })
            .cloned()
            .collect()
    }

    /// Get apps by category
    pub async fn get_apps_by_category(&self, category: &str) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        cache
            .values()
            .filter(|pkg| pkg.is_app() && pkg.metadata.categories.contains(&category.to_string()))
            .cloned()
            .collect()
    }

    /// Get all installed packages
    pub async fn get_installed_packages(&self) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        cache.values().filter(|pkg| pkg.is_installed).cloned().collect()
    }

    /// Get installed dependencies
    pub async fn get_installed_dependencies(&self) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        let mut deps: Vec<Package> = cache
            .values()
            .filter(|pkg| pkg.is_installed && pkg.is_dependency())
            .cloned()
            .collect();

        deps.sort_by(|a, b| a.identity.name.cmp(&b.identity.name));
        deps
    }

    /// Get all APT packages
    pub async fn get_all_apt_packages(&self) -> Vec<Package> {
        let cache = self.package_cache.read().await;

        cache
            .values()
            .filter(|pkg| pkg.identity.source == PackageSource::APT)
            .cloned()
            .collect()
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

    /// Get detailed package info
    pub async fn get_package_detail(&self, package_id: &str) -> Option<AppDetailInfo> {
        let cache = self.package_cache.read().await;
        
        cache.get(package_id).map(|pkg| AppDetailInfo::from(pkg.clone()))
    }

    /// Get a package by ID
    pub async fn get_package(&self, package_id: &str) -> Option<Package> {
        let cache = self.package_cache.read().await;
        cache.get(package_id).cloned()
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

    /// Execute a package operation
    pub async fn execute_operation(
        &self,
        operation: PackageOperation,
    ) -> Result<OperationResult, Box<dyn std::error::Error + Send + Sync>> {
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
                        let result = adapter.install(pkg_id).await?;
                        
                        if result.success {
                            self.notifications.notify_install(pkg_id, true);
                        } else {
                            self.notifications.notify_error("install", pkg_id, &result.message);
                        }
                        
                        Ok(result)
                    }
                    PackageOperation::Update(pkg_id) => {
                        let adapter = self.find_adapter_for_package(pkg_id).await?;
                        let result = adapter.update(pkg_id).await?;
                        
                        if result.success {
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
                        adapter.uninstall(pkg_id).await
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

        // Refresh cache after successful operation
        if result.success {
            let _ = self.refresh_cache().await;
        }

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
        let source = if pkg_id.starts_with("apt:") {
            PackageSource::APT
        } else if pkg_id.starts_with("flatpak:") {
            PackageSource::Flatpak
        } else if pkg_id.starts_with("snap:") {
            PackageSource::Snap
        } else if pkg_id.starts_with("appimage:") {
            PackageSource::AppImage
        } else if pkg_id.starts_with("soar:") {
            PackageSource::Soar
        } else if pkg_id.starts_with("github:") {
            PackageSource::GitHubRelease
        } else if pkg_id.starts_with("custom:") {
            PackageSource::OfferingsCustom
        } else {
            // Default to APT for unqualified package names
            PackageSource::APT
        };

        if let Some(&idx) = self.adapter_map.get(&source) {
            Ok(self.adapters[idx].clone())
        } else {
            Err("Adapter not found".into())
        }
    }

    /// Get dependency graph for a package
    pub async fn get_dependency_tree(&self, package_id: &str) -> Vec<String> {
        let dep_graph = self.dep_graph.read().await;
        dep_graph.get_full_dependency_tree(package_id).flatten()
    }

    /// Get reverse dependencies
    pub async fn get_reverse_dependencies(&self, package_id: &str) -> Vec<String> {
        let dep_graph = self.dep_graph.read().await;
        dep_graph.get_reverse_dependencies(package_id)
    }

    /// Trace dependency path
    pub async fn trace_dependency(&self, app_id: &str, dep_id: &str) -> Option<String> {
        let dep_graph = self.dep_graph.read().await;
        dep_graph.trace_dependency(app_id, dep_id).map(|t| t.display())
    }

    /// Find orphaned dependencies
    pub async fn find_orphans(&self) -> Vec<Package> {
        let dep_graph = self.dep_graph.read().await;
        let orphan_ids = dep_graph.find_orphans();
        
        let cache = self.package_cache.read().await;
        orphan_ids
            .into_iter()
            .filter_map(|id| cache.get(&id).cloned())
            .collect()
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
            self.notifications.notify_updates_available(all_updates.len());
        }

        all_updates
    }

    /// Get available sources on this system
    pub async fn get_available_sources(&self) -> Vec<PackageSource> {
        let mut sources = Vec::new();
        
        for adapter in &self.adapters {
            if adapter.is_available().await {
                sources.push(adapter.source());
            }
        }
        
        sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_backend_creation() {
        let backend = BackendService::new(HomePageContent::default());
        assert!(backend.is_ok());
    }
}
