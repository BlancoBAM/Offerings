// src/adapters/flatpak.rs - Flatpak Package Manager Adapter
use super::{
    command_exists, emit_progress, run_command, start_staged_progress, PackageAdapter,
    ProgressCallback,
};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::error::Error;

/// Flatpak package manager adapter
pub struct FlatpakAdapter {
    /// Default remote (usually flathub)
    remote: String,
}

#[derive(Debug, Deserialize)]
struct FlathubApp {
    name: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    screenshots: Option<Vec<FlathubScreenshot>>,
    categories: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct FlathubScreenshot {
    sizes: Option<Vec<FlathubScreenshotSize>>,
}

#[derive(Debug, Deserialize)]
struct FlathubScreenshotSize {
    src: String,
}

impl FlatpakAdapter {
    pub fn new() -> Self {
        Self {
            remote: "flathub".to_string(),
        }
    }
    async fn fetch_flathub_metadata(&self, app_id: &str) -> Option<Package> {
        let url = format!("https://flathub.org/api/v2/appstream/{}", app_id);

        let response = reqwest::get(&url).await.ok()?;
        if !response.status().is_success() {
            return None;
        }

        let flathub_app: FlathubApp = response.json().await.ok()?;

        let screenshots: Vec<String> = flathub_app
            .screenshots
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                s.sizes
                    .and_then(|sizes| sizes.first().map(|size| size.src.clone()))
            })
            .collect();

        let categories = flathub_app.categories.unwrap_or_default();

        // Map Flathub categories to our category names
        let mapped_categories = Self::map_categories(&categories);

