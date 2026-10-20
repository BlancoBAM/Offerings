#!/usr/bin/env python3
"""
Offerings Catalogue Auto-Updater
================================

Implements the store's central architecture: ONE deduplicated, desktop-only
catalogue database, refreshed automatically from its upstream seeds —
not a fleet of package-manager services queried at runtime.

Sources (all fetched automatically):
  1. flathub-appstream.xml.gz — Flathub AppStream catalog (desktop GUI apps)
  2. metadata-x86_64-linux.json — pkgforge/soarpkgs desktop container formats
  3. x86_64-appimages + x86_64-apps — AM (AppImage Manager) databases
  4. spm-feed.json — simoniz0r/spm AppImage feed
  5. x86_64.md + stats-portable-CLI — appbundle tables & CLI exclusion list
     (no upstream fetched; copied once from the manual raw snapshot dir)

Behavior:
  * Seeds land in ~/.local/share/offerings/catalogue-seeds/
  * A failed download never wipes a working seed — the last snapshot is kept
  * The rebuild happens in a temp file and is only swapped in atomically
    after validation (never regress to an empty catalog)
  * Output location: repository `assets/metadata-catalog.json` when the repo
    layout is detected, plus the user data dir
    ~/.local/share/offerings/metadata-catalog.json for installed copies
  * The running store picks the change up automatically (catalogue files are
    fingerprinted by path+mtime; the next 🔄 refresh re-loads the catalogue)
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
import tempfile
import urllib.request
from pathlib import Path

TOOLS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS_DIR))
import build_catalogue  # noqa: E402  (parse + build functions)

REPO_DIR = TOOLS_DIR.parent
FLATHUB_APPSTREAM_URL = "https://flathub.org/repo/appstream/x86_64/appstream.xml.gz"
FLATHUB_BASE_REMOTE = "https://flathub.org/repo/appstream/x86_64"
PKGFORGE_URL = (
    "https://github.com/pkgforge/soarpkgs/releases/latest/download/"
    "metadata-x86_64-linux.json"
)
AM_BASE = "https://raw.githubusercontent.com/ivan-hc/AM/master/programs"
SPM_URL = "https://raw.githubusercontent.com/simoniz0r/spm/master/spm-feed.json"


def data_dir() -> Path:
    xdg = os.environ.get("XDG_DATA_HOME")
    if xdg and xdg.strip():
        return Path(xdg).expanduser() / "offerings"
    return Path.home() / ".local/share/offerings"


def default_raw_fallback() -> Path:
    """Manual snapshot dir the user assembled (source of last resort)."""
    cand = Path.home() / "Documents/NEW-OFFERINGS-CATALOGUE"
    return cand if cand.is_dir() else Path("/nonexistent")


def download(url: str, dest: Path, timeout: int = 120) -> bool:
    """Stream a URL into dest via a temp file; on any failure keep dest."""
    tmp = dest.with_suffix(dest.suffix + ".part")
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "offerings-catalogue/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if getattr(resp, "status", 200) not in (200, 301, 302):
                return False
            with open(tmp, "wb") as fh:
                shutil.copyfileobj(resp, fh, 1024 * 256)
        os.replace(tmp, dest)
        return True
    except Exception as exc:  # noqa: BLE001
        print(f"[net] {url}: {exc}", file=sys.stderr)
        tmp.unlink(missing_ok=True)
        return bool(dest.is_file() and dest.stat().st_size > 0)


def ensure_seed(seeds: Path, name: str, url: str | None, fallback: Path) -> bool:
    """Guarantee seeds/<name> exists, preferring a fresh download."""
    path = seeds / name
    if url and download(url, path):
        print(f"[seed] fetched {name} ({path.stat().st_size} bytes)")
        return True
    if not path.is_file() and (fallback / name).is_file():
        shutil.copy2(fallback / name, path)
        print(f"[seed] copied {name} from manual snapshot")
        return True
    if path.is_file():
        print(f"[seed] {name}: keeping previous snapshot")
        return True
    print(f"[seed] {name}: MISSING — skipping this source", file=sys.stderr)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--seeds-dir", default=str(data_dir() / "catalogue-seeds"),
        help="directory for downloaded upstream seed files",
    )
    parser.add_argument(
        "--out", default=None,
        help="primary output json (default: repo assets + user data dir)",
    )
    parser.add_argument("--timeout", type=int, default=120, help="per-download timeout (s)")
    args = parser.parse_args()

    seeds = Path(args.seeds_dir)
    seeds.mkdir(parents=True, exist_ok=True)
    fallback = default_raw_fallback()

    # ── Refresh seeds (network first, snapshot fallback) ─────────────────
    ok_flathub = ensure_seed(
        seeds, "flathub-appstream.xml.gz", FLATHUB_APPSTREAM_URL, fallback
    )
    ensure_seed(seeds, "metadata-x86_64-linux.json", PKGFORGE_URL, fallback)
    ensure_seed(seeds, "x86_64-appimages", f"{AM_BASE}/x86_64-appimages", fallback)
    ensure_seed(seeds, "x86_64-apps", f"{AM_BASE}/x86_64-apps", fallback)
    ensure_seed(seeds, "spm-feed.json", SPM_URL, fallback)
    # No upstream URL (manual captures, only used for CLI exclusion +
    # appbundle tables): copy once from the manual snapshot when absent.
    for local_name in ("x86_64.md", "stats-portable-CLI"):
        ensure_seed(seeds, local_name, None, fallback)

    if not ok_flathub and not seeds.joinpath("x86_64-appimages").is_file():
        print("[error] neither flathub nor AM seeds available — aborting rebuild",
              file=sys.stderr)
        return 1

    # ── Output targets ────────────────────────────────────────────────────
    data_target = data_dir() / "metadata-catalog.json"
    out_targets: list[Path] = [data_target]
    if args.out:
        out_targets.insert(0, Path(args.out).expanduser())
    elif (REPO_DIR / "Cargo.toml").is_file():
        # Dev checkout: the compiled binary reads the repo assets copy first.
        out_targets.insert(0, REPO_DIR / "assets/metadata-catalog.json")

    # ── Rebuild into a validated temp file, then swap atomically ─────────
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        tmp_path = Path(tmp.name)

    sys.argv = ["build_catalogue", str(seeds), str(tmp_path)]
    try:
        rc = build_catalogue.main()
    finally:
        sys.argv = ["update_catalogue"]
    if rc != 0 or not tmp_path.is_file():
        tmp_path.unlink(missing_ok=True)
        print("[error] catalogue build failed — keeping previous version",
              file=sys.stderr)
        return 1

    payload = json.loads(tmp_path.read_text(encoding="utf-8"))
    if len(payload.get("entries", [])) < 100:
        tmp_path.unlink(missing_ok=True)
        print("[error] catalogue suspiciously small — keeping previous version",
              file=sys.stderr)
        return 1

    for target in out_targets:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(tmp_path, target)
    tmp_path.unlink(missing_ok=True)

    print(f"[ok] catalogue updated → {', '.join(str(t) for t in out_targets)}")
    print("     the running store reloads it on the next 🔄 refresh.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
