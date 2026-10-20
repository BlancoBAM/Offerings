# Offerings Store Architecture

Last updated: 2026-03-20

## Purpose

This file is the source-level product/implementation brief for `Offerings`.

Use it when the codebase only partially matches the intended Lilith Linux store behavior. It records the intended UX, source model, and the current bridge strategy so future models can continue without reconstructing requirements from chat logs.

## Product Goal

`Offerings` should feel like a Lilith-themed COSMIC Store / Bazaar style desktop app store:

- browseable storefront with curated sections (Featured, Essentials, Trending)
- search-first discovery
- app cards and detail views
- install / uninstall / update flows
- source-aware, but not source-fragmented
- unified Lilith theming across the entire app

The user should experience one app store, not a collection of unrelated package-manager frontends.

## Intended Sources

The long-term source surface is:

1. `Flatpak` / Flathub
2. `AM` / portable Linux apps
3. `Pacstall`
4. `Snap`
5. `Homebrew` formulae/casks that behave like user-facing apps
6. `GitHub releases` used directly as installable desktop apps in Lilith Linux (presented as **Direct Release**)
7. `Lilith curated` packages and app definitions

## Current Reality

The current backend has real adapters for:

- `Flatpak`
- `AM / AppImage`
- `Pacstall`
- `Snap`
- `Homebrew` (cask-focused, wraps `brew` CLI)
- `GitHub Release` — presented as **Direct Release**: user-entered repos/URLs + curated manifest, auto-downloads the right asset for the machine, installs binaries to `~/.local/bin/`
- `Custom` package definitions

That means the current best bridge for missing sources is:

- use native adapters for all implemented sources
- represent Lilith-curated app definitions through curated metadata until the catalog adapter is fully realized

## Source Model Direction

The next implementation layers should move toward this structure:

### Native adapters

- `FlatpakAdapter`
- `AmAdapter`
- `PacstallAdapter`
- `SnapAdapter`
- `HomebrewAdapter`
- `GitHubReleaseAdapter`

### Curated overlay

- `LilithCatalogAdapter`
- local or repo-shipped metadata for:
  - curated app picks
  - GitHub release manifests
  - install recipes for single-binary apps
  - Lilith-specific packages

### Unified store service

The backend should:

- normalize metadata from all sources into one canonical `Package`
- deduplicate by application identity/name
- preserve alternative sources on the detail page
- prefer sources in a deterministic order
- keep install/update/uninstall flows source-native behind one store UI

## Deduplication Priority

Current effective priority:

1. `Flatpak`
2. `AM / AppImage`
3. `Pacstall`
4. `Snap`
5. `Lilith`

Planned expanded priority:

1. `Flatpak`
2. `AM / portable apps`
3. `Pacstall`
4. `Snap`
5. `Homebrew`
6. `Direct Release`
7. `Lilith curated`

## UI Direction

The current theme should stay. The UX should move further toward:

- COSMIC Store / Bazaar style layout and mental model
- richer home page curation with Featured, Essentials, and Trending sections
- searchable unified results
- more complete detail pages with prominent source switching
- clearer source switching when alternatives exist
- better installed/updates views

## Implementation Notes For Future Models

- The current `AppImageAdapter` is already acting as the AM bridge and should be treated as the seed for a more explicit `AM` integration path.
- `CustomAdapter` is the current practical bridge for Lilith-curated definitions and can temporarily host Homebrew/GitHub release recipes until dedicated adapters are written.
- Preserve the Lilith visual language while improving store parity; do not reset this UI back to a generic default app store.

## Recent Improvements (2026-03-20)

- Added curated home page sections: **Featured**, **Essentials**, and **Trending** for COSMIC Store-style discovery
- Enhanced alternative source dropdown with icons, current selection indicators, and improved visual styling
- Expanded featured app curation across all categories with better app selections
- Fixed unreachable pattern warning in Flatpak category mapping
- Reduced dead code warnings from 42 to 38 through targeted `#[allow(dead_code)]` attributes

## Recent Improvements (2026-10-18)

### Catalogue-as-single-browse-source (the store's core architecture)

