// src/main.rs - Offerings Application Entry Point
#![allow(dead_code)]
mod adapters;
mod backend;
mod catalog;
mod db;
mod ipc;
mod model;
mod notifications;
mod transaction;

use backend::BackendService;
use ipc::{
    IpcCommand, IpcRequest, IpcResponse, IpcResponseData, IpcServer, PackageSummary, StatusData,
};
use model::{HomePageContent, PackageOperation};
use serde::Serialize;
use slint::Model;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

slint::include_modules!();

/// Query system fonts via fc-list and return sorted, deduplicated family names
fn get_system_fonts() -> Vec<String> {
    let mut fonts = vec!["System Default".to_string()];
    if let Ok(output) = Command::new("fc-list").args([":", "family"]).output() {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            let mut seen = std::collections::HashSet::new();
            for line in raw.lines() {
                for part in line.split(',') {
                    let name = part.trim().to_string();
                    if !name.is_empty() && seen.insert(name.clone()) {
                        fonts.push(name);
                    }
                }
            }
            if fonts.len() > 1 {
                let rest = &mut fonts[1..];
                rest.sort_unstable();
            }
        }
    }
    fonts
}

/// Source label -> representative URL mapping
fn source_url(label: &str) -> &'static str {
    match label {
        "Flatpak" => "https://flathub.org",
        "Snap" => "https://snapcraft.io",
        "AM / AppImage" => "https://portable-linux-apps.github.io/apps",
        "SOAR / PkgForge" => "https://pkgs.pkgforge.dev",
        "Homebrew" => "https://brew.sh",
        "GitHub Release" => "https://github.com",
        "Custom" => "file:///etc/offerings/custom",
        "Lilith" => "file:///etc/offerings/custom",
        _ => "unknown://",
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut clean = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            // Replace common Unicode artifacts that cause "box" rendering in some fonts
            match c {
                '\u{2022}' | '\u{00B7}' => clean.push('*'),  // Bullets
                '\u{2013}' | '\u{2014}' => clean.push('-'),  // Dashes
                '\u{2018}' | '\u{2019}' => clean.push('\''), // Smart single quotes
                '\u{201C}' | '\u{201D}' => clean.push('"'),  // Smart double quotes
                _ => clean.push(c),
            }
        }
    }
    clean
}

fn is_placeholder_text(name: &str, summary: &str, description: &str) -> bool {
    let name = name.trim().to_lowercase();
    let summary = summary.trim().to_lowercase();
    let description = description.trim().to_lowercase();

    description.is_empty()
        || description == name
        || description == summary
        || description == format!("{} application", name)
}

fn wrap_progress_callback(
    callback: Arc<dyn Fn(f32) + Send + Sync>,
) -> Arc<dyn Fn(f32) + Send + Sync> {
    let last_progress = Arc::new(Mutex::new(0.0));
    Arc::new(move |p| {
        let progress = p.clamp(0.0, 1.0);
        let mut guard = last_progress.lock().unwrap();
        if progress >= *guard {
            *guard = progress;
            callback(progress);
        }
    })
}

fn package_to_slint_info(pkg: model::Package) -> PackageInfo {
    let has_update = pkg.version.has_update();
    let cleaned_summary = strip_html_tags(&pkg.metadata.summary);
    let mut cleaned_description = strip_html_tags(&pkg.metadata.description);
    if is_placeholder_text(&pkg.identity.name, &cleaned_summary, &cleaned_description) {
        if !cleaned_summary.trim().is_empty()
            && cleaned_summary.trim().to_lowercase() != pkg.identity.name.trim().to_lowercase()
        {
            cleaned_description = cleaned_summary.clone();
        } else {
            cleaned_description = String::new();
        }
    }
    let alternatives: Vec<AlternativeSource> = pkg
        .alternatives
        .iter()
        .map(|alt| AlternativeSource {
            id: alt.id.clone().into(),
            source: alt.source.label().into(),
        })
        .collect();

    PackageInfo {
        id: pkg.identity.id.clone().into(),
        name: pkg.identity.name.into(),
        summary: cleaned_summary.into(),
        source: pkg.identity.source.label().into(),
        installed_version: pkg.version.installed.unwrap_or_default().into(),
        latest_version: pkg.version.latest.unwrap_or_default().into(),
        has_update,
        is_installed: pkg.is_installed,
        icon_url: pkg.metadata.icon_url.clone().unwrap_or_default().into(),
        rating: pkg.metadata.rating.unwrap_or(0.0),
        description: cleaned_description.into(),
        install_date: 0,
        alternatives: slint::ModelRc::new(slint::VecModel::from(alternatives)),
        tags: slint::ModelRc::new(slint::VecModel::from({
            let mut seen = std::collections::HashSet::new();
            pkg.metadata
                .categories
                .into_iter()
                .filter(|c| seen.insert(c.to_lowercase()))
                .map(Into::into)
                .collect::<Vec<slint::SharedString>>()
        })),
        screenshots: slint::ModelRc::new(slint::VecModel::from(
            pkg.metadata
                .screenshots
                .into_iter()
                .map(Into::into)
                .collect::<Vec<slint::SharedString>>(),
        )),
        homepage_url: pkg.metadata.homepage_url.clone().unwrap_or_default().into(),
        logical_id: pkg.logical_app_id.clone().unwrap_or_default().into(),
        icon: {
            let mut img = slint::Image::default();
            let safe_id = pkg.identity.id.replace([':', '/'], "_");
            let path = std::env::temp_dir()
                .join("offerings_icons")
                .join(format!("{}.png", safe_id));
            if path.exists() {
                if let Some(path_str) = path.to_str() {
                    // Try to open it. If it fails (e.g. corrupt or wrong format), we just stay default.
                    if let Ok(decoded) = image::open(path_str) {
                        let rgba = decoded.into_rgba8();
                        let buffer =
                            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                rgba.as_raw(),
                                rgba.width(),
                                rgba.height(),
                            );
                        img = slint::Image::from_rgba8(buffer);
                    }
                }
            }
            img
        },
    }
}

fn finish_category_transition(ui_weak: &slint::Weak<MainWindow>, category: String) {
    let _ = slint::invoke_from_event_loop({
        let ui_weak = ui_weak.clone();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                eprintln!(
                    "Category '{}' view updated. Completing transition.",
                    category
                );
                ui.set_loading_progress(1.0);
                let ui_weak2 = ui_weak.clone();
                slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_is_loading(false);
                        ui.set_is_transitioning(false);
                        ui.set_show_package_detail(false);
                    }
                });
            }
        }
    });
}

enum CliCommand {
    RunUi,
    PrintHelp,
    RefreshOnly,
    ExportCatalog(PathBuf),
    ImportCatalog(PathBuf),
    RefreshAndExport(PathBuf),
    SelfTest,
}

fn cli_usage() -> &'static str {
    "Usage: offerings [--refresh | --export-catalog [path] | --import-catalog <path> | --refresh-catalog [path] | --self-test | --help]"
}

fn parse_cli_args<I, S>(args: I) -> Result<CliCommand, Box<dyn std::error::Error + Send + Sync>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    match args.next().as_deref() {
        None => Ok(CliCommand::RunUi),
        Some("--help") | Some("-h") => Ok(CliCommand::PrintHelp),
        Some("--refresh") => Ok(CliCommand::RefreshOnly),
        Some("--export-catalog") => {
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_catalog_export_path);
            Ok(CliCommand::ExportCatalog(path))
        }
        Some("--import-catalog") => {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or("--import-catalog requires a file path")?;
            Ok(CliCommand::ImportCatalog(path))
        }
        Some("--refresh-catalog") => {
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_catalog_export_path);
            Ok(CliCommand::RefreshAndExport(path))
        }
        Some("--self-test") => Ok(CliCommand::SelfTest),
        Some(other) => Err(format!("Unknown argument: {}. {}", other, cli_usage()).into()),
    }
}

fn parse_cli_command() -> Result<CliCommand, Box<dyn std::error::Error + Send + Sync>> {
    parse_cli_args(std::env::args().skip(1))
}

fn default_catalog_export_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/metadata-catalog.generated.json")
}

#[derive(Serialize)]
struct SourceSelfTest {
    label: String,
    icon: String,
    available: bool,
}

#[derive(Serialize)]
struct SelfTestReport {
    app_version: String,
    db_path: String,
    cached_packages: usize,
    metadata_catalog_entries: usize,
    exported_catalog_path: String,
    ipc_socket_path: String,
    sources: Vec<SourceSelfTest>,
}

