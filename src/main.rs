// src/main.rs - Offerings Application Entry Point
mod adapters;
mod backend;
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
use slint::Model;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

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
        "AppImage" => "https://appimage.github.io",
        "Pacstall" => "https://pacstall.dev",
        "Offerings" => "file:///etc/offerings/custom",
        _ => "unknown://",
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize backend with featured apps for each category
    let home_content = HomePageContent {
        featured_apps: vec![
            "flatpak:org.mozilla.firefox".to_string(),
            "flatpak:org.videolan.VLC".to_string(),
            "flatpak:org.gimp.GIMP".to_string(),
            "flatpak:org.blender.Blender".to_string(),
            "flatpak:org.audacityteam.Audacity".to_string(),
        ],
        category_showcases: {
            let mut map = HashMap::new();
            map.insert(
                "Lilith".to_string(),
                vec![
                    "flatpak:org.mozilla.firefox".to_string(),
                    "snap:code".to_string(),
                    "appimage:Discord".to_string(),
                ],
            ); // Developer curated
            map.insert(
                "Audio".to_string(),
                vec![
                    "flatpak:org.audacityteam.Audacity".to_string(),
                    "flatpak:org.ardour.Ardour".to_string(),
                    "flatpak:org.lmms.LMMS".to_string(),
                ],
            );
            map.insert(
                "Video".to_string(),
                vec![
                    "flatpak:org.videolan.VLC".to_string(),
                    "flatpak:org.kde.kdenlive".to_string(),
                    "flatpak:com.obsproject.Studio".to_string(),
                ],
            );
            map.insert(
                "Development".to_string(),
                vec![
                    "snap:code".to_string(),
                    "flatpak:org.vim.Vim".to_string(),
                    "pacstall:neovim".to_string(),
                ],
            );
            map.insert(
                "Education".to_string(),
                vec![
                    "flatpak:org.kde.gcompris".to_string(),
                    "flatpak:org.stellarium.Stellarium".to_string(),
                ],
            );
            map.insert(
                "Game".to_string(),
                vec![
                    "flatpak:org.supertuxkart.SuperTuxKart".to_string(),
                    "flatpak:com.play0ad.zeroad".to_string(),
                ],
            );
            map.insert(
                "Graphics".to_string(),
                vec![
                    "flatpak:org.gimp.GIMP".to_string(),
                    "flatpak:org.inkscape.Inkscape".to_string(),
                    "flatpak:org.blender.Blender".to_string(),
                ],
            );
            map.insert(
                "Network".to_string(),
                vec![
                    "flatpak:org.mozilla.firefox".to_string(),
                    "flatpak:org.chromium.Chromium".to_string(),
                ],
            );
            map.insert(
                "Office".to_string(),
                vec![
                    "flatpak:org.libreoffice.LibreOffice".to_string(),
                    "flatpak:org.gnome.Evince".to_string(),
                ],
            );
            map.insert(
                "Science".to_string(),
                vec![
                    "flatpak:org.octave.Octave".to_string(),
                ],
            );
            map.insert("Settings".to_string(), vec![]);
            map.insert(
                "System".to_string(),
                vec![
                    "flatpak:org.gnome.SystemMonitor".to_string(),
                ],
            );
            map.insert(
                "Utilities".to_string(),
                vec![
                    "flatpak:org.gnome.Calculator".to_string(),
                ],
            );
            map
        },
    };

    let backend = match BackendService::new(home_content) {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("Failed to initialize backend: {}", e);
            return Err(e.into());
        }
    };

    // Start background refresh task (checks every 30 minutes for new apps/updates)
    let _background_refresh = backend.start_background_refresh(1800);

    // Do initial cache refresh to load all packages
    if let Err(e) = backend.refresh_cache().await {
        eprintln!("Warning: Initial cache refresh failed: {}", e);
    }

    // Start IPC server
    let mut ipc_server = IpcServer::new();
    let ipc_receiver = match ipc_server.start() {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("Warning: Failed to start IPC server: {}", e);
            None
        }
    };

    // Handle IPC commands in background
    if let Some(mut receiver) = ipc_receiver {
        let backend_ipc = backend.clone();
        tokio::spawn(async move {
            while let Some(cmd) = receiver.recv().await {
                match cmd {
                    IpcCommand::Request(request, response_sender) => {
                        let response = handle_ipc_request(&backend_ipc, request).await;
                        let _ = response_sender.send(response).await;
                    }
                    IpcCommand::Shutdown => {
                        break;
                    }
                }
            }
        });
    }

    // Initial cache refresh
    println!("Refreshing package cache...");
    if let Err(e) = backend.refresh_cache().await {
        eprintln!("Warning: Initial cache refresh failed: {}", e);
    }

    // Check available sources
    let sources = backend.get_available_sources().await;
    println!(
        "Available package sources: {:?}",
        sources.iter().map(|s| s.label()).collect::<Vec<_>>()
    );

    // Create Slint UI
    let ui = MainWindow::new()?;
    let backend_clone = backend.clone();

    // Populate system fonts
    let font_list: Vec<slint::SharedString> = get_system_fonts()
        .into_iter()
        .map(slint::SharedString::from)
        .collect();
    ui.set_available_fonts(slint::ModelRc::new(slint::VecModel::from(font_list)));

    // Populate initial data
    populate_ui(&ui, &backend).await;

    // Search handler
    ui.on_search_triggered({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move |query| {
            let ui_weak = ui_weak.clone();
            let backend = backend.clone();
            let query = query.to_string();

            tokio::spawn(async move {
                let results = backend.search_apps(&query).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_search_results(convert_to_slint_packages(results));
                    }
                });
            });
        }
    });

    // Install handler
    ui.on_install_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let op = PackageOperation::Install(pkg_id.clone());
                match backend.execute_operation(op).await {
                    Ok(result) => {
                        if result.success {
                            let installed = backend.get_installed_packages().await;
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    ui.set_installed_packages(convert_to_slint_packages(installed));
                                }
                            });
                        }
                    }
                    Err(e) => eprintln!("Install failed: {}", e),
                }
            });
        }
    });

    // Update handler
    ui.on_update_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let op = PackageOperation::Update(pkg_id);
                match backend.execute_operation(op).await {
                    Ok(_) => {
                        let installed = backend.get_installed_packages().await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installed_packages(convert_to_slint_packages(installed));
                            }
                        });
                    }
                    Err(e) => eprintln!("Update failed: {}", e),
                }
            });
        }
    });

    // Uninstall handler
    ui.on_uninstall_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let op = PackageOperation::Uninstall(pkg_id);
                match backend.execute_operation(op).await {
                    Ok(_) => {
                        let installed = backend.get_installed_packages().await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_installed_packages(convert_to_slint_packages(installed));
                            }
                        });
                    }
                    Err(e) => eprintln!("Uninstall failed: {}", e),
                }
            });
        }
    });

    // Update All handler
    ui.on_update_all_clicked({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move || {
            let backend = backend.clone();
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let op = PackageOperation::UpdateAll;
                match backend.execute_operation(op).await {
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
        move |pkg_id| {
            let backend = backend.clone();
            let ui_weak = ui_weak.clone();
            let pkg_id = pkg_id.to_string();

            tokio::spawn(async move {
                if let Some(pkg) = backend.get_package(&pkg_id).await {
                    let _ = slint::invoke_from_event_loop({
                        let ui_weak = ui_weak.clone();
                        move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                // Since convert_to_slint_package is async, we can't call it here easily.
                                // But wait, it doesn't HAVE to be async if we don't fetch extra data.
                                // Actually, it WAS async because I added a loop over alternatives.
                                // Let's simplify it.
                                
                                let has_update = pkg.version.has_update();
                                let alternatives: Vec<AlternativeSource> = pkg.alternatives
                                    .iter()
                                    .map(|alt| AlternativeSource {
                                        id: alt.id.clone().into(),
                                        source: alt.source.label().into(),
                                    })
                                    .collect();

                                let slint_pkg = PackageInfo {
                                    id: pkg.identity.id.into(),
                                    name: pkg.identity.name.into(),
                                    summary: pkg.metadata.summary.into(),
                                    source: pkg.identity.source.label().into(),
                                    installed_version: pkg.version.installed.unwrap_or_default().into(),
                                    latest_version: pkg.version.latest.unwrap_or_default().into(),
                                    has_update,
                                    is_installed: pkg.is_installed,
                                    icon_url: pkg.metadata.icon_url.unwrap_or_default().into(),
                                    rating: pkg.metadata.rating.unwrap_or(0.0),
                                    description: pkg.metadata.description.clone().into(),
                                    install_date: 0,
                                    alternatives: slint::ModelRc::new(slint::VecModel::from(alternatives)),
                                };
                                
                                ui.set_selected_package(slint_pkg);
                                ui.set_show_package_detail(true);
                            }
                        }
                    });
                }
            });
        }
    });


    // Category selection handler
    ui.on_category_selected({
        let ui_weak = ui.as_weak();
        move |category| {
            let _ = slint::invoke_from_event_loop({
                let ui_weak = ui_weak.clone();
                let category = category.to_string();
                move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_current_view(category.into());
                    }
                }
            });
        }
    });

    // Remove source handler — removes entry from the UI model for this session
    ui.on_remove_source_clicked({
        let ui_weak = ui.as_weak();
        move |url_to_remove| {
            let url_to_remove = url_to_remove.to_string();
            let ui_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let current: Vec<SourceItem> = ui.get_source_urls().iter().collect();
                    let filtered: Vec<SourceItem> = current
                        .into_iter()
                        .filter(|s| s.url.as_str() != url_to_remove)
                        .collect();
                    ui.set_source_urls(slint::ModelRc::new(slint::VecModel::from(filtered)));
                }
            });
        }
    });

    println!("Offerings is running!");
    ui.run()?;

    Ok(())
}

