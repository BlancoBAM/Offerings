-- Lilith Offerings Repository Database Schema
-- Unified package metadata database with multi-source support

-- Source configuration table
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    url TEXT NOT NULL,
    type TEXT NOT NULL,  -- flatpak, snap, appimage, soar, pacstall, github, appbundle
    enabled INTEGER DEFAULT 1,
    last_sync INTEGER,
    sync_interval INTEGER DEFAULT 3600,  -- seconds between syncs
    priority INTEGER DEFAULT 10,
    metadata_url TEXT,
    api_endpoint TEXT,
    created_at INTEGER DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER DEFAULT (strftime('%s', 'now'))
);

-- Categories table (freedesktop.org compatible)
CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    parent_id INTEGER,
    icon TEXT,
    description TEXT,
    FOREIGN KEY (parent_id) REFERENCES categories(id)
);

-- Main packages table
CREATE TABLE IF NOT EXISTS packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL,
    source_package_id TEXT NOT NULL,  -- Original ID from source
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,  -- Lowercase, for deduplication
    summary TEXT,
    description TEXT,
    version TEXT,
    source_type TEXT,  -- appimage, flatpak, snap, static, etc.
    homepage_url TEXT,
    documentation_url TEXT,
    icon_url TEXT,
    icon_local_path TEXT,
    screenshots TEXT,  -- JSON array
    categories TEXT,  -- JSON array
    rating REAL,
    download_size INTEGER,
    installed_size INTEGER,
    license TEXT,
    author TEXT,
    metadata_extra TEXT,  -- JSON for source-specific data
    is_featured INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY (source_id) REFERENCES sources(id),
    UNIQUE(source_id, source_package_id)
);

-- Package alternatives (for cross-source deduplication)
CREATE TABLE IF NOT EXISTS package_alternatives (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    alternative_package_id INTEGER NOT NULL,
    is_preferred INTEGER DEFAULT 0,
    FOREIGN KEY (package_id) REFERENCES packages(id),
    FOREIGN KEY (alternative_package_id) REFERENCES packages(id)
);

-- Installed packages tracking
CREATE TABLE IF NOT EXISTS installed_packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    installed_version TEXT,
    install_date INTEGER DEFAULT (strftime('%s', 'now')),
    install_source TEXT,  -- Which source it was installed from
    last_updated INTEGER,
    auto_update INTEGER DEFAULT 1,
    FOREIGN KEY (package_id) REFERENCES packages(id)
);

-- Sync history and change tracking
CREATE TABLE IF NOT EXISTS sync_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL,
    sync_start INTEGER NOT NULL,
    sync_end INTEGER,
    packages_added INTEGER DEFAULT 0,
    packages_updated INTEGER DEFAULT 0,
    packages_removed INTEGER DEFAULT 0,
    status TEXT DEFAULT 'pending',  -- pending, running, completed, failed
    error_message TEXT,
    FOREIGN KEY (source_id) REFERENCES sources(id)
);

-- Package change log (for detecting updates)
CREATE TABLE IF NOT EXISTS package_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    change_type TEXT NOT NULL,  -- added, updated, removed, version_changed
    old_version TEXT,
    new_version TEXT,
    detected_at INTEGER DEFAULT (strftime('%s', 'now')),
    notified INTEGER DEFAULT 0,
    FOREIGN KEY (package_id) REFERENCES packages(id)
);

-- Featured/curated packages
CREATE TABLE IF NOT EXISTS featured_packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    category TEXT NOT NULL,
    position INTEGER DEFAULT 0,
    featured_until INTEGER,
    created_at INTEGER DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY (package_id) REFERENCES packages(id)
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_packages_name ON packages(name);
CREATE INDEX IF NOT EXISTS idx_packages_normalized_name ON packages(normalized_name);
CREATE INDEX IF NOT EXISTS idx_packages_source ON packages(source_id);
CREATE INDEX IF NOT EXISTS idx_packages_categories ON packages(categories);
CREATE INDEX IF NOT EXISTS idx_installed_packages ON installed_packages(package_id);
CREATE INDEX IF NOT EXISTS idx_sync_history_source ON sync_history(source_id);
CREATE INDEX IF NOT EXISTS idx_package_changes_package ON package_changes(package_id);

-- Views for common queries
CREATE VIEW IF NOT EXISTS v_all_packages AS
SELECT 
    p.id,
    p.name,
    p.normalized_name,
    p.summary,
    p.version,
    p.source_type,
    s.name as source_name,
    s.url as source_url,
    p.categories,
    p.icon_url,
    p.is_featured
FROM packages p
JOIN sources s ON p.source_id = s.id
WHERE p.id IN (
    SELECT MAX(id) 
    FROM packages 
    GROUP BY normalized_name, source_id
);

CREATE VIEW IF NOT EXISTS v_installed_packages AS
SELECT 
    p.*,
    ip.installed_version,
    ip.install_date,
    ip.last_updated
FROM packages p
JOIN installed_packages ip ON p.id = ip.package_id;

-- Insert default sources
INSERT OR IGNORE INTO sources (name, url, type, priority, sync_interval) VALUES
    ('Flathub', 'https://flathub.org', 'flatpak', 1, 86400),
    ('AM', 'https://portable-linux-apps.github.io', 'appimage', 2, 86400),
    ('SOAR', 'https://pkgs.pkgforge.dev', 'soar', 3, 43200),
    ('Snap Store', 'https://snapcraft.io', 'snap', 4, 86400),
    ('Pacstall', 'https://pacstall.dev', 'pacstall', 5, 86400),
    ('GitHub Releases', 'https://github.com', 'github', 6, 43200),
    ('AppBundle Hub', 'https://xplshn.github.io/AppBundleHUB', 'appbundle', 7, 86400),
    ('Anylinux AppImages', 'https://pkgforge-dev.github.io/Anylinux-AppImages', 'appimage', 8, 86400),
    ('PkgForge Cache', 'https://pkgs.pkgforge.dev/?repo=pkgcache_amd64', 'static', 9, 86400),
    ('SOAR PKGS', 'https://pkgs.pkgforge.dev/?repo=soarpkgs', 'static', 10, 86400);

-- Insert standard categories
INSERT OR IGNORE INTO categories (name, parent_id, icon, description) VALUES
    ('AudioVideo', NULL, 'audio-card', 'Multimedia applications'),
    ('Audio', 1, 'audio-card', 'Audio editing and playback'),
    ('Video', 1, 'video-x-generic', 'Video editing and playback'),
    ('Development', NULL, 'applications-development', 'Development tools'),
    ('Education', NULL, 'applications-science', 'Educational software'),
    ('Game', NULL, 'applications-games', 'Games and entertainment'),
    ('Graphics', NULL, 'applications-graphics', 'Graphics and design'),
    ('Network', NULL, 'applications-internet', 'Internet and communication'),
    ('Office', NULL, 'applications-office', 'Office and productivity'),
    ('Science', NULL, 'applications-science', 'Scientific applications'),
    ('Settings', NULL, 'preferences-system', 'System settings'),
    ('System', NULL, 'utilities-system', 'System utilities'),
    ('Utilities', NULL, 'utilities-terminal', 'General utilities');