fn run_self_test(
    runtime: &tokio::runtime::Runtime,
    backend: &Arc<BackendService>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let database = backend.database();
    let available_sources = runtime.block_on(async { backend.get_available_sources().await });
    let ipc_server = IpcServer::new();

    let report = SelfTestReport {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        db_path: database.path().display().to_string(),
        cached_packages: database.package_count().unwrap_or(0),
        metadata_catalog_entries: database.metadata_catalog_count().unwrap_or(0),
        exported_catalog_path: default_catalog_export_path().display().to_string(),
        ipc_socket_path: ipc_server.socket_path().display().to_string(),
        sources: model::PackageSource::all()
            .into_iter()
            .map(|source| SourceSelfTest {
                label: source.label().to_string(),
                icon: source.icon().to_string(),
                available: available_sources.contains(&source),
            })
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli_command = parse_cli_command()?;

    if matches!(cli_command, CliCommand::PrintHelp) {
        println!("{}", cli_usage());
        return Ok(());
    }

    eprintln!("=== Offerings Starting ===");

    // Initialize Tokio runtime for async operations
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    eprintln!("Tokio runtime initialized");

    // Initialize backend with featured apps for each category
    // Curated selection following COSMIC Store / Bazaar style presentation
    let home_content = HomePageContent {
        featured_apps: vec![
            // Editor's picks - most essential desktop apps
            "flatpak:org.mozilla.firefox".to_string(),
            "flatpak:org.videolan.VLC".to_string(),
            "flatpak:org.gimp.GIMP".to_string(),
            "flatpak:com.spotify.Client".to_string(),
            "flatpak:org.blender.Blender".to_string(),
            "flatpak:md.obsidian.Obsidian".to_string(),
            "flatpak:com.discordapp.Discord".to_string(),
            "flatpak:org.jetbrains.IntelliJ-IDEA-Community".to_string(),
        ],
        category_showcases: {
            let mut map = HashMap::new();

            // Lilith curated - developer picks and system essentials
            map.insert(
                "Lilith".to_string(),
                vec![
                    "flatpak:io.unobserved.espansoGUI".to_string(),
                    "flatpak:com.matthiasn.lotti".to_string(),
                    "flatpak:io.github.shonebinu.Brief".to_string(),
                    "flatpak:com.tominlab.wonderpen".to_string(),
                    "flatpak:app.tintero.Tintero".to_string(),
                    "flatpak:com.vixalien.sticky".to_string(),
                    "flatpak:io.github.alainm23.planify".to_string(),
                    "flatpak:io.github.troyeguo.koodo-reader".to_string(),
                    "flatpak:com.remnote.RemNote".to_string(),
                    "flatpak:dev.mariinkys.Oboete".to_string(),
                    "flatpak:io.github.ans_ibrahim.Memento".to_string(),
                    "snap:journal".to_string(),
                    "flatpak:io.github.quantum_mutnauq.fast_reader_gtk".to_string(),
                    "flatpak:io.github.schwarzen.colormydesktop".to_string(),
                    "flatpak:page.codeberg.JakobDev.jdSystemMonitor".to_string(),
                    "flatpak:org.tabos.saldo".to_string(),
                    "flatpak:org.gnome.NetworkDisplays".to_string(),
                    "flatpak:org.gnome.SimpleScan".to_string(),
                    "flatpak:org.ksnip.ksnip".to_string(),
                    "flatpak:org.gnome.Boxes".to_string(),
                    "github:winapps-org/winapps".to_string(),
                    "github:aspizu/nixite".to_string(),
                    "github:gethomepage/homepage".to_string(),
                    "flatpak:org.kde.digikam".to_string(),
                    "flatpak:org.fontforge.FontForge".to_string(),
                    "flatpak:io.github.qcanvas.QCanvasApp".to_string(),
                    "flatpak:io.github.thiefmd.themegenerator".to_string(),
                    "flatpak:es.estoes.wallpaperDownloader".to_string(),
                    "flatpak:com.github.maoschanz.DynamicWallpaperEditor".to_string(),
                    "flatpak:com.ktechpit.wonderwall".to_string(),
                    "flatpak:com.ktechpit.colorwall".to_string(),
                    "flatpak:io.github.swordpuffin.wardrobe".to_string(),
                    "flatpak:io.github.debasish_patra_1987.linuxthemestore".to_string(),
                    "appimage:pling-store".to_string(),
                    "flatpak:app.drey.Warp".to_string(),
                    "flatpak:org.wezfurlong.wezterm".to_string(),
                    "flatpak:com.sublimehq.SublimeText".to_string(),
                    "appimage:warp".to_string(),
                    "appimage:hyper".to_string(),
                    "github:wavetermdev/waveterm".to_string(),
                    "github:johnlindquist/kit".to_string(),
                    "github:psygreg/linuxtoys".to_string(),
                    "flatpak:us.zoom.Zoom".to_string(),
                    "flatpak:com.github.IsmaelMartinez.teams_for_linux".to_string(),
                    "flatpak:me.proton.Mail".to_string(),
                    "flatpak:me.proton.Pass".to_string(),
                    "flatpak:org.ferdium.Ferdium".to_string(),
                    "flatpak:io.github.halfmexican.Mingle".to_string(),
                    "flatpak:app.zen_browser.zen".to_string(),
                    "appimage:browseros".to_string(),
                    "flatpak:io.github.alamahant.TarotCaster".to_string(),
                    "flatpak:land.arcana.TarotCanvas".to_string(),
                    "flatpak:io.github.alamahant.Asteria".to_string(),
                    "flatpak:it.astrogods.AstroGods".to_string(),
                    "flatpak:org.stellarium.Stellarium".to_string(),
                    "flatpak:com.rafaelmardojai.Blanket".to_string(),
                    "flatpak:net.ankiweb.Anki".to_string(),
                    "flatpak:dev.toastbits.spmp".to_string(),
                    "flatpak:com.wps.Office".to_string(),
                    "flatpak:net.bontal.Catalyst".to_string(),
                    "flatpak:com.github.dahenson.agenda".to_string(),
                    "flatpak:dev.qwery.AddWater".to_string(),
                    "flatpak:com.github.joseexposito.touche".to_string(),
                    "flatpak:io.github.sitraorg.sitra".to_string(),
                    "flatpak:com.github.appadeia.Taigo".to_string(),
                ],
            );

            // Essential apps for new users
            map.insert(
                "Essentials".to_string(),
                vec![
                    "flatpak:org.mozilla.firefox".to_string(),
                    "flatpak:org.libreoffice.LibreOffice".to_string(),
                    "flatpak:org.videolan.VLC".to_string(),
                    "flatpak:org.gimp.GIMP".to_string(),
                    "flatpak:com.discordapp.Discord".to_string(),
                    "flatpak:org.telegram.desktop".to_string(),
                ],
            );

            // Popular / Trending apps
            map.insert(
                "Trending".to_string(),
                vec![
                    "flatpak:com.spotify.Client".to_string(),
                    "flatpak:com.discordapp.Discord".to_string(),
                    "flatpak:md.obsidian.Obsidian".to_string(),
                    "snap:code".to_string(),
                    "flatpak:org.jetbrains.IntelliJ-IDEA-Community".to_string(),
                    "flatpak:com.valvesoftware.Steam".to_string(),
                ],
            );

            map.insert(
                "Audio".to_string(),
                vec![
                    "flatpak:com.spotify.Client".to_string(),
                    "flatpak:org.audacityteam.Audacity".to_string(),
                    "flatpak:org.ardour.Ardour".to_string(),
                    "flatpak:org.lmms.LMMS".to_string(),
                    "flatpak:com.obsproject.Studio".to_string(),
                ],
            );
            map.insert(
                "Video".to_string(),
                vec![
                    "flatpak:org.videolan.VLC".to_string(),
                    "flatpak:org.kde.kdenlive".to_string(),
                    "flatpak:com.obsproject.Studio".to_string(),
                    "flatpak:tv.kodi.Kodi".to_string(),
                    "flatpak:com.handbrake.HandBrake".to_string(),
                ],
            );
            map.insert(
                "Development".to_string(),
                vec![
                    "snap:code".to_string(),
                    "flatpak:org.jetbrains.IntelliJ-IDEA-Community".to_string(),
                    "flatpak:io.dbeaver.DBeaverCommunity".to_string(),
                    "snap:postman".to_string(),
                    "flatpak:org.gnome.Builder".to_string(),
                    "appimage:neovim".to_string(),
                ],
            );
            map.insert(
                "Education".to_string(),
                vec![
                    "flatpak:org.kde.gcompris".to_string(),
                    "flatpak:org.stellarium.Stellarium".to_string(),
                    "snap:anki-yan".to_string(),
                    "flatpak:edu.mit.Scratch".to_string(),
                    "flatpak:org.inkscape.Inkscape".to_string(),
                ],
            );
            map.insert(
                "Game".to_string(),
                vec![
                    "flatpak:com.valvesoftware.Steam".to_string(),
                    "flatpak:net.lutris.Lutris".to_string(),
                    "flatpak:org.prismlauncher.PrismLauncher".to_string(),
                    "flatpak:org.libretro.RetroArch".to_string(),
                    "flatpak:com.heroicgameslauncher.hgl".to_string(),
                    "appimage:supertuxkart".to_string(),
                ],
            );
            map.insert(
                "Graphics".to_string(),
                vec![
                    "flatpak:org.gimp.GIMP".to_string(),
                    "flatpak:org.inkscape.Inkscape".to_string(),
                    "flatpak:org.blender.Blender".to_string(),
                    "snap:krita".to_string(),
                    "flatpak:com.rafaelmardojai.Blanket".to_string(),
                ],
            );
            map.insert(
                "Network".to_string(),
                vec![
                    "flatpak:org.mozilla.firefox".to_string(),
                    "snap:chromium".to_string(),
                    "flatpak:com.discordapp.Discord".to_string(),
                    "flatpak:org.telegram.desktop".to_string(),
                    "flatpak:org.qbittorrent.qBittorrent".to_string(),
                ],
            );
            map.insert(
                "Office".to_string(),
                vec![
                    "flatpak:org.libreoffice.LibreOffice".to_string(),
                    "snap:onlyoffice-desktopeditors".to_string(),
                    "flatpak:md.obsidian.Obsidian".to_string(),
                    "flatpak:org.gnome.Evince".to_string(),
                    "flatpak:com.adwaitaqt.Adwaita".to_string(),
                ],
            );
            map.insert(
                "Science".to_string(),
                vec![
                    "flatpak:org.octave.Octave".to_string(),
                    "flatpak:org.kicad.KiCad".to_string(),
                    "flatpak:org.freecadweb.FreeCAD".to_string(),
                    "flatpak:org.stellarium.Stellarium".to_string(),
                ],
            );
            map.insert("Settings".to_string(), vec![]);
            map.insert(
                "System".to_string(),
                vec![
                    "flatpak:org.gnome.SystemMonitor".to_string(),
                    "flatpak:org.bleachbit.BleachBit".to_string(),
                    "appimage:balena-etcher".to_string(),
                    "snap:htop".to_string(),
                    "flatpak:com.mattjakeman.ExtensionManager".to_string(),
                ],
            );
            map.insert(
                "Utilities".to_string(),
                vec![
                    "flatpak:org.gnome.Calculator".to_string(),
                    "flatpak:org.keepassxc.KeePassXC".to_string(),
                    "flatpak:com.bitwarden.desktop".to_string(),
                    "appimage:appimagelauncher".to_string(),
                    "flatpak:com.github.tchx84.Flatseal".to_string(),
                ],
            );
            map
        },
    };

    // Initialize backend in a background-friendly way
    let backend = runtime.block_on(async {
        eprintln!("Initializing backend core...");
        let backend = match BackendService::new(home_content) {
            Ok(b) => Arc::new(b),
            Err(e) => {
                eprintln!("Failed to initialize backend: {}", e);
                return Err::<Arc<BackendService>, Box<dyn std::error::Error + Send + Sync>>(e);
            }
        };
        Ok(backend)
    })?;

    match cli_command {
        CliCommand::RunUi => {}
        CliCommand::PrintHelp => unreachable!("handled before runtime initialization"),
        CliCommand::RefreshOnly => {
            runtime.block_on(async { backend.refresh_cache().await })?;
            return Ok(());
        }
        CliCommand::ExportCatalog(path) => {
            backend.export_metadata_catalog(&path)?;
            eprintln!("Exported metadata catalog to {}", path.display());
            return Ok(());
        }
        CliCommand::ImportCatalog(path) => {
            let count = backend.import_metadata_catalog(&path)?;
            eprintln!(
                "Imported {} metadata catalog entries from {}",
                count,
                path.display()
            );
            return Ok(());
        }
        CliCommand::RefreshAndExport(path) => {
            runtime.block_on(async { backend.refresh_cache().await })?;
            backend.export_metadata_catalog(&path)?;
            eprintln!(
                "Refreshed package cache and exported metadata catalog to {}",
                path.display()
            );
            return Ok(());
        }
        CliCommand::SelfTest => {
            run_self_test(&runtime, &backend)?;
            return Ok(());
        }
    }

    // Create Slint UI only for interactive mode so headless commands work in CI and shell tests.
    eprintln!("Creating Slint UI...");
    let ui = MainWindow::new()?;

    // Window icon is handled via the .slint file property
    let _ui_handle = ui.as_weak();

    /* // Set window icon - disabled due to slint feature conflict in this environment
    if let Ok(img) = image::open("/home/lilith/Lilith-Linux/Offerings/assets/icon-logo.png") {
        let rgba = img.into_rgba8();
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
        ui.window().set_icon(slint::Image::from_rgba8(buffer));
    } */

    eprintln!("Slint UI created and visible!");

    // Set initial loading state
    ui.set_is_loading(true);
    ui.set_loading_status("Preparing the Offerings...".into());
    ui.set_loading_progress(0.1);

    let backend_clone = backend.clone();

    // Start background tasks for data population
    let backend_init = backend.clone();
    let ui_init = ui.as_weak();
    runtime.spawn(async move {
        // Step 1: Lighting the fires (Refresh Cache)
        let ui_handle = ui_init.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_loading_status("Lighting the Fires...".into());
                ui.set_loading_progress(0.2);
            }
        });

        if let Err(e) = backend_init.refresh_cache().await {
            eprintln!("Warning: Initial cache refresh failed: {}", e);
        }

        // Step 1.5: Wave 12.0 Background Update Check
        backend_init.check_for_updates_background().await;

        // Step 2: Syncing Sources
        let ui_handle = ui_init.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_loading_status("Syncing Package Sources...".into());
                ui.set_loading_progress(0.5);
            }
        });

        // Step 3: Populating Lists
        let ui_handle = ui_init.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_loading_status("Preparing the Offerings...".into());
                ui.set_loading_progress(0.8);
            }
        });

        // Final population of all UI data
        populate_ui_async(&ui_init, &backend_init).await;

        // Start background refresh task for ongoing updates (every 30 minutes)
        let _background_refresh = backend_init.start_background_refresh(1800);
    });

    // Start IPC server (if needed)
    let mut ipc_server = IpcServer::new();
    let ipc_receiver = match ipc_server.start(runtime.handle().clone()) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("Warning: Failed to start IPC server: {}", e);
            None
        }
    };

    // Handle IPC commands in background
    if let Some(mut receiver) = ipc_receiver {
        let backend_ipc = backend.clone();
        runtime.spawn(async move {
            while let Some(cmd) = receiver.recv().await {
                match cmd {
                    IpcCommand::Request(request, response_sender) => {
                        let response = handle_ipc_request(&backend_ipc, request).await;
                        let _ = response_sender.send(response).await;
                    }
                    IpcCommand::Shutdown => break,
                }
            }
        });
    }

    // Populate system fonts
    let font_list: Vec<slint::SharedString> = get_system_fonts()
        .into_iter()
        .map(slint::SharedString::from)
        .collect();
    ui.set_available_fonts(slint::ModelRc::new(slint::VecModel::from(font_list)));

    // Set empty models for initial UI (will be populated asynchronously)
    ui.set_featured_audio(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_video(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_development(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_education(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_game(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_graphics(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_network(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_office(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_science(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_settings(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_system(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_utilities(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_lilith(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_essentials(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_featured_trending(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_installed_packages(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_recently_updated(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    ui.set_selected_deps(slint::ModelRc::new(slint::VecModel::from(Vec::new())));

    // Set source URLs from the backend (stored in DB)
    let initial_sources = backend.get_sources();
    ui.set_source_urls(convert_to_slint_sources(initial_sources));
    eprintln!("Source URLs set from DB");

    let tokio_handle = runtime.handle().clone();

    // Search handler
    ui.on_search_triggered({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |query| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let query = query.to_string();

            let tokio_handle_spawn = tokio_handle.clone();
            tokio_handle.spawn(async move {
                let results = backend.search_apps(&query).await;
                let search_sem = Arc::new(tokio::sync::Semaphore::new(16));
                spawn_list_icon_fetcher(
                    results.clone(),
                    tokio_handle_spawn,
                    ui_weak.clone(),
                    "search".into(),
                    None,
                    search_sem,
                );
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_search_results(convert_to_slint_packages(results));
                    }
                });
            });
        }
    });

    // Install handler with progress tracking
    ui.on_install_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            eprintln!("Install clicked for: {}", pkg_id);

            // Show installing status
            let _ = slint::invoke_from_event_loop({
                let ui_weak = ui_weak.clone();
                let pkg_id = pkg_id.clone();
                move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_installing_package_id(pkg_id.into());
                        ui.set_install_progress(0.0);
                    }
                }
            });

            // Progress callback for real-time updates
            let ui_progress_weak = ui_weak.clone();
            let base_callback = Arc::new(move |p: f32| {
                let ui_weak = ui_progress_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_install_progress(p.min(0.95));
                    }
                });
            });
            let progress_callback = wrap_progress_callback(base_callback);

            tokio_handle.spawn(async move {
                // Get package name for progress overlay
                let pkg_name = if let Some(pkg) = backend.get_package(&pkg_id).await {
                    pkg.identity.name
                } else {
                    pkg_id.clone()
                };

                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak.clone();
                    let name = pkg_name.clone();
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_installing_package_name(name.into());
                            ui.set_is_uninstalling(false);
                        }
                    }
                });

                let op = PackageOperation::Install(pkg_id.clone());
                match backend.execute_operation(op, Some(progress_callback)).await {
                    Ok(result) => {
                        eprintln!(
                            "Install result: success={}, message={}",
                            result.success, result.message
                        );

                        if result.success {
                            let package = match backend.reconcile_package_state(&pkg_id, true).await
                            {
                                Some(pkg) => Some(pkg),
                                None => backend.get_package(&pkg_id).await,
                            };
                            let installed = backend.get_installed_packages().await;

                            let _ = slint::invoke_from_event_loop({
                                let ui_weak = ui_weak.clone();
                                let installed = installed.clone();
                                move || {
                                    if let Some(ui) = ui_weak.upgrade() {
                                        if let Some(pkg) = package {
                                            let mut slint_pkg = package_to_slint_info(pkg.clone());
                                            slint_pkg.is_installed = true; // Ensure UI reflects it instantly

                                            // Create desktop entry
                                            create_desktop_entry(&pkg);

                                            // Update all active models with the new state
                                            update_package_in_all_models(&ui, &slint_pkg);

                                            // Update selected package if it's the one we just installed
                                            if ui.get_selected_package().id == slint_pkg.id {
                                                ui.set_selected_package(slint_pkg);
                                            }
                                        }

                                        ui.set_installed_packages(convert_to_slint_packages(
                                            installed,
                                        ));
                                        ui.set_install_progress(1.0);

                                        // Clear overlay last so user sees the transition
                                        let ui_weak_done = ui_weak.clone();
                                        slint::Timer::single_shot(
                                            std::time::Duration::from_millis(250),
                                            move || {
                                                if let Some(ui) = ui_weak_done.upgrade() {
                                                    ui.set_installing_package_id("".into());
                                                }
                                            },
                                        );
                                    }
                                }
                            });
                        } else {
                            eprintln!("Install failed: {}", result.message);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    ui.set_installing_package_id("".into());
                                }
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("Install error: {}", e);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installing_package_id("".into());
                            }
                        });
                    }
                }
            });
        }
    });

    // Update handler
    ui.on_update_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            tokio_handle.spawn(async move {
                let op = PackageOperation::Update(pkg_id.clone());
                match backend.execute_operation(op, None).await {
                    Ok(_) => {
                        let installed = backend.get_installed_packages().await;
                        let selected = backend.get_package(&pkg_id).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installed_packages(convert_to_slint_packages(installed));
                                if let Some(selected) = selected {
                                    ui.set_selected_package(package_to_slint_info(selected));
                                }
                            }
                        });
                    }
                    Err(e) => eprintln!("Update failed: {}", e),
                }
            });
        }
    });

    // Uninstall handler
    ui.on_open_clicked({
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            tokio_handle.spawn(async move {
                if let Err(e) = backend.launch_package(&pkg_id).await {
                    eprintln!("Error launching package {}: {}", pkg_id, e);
                }
            });
        }
    });

    ui.on_uninstall_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            eprintln!("Uninstall clicked for: {}", pkg_id);

            // Show uninstall status
            let _ = slint::invoke_from_event_loop({
                let ui_weak = ui_weak.clone();
                let pkg_id = pkg_id.clone();
                move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_installing_package_id(pkg_id.into());
                        ui.set_is_uninstalling(true);
                        ui.set_install_progress(0.0);
                    }
                }
            });

            // Progress callback for real-time updates
            let ui_progress_weak = ui_weak.clone();
            let base_callback = Arc::new(move |p: f32| {
                let ui_weak = ui_progress_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_install_progress(p.min(0.95));
                    }
                });
            });
            let progress_callback = wrap_progress_callback(base_callback);

            tokio_handle.spawn(async move {
                // Get package name
                let pkg_name = if let Some(pkg) = backend.get_package(&pkg_id).await {
                    pkg.identity.name
                } else {
                    pkg_id.clone()
                };

                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak.clone();
                    let name = pkg_name.clone();
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_installing_package_name(name.into());
                        }
                    }
                });

                let op = PackageOperation::Uninstall(pkg_id.clone());
                match backend.execute_operation(op, Some(progress_callback)).await {
                    Ok(_) => {
                        let installed = backend.get_installed_packages().await;
                        let package = match backend.reconcile_package_state(&pkg_id, false).await {
                            Some(pkg) => Some(pkg),
                            None => backend.get_package(&pkg_id).await,
                        };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installed_packages(convert_to_slint_packages(installed));
                                ui.set_install_progress(1.0);
                                if let Some(pkg) = package {
                                    let slint_pkg = package_to_slint_info(pkg);
                                    update_package_in_all_models(&ui, &slint_pkg);
                                    if ui.get_selected_package().id == slint_pkg.id {
                                        ui.set_selected_package(slint_pkg);
                                    }
                                }
                                let ui_weak_done = ui_weak.clone();
                                slint::Timer::single_shot(
                                    std::time::Duration::from_millis(250),
                                    move || {
                                        if let Some(ui) = ui_weak_done.upgrade() {
                                            ui.set_installing_package_id("".into());
                                        }
                                    },
                                );
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Uninstall failed: {}", e);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installing_package_id("".into());
                            }
                        });
                    }
                }
            });
        }
    });

    // Update All handler
    ui.on_update_all_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move || {
            let backend = backend.clone();
            let ui_weak = ui_weak.clone();

            tokio_handle.spawn(async move {
                let op = PackageOperation::UpdateAll;
                match backend.execute_operation(op, None).await {
                    Ok(_) => {
                        let installed = backend.get_installed_packages().await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installed_packages(convert_to_slint_packages(installed));
                            }
                        });
                    }
                    Err(e) => eprintln!("Update all failed: {}", e),
                }
            });
        }
    });

    // Package click handler
    ui.on_package_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let ui_weak = ui_weak.clone();
            let pkg_id = pkg_id.to_string();
            eprintln!("Package clicked: {}", pkg_id);

            tokio_handle.spawn(async move {
                match backend.get_package(&pkg_id).await {
                    Some(pkg) => {
                        let screenshots = pkg.metadata.screenshots.clone();
                        let icon_url = pkg.metadata.icon_url.clone();
                        let pkg_clone = pkg.clone();

                        let _ = slint::invoke_from_event_loop({
                            let ui_weak = ui_weak.clone();
                            move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    eprintln!(
                                        "Setting selected package and showing detail view..."
                                    );
                                    ui.set_selected_package(package_to_slint_info(pkg_clone));
                                    ui.set_current_screenshot_index(0);
                                    ui.set_detail_carousel_offset(0);
                                    ui.set_selected_package_screenshots(slint::ModelRc::new(
                                        slint::VecModel::from(vec![]),
                                    ));
                                    ui.set_has_package_icon(false);
                                    ui.set_show_package_detail(true);
                                    ui.set_show_settings(false);
                                    ui.set_show_dep_detail(false);
                                    ui.set_is_transitioning(false); // Ensure we're not blocked
                                }
                            }
                        });

                        load_detail_media(ui_weak.clone(), icon_url, screenshots).await;
                    }
                    None => {
                        eprintln!("Error: Package {} not found in backend!", pkg_id);
                    }
                }
            });
        }
    });
    // Source selection handler (switches view on detail page)
    ui.on_source_selected({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let ui_weak = ui_weak.clone();
            let pkg_id = pkg_id.to_string();
            eprintln!("Source selected: {}", pkg_id);

            tokio_handle.spawn(async move {
                match backend.get_package(&pkg_id).await {
                    Some(pkg) => {
                        let screenshots = pkg.metadata.screenshots.clone();
                        let icon_url = pkg.metadata.icon_url.clone();
                        let pkg_clone = pkg.clone();

                        let _ = slint::invoke_from_event_loop({
                            let ui_weak = ui_weak.clone();
                            move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    ui.set_selected_package(package_to_slint_info(pkg_clone));
                                    ui.set_current_screenshot_index(0);
                                    ui.set_detail_carousel_offset(0);
                                    ui.set_selected_package_screenshots(slint::ModelRc::new(
                                        slint::VecModel::from(vec![]),
                                    ));
                                    ui.set_has_package_icon(false);
                                }
                            }
                        });

                        load_detail_media(ui_weak.clone(), icon_url, screenshots).await;
                    }
                    None => {
                        eprintln!("Error: Package {} not found in backend!", pkg_id);
                    }
                }
            });
        }
    });

    // Category selection handler - loads ALL packages for selected category
    // Category selection handler - loads ALL packages for selected category
    ui.on_category_selected({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |category| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let category = category.to_string();

            eprintln!("UI Category Selected: {}", category);
            let category_label = category.clone();

            // Show loading screen during transition
            let _ = slint::invoke_from_event_loop({
                let ui_weak = ui_weak.clone();
                let category = category_label.clone();
                move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        eprintln!("Showing transition loading for category: {}", category);
                        ui.set_is_loading(true);
                        ui.set_is_transitioning(true);
                        ui.set_loading_progress(0.2);
                        ui.set_loading_status(format!("Exploring {}...", category).into());
                    }
                }
            });

            let tokio_handle_spawn = tokio_handle.clone();
            tokio_handle.spawn(async move {
                // Load ALL packages for this category
                let all_matches = backend.get_apps_by_category(&category).await;
                let pkg_count = all_matches.len();
                eprintln!(
                    "Category '{}': loaded {} matches for display",
                    category, pkg_count
                );

                // INITIAL PAGE: 100 packages
                let initial_page_size = 100;
                let initial_packages: Vec<model::Package> = all_matches
                    .iter()
                    .take(initial_page_size)
                    .cloned()
                    .collect();
                let has_more = pkg_count > initial_page_size;

                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak.clone();
                    let category = category.clone();
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_loading_progress(0.8);
                            ui.set_current_view(category.clone().into());
                            ui.set_current_category_id(category.clone().into());
                            ui.set_current_category_visible_count(initial_packages.len() as i32);
                            ui.set_current_category_has_more(has_more);

                            // Find category name/icon
                            let (cat_name, cat_icon) = {
                                let mut res = (category.clone(), "📦".to_string());
                                for c in ui.get_categories().iter() {
                                    if c.id == category {
                                        res = (c.name.to_string(), c.icon.to_string());
                                        break;
                                    }
                                }
                                res
                            };

                            let slint_pkgs = convert_to_slint_packages(initial_packages.clone());

                            ui.set_current_category_packages(slint_pkgs.clone());
                            ui.set_current_category_name(cat_name.into());
                            ui.set_current_category_icon(cat_icon.into());

                            // Trigger icon fetching
                            let tokio_handle_inner = tokio_handle_spawn.clone();
                            let shared_sem = Arc::new(tokio::sync::Semaphore::new(16));
                            spawn_list_icon_fetcher(
                                initial_packages.clone(),
                                tokio_handle_inner,
                                ui_weak.clone(),
                                "catalogue".into(),
                                None,
                                shared_sem,
                            );

                            // Synchronize specific category properties if needed
                            match category.as_str() {
                                "android" => ui.set_category_android(slint_pkgs),
                                "comic" => ui.set_category_comic(slint_pkgs),
                                "command-line" => ui.set_category_command_line(slint_pkgs),
                                "communication" => ui.set_category_communication(slint_pkgs),
                                "disk" => ui.set_category_disk(slint_pkgs),
                                "file-manager" => ui.set_category_file_manager(slint_pkgs),
                                "finance" => ui.set_category_finance(slint_pkgs),
                                "gnome" => ui.set_category_gnome(slint_pkgs),
                                "kde" => ui.set_category_kde(slint_pkgs),
                                "password" => ui.set_category_password(slint_pkgs),
                                "steam" => ui.set_category_steam(slint_pkgs),
                                "web-app" => ui.set_category_web_app(slint_pkgs),
                                "web-browser" => ui.set_category_web_browser(slint_pkgs),
                                "wine" => ui.set_category_wine(slint_pkgs),
                                "system-monitor" => ui.set_category_system_monitor(slint_pkgs),
                                "miscellaneous" => ui.set_category_miscellaneous(slint_pkgs),
                                _ => {}
                            }
                        }
                    }
                });

                finish_category_transition(&ui_weak, category.clone());
            });
        }
    });

    // Wave 15.0: Load More pagination handler
    ui.on_load_more({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        let tokio_handle = tokio_handle.clone();
        move |category_id| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let category_id = category_id.to_string();
            let tokio_handle_spawn = tokio_handle.clone();
            let icon_fetch_handle = tokio_handle_spawn.clone();

            tokio_handle_spawn.spawn(async move {
                let all_matches = backend.get_apps_by_category(&category_id).await;

                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak.clone();
                    let all_matches = all_matches.clone();
                    move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            let current_count = ui.get_current_category_visible_count() as usize;

                            let next_page_size = 100;
                            let next_packages: Vec<model::Package> = all_matches
                                .iter()
                                .skip(current_count)
                                .take(next_page_size)
                                .cloned()
                                .collect();

                            if next_packages.is_empty() {
                                ui.set_current_category_has_more(false);
                                return;
                            }

                            let has_more =
                                all_matches.len() > (current_count + next_packages.len());

                            let mut combined_packages = all_matches
                                .iter()
                                .take(current_count + next_packages.len())
                                .cloned()
                                .collect::<Vec<_>>();
                            combined_packages.shrink_to_fit();
                            ui.set_current_category_packages(convert_to_slint_packages(
                                combined_packages,
                            ));
                            ui.set_current_category_visible_count(
                                (current_count + next_packages.len()) as i32,
                            );
                            ui.set_current_category_has_more(has_more);

                            // Fetch icons for new batch
                            let tokio_handle_inner = icon_fetch_handle.clone();
                            let shared_sem = Arc::new(tokio::sync::Semaphore::new(16));
                            spawn_list_icon_fetcher(
                                next_packages,
                                tokio_handle_inner,
                                ui_weak.clone(),
                                "catalogue".into(),
                                None,
                                shared_sem,
                            );
                        }
                    }
                });
            });
        }
    });

    // Add source handler
    ui.on_add_source({
        let ui_weak = ui.as_weak();
        let backend = backend.clone();
        let tokio_handle = tokio_handle.clone();
        move |name, url| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let name = name.to_string();
            let url = url.to_string();
            tokio_handle.spawn(async move {
                if backend.add_source(name, url).await.is_ok() {
                    let sources = backend.get_sources();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_source_urls(convert_to_slint_sources(sources));
                        }
                    });
                }
            });
        }
    });

    // Remove source handler
    ui.on_remove_source({
        let ui_weak = ui.as_weak();
        let backend = backend.clone();
        let tokio_handle = tokio_handle.clone();
        move |url| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let url = url.to_string();
            tokio_handle.spawn(async move {
                if backend.remove_source(url).await.is_ok() {
                    let sources = backend.get_sources();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_source_urls(convert_to_slint_sources(sources));
                        }
                    });
                }
            });
        }
    });

    ui.on_open_homepage({
        move |url| {
            let url = url.to_string();
            if !url.is_empty() {
                eprintln!("Opening homepage: {}", url);
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            }
        }
    });

    eprintln!("Offerings is starting event loop!");
    println!("Offerings is running!");

    // Show loading screen initially
    ui.set_is_loading(true);
    ui.set_loading_progress(0.0);
    ui.set_loading_status("Initializing...".into());

    // Set up a one-shot timer to populate data after UI is shown
    let ui_weak = ui.as_weak();
    let backend_clone = backend.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_millis(100),
        move || {
            eprintln!("Timer fired, populating UI data...");
            let ui_weak2 = ui_weak.clone();
            let backend_clone2 = backend_clone.clone();

            // Show loading state
            let _ = slint::invoke_from_event_loop({
                let ui_weak2 = ui_weak2.clone();
                move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_is_loading(true);
                        ui.set_loading_progress(0.0);
                        ui.set_loading_status("Loading packages from sources...".into());
                    }
                }
            });

            // Use std::thread for async work, then invoke_from_event_loop for UI updates
            std::thread::spawn(move || {
                // Create a new runtime for this thread
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
                    populate_ui_async(&ui_weak2, &backend_clone2).await;
                });
            });
        },
    );
    eprintln!("Timer started for UI population");

    // Run the UI - this will block until the window is closed
    // Async tasks will update the UI via invoke_from_event_loop
    let result = ui.run();
    eprintln!("Offerings event loop exited with: {:?}", result);
    result?;

    Ok(())
}

