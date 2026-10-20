#!/usr/bin/env python3
"""
Offerings Catalogue Builder
===========================

Builds the unified `assets/metadata-catalog.json` metadata overlay consumed by
`MetadataCatalog::load()` (src/catalog.rs) from the raw source files collected in
`NEW-OFFERINGS-CATALOGUE`:

  1. metadata-x86_64-linux.json  — pkgforge/soar metadata (freedesktop categories,
                                   homepage, license, src_url, download_url)
  2. spm-feed.json               — simoniz0r/spm AppImage feed (GitHub links)
  3. x86_64.md                   — appbundle / busybox portable binary listings
  4. x86_64-appimages            — AM / portable-linux-apps AppImage listing

Every entry is normalized into the store's canonical CatalogEntry schema
(package_id, source_ids, name, summary, description, icon_url, screenshots,
homepage_url, documentation_url, categories, rating, popularity, last_updated)
with categories mapped onto the freedesktop.org Main Categories
(AudioVideo/Audio/Video/Development/Education/HealthFitness/Game/Graphics/
Network/Office/Science/Settings/System/Utility) so the store can sort apps into
uniform categories.

Usage:
    python3 tools/build_catalogue.py [RAW_DIR] [OUT_FILE]

Defaults:
    RAW_DIR := /home/$USER/Documents/NEW-OFFERINGS-CATALOGUE
    OUT     := assets/metadata-catalog.json   (next to the repo root)
"""

from __future__ import annotations

import json
import os
import re
import sys
from collections import OrderedDict
from pathlib import Path

# ---------------------------------------------------------------------------
# freedesktop Main-Category normalization
# ---------------------------------------------------------------------------

