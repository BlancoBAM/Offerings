// src/adapters/appimage.rs - AppImage Management Adapter
use super::{
    command_exists, emit_progress, run_command, start_staged_progress, PackageAdapter,
    ProgressCallback,
};
use crate::model::{
    OperationResult, Package, PackageIdentity, PackageMetadata, PackageSource, PackageVersion,
};
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;
use tokio::fs;

/// AppImage management adapter
/// Scans standard directories for .AppImage files
pub struct AppImageAdapter {
    /// Directories to scan for AppImages
    scan_dirs: Vec<PathBuf>,
    /// Directory to install AppImages to
    install_dir: PathBuf,
}

impl AppImageAdapter {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        Self {
            scan_dirs: vec![
                home.join("Applications"),
                home.join(".local/share/applications"),
                home.join("Downloads"),
                PathBuf::from("/opt"),
                PathBuf::from("/usr/local/bin"),
            ],
            install_dir: home.join("Applications"),
        }
    }
    /// Extract app info from an AppImage filename
    fn parse_appimage_name(&self, filename: &str) -> (String, Option<String>) {
        // Common patterns: AppName-version-arch.AppImage, AppName_version.AppImage
        let name = filename
            .trim_end_matches(".AppImage")
            .trim_end_matches(".appimage");

        // Try to extract version
        let parts: Vec<&str> = name.split(|c| c == '-' || c == '_').collect();

        if parts.len() >= 2 {
            // Check if second part looks like a version
            let potential_version = parts[1];
            if potential_version
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                return (parts[0].to_string(), Some(potential_version.to_string()));
            }
        }

        (name.to_string(), None)
    }

    /// Create desktop entry for an AppImage
    async fn create_desktop_entry(
        &self,
        path: &PathBuf,
        name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let desktop_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("applications");

        fs::create_dir_all(&desktop_dir).await?;

        let desktop_file =
            desktop_dir.join(format!("{}.desktop", name.to_lowercase().replace(' ', "-")));

        let content = format!(
            r#"[Desktop Entry]
Type=Application
Name={}
Exec="{}"
Icon=application-x-executable
Categories=Application;
Terminal=false
"#,
            name,
            path.display()
        );

        fs::write(&desktop_file, content).await?;

        Ok(())
    }

    /// Remove desktop entry for an AppImage
    async fn remove_desktop_entry(&self, name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let desktop_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("applications");

        let desktop_file =
            desktop_dir.join(format!("{}.desktop", name.to_lowercase().replace(' ', "-")));

        if desktop_file.exists() {
            fs::remove_file(&desktop_file).await?;
        }

        Ok(())
    }

    /// Infer category from package name - comprehensive patterns with new categories
    fn infer_category(name: &str) -> Vec<String> {
        let name_lower = name.to_lowercase();

        // AI / Machine Learning
        if name_lower.contains("ai")
            || name_lower.contains("llm")
            || name_lower.contains("neural")
            || name_lower.contains("machine")
            || name_lower.contains("learning")
            || name_lower.contains("tensorflow")
            || name_lower.contains("pytorch")
            || name_lower.contains("ollama")
            || name_lower.contains("stable-diffusion")
            || name_lower.contains("comfyui")
            || name_lower.contains("automatic1111")
            || name_lower.contains("chatbot")
            || name_lower.contains("inference")
        {
            return vec!["AI".to_string(), "Development".to_string()];
        }
        // Desktop Customization
        if name_lower.contains("theme")
            || name_lower.contains("icon")
            || name_lower.contains("cursor")
            || name_lower.contains("plank")
            || name_lower.contains("dock")
            || name_lower.contains("conky")
            || name_lower.contains("wallpaper")
            || name_lower.contains("customization")
            || name_lower.contains("tweak")
            || name_lower.contains("extension")
            || name_lower.contains("cosmic")
            || name_lower.contains("gnome")
            || name_lower.contains("kde")
            || name_lower.contains("desktop")
        {
            return vec!["Desktop Customization".to_string(), "System".to_string()];
        }
        // Productivity
        if name_lower.contains("productivity")
            || name_lower.contains("task")
            || name_lower.contains("todo")
            || name_lower.contains("note")
            || name_lower.contains("calendar")
            || name_lower.contains("schedule")
            || name_lower.contains("project")
            || name_lower.contains("kanban")
            || name_lower.contains("timetrack")
            || name_lower.contains("focus")
            || name_lower.contains("habit")
            || name_lower.contains("planner")
            || name_lower.contains("organizer")
        {
            return vec!["Productivity".to_string(), "Office".to_string()];
        }
        // Lifestyle
        if name_lower.contains("fitness")
            || name_lower.contains("health")
            || name_lower.contains("diet")
            || name_lower.contains("meditation")
            || name_lower.contains("yoga")
            || name_lower.contains("recipe")
            || name_lower.contains("cooking")
            || name_lower.contains("budget")
            || name_lower.contains("finance")
            || name_lower.contains("expense")
            || name_lower.contains("lifestyle")
            || name_lower.contains("wellness")
        {
            return vec!["Lifestyle".to_string()];
        }
        // Security / Privacy
        if name_lower.contains("security")
            || name_lower.contains("privacy")
            || name_lower.contains("password")
            || name_lower.contains("encrypt")
            || name_lower.contains("vpn")
            || name_lower.contains("firewall")
            || name_lower.contains("antivirus")
            || name_lower.contains("malware")
            || name_lower.contains("keepass")
            || name_lower.contains("gpg")
            || name_lower.contains("tor")
        {
            return vec!["Security".to_string(), "System".to_string()];
        }
        // Communication
        if name_lower.contains("email")
            || name_lower.contains("mail")
            || name_lower.contains("chat")
            || name_lower.contains("messenger")
            || name_lower.contains("voip")
            || name_lower.contains("conference")
            || name_lower.contains("signal")
            || name_lower.contains("whatsapp")
            || name_lower.contains("irc")
            || name_lower.contains("matrix")
            || name_lower.contains("xmpp")
        {
            return vec!["Communication".to_string(), "Network".to_string()];
        }
        // File Management
        if name_lower.contains("file")
            || name_lower.contains("manager")
            || name_lower.contains("explorer")
            || name_lower.contains("browser")
            || name_lower.contains("archive")
            || name_lower.contains("compress")
            || name_lower.contains("backup")
            || name_lower.contains("sync")
            || name_lower.contains("cloud")
            || name_lower.contains("dropbox")
            || name_lower.contains("nextcloud")
        {
            return vec!["File Management".to_string(), "Utilities".to_string()];
        }
        // Development - broader patterns
        if name_lower.contains("code")
            || name_lower.contains("dev")
            || name_lower.contains("ide")
            || name_lower.contains("editor")
            || name_lower.contains("vim")
            || name_lower.contains("emacs")
            || name_lower.contains("jetbrains")
            || name_lower.contains("eclipse")
            || name_lower.contains("git")
            || name_lower.contains("sdk")
            || name_lower.contains("api")
            || name_lower.contains("python")
            || name_lower.contains("node")
            || name_lower.contains("rust")
            || name_lower.contains("java")
            || name_lower.contains("cpp")
            || name_lower.contains("terminal")
            || name_lower.contains("shell")
            || name_lower.contains("docker")
            || name_lower.contains("kubernetes")
            || name_lower.contains("container")
        {
            return vec!["Development".to_string()];
        }
        // Games - broader patterns
        if name_lower.contains("game")
            || name_lower.contains("play")
            || name_lower.contains("steam")
            || name_lower.contains("lutris")
            || name_lower.contains("retroarch")
            || name_lower.contains("emulator")
            || name_lower.contains("mame")
            || name_lower.contains("dolphin")
            || name_lower.contains("cemu")
            || name_lower.contains("ryujinx")
            || name_lower.contains("prism")
            || name_lower.contains("heroic")
            || name_lower.contains("bottles")
            || name_lower.contains("rpg")
            || name_lower.contains("adventure")
            || name_lower.contains("strategy")
            || name_lower.contains("puzzle")
        {
            let mut cats = vec!["Game".to_string()];
            if name_lower.contains("steam") {
                cats.push("Steam".to_string());
            }
            return cats;
        }
        // AI / Machine Learning
        if name_lower.contains("ai")
            || name_lower.contains("llm")
            || name_lower.contains("neural")
            || name_lower.contains("machine")
            || name_lower.contains("learning")
            || name_lower.contains("tensorflow")
            || name_lower.contains("pytorch")
            || name_lower.contains("ollama")
            || name_lower.contains("stable-diffusion")
            || name_lower.contains("comfyui")
            || name_lower.contains("automatic1111")
            || name_lower.contains("chatbot")
            || name_lower.contains("inference")
        {
            return vec!["AI".to_string(), "Development".to_string()];
        }
        // Communication
        if name_lower.contains("email")
            || name_lower.contains("mail")
            || name_lower.contains("chat")
            || name_lower.contains("messenger")
            || name_lower.contains("voip")
            || name_lower.contains("conference")
            || name_lower.contains("signal")
            || name_lower.contains("whatsapp")
            || name_lower.contains("irc")
            || name_lower.contains("matrix")
            || name_lower.contains("xmpp")
        {
            return vec!["Communication".to_string(), "Network".to_string()];
        }
        // Desktop Environments
        if name_lower.contains("gnome") || name_lower.contains("gtk") {
            return vec!["Gnome".to_string(), "System".to_string()];
        }
        if name_lower.contains("kde") || name_lower.contains("qt") || name_lower.contains("plasma")
        {
            return vec!["KDE".to_string(), "System".to_string()];
        }
        // Comic / Books
        if name_lower.contains("comic")
            || name_lower.contains("book")
            || name_lower.contains("reader")
            || name_lower.contains("manga")
            || name_lower.contains("epub")
            || name_lower.contains("pdf")
        {
            return vec!["Comic".to_string(), "Office".to_string()];
        }
        // Disk / Files
        if name_lower.contains("disk")
            || name_lower.contains("partition")
            || name_lower.contains("format")
            || name_lower.contains("mount")
        {
            return vec!["Disk".to_string(), "System".to_string()];
        }
        if name_lower.contains("file")
            || name_lower.contains("manager")
            || name_lower.contains("explorer")
        {
            return vec!["File Management".to_string(), "Utilities".to_string()];
        }
        // Monitor
        if name_lower.contains("monitor")
            || name_lower.contains("top")
            || name_lower.contains("stat")
            || name_lower.contains("usage")
            || name_lower.contains("process")
        {
            return vec!["Monitor".to_string(), "System".to_string()];
        }
        // WebApp / Browser
        if name_lower.contains("browser")
            || name_lower.contains("web")
            || name_lower.contains("firefox")
            || name_lower.contains("chrome")
            || name_lower.contains("chromium")
            || name_lower.contains("opera")
            || name_lower.contains("vivaldi")
            || name_lower.contains("brave")
        {
            return vec!["Browser".to_string(), "Network".to_string()];
        }
        if name_lower.contains("app")
            && (name_lower.contains("web")
                || name_lower.contains("site")
                || name_lower.contains("electron"))
        {
            return vec!["WebApp".to_string(), "Network".to_string()];
        }
        // Wine
        if name_lower.contains("wine")
            || name_lower.contains("proton")
            || name_lower.contains("dxvk")
            || name_lower.contains("winbox")
            || name_lower.contains("windows")
        {
            return vec!["Wine".to_string(), "System".to_string()];
        }

        // Default - Miscellaneous for uncategorized
        vec!["Miscellaneous".to_string()]
    }
}