/// Populate UI with initial data (reserved for future use)
#[allow(dead_code)]
async fn populate_ui(ui: &MainWindow, backend: &BackendService) {
    let home_content = backend.get_home_content().await;

    // Get packages by category
    let audio_pkgs = get_category_packages(backend, "Audio", &home_content).await;
    let video_pkgs = get_category_packages(backend, "Video", &home_content).await;
    let dev_pkgs = get_category_packages(backend, "Development", &home_content).await;
    let edu_pkgs = get_category_packages(backend, "Education", &home_content).await;
    let game_pkgs = get_category_packages(backend, "Game", &home_content).await;
    let graphics_pkgs = get_category_packages(backend, "Graphics", &home_content).await;
    let network_pkgs = get_category_packages(backend, "Network", &home_content).await;
    let office_pkgs = get_category_packages(backend, "Office", &home_content).await;
    let science_pkgs = get_category_packages(backend, "Science", &home_content).await;
    let settings_pkgs = get_category_packages(backend, "Settings", &home_content).await;
    let system_pkgs = get_category_packages(backend, "System", &home_content).await;
    let utils_pkgs = get_category_packages(backend, "Utilities", &home_content).await;
    let lilith_pkgs = get_category_packages(backend, "Lilith", &home_content).await;

    // Set category packages
    ui.set_featured_audio(convert_to_slint_packages(audio_pkgs));
    ui.set_featured_video(convert_to_slint_packages(video_pkgs));
    ui.set_featured_development(convert_to_slint_packages(dev_pkgs));
    ui.set_featured_education(convert_to_slint_packages(edu_pkgs));
    ui.set_featured_game(convert_to_slint_packages(game_pkgs));
    ui.set_featured_graphics(convert_to_slint_packages(graphics_pkgs));
    ui.set_featured_network(convert_to_slint_packages(network_pkgs));
    ui.set_featured_office(convert_to_slint_packages(office_pkgs));
    ui.set_featured_science(convert_to_slint_packages(science_pkgs));
    ui.set_featured_settings(convert_to_slint_packages(settings_pkgs));
    ui.set_featured_system(convert_to_slint_packages(system_pkgs));
    ui.set_featured_utilities(convert_to_slint_packages(utils_pkgs));
    ui.set_featured_lilith(convert_to_slint_packages(lilith_pkgs));

    // Installed packages (will be sorted by install date in the UI)
    let installed = backend.get_installed_packages().await;
    ui.set_installed_packages(convert_to_slint_packages(installed));

    // Recently updated packages
    let updates = backend.get_updates().await;
    ui.set_recently_updated(convert_to_slint_packages(updates));
    ui.set_selected_deps(slint::ModelRc::new(slint::VecModel::from(Vec::<
        DependencyItem,
    >::new())));

    // Populate source URLs for Settings -> Sources tab
    let available_sources = backend.get_available_sources().await;
    let source_items: Vec<model::SourceItem> = available_sources
        .iter()
        .map(|src| {
            let label = src.label();
            model::SourceItem {
                name: label.into(),
                url: source_url(label).into(),
            }
        })
        .collect();
    ui.set_source_urls(convert_to_slint_sources(source_items));
}