# Auxiliary freedesktop.org categories → canonical Main Category(ies).
AUX_MAP = {
    "2DGraphics": ["Graphics"],
    "3DGraphics": ["Graphics"],
    "ActionGame": ["Game"],
    "AdventureGame": ["Game"],
    "Amusement": ["Game"],
    "ArcadeGame": ["Game"],
    "BlocksGame": ["Game"],
    "BoardGame": ["Game"],
    "CardGame": ["Game"],
    "KidsGame": ["Game"],
    "LogicGame": ["Game"],
    "RolePlaying": ["Game"],
    "Shooter": ["Game"],
    "Simulation": ["Game"],
    "SportsGame": ["Game"],
    "StrategyGame": ["Game"],
    "Sports": ["Game"],
    "Emulator": ["Game"],
    "Audio": ["AudioVideo", "Audio"],
    "Video": ["AudioVideo", "Video"],
    "AudioVideoEditing": ["AudioVideo"],
    "Midi": ["AudioVideo", "Audio"],
    "Mixer": ["AudioVideo", "Audio"],
    "Music": ["AudioVideo", "Audio"],
    "Player": ["AudioVideo"],
    "Recorder": ["AudioVideo"],
    "Tuner": ["AudioVideo"],
    "TV": ["AudioVideo", "Video"],
    "DiscBurning": ["AudioVideo", "Utility"],
    "Chat": ["Network"],
    "InstantMessaging": ["Network"],
    "IRCClient": ["Network"],
    "Telephony": ["Network"],
    "News": ["Network"],
    "Feed": ["Network"],
    "Email": ["Network", "Office"],
    "P2P": ["Network"],
    "FileTransfer": ["Network", "Utility"],
    "TelephonyTools": ["Utility"],
    "WebBrowser": ["Network"],
    "WebDevelopment": ["Development"],
    "IDE": ["Development"],
    "GUIDesigner": ["Development"],
    "Profiling": ["Development"],
    "RevisionControl": ["Development"],
    "Translation": ["Development"],
    "Debugger": ["Development"],
    "Database": ["Development", "Office"],
    "Dictionary": ["Office", "Education"],
    "Chart": ["Office"],
    "ContactManagement": ["Office"],
    "Calendar": ["Office"],
    "Presentation": ["Office"],
    "ProjectManagement": ["Office", "Development"],
    "Publishing": ["Office", "Graphics"],
    "Spreadsheet": ["Office"],
    "WordProcessor": ["Office"],
    "TextTools": ["Utility"],
    "TextEditor": ["Utility", "Development"],
    "Calculator": ["Utility", "Science"],
    "Clock": ["Utility"],
    "Archiving": ["Utility"],
    "Compression": ["Utility"],
    "FileTools": ["Utility"],
    "FileManager": ["Utility", "System"],
    "Filesystem": ["System", "Utility"],
    "Monitor": ["System"],
    "PackageManager": ["System"],
    "TerminalEmulator": ["System"],
    "TrayIcon": ["Utility"],
    "System": ["System"],
    "Settings": ["Settings"],
    "HardwareSettings": ["Settings"],
    "Printing": ["System", "Settings"],
    "Security": ["System", "Settings"],
    "Accessibility": ["Settings", "Utility"],
    "Core": ["System"],
    "Screensaver": ["Utility"],
    "Applet": ["Utility"],
    "Shell": ["System"],
    "Construction": ["Graphics", "Science"],
    "DataVisualization": ["Science", "Graphics"],
    "Electronics": ["Science", "Development"],
    "Engineering": ["Science"],
    "Physics": ["Science"],
    "Math": ["Science", "Education"],
    "Education": ["Education"],
    "ArtificialIntelligence": ["Science", "Development"],
    "Robotics": ["Science"],
    "Astronomy": ["Science"],
    "Biology": ["Science"],
    "Chemistry": ["Science"],
    "Geology": ["Science"],
    "Geoscience": ["Science"],
    "History": ["Education"],
    "Humanities": ["Education"],
    "ImageProcessing": ["Graphics"],
    "OCR": ["Graphics"],
    "Photography": ["Graphics"],
    "RasterGraphics": ["Graphics"],
    "Scanning": ["Graphics"],
    "VectorGraphics": ["Graphics"],
    "Literature": ["Education"],
    "Maps": ["Education", "Science"],
    "MedicalSoftware": ["Education", "Science"],
    "NumericalAnalysis": ["Science"],
    "ParallelComputing": ["Development", "Science"],    "Spirituality": ["Education"],
    "Art": ["Graphics", "Education"],
    "Building": ["Development", "Utility"],
    "Debugger." : ["Development"],
    "Documentation": ["Development", "Office"],
    "Finance": ["Office"],
    "PointOfSale": ["Office"],
    "Packing": ["Utility"],
    "Adult": ["Game"],
    "Utility": ["Utility"],
    "Development": ["Development"],
    "Game": ["Game"],
    "Graphics": ["Graphics"],
    "Network": ["Network"],
    "Office": ["Office"],
    "Science": ["Science"],
    "AudioVideo": ["AudioVideo"],
    "ConsoleOnly": [],
}

# Preferred canonical order for the store sidebar.
CANONICAL_ORDER = [
    "AudioVideo", "Audio", "Video", "Development", "Education", "HealthFitness",
    "Game", "Graphics", "Network", "Office", "Science", "Settings", "System",
    "Utility",
]


def normalize_categories(raw) -> list[str]:
    """Map raw (possibly auxiliary) freedesktop categories onto Main Categories."""
    canon: list[str] = []
    for cat in raw or []:
        cat = cat.strip()
        if not cat:
            continue
        mapped = AUX_MAP.get(cat, [])
        targets = mapped if mapped else ([cat] if cat in CANONICAL_ORDER else [])
        for t in targets:
            if t not in canon:
                canon.append(t)
    # Always keep at least a sane catch-all.
    if not canon:
        canon.append("Utility")
    order_idx = {name: i for i, name in enumerate(CANONICAL_ORDER)}
    canon.sort(key=lambda c: order_idx.get(c, len(order_idx)))
    return canon


def slug(name: str) -> str:
    return re.sub(r"\s+", "-", name.strip().lower())


def _lang_attr(el) -> str:
    """xml:lang of an element (namespace-aware), "" when unlocalised."""
    return el.attrib.get("{http://www.w3.org/XML/1998/namespace}lang", "") or ""