impl Default for AppImageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageAdapter for AppImageAdapter {
    fn source(&self) -> PackageSource {
        PackageSource::AppImage
    }

    async fn is_available(&self) -> bool {
        // AppImages are always "available" on Linux
        true
    }

    async fn launch(
        &self,
        package_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let id = package_id.strip_prefix("appimage:").unwrap_or(package_id);
        // Try to find the binary name
        let bin_name = id.split('/').last().unwrap_or(id);
        tokio::process::Command::new(bin_name).spawn()?;
        Ok(())
    }

    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        if command_exists("am").await {
            // Use "am -l --all" to list all available packages from all databases
            if let Ok(output) = run_command("am", &["-l", "--all"]).await {
                for line in output.lines() {
                    let line = line.trim();

                    // Parse package lines: " ◆ packagename : description"
                    if line.contains("◆") {
                        // Remove the ◆ and split by colon
                        let line_without_bullet = line.split("◆").nth(1).unwrap_or("").trim();
                        let parts: Vec<&str> = line_without_bullet.splitn(2, ':').collect();

                        if !parts.is_empty() {
                            let name_part = parts[0].trim();
                            let description =
                                parts.get(1).map(|s| s.trim()).unwrap_or("AppImage package");

                            if !name_part.is_empty()
                                && !name_part.starts_with("To ")
                                && !name_part.starts_with("Description")
                            {
                                let pkg = Package {
                                    identity: PackageIdentity {
                                        id: format!("appimage:{}", name_part),
                                        name: name_part.to_string(),
                                        source: PackageSource::AppImage,
                                    },
                                    metadata: PackageMetadata {
                                        summary: description.to_string(),
                                        description: description.to_string(),
                                        icon_url: None,
                                        screenshots: vec![],
                                        documentation_url: None,
                                        homepage_url: None,
                                        categories: Self::infer_category(name_part),
                                        rating: None,
                                    },
                                    version: PackageVersion {
                                        installed: None,
                                        latest: Some("latest".to_string()),
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
                    }
                }
            }
        }

        Ok(packages)
    }

    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        let mut packages = Vec::new();

        for dir in &self.scan_dirs {
            if !dir.exists() {
                continue;
            }

            let mut entries = match fs::read_dir(dir).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if filename.to_lowercase().ends_with(".appimage") {
                    let (name, version) = self.parse_appimage_name(filename);

                    // Check if executable
                    let metadata = fs::metadata(&path).await;
                    let is_executable = metadata
                        .map(|m| {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                m.permissions().mode() & 0o111 != 0
                            }
                            #[cfg(not(unix))]
                            {
                                true
                            }
                        })
                        .unwrap_or(false);

                    if !is_executable {
                        continue;
                    }

                    let pkg = Package {
                        identity: PackageIdentity {
                            id: format!("appimage:{}", path.display()),
                            name: name.clone(),
                            source: PackageSource::AppImage,
                        },
                        metadata: PackageMetadata {
                            summary: format!("AppImage: {}", filename),
                            description: format!("Local AppImage at {}", path.display()),
                            icon_url: None,
                            screenshots: vec![],
                            documentation_url: None,
                            homepage_url: None,
                            categories: vec!["Application".to_string()],
                            rating: None,
                        },
                        version: PackageVersion {
                            installed: version.clone(),
                            latest: version,
                        },
                        is_installed: true,
                        logical_app_id: None,
                        alternatives: vec![],
                        last_updated: 0,
                        popularity: 0.0,
                    };

                    packages.push(pkg);
                }
            }
        }

        Ok(packages)
    }

    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>> {
        let path_str = id.strip_prefix("appimage:").unwrap_or(id);

        // 1. Try local file path
        if path_str.contains('/') || path_str.to_lowercase().ends_with(".appimage") {
            let path = PathBuf::from(path_str);
            if path.exists() {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                let (name, version) = self.parse_appimage_name(filename);
                return Ok(Some(Package {
                    identity: PackageIdentity {
                        id: id.to_string(),
                        name,
                        source: PackageSource::AppImage,
                    },
                    metadata: PackageMetadata {
                        summary: format!("AppImage: {}", filename),
                        description: format!("Local AppImage at {}", path.display()),
                        icon_url: None,
                        screenshots: vec![],
                        documentation_url: None,
                        homepage_url: None,
                        categories: vec!["Application".to_string()],
                        rating: None,
                    },
                    version: PackageVersion {
                        installed: version.clone(),
                        latest: version,
                    },
                    is_installed: true,
                    alternatives: vec![],
                    logical_app_id: None,
                    last_updated: 0,
                    popularity: 0.0,
                }));
            }
        }

        // 2. Try 'am' package manager for available online apps
        if command_exists("am").await {
            // we can list or search 'am' but 'am -q' or just returning a stub works
            // since list_available populated it, or we can just run am -f and grep for it
            return Ok(Some(Package {
                identity: PackageIdentity {
                    id: format!("appimage:{}", path_str),
                    name: path_str.to_string(),
                    source: PackageSource::AppImage,
                },
                metadata: PackageMetadata {
                    summary: format!("AppImage AM Package: {}", path_str),
                    description: format!("Provided by AM package manager"),
                    icon_url: None,
                    screenshots: vec![],
                    documentation_url: None,
                    homepage_url: None,
                    categories: vec!["Application".to_string()],
                    rating: None,
                },
                version: PackageVersion {
                    installed: None,
                    latest: Some("latest".to_string()),
                },
                is_installed: false,
                alternatives: vec![],
                logical_app_id: None,
                last_updated: 0,
                popularity: 0.0,
            }));
        }

        Ok(None)
    }

    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>> {
        // AppImage update checking would require zsync or similar
        // For now, return empty list
        Ok(vec![])
    }

    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let id_clean = package_id.strip_prefix("appimage:").unwrap_or(package_id);

