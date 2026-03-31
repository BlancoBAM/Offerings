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
6. `GitHub releases` used directly as installable desktop apps in Lilith Linux
7. `Lilith curated` packages and app definitions

## Current Reality

The current backend has real adapters for:

- `Flatpak`
- `AM / AppImage`
- `Pacstall`
- `Snap`
- `Homebrew` (cask-focused, wraps `brew` CLI)
- `GitHub Release` (curated manifest + GitHub API, installs binaries to `~/.local/bin/`)
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
6. `GitHub releases`
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