def pick_default_locale_text(comp, q, tag: str) -> str:
    """AppStream exposes <name>/<summary> in many languages. `findtext` grabs
    the first element in document order, which may be ANY locale (Hebrew,
    Arabic…) and English cards rendered foreign titles. Selection order:
    un-localised default → en → first."""
    nodes = comp.findall(q(tag))
    if not nodes:
        return ""
    for node in nodes:
        if not _lang_attr(node):
            return (node.text or "").strip()
    for node in nodes:
        if _lang_attr(node).lower().startswith("en"):
            return (node.text or "").strip()
    return (nodes[0].text or "").strip()


RTL_RE = re.compile(r"[\u0590-\u05FF\u0600-\u06FF]")
LATIN_RE = re.compile(r"^[A-Za-z0-9][\x20-\u024F·.+\-']*")


def fix_foreign_locale_names(entries: dict[str, dict]) -> list[str]:
    """Some upstream feeds (soar/PkgForge forks) publish titles in Arabic or
    Hebrew while every screenshot/homepage stays English. Replace such names
    with a Latin name from a sibling catalogue entry referenced in this
    entry's own `source_ids` (e.g. `flatpak:org.kde.ark.desktop` → "Ark").
    Returns the ids that were patched."""
    patched: list[str] = []

    def is_latin(text: str) -> bool:
        return bool(text) and not RTL_RE.search(text)

    by_id = dict(entries)
    for eid, entry in entries.items():
        name = entry.get("name") or ""
        summary = entry.get("summary") or ""
        if is_latin(name) and is_latin(summary):
            continue
        for sibling in entry.get("source_ids") or []:
            other = by_id.get(sibling)
            if not other or other is entry:
                continue
            other_name = other.get("name") or ""
            if not is_latin(other_name):
                continue
            if not is_latin(name):
                entry["name"] = other_name
                patched.append(eid)
            if not is_latin(summary) and is_latin(other.get("summary") or ""):
                entry["summary"] = other["summary"]
            break
    # Also patch the whole appimage:xxx sibling ids which never appear as
    # entries themselves: a name like "appimage:آرك" carries the Arabic in the
    # id, so latin entries win as the canonical display row.
    return patched