/// Populate UI with initial data (async version for use with weak reference)
async fn populate_ui_async(ui_weak: &slint::Weak<MainWindow>, backend: &BackendService) {
    // Clone the weak reference for use after async operations
    let ui_handle = ui_weak.clone();
    let shared_icon_sem = Arc::new(tokio::sync::Semaphore::new(24)); // Shared limit for all rows
    let tokio_handle = tokio::runtime::Handle::current();

    eprintln!("Loading category packages...");

    // Use a timer-based smooth progress that completes as categories load
    let ui_handle_timer = ui_handle.clone();

    // Initial state
    let _ = slint::invoke_from_event_loop({
        let ui_handle = ui_handle_timer.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_is_loading(true);
                ui.set_loading_progress(0.01);
                ui.set_loading_status("Preparing the Offerings...".into());
            }
        }
    });

    // Start a background smooth progress incrementer (2% per 300ms = ~50s to reach 95%)
    let ui_handle_smooth = ui_handle.clone();
    tokio::spawn(async move {
        let mut p: f32 = 0.01;
        while p < 0.95 {
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            p = (p + 0.02).min(0.95);
            let new_p = p;
            let _ = slint::invoke_from_event_loop({
                let ui_handle = ui_handle_smooth.clone();
                move || {
                    if let Some(ui) = ui_handle.upgrade() {
                        // Only advance, never go backwards
                        if ui.get_loading_progress() < new_p {
                            ui.set_loading_progress(new_p);
                        }
                    }
                }
            });
            // Stop if window was closed
            if ui_handle_smooth.upgrade().is_none() {
                break;
            }
        }
    });

    let featured_audio = backend.get_apps_by_category("Audio").await;
    let featured_video = backend.get_apps_by_category("Video").await;
    let featured_dev = backend.get_apps_by_category("Development").await;
    let featured_edu = backend.get_apps_by_category("Education").await;
    let featured_game = backend.get_apps_by_category("Game").await;
    let featured_graphics = backend.get_apps_by_category("Graphics").await;
    let featured_network = backend.get_apps_by_category("Network").await;
    let featured_office = backend.get_apps_by_category("Office").await;
    let featured_science = backend.get_apps_by_category("Science").await;
    let featured_system = backend.get_apps_by_category("System").await;
    let featured_utilities = backend.get_apps_by_category("Utilities").await;
    let featured_lilith = backend.get_apps_by_category("Lilith").await;
    let featured_essentials = backend.get_apps_by_category("Essentials").await;
    let featured_trending = backend.get_apps_by_category("Trending").await;
    let featured_ai = backend.get_apps_by_category("AI").await;
    let featured_productivity = backend.get_apps_by_category("Productivity").await;
    let featured_desktop = backend.get_apps_by_category("Desktop Customization").await;

    let home_limit = Some(24);
    spawn_list_icon_fetcher(
        featured_audio.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "audio".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_video.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "video".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_dev.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "development".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_edu.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "education".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_game.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "game".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_graphics.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "graphics".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_network.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "network".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_office.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "office".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_science.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "science".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_system.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "system".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_utilities.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "utilities".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_lilith.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "lilith".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_essentials.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "essentials".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_trending.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "trending".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_ai.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "ai".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_productivity.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "productivity".into(),
        home_limit,
        shared_icon_sem.clone(),
    );
    spawn_list_icon_fetcher(
        featured_desktop.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "desktop".into(),
        home_limit,
        shared_icon_sem.clone(),
    );

    // Expanded categories from AM-GUI
    let cat_android = backend.get_apps_by_category("Android").await;
    spawn_list_icon_fetcher(
        cat_android.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "android".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_comic = backend.get_apps_by_category("Comic").await;
    spawn_list_icon_fetcher(
        cat_comic.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "comic".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_cmd = backend.get_apps_by_category("Command-line").await;
    spawn_list_icon_fetcher(
        cat_cmd.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "command-line".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_comm = backend.get_apps_by_category("Communication").await;
    spawn_list_icon_fetcher(
        cat_comm.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "communication".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_disk = backend.get_apps_by_category("Disk").await;
    spawn_list_icon_fetcher(
        cat_disk.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "disk".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_fm = backend.get_apps_by_category("File-manager").await;
    spawn_list_icon_fetcher(
        cat_fm.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "file-manager".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_finance = backend.get_apps_by_category("Finance").await;
    spawn_list_icon_fetcher(
        cat_finance.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "finance".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_gnome = backend.get_apps_by_category("Gnome").await;
    spawn_list_icon_fetcher(
        cat_gnome.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "gnome".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_kde = backend.get_apps_by_category("Kde").await;
    spawn_list_icon_fetcher(
        cat_kde.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "kde".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_pwd = backend.get_apps_by_category("Password").await;
    spawn_list_icon_fetcher(
        cat_pwd.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "password".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_steam = backend.get_apps_by_category("Steam").await;
    spawn_list_icon_fetcher(
        cat_steam.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "steam".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_sysmon = backend.get_apps_by_category("Monitor").await;
    spawn_list_icon_fetcher(
        cat_sysmon.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "system-monitor".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_webapp = backend.get_apps_by_category("WebApp").await;
    spawn_list_icon_fetcher(
        cat_webapp.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "web-app".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_webbrowser = backend.get_apps_by_category("Browser").await;
    spawn_list_icon_fetcher(
        cat_webbrowser.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "web-browser".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_wine = backend.get_apps_by_category("Wine").await;
    spawn_list_icon_fetcher(
        cat_wine.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "wine".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    let cat_misc = backend.get_apps_by_category("Miscellaneous").await;
    spawn_list_icon_fetcher(
        cat_misc.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "miscellaneous".into(),
        Some(12),
        shared_icon_sem.clone(),
    );

    // Installed packages
    let installed = backend.get_installed_packages().await;
    let has_updates = installed.iter().any(|p| p.version.has_update());
    let has_up_to_date = installed.iter().any(|p| !p.version.has_update());

    let _ = slint::invoke_from_event_loop({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_has_updates(has_updates);
                ui.set_has_up_to_date(has_up_to_date);
            }
        }
    });

    spawn_list_icon_fetcher(
        installed.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "installed".into(),
        None,
        shared_icon_sem.clone(),
    );

    let recently_updated = backend.get_updates().await;
    spawn_list_icon_fetcher(
        recently_updated.clone(),
        tokio_handle.clone(),
        ui_handle.clone(),
        "recently_updated".into(),
        None,
        shared_icon_sem.clone(),
    );

    let available_sources = backend.get_available_sources().await;
    let source_items: Vec<model::SourceItem> = available_sources
        .iter()
        .map(|src| {
            let label = src.label();
            model::SourceItem {
                name: label.into(),
                url: source_url(label).into(),
            }
        })
        .collect();

    let cat_settings = backend.get_apps_by_category("Settings").await;

    // Final update
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.set_featured_audio(limit_and_convert(featured_audio.clone(), 12));
            ui.set_featured_video(limit_and_convert(featured_video.clone(), 12));
            ui.set_featured_development(limit_and_convert(featured_dev.clone(), 12));
            ui.set_featured_education(limit_and_convert(featured_edu.clone(), 12));
            ui.set_featured_game(limit_and_convert(featured_game.clone(), 12));
            ui.set_featured_graphics(limit_and_convert(featured_graphics.clone(), 12));
            ui.set_featured_network(limit_and_convert(featured_network.clone(), 12));
            ui.set_featured_office(limit_and_convert(featured_office.clone(), 12));
            ui.set_featured_science(limit_and_convert(featured_science.clone(), 12));
            ui.set_featured_system(limit_and_convert(featured_system.clone(), 12));
            ui.set_featured_utilities(limit_and_convert(featured_utilities.clone(), 12));
            ui.set_featured_lilith(limit_and_convert(featured_lilith.clone(), 12));
            ui.set_featured_essentials(limit_and_convert(featured_essentials.clone(), 12));
            ui.set_featured_trending(limit_and_convert(featured_trending.clone(), 12));
            ui.set_featured_ai(limit_and_convert(featured_ai.clone(), 12));
            ui.set_featured_productivity(limit_and_convert(featured_productivity.clone(), 12));
            ui.set_featured_desktop(limit_and_convert(featured_desktop.clone(), 12));
            ui.set_installed_packages(convert_to_slint_packages(installed));
            ui.set_recently_updated(convert_to_slint_packages(recently_updated));
            ui.set_source_urls(convert_to_slint_sources(source_items));

            // Populate sidebar/homepage categories
            // Track which package IDs are already assigned to prevent cross-category duplicates
            let mut used_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

            // Helper: filter a category's packages, removing already-assigned ones from home page preview
            // BUT keep the true count from the full package list (not dedup-filtered) for the sidebar badge
            let mut make_cat =
                |id: &str, name: &str, icon: &str, pkgs: Vec<model::Package>| -> CategoryInfo {
                    let true_count = pkgs.len() as i32; // Accurate count before home-page dedup
                    let filtered: Vec<model::Package> = pkgs
                        .into_iter()
                        .filter(|p| !used_ids.contains(&p.identity.id))
                        .collect();
                    for p in &filtered {
                        used_ids.insert(p.identity.id.clone());
                    }
                    CategoryInfo {
                        id: id.into(),
                        name: name.into(),
                        icon: icon.into(),
                        package_count: true_count, // Show true count in sidebar badge
                        preview_packages: limit_and_convert(filtered.clone(), 12),
                        has_more: true_count > 12,
                    }
                };

            let cats = vec![
                // Process NICHE categories first so they claim their packages before broad ones
                make_cat("ai", "AI", "🤖", featured_ai.clone()),
                make_cat("game", "Games", "🎮", featured_game.clone()),
                make_cat("audio", "Audio", "🎵", featured_audio.clone()),
                make_cat("video", "Video", "🎬", featured_video.clone()),
                make_cat("education", "Education", "📚", featured_edu.clone()),
                make_cat("graphics", "Graphics", "🎨", featured_graphics.clone()),
                make_cat("office", "Office", "📄", featured_office.clone()),
                make_cat("science", "Science", "🔬", featured_science.clone()),
                make_cat("finance", "Finance", "💰", cat_finance.clone()),
                make_cat("development", "Development", "💻", featured_dev.clone()),
                make_cat("network", "Network", "🌐", featured_network.clone()),
                make_cat("communication", "Chat", "💬", cat_comm.clone()),
                make_cat("web-browser", "Browser", "🌍", cat_webbrowser.clone()),
                make_cat("desktop", "Desktop", "🖼️", featured_desktop.clone()),
                make_cat("steam", "Steam", "🎮", cat_steam.clone()),
                make_cat("android", "Android", "🤖", cat_android.clone()),
                make_cat("comic", "Comic", "📖", cat_comic.clone()),
                make_cat("wine", "Wine", "🍷", cat_wine.clone()),
                make_cat("gnome", "Gnome", "👣", cat_gnome.clone()),
                make_cat("kde", "KDE", "🐧", cat_kde.clone()),
                make_cat("password", "Security", "🔑", cat_pwd.clone()),
                make_cat("system-monitor", "Monitor", "📊", cat_sysmon.clone()),
                make_cat("web-app", "WebApp", "☁️", cat_webapp.clone()),
                make_cat("disk", "Disk", "💾", cat_disk.clone()),
                make_cat("file-manager", "Files", "📁", cat_fm.clone()),
                make_cat("command-line", "CLI", "🐚", cat_cmd.clone()),
                make_cat(
                    "productivity",
                    "Productivity",
                    "⏱️",
                    featured_productivity.clone(),
                ),
                // BROAD categories last - they absorb leftovers
                make_cat("system", "System", "⚙️", featured_system.clone()),
                make_cat("utilities", "Utilities", "🛠️", featured_utilities.clone()),
                make_cat("miscellaneous", "Miscellaneous", "📦", cat_misc.clone()),
            ];

            // Set the full category models (limited to 500 for performance)
            // This fixes the "7000 packages in Miscellaneous" lag issue
            let limit_full = |pkgs: Vec<model::Package>| -> slint::ModelRc<PackageInfo> {
                convert_to_slint_packages(pkgs.into_iter().take(500).collect())
            };

            ui.set_category_audio(limit_full(featured_audio.clone()));
            ui.set_category_video(limit_full(featured_video.clone()));
            ui.set_category_development(limit_full(featured_dev.clone()));
            ui.set_category_education(limit_full(featured_edu.clone()));
            ui.set_category_games(limit_full(featured_game.clone()));
            ui.set_category_graphics(limit_full(featured_graphics.clone()));
            ui.set_category_network(limit_full(featured_network.clone()));
            ui.set_category_office(limit_full(featured_office.clone()));
            ui.set_category_science(limit_full(featured_science.clone()));
            ui.set_category_settings(limit_full(cat_settings.clone()));
            ui.set_category_system(limit_full(featured_system.clone()));
            ui.set_category_utilities(limit_full(featured_utilities.clone()));
            ui.set_category_android(limit_full(cat_android.clone()));
            ui.set_category_comic(limit_full(cat_comic.clone()));
            ui.set_category_command_line(limit_full(cat_cmd.clone()));
            ui.set_category_communication(limit_full(cat_comm.clone()));
            ui.set_category_disk(limit_full(cat_disk.clone()));
            ui.set_category_file_manager(limit_full(cat_fm.clone()));
            ui.set_category_finance(limit_full(cat_finance.clone()));
            ui.set_category_gnome(limit_full(cat_gnome.clone()));
            ui.set_category_kde(limit_full(cat_kde.clone()));
            ui.set_category_password(limit_full(cat_pwd.clone()));
            ui.set_category_steam(limit_full(cat_steam.clone()));
            ui.set_category_system_monitor(limit_full(cat_sysmon.clone()));
            ui.set_category_web_app(limit_full(cat_webapp.clone()));
            ui.set_category_web_browser(limit_full(cat_webbrowser.clone()));
            ui.set_category_wine(limit_full(cat_wine.clone()));
            ui.set_category_miscellaneous(limit_full(cat_misc.clone()));

            // Remove empty categories from home page and sidebar
            let cats: Vec<CategoryInfo> =
                cats.into_iter().filter(|c| c.package_count > 0).collect();
            ui.set_categories(slint::ModelRc::new(slint::VecModel::from(cats)));

            ui.set_loading_progress(1.0);
            ui.set_loading_status("Complete!".into());

            // Wait a moment before hiding
            let ui_weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_is_loading(false);
                }
            });
        }
    });
}

fn limit_and_convert(packages: Vec<model::Package>, limit: usize) -> slint::ModelRc<PackageInfo> {
    let limited: Vec<model::Package> = packages.into_iter().take(limit).collect();
    convert_to_slint_packages(limited)
}

/// Get packages for a category (reserved for future use)
#[allow(dead_code)]
async fn get_category_packages(
    backend: &BackendService,
    category: &str,
    home_content: &HomePageContent,
) -> Vec<model::Package> {
    let mut packages = Vec::new();

    // First, try to fetch the featured/showcase packages for this category
    if let Some(pkg_ids) = home_content.category_showcases.get(category) {
        for id in pkg_ids {
            if let Some(pkg) = backend.get_package(id).await {
                packages.push(pkg);
            }
        }
    }

    // Also get packages from cache that match this category
    let category_pkgs = backend.get_apps_by_category(category).await;
    for pkg in category_pkgs {
        if !packages.iter().any(|p| p.identity.id == pkg.identity.id) {
            packages.push(pkg);
        }
    }

    packages
}

/// Handle IPC requests
async fn handle_ipc_request(backend: &BackendService, request: IpcRequest) -> IpcResponse {
    match request {
        IpcRequest::Status => {
            let installed = backend.get_installed_packages().await;
            let updates = backend.get_updates().await;

            IpcResponse {
                success: true,
                message: "OK".to_string(),
                data: Some(IpcResponseData::Status(StatusData {
                    running: true,
                    installed_count: installed.len(),
                    updates_available: updates.len(),
                    active_operations: 0,
                })),
            }
        }
        IpcRequest::Search { query } => {
            let results = backend.search_apps(&query).await;
            IpcResponse {
                success: true,
                message: format!("Found {} packages", results.len()),
                data: Some(IpcResponseData::Packages(
                    results.iter().map(PackageSummary::from).collect(),
                )),
            }
        }
        IpcRequest::GetPackage { id } => match backend.get_package(&id).await {
            Some(pkg) => IpcResponse {
                success: true,
                message: "OK".to_string(),
                data: Some(IpcResponseData::Package(PackageSummary::from(&pkg))),
            },
            None => IpcResponse {
                success: false,
                message: "Package not found".to_string(),
                data: None,
            },
        },
        IpcRequest::Install { id } => {
            match backend
                .execute_operation(PackageOperation::Install(id.clone()), None)
                .await
            {
                Ok(result) => IpcResponse {
                    success: result.success,
                    message: result.message.clone(),
                    data: Some(IpcResponseData::Operation(result)),
                },
                Err(e) => IpcResponse {
                    success: false,
                    message: e.to_string(),
                    data: None,
                },
            }
        }
        IpcRequest::Uninstall { id } => {
            match backend
                .execute_operation(PackageOperation::Uninstall(id), None)
                .await
            {
                Ok(result) => IpcResponse {
                    success: result.success,
                    message: result.message.clone(),
                    data: Some(IpcResponseData::Operation(result)),
                },
                Err(e) => IpcResponse {
                    success: false,
                    message: e.to_string(),
                    data: None,
                },
            }
        }
        IpcRequest::Update { id } => {
            match backend
                .execute_operation(PackageOperation::Update(id), None)
                .await
            {
                Ok(result) => IpcResponse {
                    success: result.success,
                    message: result.message.clone(),
                    data: Some(IpcResponseData::Operation(result)),
                },
                Err(e) => IpcResponse {
                    success: false,
                    message: e.to_string(),
                    data: None,
                },
            }
        }
        IpcRequest::UpdateAll => {
            match backend
                .execute_operation(PackageOperation::UpdateAll, None)
                .await
            {
                Ok(result) => IpcResponse {
                    success: result.success,
                    message: result.message.clone(),
                    data: Some(IpcResponseData::Operation(result)),
                },
                Err(e) => IpcResponse {
                    success: false,
                    message: e.to_string(),
                    data: None,
                },
            }
        }
        IpcRequest::ListInstalled => {
            let installed = backend.get_installed_packages().await;
            IpcResponse {
                success: true,
                message: format!("{} installed packages", installed.len()),
                data: Some(IpcResponseData::Packages(
                    installed.iter().map(PackageSummary::from).collect(),
                )),
            }
        }
        IpcRequest::ListUpdates => {
            let updates = backend.get_updates().await;
            IpcResponse {
                success: true,
                message: format!("{} updates available", updates.len()),
                data: Some(IpcResponseData::Packages(
                    updates.iter().map(PackageSummary::from).collect(),
                )),
            }
        }
        IpcRequest::Refresh => match backend.refresh_cache().await {
            Ok(_) => IpcResponse {
                success: true,
                message: "Cache refreshed".to_string(),
                data: None,
            },
            Err(e) => IpcResponse {
                success: false,
                message: e.to_string(),
                data: None,
            },
        },
        IpcRequest::Quit => IpcResponse {
            success: true,
            message: "Goodbye".to_string(),
            data: None,
        },
    }
}

/// Create a .desktop entry for an installed application to ensure it appears in the system menu
fn create_desktop_entry(pkg: &model::Package) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/lilith".to_string());
    let desktop_dir = format!("{}/.local/share/applications", home);
    let _ = std::fs::create_dir_all(&desktop_dir);

    // Sanitize ID for filename
    let safe_id = pkg.identity.id.replace([':', '/'], "-");
    let file_name = format!("{}.desktop", safe_id);
    let file_path = std::path::Path::new(&desktop_dir).join(file_name);

    // For Flatpaks, the system usually handles this, but let's ensure it exists if requested
    let app_id = if pkg.identity.id.contains(':') {
        pkg.identity
            .id
            .split(':')
            .nth(1)
            .unwrap_or(&pkg.identity.id)
    } else {
        &pkg.identity.id
    };

    let exec_cmd = if pkg.identity.id.contains("flatpak") {
        format!("flatpak run {}", app_id)
    } else {
        app_id.to_string()
    };

    let content = format!(
        "[Desktop Entry]\n\
        Type=Application\n\
        Name={}\n\
        Comment={}\n\
        Exec={}\n\
        Icon={}\n\
        Terminal=false\n\
        Categories=Utility;Application;\n\
        Keywords=Offerings;{};\n\
        X-Offerings-Pkg={}\n",
        pkg.identity.name,
        pkg.metadata.summary,
        exec_cmd,
        pkg.metadata
            .icon_url
            .clone()
            .unwrap_or_else(|| "package-x-generic".to_string()),
        app_id,
        pkg.identity.id
    );

    if let Err(e) = std::fs::write(&file_path, content) {
        eprintln!("Failed to create desktop entry at {:?}: {}", file_path, e);
    } else {
        eprintln!(
            "Successfully created desktop entry for {} at {:?}",
            pkg.identity.name, file_path
        );
    }
}

async fn fetch_image_from_url(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Offerings/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let width = decoded.width();
    let height = decoded.height();
    let rgba = decoded.into_rgba8();
    Some((rgba.into_raw(), width, height))
}

async fn load_detail_media(
    ui_weak: slint::Weak<MainWindow>,
    icon_url: Option<String>,
    screenshots: Vec<String>,
) {
    if let Some(url) = icon_url {
        if let Some((rgba, width, height)) = fetch_image_from_url(&url).await {
            let ui_icon_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_icon_weak.upgrade() {
                    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                        &rgba, width, height,
                    );
                    ui.set_selected_package_icon(slint::Image::from_rgba8(buffer));
                    ui.set_has_package_icon(true);
                }
            });
        }
    }

    if screenshots.is_empty() {
        return;
    }

    let mut images = Vec::new();
    for url in screenshots {
        if let Some(image) = fetch_image_from_url(&url).await {
            images.push(image);
        }
    }

    if !images.is_empty() {
        let ui_screenshots_weak = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_screenshots_weak.upgrade() {
                let decoded_images: Vec<slint::Image> = images
                    .into_iter()
                    .map(|(rgba, width, height)| {
                        let buffer =
                            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                &rgba, width, height,
                            );
                        slint::Image::from_rgba8(buffer)
                    })
                    .collect();
                ui.set_selected_package_screenshots(slint::ModelRc::new(slint::VecModel::from(
                    decoded_images,
                )));
                ui.set_current_screenshot_index(0);
                ui.set_detail_carousel_offset(0);
            }
        });
    }
}