- The **unified catalogue is the one browse source at runtime**. Adapters for
  runtime package managers (flatpak, AM, snap, brew) no longer push their raw
  listings into the browse grid on every refresh — they only serve
  installed-state tracking, install/uninstall, and update checks
  (`PackageAdapter::contributes_browse() -> bool`, backend refresh gating).
- `tools/build_catalogue.py` merges upstream seeds into
  `assets/metadata-catalog.json`: **10,962 desktop-only entries** with real
  Flathub AppStream categories, icons and homepages (3,297 fresh Flathub apps
  parsed from `appstream.xml.gz`), pkgforge desktop container formats
  (`pkg_type` filter drops the static-binary class), AM databases, appbundle
  and spm-feed. CLI plumbing (busybox/coreutils tables, portable-CLI list,
  pkgforge `static` binaries) is filtered out.
- `tools/update_catalogue.py` (plus `tools/systemd/` user units) refetches
  every seed automatically — Flathub AppStream, pkgforge/soarpkgs release
  metadata, AM `x86_64-appimages`/`x86_64-apps`, spm feed — with
  **never-regress semantics**: failed downloads keep the last seed snapshot,
  and the rebuild is validated then atomically swapped.
- The running store fingerprints catalogue files (path + mtime); after an
  external rebuild the next 🔄 refresh (or app restart) serves the new
  catalogue with no restart required.
- Browse rows carry their catalogue `source_ids` as install alternatives
  (Flathub first), so a failed soar install falls back to the flathub or
  AM route through the normal pipeline.
- One-time cache dedup: one canonical row per app with multi-source
  alternatives; per-source duplicates are pruned from SQLite (stage-4
  `prune_packages_except`) and WAL/synchronous pragmas speed the bulk write.

### New store catalogue (assets/metadata-catalog.json)

- `tools/build_catalogue.py` merges the raw sources in `NEW-OFFERINGS-CATALOGUE`
  and live-fetched seeds with unioned freedesktop categories
  (`main-categories.txt`).

### Favorites ("Saved for later")

- SQLite `favorites` table + `add_favorite` / `remove_favorite` / `get_favorites`
  / `toggle_favorite` (`db.rs`), surfaced through `BackendService::set_favorite`
  and friends (`backend.rs`).
- UI: infernal **heart toggle** in the package detail header (`HeartButton` in
  `ui/main.slint`), a **Saved** sidebar section with its own view, and a
  process-wide favorites registry in `main.rs` that stamps the `favorite` flag
  onto every `PackageInfo` the UI renders.

### Real progress everywhere (no fake staged timers)

- `start_staged_progress` removed from all adapters. New shared helpers:
  `RealProgress` + `extract_percent` in `src/adapters/mod.rs`.
- `BackendService::refresh_cache_with_progress(Option<DbProgressCallback>)`
  reports **actual database rows written** ("N of M rows written", per-adapter
  windows). The UI refresh button (🔄 in the top bar) forwards every
  (fraction, status) pair into the `db-refresh-*` properties and shows a live
  percentage bar.
- Snap installs/uninstalls poll real `snap change` task completion; GitHub
  (Direct Release) downloads stream real byte counts; Homebrew parses `==>`
  milestones and embedded percentages.

### Direct Release source (replaces "GitHub Releases")

- Label renamed to **Direct Release** (`model.rs`); icon + alternative icons
  updated in the UI.
- New **"⚡ Direct Release"** sidebar view: type any `owner/repo` or
  github.com URL → the adapter normalizes it, fetches repo metadata from the
  GitHub API, auto-picks a platform-appropriate asset pattern
  (`pick_asset_pattern`), persists the entry in
  `~/.config/offerings/github_apps.json` (`add_repo`), and installs through the
  normal pipeline (`BackendService::install_direct_release`) with real download
  progress.
- Archived files that don't ship an exact binary name fall back to the
  archive's single executable (`find_any_executable_recursive`).

### Locally downloaded apps appear automatically

- `AppImageAdapter` now scans `~/.local/bin` too, so apps downloaded by hand
  or by the Direct Release flow surface in the store as installed.

### Tiling-friendly adaptive layout

- `grid-columns` is now computed live from the window width
  (5 columns at ~1300px content down to 1), so maximized/tiled windows on
  cosmic keep an even card grid instead of a fixed 3-column layout.