def parse_flathub_appstream(path: Path, base_remote: str | None = None) -> dict[str, dict]:
    """Flathub AppStream catalog (appstream.xml[.gz]) — every GUI application
    with real freedesktop categories, icons, screenshots and release info.

    AppStream itself already separates desktop apps (`type="desktop"`) from
    CLI mirrors (`type="console-application"`), runtimes, codecs and add-ons,
    so this source is desktop-only by filter on component type.

    `base_remote` is the AppStream base URL (e.g.
    https://flathub.org/repo/appstream/x86_64) used to resolve spec-relative
    cached icons that Flathub publishes instead of remote <icon> URLs.
    """
    import gzip
    import xml.etree.ElementTree as ET

    out: dict[str, dict] = {}
    try:
        opener = gzip.open if str(path).endswith(".gz") else open
        with opener(path, "rb") as fh:
            tree = ET.parse(fh)
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read flathub appstream {path}: {exc}", file=sys.stderr)
        return out

    root = tree.getroot()
    ns = root.tag[1:].split("}", 1)[0] if root.tag.startswith("{") else ""

    def q(tag: str) -> str:
        return f"{{{ns}}}{tag}" if ns else tag

    for comp in root.findall(q("component")):
        ctype = (comp.get("type") or "desktop").strip()
        # GUI apps on Flathub are "desktop-application" (or legacy "desktop");
        # console-application mirrors, runtimes, add-ons and localizations
        # are deliberately not store offerings.
        if ctype not in ("desktop", "desktop-application"):
            continue

        appid = (comp.findtext(q("id")) or "").strip()
        name = pick_default_locale_text(comp, q, "name")
        if not name or not appid:
            continue

        summary = pick_default_locale_text(comp, q, "summary")
        # <description> contains <p> paragraphs — join their text.
        desc_parts = []
        desc_el = comp.find(q("description"))
        if desc_el is not None:
            for p in desc_el.iter():
                if p.tag == q("p") and p.text:
                    desc_parts.append(p.text.strip())
        description = " ".join(desc_parts).strip()

        icon_url = None
        for icon in comp.findall(q("icon")):
            if icon.get("type") == "remote":
                u = (icon.text or "").strip()
                if u.startswith("http"):
                    icon_url = u
                    break
        if not icon_url and base_remote:
            # spec-cached icons are relative to the AppStream base URL:
            #   <base>/icons/<size>/<appid>.png
            icon_url = f"{base_remote}/icons/128x128/{appid}.png"

        screenshots = []
        for shot in comp.iter(q("screenshot")):
            # One image per <screenshot>: <image> children of the same shot are
            # size variants of the SAME media (624x/752x/1128x) — taking them
            # all produced "three screenshots" that were three resolutions of
            # one image. Prefer the plain "source" variant, else the first.
            images = [(img.get("type") or "", (img.text or "").strip())
                      for img in shot.iter(q("image"))]
            urls = [u for _, u in images if u.startswith("http")]
            if not urls:
                continue
            plain = next((u for t, u in images if t == "source" and u.startswith("http")), None)
            screenshots.append(plain or urls[0])
            if len(screenshots) >= 3:
                break

        homepage_url = None
        documentation_url = None
        for url in comp.findall(q("url")):
            usrc = (url.text or "").strip()
            utype = (url.get("type") or "").strip()
            if not usrc.startswith("http"):
                continue
            if utype == "homepage" and not homepage_url:
                homepage_url = usrc
            elif utype in ("help", "faq", "contact", "bugtracker") and not documentation_url:
                documentation_url = usrc

        cats_raw = [c.text or "" for c in comp.iter(q("category"))]
        version, last_updated = None, 0
        releases = comp.find(q("releases"))
        if releases is not None:
            rel = releases.find(q("release"))
            if rel is not None:
                version = (rel.get("version") or "").strip() or None
                ts_str = rel.get("timestamp") or ""
                date_str = rel.get("date") or ""
                try:
                    from datetime import datetime, timezone
                    if ts_str and ts_str.isdigit():
                        last_updated = int(ts_str)
                    elif date_str:
                        last_updated = int(
                            datetime.strptime(date_str[:10], "%Y-%m-%d")
                            .replace(tzinfo=timezone.utc).timestamp()
                        )
                except ValueError:
                    last_updated = 0

        out[name.lower()] = {
            "name": name,
            "summary": summary,
            "description": description or summary,
            "icon_url": icon_url,
            "screenshots": screenshots,
            "homepage_url": homepage_url,
            "documentation_url": documentation_url,
            "categories": normalize_categories(cats_raw),
            "license": first_str([lic.text for lic in comp.iter(q("license"))]),
            "last_updated": last_updated,
            "latest_version": version,
            "flatpak_id": appid,
            "origin": "flathub",
        }
    return out


def first_str(values) -> str | None:
    if not values:
        return None
    v = values[0]
    if isinstance(v, str):
        v = v.strip()
        return v or None
    return None


# ---------------------------------------------------------------------------
# Source parsers
# ---------------------------------------------------------------------------

