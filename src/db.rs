// src/db.rs - SQLite Persistence Layer
use crate::model::{
    Package, PackageIdentity, PackageMetadata, PackageSource,
    PackageVersion, TransactionLog, TransactionStatus,
};
use rusqlite::{params, Connection, Result as SqliteResult};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Database manager for package cache and transaction logging
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Create or open the database at the default location
    pub fn new() -> SqliteResult<Self> {
        let db_path = Self::default_db_path();
        Self::open(&db_path)
    }

    /// Open database at a specific path
    pub fn open(path: &PathBuf) -> SqliteResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.initialize_schema()?;
        Ok(db)
    }

    /// Default database location
    fn default_db_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("offerings")
            .join("offerings.db")
    }

    /// Initialize database schema
    fn initialize_schema(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r#"
            -- Packages table: cached package information
                last_updated INTEGER NOT NULL,
                UNIQUE(id, source)
            );

            -- Transaction logs for rollback support
            CREATE TABLE IF NOT EXISTS transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation TEXT NOT NULL,
                package_id TEXT NOT NULL,
                package_source TEXT NOT NULL,
                previous_state TEXT,
                new_state TEXT,
                status TEXT DEFAULT 'pending',
                started_at INTEGER NOT NULL,
                completed_at INTEGER,
                error_message TEXT
            );

            -- Operation queue for batch operations
            CREATE TABLE IF NOT EXISTS operation_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation TEXT NOT NULL,
                package_id TEXT NOT NULL,
                priority INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL,
                status TEXT DEFAULT 'pending'
            );

            -- Indexes for performance
            CREATE INDEX IF NOT EXISTS idx_packages_source ON packages(source);
            CREATE INDEX IF NOT EXISTS idx_packages_installed ON packages(is_installed);
            CREATE INDEX IF NOT EXISTS idx_transactions_status ON transactions(status);
            "#,
        )?;

        Ok(())
    }

    /// Get current Unix timestamp
    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    // ==================== Package Operations ====================

    /// Insert or update a package in the cache
    pub fn upsert_package(&self, package: &Package) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();

        let categories_json = serde_json::to_string(&package.metadata.categories).unwrap_or_default();
        let screenshots_json = serde_json::to_string(&package.metadata.screenshots).unwrap_or_default();
        let source_str = format!("{:?}", package.identity.source);

        conn.execute(
            r#"
            INSERT INTO packages (
                id, name, source, summary, description, icon_url, homepage_url,
                documentation_url, categories, screenshots, rating, installed_version,
                latest_version, is_installed, last_updated
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id, source) DO UPDATE SET
                name = excluded.name,
                summary = excluded.summary,
                description = excluded.description,
                icon_url = excluded.icon_url,
                homepage_url = excluded.homepage_url,
                documentation_url = excluded.documentation_url,
                categories = excluded.categories,
                screenshots = excluded.screenshots,
                rating = excluded.rating,
                installed_version = excluded.installed_version,
                latest_version = excluded.latest_version,
                is_installed = excluded.is_installed,
                last_updated = excluded.last_updated
            "#,
            params![
                package.identity.id,
                package.identity.name,
                source_str,
                package.metadata.summary,
                package.metadata.description,
                package.metadata.icon_url,
                package.metadata.homepage_url,
                package.metadata.documentation_url,
                categories_json,
                screenshots_json,
                package.metadata.rating,
                package.version.installed,
                package.version.latest.clone(),
                package.is_installed as i32,
                Self::now(),
            ],
        )?;

        Ok(())
    }

    /// Get a package by ID
    pub fn get_package(&self, id: &str) -> SqliteResult<Option<Package>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, name, source, summary, description, icon_url, homepage_url,
             documentation_url, categories, screenshots, rating, installed_version,
             latest_version, is_installed
             FROM packages WHERE id = ?1",
        )?;

        let package = stmt.query_row(params![id], |row| self.row_to_package(row)).optional()?;

        Ok(package)
    }

    /// Get all packages from a specific source
    pub fn get_packages_by_source(&self, source: PackageSource) -> SqliteResult<Vec<Package>> {
        let conn = self.conn.lock().unwrap();
        let source_str = format!("{:?}", source);

        let mut stmt = conn.prepare(
            "SELECT id, name, source, summary, description, icon_url, homepage_url,
             documentation_url, categories, screenshots, rating, installed_version,
             latest_version, is_installed
             FROM packages WHERE source = ?1",
        )?;

        let packages = stmt
            .query_map(params![source_str], |row| self.row_to_package(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(packages)
    }

    /// Get all installed packages
    pub fn get_installed_packages(&self) -> SqliteResult<Vec<Package>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, name, source, summary, description, icon_url, homepage_url,
             documentation_url, categories, screenshots, rating, installed_version,
             latest_version, is_installed
             FROM packages WHERE is_installed = 1
             ORDER BY last_updated DESC",
        )?;

        let packages = stmt
            .query_map([], |row| self.row_to_package(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(packages)
    }

    /// Search packages by name or summary
    pub fn search_packages(&self, query: &str) -> SqliteResult<Vec<Package>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query.to_lowercase());

        let mut stmt = conn.prepare(
            "SELECT id, name, source, summary, description, icon_url, homepage_url,
             documentation_url, categories, screenshots, rating, installed_version,
             latest_version, is_installed
             FROM packages WHERE LOWER(name) LIKE ?1 OR LOWER(summary) LIKE ?1
             ORDER BY is_installed DESC, name ASC
             LIMIT 100",
        )?;

        let packages = stmt
            .query_map(params![pattern], |row| self.row_to_package(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(packages)
    }

    /// Clear all packages from cache (used before refresh)
    pub fn clear_packages_by_source(&self, source: PackageSource) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        let source_str = format!("{:?}", source);
        conn.execute("DELETE FROM packages WHERE source = ?1", params![source_str])?;
        Ok(())
    }

    /// Helper to convert a row to Package
    fn row_to_package(&self, row: &rusqlite::Row) -> rusqlite::Result<Package> {
        let source_str: String = row.get(2)?;
        let categories_json: String = row.get(8)?;
        let screenshots_json: String = row.get(9)?;
        let source = Self::parse_source(&source_str);

        let categories: Vec<String> = serde_json::from_str(&categories_json).unwrap_or_default();
        let screenshots: Vec<String> = serde_json::from_str(&screenshots_json).unwrap_or_default();

        Ok(Package {
            identity: PackageIdentity {
                id: row.get(0)?,
                name: row.get(1)?,
                source,
            },
            metadata: PackageMetadata {
                summary: row.get(3)?,
                description: row.get(4)?,
                icon_url: row.get(5)?,
                homepage_url: row.get(6)?,
                documentation_url: row.get(7)?,
                categories,
                screenshots,
                rating: row.get(10)?,
            },
            version: PackageVersion {
                installed: row.get(11)?,
                latest: row.get(12)?,
            },
            is_installed: row.get::<_, i32>(13)? != 0,
            alternatives: vec![],
        })
    }

    fn parse_source(s: &str) -> PackageSource {
        match s {
            "Flatpak" => PackageSource::Flatpak,
            "Snap" => PackageSource::Snap,
            "AppImage" => PackageSource::AppImage,
            "Pacstall" => PackageSource::Pacstall,
            _ => PackageSource::OfferingsCustom,
        }
    }

    // Dependency management removed


    // ==================== Transaction Operations ====================

    /// Start a new transaction log entry
    pub fn start_transaction(
        &self,
        operation: &str,
        package_id: &str,
        package_source: &str,
        previous_state: Option<&str>,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transactions (operation, package_id, package_source, previous_state, status, started_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![operation, package_id, package_source, previous_state, Self::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Complete a transaction
    pub fn complete_transaction(
        &self,
        transaction_id: i64,
        success: bool,
        new_state: Option<&str>,
        error_message: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        let status = if success { "completed" } else { "failed" };
        conn.execute(
            "UPDATE transactions SET status = ?1, new_state = ?2, error_message = ?3, completed_at = ?4
             WHERE id = ?5",
            params![status, new_state, error_message, Self::now(), transaction_id],
        )?;
        Ok(())
    }

    /// Get recent transactions
    pub fn get_recent_transactions(&self, limit: i32) -> SqliteResult<Vec<TransactionLog>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, operation, package_id, package_source, previous_state, new_state, status, started_at, completed_at, error_message
             FROM transactions ORDER BY started_at DESC LIMIT ?1",
        )?;

        let logs = stmt
            .query_map(params![limit], |row| {
                let status_str: String = row.get(6)?;
                let status = match status_str.as_str() {
                    "completed" => TransactionStatus::Completed,
                    "failed" => TransactionStatus::Failed,
                    "rolledback" => TransactionStatus::RolledBack,
                    _ => TransactionStatus::Pending,
                };

                Ok(TransactionLog {
                    id: row.get(0)?,
                    operation: row.get(1)?,
                    package_id: row.get(2)?,
                    package_source: row.get(3)?,
                    previous_state: row.get(4)?,
                    new_state: row.get(5)?,
                    status,
                    started_at: row.get(7)?,
                    completed_at: row.get(8)?,
                    error_message: row.get(9)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(logs)
    }

    /// Mark a transaction as rolled back
    pub fn rollback_transaction(&self, transaction_id: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE transactions SET status = 'rolledback' WHERE id = ?1",
            params![transaction_id],
        )?;
        Ok(())
    }
}

// Trait implementations for optional rusqlite methods
trait QueryRowOptional<T> {
    fn optional(self) -> SqliteResult<Option<T>>;
}

impl<T> QueryRowOptional<T> for rusqlite::Result<T> {
    fn optional(self) -> SqliteResult<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_database_creation() {
        let db = Database::open(&PathBuf::from(":memory:")).unwrap();
        assert!(db.get_installed_packages().unwrap().is_empty());
    }

    #[test]
    fn test_package_upsert_and_get() {
        let db = Database::open(&PathBuf::from(":memory:")).unwrap();

        let pkg = Package {
            identity: PackageIdentity {
                id: "test-pkg".to_string(),
                name: "Test Package".to_string(),
                source: PackageSource::Flatpak,
            },
            metadata: PackageMetadata {
                summary: "A test package".to_string(),
                description: "Full description".to_string(),
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: None,
                categories: vec!["Development".to_string()],
                rating: Some(4.5),
            },
            version: PackageVersion {
                installed: Some("1.0.0".to_string()),
                latest: Some("1.1.0".to_string()),
            },
            is_installed: true,
            alternatives: vec![],
        };

        db.upsert_package(&pkg).unwrap();
        let retrieved = db.get_package("test-pkg").unwrap().unwrap();
        assert_eq!(retrieved.identity.name, "Test Package");
        assert!(retrieved.is_installed);
    }
}
