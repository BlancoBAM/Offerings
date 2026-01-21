// src/main.rs - Offerings Application Entry Point
mod adapters;
mod backend;
mod db;
mod depgraph;
mod ipc;
mod model;
mod notifications;
mod transaction;

use backend::BackendService;
use ipc::{IpcCommand, IpcRequest, IpcResponse, IpcResponseData, IpcServer, PackageSummary, StatusData};
use model::{HomePageContent, PackageOperation};
use std::collections::HashMap;
use std::sync::Arc;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize backend with featured apps for each category
    let home_content = HomePageContent {
        featured_apps: vec![
            "apt:firefox".to_string(),
            "apt:vlc".to_string(),
            "apt:gimp".to_string(),
            "apt:blender".to_string(),
            "apt:audacity".to_string(),
        ],
        category_showcases: {
            let mut map = HashMap::new();
            map.insert("Lilith".to_string(), vec![]);  // Custom Lilith packages
            map.insert("Audio".to_string(), vec!["apt:audacity".to_string(), "apt:ardour".to_string(), "apt:lmms".to_string()]);
            map.insert("Video".to_string(), vec!["apt:vlc".to_string(), "apt:kdenlive".to_string(), "apt:obs-studio".to_string()]);
            map.insert("Development".to_string(), vec!["apt:git".to_string(), "apt:vim".to_string(), "apt:code".to_string()]);
            map.insert("Education".to_string(), vec!["apt:gcompris".to_string(), "apt:stellarium".to_string()]);
            map.insert("Game".to_string(), vec!["apt:supertuxkart".to_string(), "apt:0ad".to_string()]);
            map.insert("Graphics".to_string(), vec!["apt:gimp".to_string(), "apt:inkscape".to_string(), "apt:blender".to_string()]);
            map.insert("Network".to_string(), vec!["apt:firefox".to_string(), "apt:chromium".to_string(), "apt:thunderbird".to_string()]);
            map.insert("Office".to_string(), vec!["apt:libreoffice".to_string(), "apt:evince".to_string()]);
            map.insert("Science".to_string(), vec!["apt:octave".to_string(), "apt:gnuplot".to_string()]);
            map.insert("Settings".to_string(), vec!["apt:gnome-tweaks".to_string()]);
            map.insert("System".to_string(), vec!["apt:htop".to_string(), "apt:gnome-system-monitor".to_string()]);
            map.insert("Utilities".to_string(), vec!["apt:gnome-calculator".to_string(), "apt:file-roller".to_string()]);
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
    println!("Available package sources: {:?}", sources.iter().map(|s| s.label()).collect::<Vec<_>>());

    // Create Slint UI
    let ui = MainWindow::new()?;
    let backend_clone = backend.clone();

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
        let _ui_weak = ui.as_weak();
        let _backend = backend_clone.clone();
        move |_pkg_id| {
            // Package detail view - will be implemented with detail popup
        }
    });

    // View dependencies handler
    ui.on_view_dependencies({
        let ui_weak = ui.as_weak();
        let backend = backend_clone.clone();
        move |pkg_id| {
            let backend = backend.clone();
            let pkg_id = pkg_id.to_string();
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let deps = backend.get_dependency_tree(&pkg_id).await;
                let dep_items: Vec<DependencyItem> = deps.into_iter().map(|d| {
                    DependencyItem {
                        id: d.clone().into(),
                        name: d.split(':').last().unwrap_or(&d).to_string().into(),
                        version: "".into(),
                        reason: "dependency".into(),
                    }
                }).collect();
                
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_selected_deps(slint::ModelRc::new(slint::VecModel::from(dep_items)));
                    }
                });
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
}

/// Get packages for a category
async fn get_category_packages(backend: &BackendService, category: &str, home_content: &HomePageContent) -> Vec<model::Package> {
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
        IpcRequest::GetPackage { id } => {
            match backend.get_package(&id).await {
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
            }
        }
        IpcRequest::Install { id } => {
            match backend.execute_operation(PackageOperation::Install(id.clone())).await {
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
            match backend.execute_operation(PackageOperation::Uninstall(id)).await {
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
            match backend.execute_operation(PackageOperation::Update(id)).await {
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
        IpcRequest::Refresh => {
            match backend.refresh_cache().await {
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
            }
        }
        IpcRequest::Quit => IpcResponse {
            success: true,
            message: "Goodbye".to_string(),
            data: None,
        },
    }
}

/// Convert internal Package struct to Slint PackageInfo
fn convert_to_slint_packages(packages: Vec<model::Package>) -> slint::ModelRc<PackageInfo> {
    let slint_packages: Vec<PackageInfo> = packages
        .into_iter()
        .map(|pkg| {
            let has_update = pkg.version.has_update();
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
                install_date: 0,  // TODO: Get actual install date from database
            }
        })
        .collect();

    slint::ModelRc::new(slint::VecModel::from(slint_packages))
}