def parse_soar_metadata(path: Path) -> dict[str, dict]:
    """pkgforge/soar metadata json: categories + real download urls.

    Only packages in desktop-class container formats (appimage, appbundle,
    flatimage, …) are kept — pkgforge also publishes plain static binaries
    (pkg_type "static"/"onelf") which are CLI plumbing, not store apps.
    """
    DESKTOP_PKG_TYPES = {
        "appimage", "appbundle", "flatimage", "runimage",
        "wrappe", "nixappimage", "onexec",
    }
    out: dict[str, dict] = {}
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read soar metadata {path}: {exc}", file=sys.stderr)
        return out
    for pkg in data.get("packages", []):
        name = (pkg.get("pkg_name") or "").strip()
        if not name:
            continue
        pkg_type = (pkg.get("pkg_type") or "").strip().lower()
        if pkg_type and pkg_type not in DESKTOP_PKG_TYPES:
            continue
        date = pkg.get("date") or ""
        ts = 0
        m = re.match(r"(\d{4})-(\d{2})-(\d{2})", date)
        if m:
            from datetime import datetime, timezone
            try:
                ts = int(
                    datetime.strptime(m.group(0), "%Y-%m-%d")
                    .replace(tzinfo=timezone.utc)
                    .timestamp()
                )
            except ValueError:
                ts = 0
        src_urls = pkg.get("src_url") or []
        src_url = first_str(src_urls)
        homepage = first_str(pkg.get("homepage") or [])
        # Keep the download url as documentation fallback when src targets GitHub.
        out[name.lower()] = {
            "name": name,
            "summary": (pkg.get("description") or "").strip(),
            "description": (pkg.get("description") or "").strip(),
            "homepage_url": homepage or src_url,
            "documentation_url": None,
            "categories": normalize_categories(pkg.get("category")),
            "license": first_str(pkg.get("license")),
            "maintainer": first_str(pkg.get("maintainer")),
            "last_updated": ts,
            "sha": pkg.get("shasum"),
            "size": pkg.get("size"),
            "download_url": pkg.get("download_url"),
            "src_url": src_url,
            "origin": "soarpkgs",
        }
    return out


def parse_spm_feed(path: Path) -> dict[str, dict]:
    """simoniz0r/spm feed: AppImage packages with GitHub links."""
    out: dict[str, dict] = {}
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read spm feed {path}: {exc}", file=sys.stderr)
        return out
    for pkg in data.get("appimages", []):
        name = (pkg.get("name") or "").strip()
        if not name:
            continue
        desc = (pkg.get("description") or "").strip()
        repo_url = None
        install_url = None
        for link in pkg.get("links", []):
            ltype = (link.get("type") or "").lower()
            url = (link.get("url") or "").strip()
            if ltype == "github" and "/" in url:
                repo_url = f"https://github.com/{url}"
            elif ltype in ("install", "homepage", "website"):
                install_url = url
        author = ""
        authors = pkg.get("authors") or []
        if authors:
            author = (authors[0].get("name") or "").strip()
        out[name.lower()] = {
            "name": name,
            "summary": desc,
            "description": desc,
            "homepage_url": repo_url or install_url,
            "documentation_url": None,
            "categories": normalize_categories([]),
            "author": author,
            "origin": "spm-feed",
        }
    return out


def parse_am_listing(path: Path) -> dict[str, dict]:
    """`◆ name : description` listing (x86_64-appimages / stats-appimages)."""
    out: dict[str, dict] = {}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read AM listing {path}: {exc}", file=sys.stderr)
        return out
    pat = re.compile(r"^[◆●○]\s+(\S+)\s*:\s*(.+?)\s*#\S*\s*$")
    pat_nodesc = re.compile(r"^[◆●○]\s+(\S+)\s*:\s*(.+?)\s*$")
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith(("◆", "●", "○")):
            continue
        m = pat.match(line) or pat_nodesc.match(line)
        if not m:
            continue
        name, desc = m.group(1).strip(), m.group(2).strip()
        # Classify cheap keyword hints from AM tag lines.
        cats: list[str] = []
        lowered = desc.lower()
        hints = (
            (["game", "rts game", "platformer", "emulator"], "Game"),
            (["audio", "music", "player", "synth", "mp3", "radio"], "AudioVideo"),
            (["video", "movie", "media player", "dlna"], "AudioVideo"),
            (["browser"], "Network"),
            (["editor", "ide", "code", "terminal", "git"], "Development"),
            (["image", "photo", "paint", "vector", "graphics"], "Graphics"),
            (["office", "pdf", "document"], "Office"),
            (["scanner", "monitor", "backup", "disk"], "System"),
        )
        for terms, cat in hints:
            if any(t in lowered for t in terms):
                if cat not in cats:
                    cats.append(cat)
        out[name.lower()] = {
            "name": name,
            "summary": desc,
            "description": desc,
            "homepage_url": f"https://github.com/search?q={name}",
            "categories": normalize_categories(cats),
            "origin": "portable-linux-apps",
        }
    return out