        // If it looks like a local file path
        if id_clean.contains('/') || id_clean.to_lowercase().ends_with(".appimage") {
            let source_path = PathBuf::from(id_clean);

            if !source_path.exists() {
                return Ok(OperationResult {
                    success: false,
                    message: "AppImage file not found".to_string(),
                    updated_packages: vec![],
                });
            }

            // Ensure install directory exists
            fs::create_dir_all(&self.install_dir).await?;

            let filename = source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("app.AppImage");

            let dest_path = self.install_dir.join(filename);

            // Copy if not already in install dir
            if source_path != dest_path {
                fs::copy(&source_path, &dest_path).await?;
            }

            // Make executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&dest_path).await?.permissions();
                perms.set_mode(perms.mode() | 0o755);
                fs::set_permissions(&dest_path, perms).await?;
            }

            // Create desktop entry
            let (name, _) = self.parse_appimage_name(filename);
            self.create_desktop_entry(&dest_path, &name).await?;

            return Ok(OperationResult {
                success: true,
                message: format!("Installed {} to {}", filename, self.install_dir.display()),
                updated_packages: vec![format!("appimage:{}", dest_path.display())],
            });
        }

        // Use 'am' package manager
        if command_exists("am").await {
            match run_command("am", &["-i", id_clean]).await {
                Ok(_) => Ok(OperationResult {
                    success: true,
                    message: format!("Successfully installed AM package: {}", id_clean),
                    updated_packages: vec![package_id.to_string()],
                }),
                Err(e) => Ok(OperationResult {
                    success: false,
                    message: format!("AM installation failed: {}", e),
                    updated_packages: vec![],
                }),
            }
        } else {
            Ok(OperationResult {
                success: false,
                message: "AM package manager not found. Please provide a local path or install AM."
                    .to_string(),
                updated_packages: vec![],
            })
        }
    }

    async fn install_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.05,
            0.9,
            0.08,
            std::time::Duration::from_millis(700),
        );
        let result = self.install(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn update(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let id_clean = package_id.strip_prefix("appimage:").unwrap_or(package_id);

        if id_clean.contains('/') || id_clean.to_lowercase().ends_with(".appimage") {
            Ok(OperationResult {
                success: false,
                message: "Local AppImages must be updated manually via downloading the new file"
                    .to_string(),
                updated_packages: vec![],
            })
        } else {
            if command_exists("am").await {
                match run_command("am", &["-u", id_clean]).await {
                    Ok(_) => Ok(OperationResult {
                        success: true,
                        message: format!("Successfully updated AM package: {}", id_clean),
                        updated_packages: vec![package_id.to_string()],
                    }),
                    Err(e) => Ok(OperationResult {
                        success: false,
                        message: format!("AM update failed: {}", e),
                        updated_packages: vec![],
                    }),
                }
            } else {
                Ok(OperationResult {
                    success: false,
                    message: "AM package manager not found.".to_string(),
                    updated_packages: vec![],
                })
            }
        }
    }

    async fn update_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.08,
            0.92,
            0.08,
            std::time::Duration::from_millis(700),
        );
        let result = self.update(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn uninstall(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let id_clean = package_id.strip_prefix("appimage:").unwrap_or(package_id);

        if id_clean.contains('/') || id_clean.to_lowercase().ends_with(".appimage") {
            let path = PathBuf::from(id_clean);

            if !path.exists() {
                return Ok(OperationResult {
                    success: false,
                    message: "AppImage not found".to_string(),
                    updated_packages: vec![],
                });
            }

            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            let (name, _) = self.parse_appimage_name(filename);

            self.remove_desktop_entry(&name).await?;
            fs::remove_file(&path).await?;

            Ok(OperationResult {
                success: true,
                message: format!("Removed {}", filename),
                updated_packages: vec![package_id.to_string()],
            })
        } else {
            if command_exists("am").await {
                match run_command("am", &["-R", id_clean]).await {
                    Ok(_) => Ok(OperationResult {
                        success: true,
                        message: format!("Successfully removed AM package: {}", id_clean),
                        updated_packages: vec![package_id.to_string()],
                    }),
                    Err(e) => Ok(OperationResult {
                        success: false,
                        message: format!("AM uninstallation failed: {}", e),
                        updated_packages: vec![],
                    }),
                }
            } else {
                Ok(OperationResult {
                    success: false,
                    message: "AM package manager not found.".to_string(),
                    updated_packages: vec![],
                })
            }
        }
    }

    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        let progress_task = start_staged_progress(
            callback.clone(),
            0.1,
            0.9,
            0.08,
            std::time::Duration::from_millis(650),
        );
        let result = self.uninstall(package_id).await;
        if let Some(task) = progress_task {
            task.abort();
        }
        if result.as_ref().map(|r| r.success).unwrap_or(false) {
            emit_progress(&callback, 1.0);
        }
        result
    }

    async fn get_dependencies(
        &self,
        _package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // AppImages are self-contained
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_appimage_name() {
        let adapter = AppImageAdapter::new();

        let (name, version) = adapter.parse_appimage_name("Firefox-102.0.AppImage");
        assert_eq!(name, "Firefox");
        assert_eq!(version, Some("102.0".to_string()));

        let (name, version) = adapter.parse_appimage_name("MyApp.AppImage");
        assert_eq!(name, "MyApp");
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn test_appimage_adapter_creation() {
        let adapter = AppImageAdapter::new();
        assert_eq!(adapter.source(), PackageSource::AppImage);
    }
}