        Some(Package {
            identity: PackageIdentity {
                id: format!("flatpak:{}", app_id),
                name: flathub_app
                    .name
                    .unwrap_or_else(|| app_id.split('.').next_back().unwrap_or(app_id).to_string()),
                source: PackageSource::Flatpak,
            },
            metadata: PackageMetadata {
                summary: flathub_app.summary.unwrap_or_default(),
                description: flathub_app.description.unwrap_or_default(),
                icon_url: flathub_app.icon.map(|icon| {
                    if icon.starts_with("http") {
                        icon
                    } else {
                        format!(
                            "https://dl.flathub.org/repo/appstream/x86_64/icons/128x128/{}",
                            icon
                        )
                    }
                }),
                screenshots,
                documentation_url: None,
                homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                categories: mapped_categories,
                rating: None,
            },
            version: PackageVersion {
                installed: None,
                latest: None,
            },
            is_installed: false,
            logical_app_id: None,
            alternatives: vec![],
            last_updated: 0,
            popularity: 0.0,
        })
    }

    /// Map Flathub categories to our category names with new general categories
    fn map_categories(flathub_categories: &[String]) -> Vec<String> {
        let mut result = Vec::new();

        for cat in flathub_categories {
            // Map to our standard category names
            match cat.as_str() {
                "AudioVideo" | "Audio" | "Video" | "Music" | "Player" => {
                    result.push("AudioVideo".to_string());
                    if cat == "Audio" || cat == "Music" {
                        result.push("Audio".to_string());
                    }
                    if cat == "Video" || cat == "Player" {
                        result.push("Video".to_string());
                    }
                }
                "Development" | "Programming" | "IDE" | "Debugger" | "AI" | "MachineLearning" => {
                    result.push("Development".to_string());
                    if cat == "AI" || cat == "MachineLearning" {
                        result.push("AI".to_string());
                    }
                }
                "Education" | "Learning" => {
                    result.push("Education".to_string());
                }
                "Game" | "Games" | "Amusement" | "Entertainment" => {
                    result.push("Game".to_string());
                }
                "Graphics" | "Design" | "Art" | "Photography" => {
                    result.push("Graphics".to_string());
                }
                "Network" | "Internet" | "Communication" | "Web" => {
                    result.push("Network".to_string());
                }
                "Office" | "Productivity" | "Calendar" | "Spreadsheet" => {
                    result.push("Office".to_string());
                    if cat == "Productivity" {
                        result.push("Productivity".to_string());
                    }
                }
                "Science" | "Research" | "Data" => {
                    result.push("Science".to_string());
                }
                "Settings" | "Preferences" | "Configuration" => {
                    result.push("Settings".to_string());
                }
                "System" | "Utility" | "Utilities" | "Tools" | "FileTools" => {
                    result.push("System".to_string());
                    result.push("Utilities".to_string());
                }
                _ => {}
            }
        }

        if result.is_empty() {
            result.push("Miscellaneous".to_string());
        }

        result
    }

    /// Parse flatpak list output
    fn parse_list_output(&self, output: &str, is_installed: bool) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let name = parts[0].trim();
                let app_id = parts[1].trim();
                let version = parts[2].trim();

                // Skip runtimes unless they have a useful name
                if app_id.contains(".Platform") || app_id.contains(".Sdk") {
                    continue;
                }

                // Try to extract a human-readable name from app_id
                let display_name = if name.is_empty() || name == app_id {
                    app_id.split('.').next_back().unwrap_or(app_id)
                } else {
                    name
                };

                // Assign category based on app_id patterns
                let categories = Self::infer_category_from_id(app_id);

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("flatpak:{}", app_id),
                        name: display_name.to_string(),
                        source: PackageSource::Flatpak,
                    },
                    metadata: PackageMetadata {
                        summary: format!("{} application", display_name),
                        description: String::new(),
                        icon_url: Some(format!(
                            "https://dl.flathub.org/repo/appstream/x86_64/icons/128x128/{}.png",
                            app_id
                        )),
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                        categories,
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: if is_installed {
                            Some(version.to_string())
                        } else {
                            None
                        },
                        latest: Some(version.to_string()),
                    },
                    is_installed,
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
                };

                packages.push(pkg);
            }
        }

        packages
    }

    /// Infer category from app ID patterns
    fn infer_category_from_id(app_id: &str) -> Vec<String> {
        let id_lower = app_id.to_lowercase();

        // Development
        if id_lower.contains("jetbrains")
            || id_lower.contains("eclipse")
            || id_lower.contains("code")
            || id_lower.contains("editor")
            || id_lower.contains("ide")
        {
            return vec!["Development".to_string()];
        }
        // Games
        if id_lower.contains("game")
            || id_lower.contains("steam")
            || id_lower.contains("lutris")
            || id_lower.contains("retroarch")
            || id_lower.contains("prism")
        {
            return vec!["Game".to_string()];
        }
        // Graphics
        if id_lower.contains("gimp")
            || id_lower.contains("inkscape")
            || id_lower.contains("blender")
            || id_lower.contains("krita")
            || id_lower.contains("photo")
            || id_lower.contains("design")
        {
            return vec!["Graphics".to_string()];
        }
        // Audio
        if id_lower.contains("audacity")
            || id_lower.contains("spotify")
            || id_lower.contains("music")
            || id_lower.contains("audio")
            || id_lower.contains("player")
            || id_lower.contains("rhythm")
        {
            return vec!["Audio".to_string(), "AudioVideo".to_string()];
        }
        // Video
        if id_lower.contains("vlc")
            || id_lower.contains("kdenlive")
            || id_lower.contains("obs")
            || id_lower.contains("video")
            || id_lower.contains("player")
            || id_lower.contains("stream")
        {
            return vec!["Video".to_string(), "AudioVideo".to_string()];
        }
        // Network
        if id_lower.contains("firefox")
            || id_lower.contains("chrome")
            || id_lower.contains("discord")
            || id_lower.contains("telegram")
            || id_lower.contains("browser")
            || id_lower.contains("chat")
        {
            return vec!["Network".to_string()];
        }
        // Office
        if id_lower.contains("libreoffice")
            || id_lower.contains("onlyoffice")
            || id_lower.contains("office")
            || id_lower.contains("pdf")
            || id_lower.contains("evince")
            || id_lower.contains("obsidian")
        {
            return vec!["Office".to_string()];
        }
        // Education/Science
        if id_lower.contains("stellarium")
            || id_lower.contains("gcompris")
            || id_lower.contains("scratch")
            || id_lower.contains("edu")
            || id_lower.contains("science")
            || id_lower.contains("octave")
            || id_lower.contains("kicad")
            || id_lower.contains("freecad")
        {
            return vec!["Education".to_string(), "Science".to_string()];
        }
        // System/Utilities
        if id_lower.contains("monitor")
            || id_lower.contains("bleachbit")
            || id_lower.contains("cleaner")
            || id_lower.contains("calculator")
            || id_lower.contains("extension")
            || id_lower.contains("system")
            || id_lower.contains("utility")
        {
            return vec!["System".to_string(), "Utilities".to_string()];
        }

        // Default
        vec!["Application".to_string()]
    }

    /// Parse flatpak info output for detailed package info
    fn parse_info_output(&self, output: &str) -> Option<Package> {
        let mut name = String::new();
        let mut app_id = String::new();
        let mut version = String::new();
        let mut description = String::new();

        for line in output.lines() {
            let line = line.trim();
            if line.starts_with("Name:") {
                name = line.trim_start_matches("Name:").trim().to_string();
            } else if line.starts_with("ID:") || line.starts_with("Application ID:") {
                app_id = line.split(':').nth(1).unwrap_or("").trim().to_string();
            } else if line.starts_with("Version:") {
                version = line.trim_start_matches("Version:").trim().to_string();
            } else if line.starts_with("Description:") {
                description = line.trim_start_matches("Description:").trim().to_string();
            }
        }

        if app_id.is_empty() {
            return None;
        }

        Some(Package {
            identity: PackageIdentity {
                id: format!("flatpak:{}", app_id),
                name: if name.is_empty() {
                    app_id.clone()
                } else {
                    name
                },
                source: PackageSource::Flatpak,
            },
            metadata: PackageMetadata {
                summary: description.clone(),
                description,
                icon_url: None,
                screenshots: vec![],
                documentation_url: None,
                homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                categories: vec!["Application".to_string()],
                rating: None,
            },
            version: PackageVersion {
                installed: Some(version.clone()),
                latest: Some(version),
            },
            is_installed: true,
            logical_app_id: None,
            alternatives: vec![],
            last_updated: 0,
            popularity: 0.0,
        })
    }

    /// Parse flatpak remote-ls output for available packages
    fn parse_remote_ls_output(&self, output: &str) -> Vec<Package> {
        let mut packages = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if !parts.is_empty() {
                let app_id = parts[0].trim();
                let name = if parts.len() >= 2 && !parts[1].trim().is_empty() {
                    parts[1].trim()
                } else {
                    // Fallback string manipulation if name is absent or broken
                    app_id.split('.').next_back().unwrap_or(app_id)
                };

                // Skip runtimes
                let categories = Self::infer_category_from_id(app_id);

                let pkg = Package {
                    identity: PackageIdentity {
                        id: format!("flatpak:{}", app_id),
                        name: name.to_string(),
                        source: PackageSource::Flatpak,
                    },
                    metadata: PackageMetadata {
                        summary: format!("{} application", name),
                        description: String::new(),
                        icon_url: Some(format!(
                            "https://dl.flathub.org/repo/appstream/x86_64/icons/128x128/{}.png",
                            app_id
                        )),
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: Some(format!("https://flathub.org/apps/{}", app_id)),
                        categories,
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: None,
                        latest: None,
                    },
                    is_installed: false,
                    logical_app_id: None,
                    alternatives: vec![],
                    last_updated: 0,
                    popularity: 0.0,
                };

                packages.push(pkg);
            }
        }

        packages
    }
}