def parse_appbundle_md(path: Path) -> dict[str, dict]:
    """x86_64.md — appbundle & related AppImage sections.

    The file also holds busybox / coreutils rebuild tables and the raw
    pkgforge "soarpkg" binary CarQUIET — those are CLI/daemon plumbing and
    are deliberately skipped so the store only carries desktop offerings.
    """
    out: dict[str, dict] = {}
    section = None
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read appbundle md {path}: {exc}", file=sys.stderr)
        return out
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("# ") and not s.startswith("|"):
            section = s[2:].strip().lower()
            continue
        if section not in ("appbundle", "python"):
            continue
        if not s.startswith("|"):
            continue
        cells = [c.strip() for c in s.strip("|").split("|")]
        if len(cells) < 4:
            continue
        if cells[0] in ("appname", "---") or set(cells[0]) <= {"-", " "}:
            continue
        name, desc, site, download = cells[0], cells[1], cells[2], cells[3]
        version = cells[4] if len(cells) > 4 else ""
        if not name or name in ("appname", "---"):
            continue
        cats: list[str] = []
        lowered = (desc or "").lower()
        hints = (
            (["game", "emulator"], "Game"),
            (["audio", "music", "player"], "AudioVideo"),
            (["browser"], "Network"),
            (["editor", "terminal", "git"], "Development"),
            (["image", "viewer"], "Graphics"),
            (["file manager"], "System"),
            (["tools", "archiver", "monitor"], "Utility"),
        )
        for terms, cat in hints:
            if any(t in lowered for t in terms):
                if cat not in cats:
                    cats.append(cat)
        if not cats:
            cats = ["Utility"]
        out[name.lower()] = {
            "name": name,
            "summary": desc,
            "description": desc,
            "homepage_url": site or None,
            "categories": normalize_categories(cats),
            "last_updated": 0,
            "download_url": download,
            "version": version,
            "origin": f"appbundle/{section}" if section else "appbundle",
        }
    return out


def parse_portable_cli(path: Path) -> dict[str, dict]:
    """stats-portable-CLI listing — CLI tools, category System/Utility."""
    out: dict[str, dict] = {}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except Exception as exc:  # noqa: BLE001
        print(f"[warn] cannot read CLI listing {path}: {exc}", file=sys.stderr)
        return out
    pat = re.compile(r"^[◆●○]\s+(\S+)\s*:\s*(.+?)\s*$")
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith(("◆", "●", "○")):
            continue
        m = pat.match(line)
        if not m:
            continue
        name, desc = m.group(1).strip(), m.group(2).strip()
        out[name.lower()] = {
            "name": name,
            "summary": desc,
            "description": desc,
            "homepage_url": f"https://github.com/search?q={name}",
            "categories": ["System", "Utility"],  # CLI tools
            "origin": "portable-CLI",
        }
    return out


# ---------------------------------------------------------------------------
# Merge & emit
# ---------------------------------------------------------------------------