/// Convert internal Package struct to Slint PackageInfo (Bulk)
fn convert_to_slint_packages(packages: Vec<model::Package>) -> slint::ModelRc<PackageInfo> {
    let slint_packages: Vec<PackageInfo> =
        packages.into_iter().map(package_to_slint_info).collect();

    slint::ModelRc::new(slint::VecModel::from(slint_packages))
}

/// Convert internal SourceItem struct to Slint SourceItem (Bulk)
fn convert_to_slint_sources(sources: Vec<model::SourceItem>) -> slint::ModelRc<SourceItem> {
    let slint_sources: Vec<SourceItem> = sources
        .into_iter()
        .map(|s| SourceItem {
            name: s.name.into(),
            url: s.url.into(),
        })
        .collect();

    slint::ModelRc::new(slint::VecModel::from(slint_sources))
}

/// Update a package in ALL active UI models to ensure real-time state synchronization
fn update_package_in_all_models(ui: &MainWindow, updated_pkg: &PackageInfo) {
    // Helper to update a specific model and re-set it in the UI
    fn update_model(
        model_rc: slint::ModelRc<PackageInfo>,
        updated_pkg: &PackageInfo,
    ) -> slint::ModelRc<PackageInfo> {
        let mut found = false;
        let vec: Vec<PackageInfo> = model_rc
            .iter()
            .map(|mut pkg| {
                if pkg.id == updated_pkg.id {
                    pkg.is_installed = updated_pkg.is_installed;
                    pkg.installed_version = updated_pkg.installed_version.clone();
                    pkg.has_update = updated_pkg.has_update;
                    found = true;
                }
                pkg
            })
            .collect();

        if found {
            slint::ModelRc::new(slint::VecModel::from(vec))
        } else {
            model_rc // Return unchanged if not found to avoid unnecessary allocations
        }
    }

    ui.set_featured_lilith(update_model(ui.get_featured_lilith(), updated_pkg));
    ui.set_featured_essentials(update_model(ui.get_featured_essentials(), updated_pkg));
    ui.set_featured_trending(update_model(ui.get_featured_trending(), updated_pkg));
    ui.set_featured_audio(update_model(ui.get_featured_audio(), updated_pkg));
    ui.set_featured_video(update_model(ui.get_featured_video(), updated_pkg));
    ui.set_featured_development(update_model(ui.get_featured_development(), updated_pkg));
    ui.set_featured_education(update_model(ui.get_featured_education(), updated_pkg));
    ui.set_featured_game(update_model(ui.get_featured_game(), updated_pkg));
    ui.set_featured_graphics(update_model(ui.get_featured_graphics(), updated_pkg));
    ui.set_featured_network(update_model(ui.get_featured_network(), updated_pkg));
    ui.set_featured_office(update_model(ui.get_featured_office(), updated_pkg));
    ui.set_featured_science(update_model(ui.get_featured_science(), updated_pkg));
    ui.set_featured_system(update_model(ui.get_featured_system(), updated_pkg));
    ui.set_featured_utilities(update_model(ui.get_featured_utilities(), updated_pkg));
    ui.set_featured_ai(update_model(ui.get_featured_ai(), updated_pkg));
    ui.set_featured_productivity(update_model(ui.get_featured_productivity(), updated_pkg));
    ui.set_featured_desktop(update_model(ui.get_featured_desktop(), updated_pkg));
    ui.set_featured_security(update_model(ui.get_featured_security(), updated_pkg));
    ui.set_featured_lifestyle(update_model(ui.get_featured_lifestyle(), updated_pkg));
    ui.set_featured_miscellaneous(update_model(ui.get_featured_miscellaneous(), updated_pkg));
    ui.set_search_results(update_model(ui.get_search_results(), updated_pkg));
    ui.set_current_category_packages(update_model(
        ui.get_current_category_packages(),
        updated_pkg,
    ));

    // Update the dynamic categories list (homepage sections)
    let cats_model = ui.get_categories();
    let updated_cats: Vec<CategoryInfo> = cats_model
        .iter()
        .map(|mut cat| {
            let mut preview_found = false;
            let updated_previews: Vec<PackageInfo> = cat
                .preview_packages
                .iter()
                .map(|mut p| {
                    if p.id == updated_pkg.id {
                        p.is_installed = updated_pkg.is_installed;
                        p.installed_version = updated_pkg.installed_version.clone();
                        p.has_update = updated_pkg.has_update;
                        preview_found = true;
                    }
                    p
                })
                .collect();

            if preview_found {
                cat.preview_packages = slint::ModelRc::new(slint::VecModel::from(updated_previews));
            }
            cat
        })
        .collect();
    ui.set_categories(slint::ModelRc::new(slint::VecModel::from(updated_cats)));
}