fn parse_percent(line: &str) -> Option<f32> {
    let pct_pos = line.find('%')?;
    let prefix = &line[..pct_pos];
    let digits: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || c.is_whitespace())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    digits.trim().parse::<f32>().ok()
}

impl Default for FlatpakAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for FlatpakAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::Flatpak
    }

    async fn is_available(&self) -> bool {
        command_exists("flatpak").await
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command(
            "flatpak",
            &[
                "remote-ls",
                "--app",
                "--columns=application,name,version",
                &self.remote,
            ][..],
        )
        .await?;
        Ok(self.parse_remote_ls_output(&output))
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command(
            "flatpak",
            &["list", "--app", "--columns=name,application,version"][..],
        )
        .await?;
        Ok(self.parse_list_output(&output, true))
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let app_id = id.strip_prefix("flatpak:").unwrap_or(id);

        // First try to fetch from Flathub API for rich metadata
        if let Some(pkg) = self.fetch_flathub_metadata(app_id).await {
            return Ok(Some(pkg));
        }

        // Fallback to local flatpak info
        let mut output = run_command("flatpak", &["info", app_id][..]).await;

        if output.is_err() {
            // If it's not installed, try remote-info
            output = run_command("flatpak", &["remote-info", &self.remote, app_id][..]).await;
        }

        match output {
            Ok(output) => Ok(self.parse_info_output(&output)),
            Err(_) => Ok(None),
        }
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let output = run_command(
            "flatpak",
            &[
                "remote-ls",
                "--app",
                "--updates",
                "--columns=application,name,version",
                &self.remote,
            ][..],
        )
        .await?;

        let mut packages = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if !parts.is_empty() {
                let app_id = parts[0].trim();
                let name = if parts.len() >= 2 && !parts[1].trim().is_empty() {
                    parts[1].trim()
                } else {
                    app_id.split('.').next_back().unwrap_or(app_id)
                };
                if !app_id.is_empty() {
                    packages.push(Package {
                        identity: PackageIdentity {
                            id: format!("flatpak:{}", app_id),
                            name: name.to_string(),
                            source: PackageSource::Flatpak,
                        },
                        metadata: PackageMetadata::default(),
                        version: PackageVersion::default(),
                        is_installed: true,
                        logical_app_id: None,
                        alternatives: vec![],
                        last_updated: 0,
                        popularity: 0.0,
                    });
                }
            }
        }

        Ok(packages)
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.install_with_progress(package_id, None).await
    }

    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);

        let args = ["install", "--user", "-y", &self.remote, app_id];
        emit_progress(&callback, 0.02);

        if let Some(cb) = callback {
            let result = super::run_command_with_stream("flatpak", &args, move |line, _| {
                if let Some(pct) = parse_percent(line) {
                    cb(pct / 100.0);
                }
            })
            .await;

            match result {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Successfully installed {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: format!("Failed to install {}: {}", app_id, e),
                    updated_packages: vec![],
                }),
            }
        } else {
            match super::run_command("flatpak", &args).await {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Successfully installed {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: format!("Failed to install {}: {}", app_id, e),
                    updated_packages: vec![],
                }),
            }
        }
    }

    async fn update(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.update_with_progress(package_id, None).await
    }

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        let args = ["update", "--user", "-y", app_id];
        emit_progress(&callback, 0.02);

        if let Some(cb) = callback {
            let result = super::run_command_with_stream("flatpak", &args, move |line, _| {
                if let Some(pct) = parse_percent(line) {
                    cb(pct / 100.0);
                }
            })
            .await;

            match result {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Successfully updated {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: format!("Failed to update {}: {}", app_id, e),
                    updated_packages: vec![],
                }),
            }
        } else {
            match super::run_command("flatpak", &args).await {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Updated {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: e.to_string(),
                    updated_packages: vec![],
                }),
            }
        }
    }

    async fn uninstall(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.uninstall_with_progress(package_id, None).await
    }

    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        let args = ["uninstall", "--user", "-y", app_id];

        // Start staged progress immediately at 5%, going up to 90% in larger steps
        // Flatpak uninstall doesn't provide percentage output, so we rely entirely on staged progress
        let mut progress_task: Option<tokio::task::JoinHandle<()>> = start_staged_progress(
            callback.clone(),
            0.05,
            0.90,
            0.15,
            std::time::Duration::from_millis(300),
        );

        if let Some(cb) = callback {
            let cb_clone = cb.clone();
            let result = super::run_command_with_stream("flatpak", &args, move |line, _| {
                // Flatpak uninstall doesn't output percentages, but we check anyway for future compatibility
                if let Some(pct) = parse_percent(line) {
                    cb_clone(pct / 100.0);
                }
            })
            .await;

            // Ensure staged progress completes at 100% before returning
            if let Some(task) = progress_task.take() {
                task.abort();
                // Final progress update to 100%
                cb(1.0);
            }

            match result {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Successfully removed {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: format!("Failed to remove {}: {}", app_id, e),
                    updated_packages: vec![],
                }),
            }
        } else {
            match super::run_command("flatpak", &args).await {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Uninstalled {}", app_id),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: e.to_string(),
                    updated_packages: vec![],
                }),
            }
        }
    }

    async fn get_dependencies(
        &self,
        package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let app_id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);

        // Flatpak shows runtime dependencies in info
        let output = run_command("flatpak", &["info", "--show-runtime", app_id][..]).await?;

        let mut deps = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with("Runtime:") {
                deps.push(format!("flatpak:{}", line));
            }
        }

        Ok(deps)
    }

    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Flatpak automatically updates remote metadata
        Ok(())
    }

    async fn launch(&self, package_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let id = package_id.strip_prefix("flatpak:").unwrap_or(package_id);
        tokio::process::Command::new("flatpak")
            .arg("run")
            .arg(id)
            .spawn()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flatpak_adapter_creation() {
        let adapter = FlatpakAdapter::new();
        assert_eq!(adapter.source(), PackageSource::Flatpak);
    }
}