def merge_entries(*sources: tuple[str, dict[str, dict]]) -> list[dict]:
    """Merge raw dicts (in priority order) into CatalogEntry-compatible dicts.

    Priority order = the order in which `sources` is given: the first source
    that supplies a non-empty field wins for that field, and *all* matching
    package ids across sources are preserved in `source_ids` so every store
    adapter (soar, appimage, github) benefits from the enriched metadata.
    """
    merged: dict[str, dict] = {}

    for _label, table in sources:
        for key, item in table.items():
            entry = merged.get(key)
            if entry is None:
                entry = {
                    "name": item["name"],
                    "summary": "",
                    "description": "",
                    "icon_url": None,
                    "screenshots": [],
                    "homepage_url": None,
                    "documentation_url": None,
                    "categories": [],
                    "rating": None,
                    "popularity": None,
                    "last_updated": 0,
                    "package_id": None,   # set below
                    "source_ids": [],
                }
                merged[key] = entry

            # Merge category information (union, canonical-ordered afterwards).
            if item.get("categories"):
                entry_set = [c for c in entry["categories"]]
                for c in item["categories"]:
                    if c not in entry_set:
                        entry_set.append(c)
                entry_set.sort(key=CANONICAL_ORDER.index)
                entry["categories"] = entry_set

            if not entry["summary"] and item.get("summary"):
                entry["summary"] = item["summary"]
            if not entry["description"] and item.get("description"):
                entry["description"] = item["description"]
            if not entry["icon_url"] and item.get("icon_url"):
                entry["icon_url"] = item["icon_url"]
            if not entry["screenshots"] and item.get("screenshots"):
                entry["screenshots"] = item["screenshots"]
            if entry["homepage_url"] in (None, "", "unknown://") and item.get("homepage_url"):
                entry["homepage_url"] = item["homepage_url"]
            if item.get("documentation_url") and not entry["documentation_url"]:
                entry["documentation_url"] = item["documentation_url"]
            if item.get("last_updated") and item["last_updated"] > (entry["last_updated"] or 0):
                entry["last_updated"] = item["last_updated"]
            if item.get("latest_version") and not entry.get("latest_version"):
                entry["latest_version"] = item["latest_version"]
            # The Flathub AppStream id is precious: it is the one install
            # route with auto-updates and rich metadata. Carry it through.
            if item.get("flatpak_id") and not entry.get("flatpak_id"):
                entry["flatpak_id"] = item["flatpak_id"]

    # Final pass: build package_id (canonical: soar:<name>) + source_ids.
    entries: list[dict] = []
    for key, entry in merged.items():
        name = entry["name"]
        soar_id = f"soar:{name}"
        appimage_id = entry.get("appimage_id") or f"appimage:{slug(name)}"
        entry["package_id"] = soar_id
        # source_ids doubles as the install-fallback preference list at
        # runtime: source adapters without a live listing (flatpak/am) must
        # come directly after the canonical soar id so a failed soar install
        # falls back through them in this order.
        ids: list[str] = [soar_id]
        if entry.get("flatpak_id"):
            ids.append(f"flatpak:{entry['flatpak_id']}")
        ids.append(appimage_id)
        ids.append(name)
        # de-duplicate, preserve order
        seen: set[str] = set()
        entry["source_ids"] = [i for i in ids if not (i in seen or seen.add(i))]

        # Drop empty optional fields to keep the JSON lean.
        if not entry["summary"]:
            entry["summary"] = entry["description"] or f"{name} — portable offering"
        if not entry["description"]:
            entry["description"] = entry["summary"]
        if not entry["categories"]:
            entry["categories"] = ["Utility"]

        entries.append(entry)
    entries.sort(key=lambda e: e["name"].lower())
    return entries