fn spawn_list_icon_fetcher(
    packages: Vec<model::Package>,
    tokio_handle: tokio::runtime::Handle,
    ui_weak: slint::Weak<MainWindow>,
    category: String,
    limit: Option<usize>,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    tokio_handle.spawn(async move {
        let client = reqwest::Client::new();
        let pkgs_to_process = if let Some(l) = limit {
            packages.iter().take(l).cloned().collect::<Vec<_>>()
        } else {
            packages.clone()
        };

        let mut temp_dir = std::env::temp_dir();
        temp_dir.push("offerings_icons");
        let _ = std::fs::create_dir_all(&temp_dir);

        for chunk in pkgs_to_process.chunks(8) {
            // Update in small batches to feel responsive
            let mut chunk_futures = Vec::new();
            for pkg in chunk {
                let mut icon_url = pkg.metadata.icon_url.clone();
                // Guess icon URL if missing
                if icon_url.is_none() {
                    let id = &pkg.identity.id;
                    let name = pkg.identity.name.to_lowercase();
                    if id.starts_with("soar:") || id.starts_with("appimage:") {
                        icon_url = Some(format!("https://appimage.github.io/icons/{}.png", name));
                    } else if id.starts_with("flatpak:") {
                        let flatpak_id = id.strip_prefix("flatpak:").unwrap_or(id);
                        icon_url = Some(format!(
                            "https://dl.flathub.org/repo/appstream/x86_64/icons/128x128/{}.png",
                            flatpak_id
                        ));
                    }
                }

                if let Some(url) = icon_url {
                    let safe_id = pkg.identity.id.replace([':', '/'], "_");
                    let path = temp_dir.join(format!("{}.png", safe_id));
                    if !path.exists() {
                        let client = client.clone();
                        let sem = semaphore.clone();
                        let path_clone = path.clone();
                        chunk_futures.push(tokio::spawn(async move {
                            let _permit = sem.acquire().await.ok();
                            if let Ok(resp) = client.get(&url).send().await {
                                if resp.status().is_success() {
                                    if let Ok(bytes) = resp.bytes().await {
                                        let _ = std::fs::write(&path_clone, &bytes);
                                    }
                                }
                            }
                        }));
                    }
                }
            }

            if !chunk_futures.is_empty() {
                futures::future::join_all(chunk_futures).await;
            }

            // Push partial updates to UI even if no icons were downloaded (to ensure consistency)
            let ui_weak_refresh = ui_weak.clone();
            let pkgs_refresh = packages.clone();
            let cat_refresh = category.clone();
            let limit_refresh = limit;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_refresh.upgrade() {
                    let slint_pkgs = if let Some(l) = limit_refresh {
                        limit_and_convert(pkgs_refresh, l)
                    } else {
                        convert_to_slint_packages(pkgs_refresh)
                    };
                    match cat_refresh.as_str() {
                        "search" => ui.set_search_results(slint_pkgs),
                        "installed" => ui.set_installed_packages(slint_pkgs),
                        "trending" => ui.set_featured_trending(slint_pkgs),
                        "lilith" => ui.set_featured_lilith(slint_pkgs),
                        "essentials" => ui.set_featured_essentials(slint_pkgs),
                        "recently_updated" => ui.set_recently_updated(slint_pkgs),
                        "audio" => ui.set_featured_audio(slint_pkgs),
                        "video" => ui.set_featured_video(slint_pkgs),
                        "development" => ui.set_featured_development(slint_pkgs),
                        "education" => ui.set_featured_education(slint_pkgs),
                        "game" => ui.set_featured_game(slint_pkgs),
                        "graphics" => ui.set_featured_graphics(slint_pkgs),
                        "network" => ui.set_featured_network(slint_pkgs),
                        "office" => ui.set_featured_office(slint_pkgs),
                        "science" => ui.set_featured_science(slint_pkgs),
                        "settings" => ui.set_featured_settings(slint_pkgs),
                        "system" => ui.set_featured_system(slint_pkgs),
                        "utilities" => ui.set_featured_utilities(slint_pkgs),
                        "ai" => ui.set_featured_ai(slint_pkgs),
                        "productivity" => ui.set_featured_productivity(slint_pkgs),
                        "desktop" => ui.set_featured_desktop(slint_pkgs),
                        "security" => ui.set_featured_security(slint_pkgs),
                        "lifestyle" => ui.set_featured_lifestyle(slint_pkgs),
                        "android" => ui.set_category_android(slint_pkgs),
                        "comic" => ui.set_category_comic(slint_pkgs),
                        "command-line" => ui.set_category_command_line(slint_pkgs),
                        "communication" => ui.set_category_communication(slint_pkgs),
                        "disk" => ui.set_category_disk(slint_pkgs),
                        "file-manager" => ui.set_category_file_manager(slint_pkgs),
                        "finance" => ui.set_category_finance(slint_pkgs),
                        "gnome" => ui.set_category_gnome(slint_pkgs),
                        "kde" => ui.set_category_kde(slint_pkgs),
                        "password" => ui.set_category_password(slint_pkgs),
                        "steam" => ui.set_category_steam(slint_pkgs),
                        "system-monitor" => ui.set_category_system_monitor(slint_pkgs),
                        "web-app" => ui.set_category_web_app(slint_pkgs),
                        "web-browser" => ui.set_category_web_browser(slint_pkgs),
                        "wine" => ui.set_category_wine(slint_pkgs),
                        "miscellaneous" => ui.set_category_miscellaneous(slint_pkgs),
                        "catalogue" => ui.set_current_category_packages(slint_pkgs),
                        _ => {}
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_defaults_to_ui() {
        assert!(matches!(
            parse_cli_args(Vec::<String>::new()).unwrap(),
            CliCommand::RunUi
        ));
    }

    #[test]
    fn parse_cli_supports_self_test() {
        assert!(matches!(
            parse_cli_args(vec!["--self-test"]).unwrap(),
            CliCommand::SelfTest
        ));
    }

    #[test]
    fn parse_cli_supports_catalog_export_path() {
        match parse_cli_args(vec!["--export-catalog", "/tmp/catalog.json"]).unwrap() {
            CliCommand::ExportCatalog(path) => {
                assert_eq!(path, PathBuf::from("/tmp/catalog.json"));
            }
            other => panic!("unexpected command: {:?}", std::mem::discriminant(&other)),
        }
    }
}