/// Populate UI with initial data
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

    // Populate source URLs for Settings -> Sources tab
    let available_sources = backend.get_available_sources().await;
    let source_items: Vec<SourceItem> = available_sources
        .iter()
        .map(|src| {
            let label = src.label();
            SourceItem {
                name: label.into(),
                url: source_url(label).into(),
            }
        })
        .collect();
    ui.set_source_urls(slint::ModelRc::new(slint::VecModel::from(source_items)));
}

/// Get packages for a category
async fn get_category_packages(
    backend: &BackendService,
    category: &str,
    home_content: &HomePageContent,
) -> Vec<model::Package> {
    let mut packages = Vec::new();

    if let Some(pkg_ids) = home_content.category_showcases.get(category) {
        for id in pkg_ids {
            if let Some(pkg) = backend.get_package(id).await {
                packages.push(pkg);
            }
        }
    }

    // Also get packages from cache that match this category
    let category_pkgs = backend.get_apps_by_category(category).await;
    for pkg in category_pkgs.into_iter().take(10) {
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
                .execute_operation(PackageOperation::Install(id.clone()))
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
                .execute_operation(PackageOperation::Uninstall(id))
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
                .execute_operation(PackageOperation::Update(id))
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
            match backend.execute_operation(PackageOperation::UpdateAll).await {
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

/// Convert internal Package struct to Slint PackageInfo (Bulk)
fn convert_to_slint_packages(packages: Vec<model::Package>) -> slint::ModelRc<PackageInfo> {
    let slint_packages: Vec<PackageInfo> = packages
        .into_iter()
        .map(|pkg| {
            let has_update = pkg.version.has_update();
            
            // For simple lists, we don't need full alternatives detail yet
            let alternatives: Vec<AlternativeSource> = pkg.alternatives
                .iter()
                .map(|alt| AlternativeSource {
                    id: alt.id.clone().into(),
                    source: alt.source.label().into(),
                })
                .collect();

            PackageInfo {
                id: pkg.identity.id.into(),
                name: pkg.identity.name.into(),
                summary: pkg.metadata.summary.into(),
                source: pkg.identity.source.label().into(),
                installed_version: pkg.version.installed.unwrap_or_default().into(),
                latest_version: pkg.version.latest.unwrap_or_default().into(),
                has_update,
                is_installed: pkg.is_installed,
                icon_url: pkg.metadata.icon_url.unwrap_or_default().into(),
                rating: pkg.metadata.rating.unwrap_or(0.0),
                description: pkg.metadata.description.clone().into(),
                install_date: 0,
                alternatives: slint::ModelRc::new(slint::VecModel::from(alternatives)),
            }
        })
        .collect();

    slint::ModelRc::new(slint::VecModel::from(slint_packages))
}
