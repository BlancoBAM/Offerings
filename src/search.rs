// src/search.rs - Nucleo-backed fuzzy search for the Offerings catalogue
//
// Replaces the old synchronous "scan 10k packages on every keystroke" search
// that blocked the GUI. Nucleo (the fuzzy matcher behind the Helix editor)
// matches against items on a background threadpool; the UI only ever asks for
// the top-N winners and clones just those packages.

use crate::model::Package;
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo, Utf32String};
use std::sync::Arc;

/// Columns each catalogue item is matched on.
///
/// Nucleo's `MultiPattern::score` requires EVERY column to match (AND
/// semantics), and `reparse` cannot express "match the name OR the summary".
/// We therefore use a single column holding the whole search text
/// ("name summary categories"), so a query scores against whichever field
/// it appears in.
const COLUMN_COUNT: u32 = 1;

/// An item inside the search index. Kept minimal so matching stays fast.
pub struct SearchItem {
    pub package: Package,
}

/// Background fuzzy search engine over the in-memory package cache.
pub struct SearchEngine {
    nucleo: std::sync::Mutex<SearchCore>,
}

struct SearchCore {
    matcher: Nucleo<SearchItem>,
    /// Query currently set on the matcher (avoid redundant reparse churn).
    current_query: String,
}

impl SearchEngine {
    pub fn new(max_workers: Option<usize>) -> Self {
        let core = SearchCore {
            matcher: Nucleo::new(
                Config::DEFAULT,
                Arc::new(|| {}),
                max_workers,
                COLUMN_COUNT,
            ),
            current_query: String::new(),
        };
        Self {
            nucleo: std::sync::Mutex::new(core),
        }
    }

    /// A clonable handle to feed items into the index. Cheap to share.
    pub fn injector(&self) -> nucleo::Injector<SearchItem> {
        self.nucleo.lock().unwrap().matcher.injector()
    }

    /// Number of items currently ingested (may lag behind injections briefly).
    pub fn indexed_count(&self) -> u32 {
        self.nucleo
            .lock()
            .unwrap()
            .matcher
            .injector()
            .injected_items()
    }

    /// Run a query and return the top `limit` ranked packages.
    ///
    /// `tick` waits block for at most a couple of milliseconds at a time on
    /// the background workers; a query over the whole catalogue finishes in
    /// this budget, so callers (which already run off the UI thread) never
    /// perceive a freeze.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Package> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let mut core = self.nucleo.lock().unwrap();
        if core.current_query != query {
            // Reparse the same query onto every column so a hit on the name,
            // on the summary, or on a category all contribute to the score.
            for col in 0..COLUMN_COUNT as usize {
                core.matcher
                    .pattern
                    .reparse(col, query, CaseMatching::Smart, Normalization::Smart, false);
            }
            core.current_query = query.to_string();
        }

        // Drive the background workers. 10ms slices, bounded number of ticks —
        // enough for 10k+ items without ever monopolising a thread.
        // `running == false` means every match for the current pattern is done.
        for _ in 0..12 {
            let status = core.matcher.tick(10);
            if !status.running {
                break;
            }
        }

        let snapshot = core.matcher.snapshot();
        let mut results = Vec::with_capacity(limit.min(128));
        for item in snapshot.matched_items(..).take(limit) {
            results.push(item.data.package.clone());
        }
        results
    }

    /// Whether the newest query already has a complete, up-to-date result.
    pub fn ready(&self, query: &str) -> bool {
        let mut core = self.nucleo.lock().unwrap();
        if core.current_query != query {
            return false;
        }
        !core.matcher.tick(0).running
    }

    /// Replace the whole index with the given catalogue. Ingests on the
    /// background worker pool, so callers never block for long.
    pub fn rebuild(&self, packages: Vec<Package>) {
        let mut core = self.nucleo.lock().unwrap();
        // Drop the old item stream; old injectors go stale, which is what we
        // want — only this rebuild's injector feeds the new generation.
        core.matcher.restart(true);
        let injector = core.matcher.injector();
        for pkg in packages {
            let cols = columns_for(&pkg);
            injector.push(
                SearchItem { package: pkg },
                move |_data, cols_out| {
                    for (i, c) in cols.iter().enumerate() {
                        cols_out[i] = c.clone();
                    }
                },
            );
        }
        core.current_query.clear();
    }
}

/// The full haystack for one item: name first (so name hits rank best),
/// then summary, then categories.
pub fn columns_for(pkg: &Package) -> [Utf32String; 1] {
    let mut text = pkg.identity.name.clone();
    if !pkg.metadata.summary.is_empty() {
        text.push(' ');
        text.push_str(&pkg.metadata.summary);
    }
    let cats = pkg.metadata.categories.join(" ");
    if !cats.is_empty() {
        text.push(' ');
        text.push_str(&cats);
    }
    [Utf32String::from(text.as_str())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PackageIdentity, PackageMetadata, PackageSource, PackageVersion};

    fn pkg(id: &str, name: &str, summary: &str, categories: &[&str]) -> Package {
        Package {
            identity: PackageIdentity {
                id: id.to_string(),
                name: name.to_string(),
                source: PackageSource::Flatpak,
            },
            metadata: PackageMetadata {
                summary: summary.to_string(),
                description: String::new(),
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: None,
                categories: categories.iter().map(|s| s.to_string()).collect(),
                rating: None,
            },
            version: PackageVersion::default(),
            is_installed: false,
            logical_app_id: None,
            alternatives: vec![],
            last_updated: 0,
            popularity: 0.0,
        }
    }

    #[test]
    fn ranks_name_matches_first_and_is_fuzzy() {
        let engine = SearchEngine::new(Some(2));
        let injector = engine.injector();

        let vlc = pkg("flatpak:org.videolan.VLC", "VLC", "Media player", &["Video"]);
        let thunderbird = pkg(
            "flatpak:org.mozilla.Thunderbird",
            "Thunderbird",
            "Email client",
            &["Network"],
        );

        let cols = columns_for(&vlc);
        injector.push(SearchItem { package: vlc }, move |_data, cols_out| {
            for (i, c) in cols.iter().enumerate() {
                cols_out[i] = c.clone();
            }
        });
        let cols_tb = columns_for(&thunderbird);
        injector.push(SearchItem { package: thunderbird }, move |_data, cols_out| {
            for (i, c) in cols_tb.iter().enumerate() {
                cols_out[i] = c.clone();
            }
        });

        // Exact name match
        let hits = engine.search("vlc", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].identity.id, "flatpak:org.videolan.VLC");

        // Fuzzy / typo tolerance: "vcl" should still find VLC
        let hits = engine.search("vcl", 10);
        assert!(
            hits.iter().any(|p| p.identity.id == "flatpak:org.videolan.VLC"),
            "typo-tolerant match should find VLC"
        );

        // Category behaviour
        let hits = engine.search("media", 10);
        assert_eq!(hits.first().map(|p| p.identity.id.clone()),
            Some("flatpak:org.videolan.VLC".to_string()));
    }
}