def main() -> int:
    raw_dir = Path(sys.argv[1] if len(sys.argv) > 1 else
                   "/home/s8n/Documents/NEW-OFFERINGS-CATALOGUE")
    out_file = Path(sys.argv[2] if len(sys.argv) > 2 else
                    Path(__file__).resolve().parent.parent / "assets/metadata-catalog.json")

    if not raw_dir.is_dir():
        print(f"[error] raw catalogue dir not found: {raw_dir}", file=sys.stderr)
        return 1

    print(f"[info] reading raw catalogue sources from {raw_dir}")

    soar = parse_soar_metadata(raw_dir / "metadata-x86_64-linux.json")
    appbundle = parse_appbundle_md(raw_dir / "x86_64.md")
    am_apps = parse_am_listing(raw_dir / "x86_64-appimages")
    # AM keeps a second, broader "apps" listing that also indexes its
    # portable-program (non-AppImage) section — parse it to not lose those.
    am_all = parse_am_listing(raw_dir / "x86_64-apps") if (raw_dir / "x86_64-apps").is_file() else {}
    spm = parse_spm_feed(raw_dir / "spm-feed.json")
    cli = parse_portable_cli(raw_dir / "stats-portable-CLI")

    flathub = {}
    for fh_candidate in (
        raw_dir / "flathub-appstream.xml.gz",
        raw_dir / "flathub-appstream.xml",
    ):
        if fh_candidate.is_file():
            # Derive the AppStream base for spec-cached icon resolution when
            # the seed file carries provenance metadata; local manual
            # snapshots keep the flathub base (icons are still hosted there).
            base_remote = "https://flathub.org/repo/appstream/x86_64"
            meta_file = raw_dir / "flathub-appstream.meta.json"
            if meta_file.is_file():
                try:
                    meta = json.loads(meta_file.read_text(encoding="utf-8"))
                    base_remote = meta.get("base_remote") or base_remote
                except Exception:  # noqa: BLE001
                    pass
            flathub = parse_flathub_appstream(fh_candidate, base_remote)
            break

    # Priority for metadata richness: flathub > soar > appbundle > AM > spm
    entries = merge_entries(
        ("flathub", flathub),
        ("soarpkgs", soar),
        ("appbundle", appbundle),
        ("portable-apps", am_apps),
        ("portable-all", am_all),
        ("spm-feed", spm),
        ("portable-cli", cli),
    )

    # ── Desktop-only filtering ─────────────────────────────────────────────
    # Store requirement: the catalogue must contain desktop applications
    # only. A package survives if ANY of these hold:
    #   * it ships an actual AppImage/App-bundle from AM, pkgforge metadata,
    #     appbundle or spm, OR a Flathub desktop AppStream entry
    # It is dropped when its ONLY origin is a CLI listing (portable-CLI) or
    # the plain soar name index with no GUI packaging behind it.
    # NOTE: am_all intentionally NOT a member here — it indexes the CLI
    # portable tools too, so it must never rescue a CLI name.
    desktop_names = (
        set(soar.keys())
        | set(appbundle.keys())
        | set(am_apps.keys())
        | set(spm.keys())
        | set(flathub.keys())
    )
    cli_names = set(cli.keys())
    kept, dropped_cli = [], 0
    for entry in entries:
        if entry["name"].lower() in cli_names and entry["name"].lower() not in desktop_names:
            dropped_cli += 1
            continue
        kept.append(entry)
    entries = kept


    # Non-Latin titles from foreign-locale upstream feeds: swap for a Latin
    # sibling name (source_ids co-references) so the store stays readable.
    # entries is a list of dicts here; normalize to id→dict for the helper.
    entries_by_id = {e["package_id"]: e for e in entries}
    fix_foreign_locale_names(entries_by_id)
    entries = list(entries_by_id.values())

    catalogue = {
        "version": 1,
        "generated_at": __import__("datetime").datetime.now(
            __import__("datetime").timezone.utc
        ).isoformat(),
        "sources": {
            "flathub": len(flathub),
            "soarpkgs": len(soar),
            "appbundle": len(appbundle),
            "portable-linux-apps": len(am_apps) + len(am_all),
            "spm-feed": len(spm),
            "portable-cli": len(cli),
        },
        "entries": entries,
    }

    out_file.parent.mkdir(parents=True, exist_ok=True)
    with open(out_file, "w", encoding="utf-8") as fh:
        json.dump(catalogue, fh, ensure_ascii=False, separators=(",", ":"))
        fh.write("\n")

    print(f"[ok] wrote {len(entries)} entries → {out_file}")
    print(f"     sources: {catalogue['sources']}")

    # Category histogram for the summary.
    hist: dict[str, int] = {}
    for e in entries:
        for c in e["categories"]:
            hist[c] = hist.get(c, 0) + 1
    print("     categories:", ", ".join(f"{k}={v}" for k, v in sorted(hist.items(), key=lambda x: -x[1])))
    return 0


if __name__ == "__main__":
    sys.exit(main())
