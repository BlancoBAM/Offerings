// src/local_pkg.rs - Local package file handler for Offerings
//
// When a user double-clicks a .deb, .AppImage, .flatpak, or .snap file,
// the desktop environment passes it to `offerings <path>`. This module
// parses the metadata from the local file and produces a display-ready struct.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The type of local package file
#[derive(Debug, Clone, PartialEq)]
pub enum LocalPkgType {
    Deb,
    AppImage,
    Flatpak,
    Snap,
    Unknown,
}

impl LocalPkgType {
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "deb" => Self::Deb,
            "appimage" => Self::AppImage,
            "flatpak" => Self::Flatpak,
            "snap" => Self::Snap,
            _ => {
                // Try by filename pattern for extensionless AppImages
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if name.ends_with(".appimage") {
                    Self::AppImage
                } else {
                    Self::Unknown
                }
            }
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Deb => "Debian Package (.deb)",
            Self::AppImage => "AppImage",
            Self::Flatpak => "Flatpak Bundle (.flatpak)",
            Self::Snap => "Snap Package (.snap)",
            Self::Unknown => "Local Package",
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self {
            Self::Deb => "Local .deb",
            Self::AppImage => "Local AppImage",
            Self::Flatpak => "Local Flatpak",
            Self::Snap => "Local Snap",
            Self::Unknown => "Local File",
        }
    }

    pub fn install_command(&self, path: &Path) -> (String, Vec<String>) {
        let path_str = path.to_string_lossy().to_string();
        match self {
            Self::Deb => (
                "pkexec".to_string(),
                vec!["dpkg".to_string(), "-i".to_string(), path_str],
            ),
            Self::AppImage => {
                // Make executable and move to ~/.local/bin
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("app")
                    .to_string();
                let dest = format!(
                    "{}/.local/bin/{}",
                    std::env::var("HOME").unwrap_or_default(),
                    name
                );
                (
                    "sh".to_string(),
                    vec![
                        "-c".to_string(),
                        format!(
                            "cp '{}' '{}' && chmod +x '{}'",
                            path_str, dest, dest
                        ),
                    ],
                )
            }
            Self::Flatpak => (
                "flatpak".to_string(),
                vec![
                    "install".to_string(),
                    "--bundle".to_string(),
                    "-y".to_string(),
                    path_str,
                ],
            ),
            Self::Snap => (
                "pkexec".to_string(),
                vec!["snap".to_string(), "install".to_string(), path_str],
            ),
            Self::Unknown => (
                "xdg-open".to_string(),
                vec![path_str],
            ),
        }
    }
}

/// A locally-opened package file with parsed metadata
#[derive(Debug, Clone)]
pub struct LocalPackage {
    /// The path to the package file on disk
    pub path: PathBuf,
    /// The type of package
    pub pkg_type: LocalPkgType,
    /// Package name (parsed from metadata)
    pub name: String,
    /// Package version string
    pub version: String,
    /// Human-readable description
    pub description: String,
    /// Summary / one-line description
    pub summary: String,
    /// File size in bytes
    pub size_bytes: u64,
    /// Whether this package is currently installed on the system
    pub is_installed: bool,
    /// Maintainer / author
    pub maintainer: String,
    /// Homepage URL if available
    pub homepage: String,
}

impl LocalPackage {
    /// Parse a local package file and return metadata.
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let pkg_type = LocalPkgType::from_path(path);

        if !path.exists() {
            return Err(format!("File not found: {}", path.display()));
        }

        let size_bytes = std::fs::metadata(path)
            .map(|m| m.len())
            .unwrap_or(0);

        match &pkg_type {
            LocalPkgType::Deb => parse_deb(path, size_bytes),
            LocalPkgType::AppImage => parse_appimage(path, size_bytes),
            LocalPkgType::Flatpak => parse_flatpak(path, size_bytes),
            LocalPkgType::Snap => parse_snap(path, size_bytes),
            LocalPkgType::Unknown => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                Ok(Self {
                    path: path.to_path_buf(),
                    pkg_type,
                    name,
                    version: String::new(),
                    description: String::new(),
                    summary: String::new(),
                    size_bytes,
                    is_installed: false,
                    maintainer: String::new(),
                    homepage: String::new(),
                })
            }
        }
    }

    /// Check if the package is installed using the appropriate system tool
    pub fn check_installed(&mut self) {
        self.is_installed = match &self.pkg_type {
            LocalPkgType::Deb => {
                Command::new("dpkg")
                    .args(["-s", &self.name])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            }
            LocalPkgType::Flatpak => {
                Command::new("flatpak")
                    .args(["info", &self.name])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            }
            LocalPkgType::Snap => {
                Command::new("snap")
                    .args(["info", &self.name])
                    .output()
                    .map(|o| {
                        if o.status.success() {
                            let out = String::from_utf8_lossy(&o.stdout);
                            out.contains("installed:")
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            }
            LocalPkgType::AppImage => {
                // Check if an executable with the same name exists in ~/.local/bin
                let home = std::env::var("HOME").unwrap_or_default();
                let dest = format!(
                    "{}/.local/bin/{}",
                    home,
                    self.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                );
                std::path::Path::new(&dest).exists()
            }
            LocalPkgType::Unknown => false,
        };
    }
}

// ── Per-format parsers ───────────────────────────────────────────────────────

fn parse_deb(path: &Path, size_bytes: u64) -> Result<LocalPackage, String> {
    // dpkg-deb -f <file> outputs control fields
    let output = Command::new("dpkg-deb")
        .args(["-f", &path.to_string_lossy()])
        .output()
        .map_err(|e| format!("dpkg-deb not found: {}. Install with: sudo apt install dpkg", e))?;

    let text = String::from_utf8_lossy(&output.stdout);

    let name = extract_field(&text, "Package").unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });
    let version = extract_field(&text, "Version").unwrap_or_default();
    let description = extract_deb_description(&text);
    let summary = extract_field(&text, "Description")
        .unwrap_or_else(|| description.lines().next().unwrap_or("").to_string());
    let maintainer = extract_field(&text, "Maintainer").unwrap_or_default();
    let homepage = extract_field(&text, "Homepage").unwrap_or_default();

    let is_installed = Command::new("dpkg")
        .args(["-s", &name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    Ok(LocalPackage {
        path: path.to_path_buf(),
        pkg_type: LocalPkgType::Deb,
        name,
        version,
        description,
        summary,
        size_bytes,
        is_installed,
        maintainer,
        homepage,
    })
}

fn parse_appimage(path: &Path, size_bytes: u64) -> Result<LocalPackage, String> {
    // Try to extract embedded .desktop file from the AppImage
    // AppImages have a squashfs at offset 0 or a well-known magic offset
    // Simplest approach: run the AppImage with --appimage-extract-and-run to
    // get embedded desktop metadata, or use file magic to identify structure.
    //
    // For now, use the filename as the primary name source, with a best-effort
    // approach to reading metadata.

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown");

    // Try appimage-inspect tool if available, or just use filename
    let (name, version, description) = if let Ok(output) = Command::new("file")
        .arg(path)
        .output()
    {
        let file_info = String::from_utf8_lossy(&output.stdout).to_string();
        let clean_name = filename
            .trim_end_matches(".AppImage")
            .trim_end_matches(".appimage")
            .to_string();
        // Extract version from filename pattern like "App-1.2.3-x86_64.AppImage"
        let version = extract_version_from_filename(&clean_name);
        let base_name = clean_name
            .split('-')
            .next()
            .unwrap_or(&clean_name)
            .to_string();
        let desc = if file_info.contains("ELF") {
            format!("AppImage application: {}", base_name)
        } else {
            format!("AppImage package: {}", base_name)
        };
        (base_name, version, desc)
    } else {
        let clean_name = filename
            .trim_end_matches(".AppImage")
            .trim_end_matches(".appimage")
            .to_string();
        let version = extract_version_from_filename(&clean_name);
        let base_name = clean_name
            .split('-')
            .next()
            .unwrap_or(&clean_name)
            .to_string();
        (base_name, version, String::new())
    };

    // Check if installed (in ~/.local/bin)
    let home = std::env::var("HOME").unwrap_or_default();
    let dest = format!("{}/.local/bin/{}", home, filename);
    let is_installed = std::path::Path::new(&dest).exists();

    Ok(LocalPackage {
        path: path.to_path_buf(),
        pkg_type: LocalPkgType::AppImage,
        name,
        version,
        description,
        summary: format!("AppImage portable application"),
        size_bytes,
        is_installed,
        maintainer: String::new(),
        homepage: String::new(),
    })
}

fn parse_flatpak(path: &Path, size_bytes: u64) -> Result<LocalPackage, String> {
    // Try flatpak info on the bundle path
    let output = Command::new("flatpak")
        .args(["info", "--bundle", &path.to_string_lossy()])
        .output();

    let (name, version, description) = if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let name = extract_field(&text, "Name").or_else(|| extract_field(&text, "ID"))
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
        let version = extract_field(&text, "Version").unwrap_or_default();
        let description = extract_field(&text, "Description").unwrap_or_default();
        (name, version, description)
    } else {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        (name, String::new(), String::new())
    };

    Ok(LocalPackage {
        path: path.to_path_buf(),
        pkg_type: LocalPkgType::Flatpak,
        name,
        version,
        description,
        summary: "Flatpak bundle".to_string(),
        size_bytes,
        is_installed: false,
        maintainer: String::new(),
        homepage: String::new(),
    })
}

fn parse_snap(path: &Path, size_bytes: u64) -> Result<LocalPackage, String> {
    // Snaps are SquashFS archives; we can use unsquashfs or snap info
    // Try `snap info <file>` first (works for loose snaps on some versions)
    let output = Command::new("snap")
        .args(["info", &path.to_string_lossy()])
        .output();

    let (name, version, description) = if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let name = extract_field(&text, "name").unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .trim_end_matches(".snap")
                .to_string()
        });
        let version = extract_field(&text, "version").unwrap_or_default();
        let summary = extract_field(&text, "summary").unwrap_or_default();
        let description = extract_field(&text, "description").unwrap_or(summary.clone());
        (name, version, description)
    } else {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .trim_end_matches(".snap")
            .to_string();
        (name, String::new(), String::new())
    };

    Ok(LocalPackage {
        path: path.to_path_buf(),
        pkg_type: LocalPkgType::Snap,
        name,
        version,
        description,
        summary: "Snap package".to_string(),
        size_bytes,
        is_installed: false,
        maintainer: String::new(),
        homepage: String::new(),
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract a single-line field from dpkg-deb -f output (or similar key: value format)
fn extract_field(text: &str, field: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{}:", field)) {
            let val = rest.trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
        // Case-insensitive fallback
        let lower = line.to_lowercase();
        let field_lower = format!("{}:", field.to_lowercase());
        if lower.starts_with(&field_lower) {
            if let Some(rest) = line.get(field_lower.len()..) {
                let val = rest.trim().to_string();
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extract the multi-line Description from dpkg-deb -f output.
/// The description starts after "Description: <summary>" and continues
/// with lines prefixed by a space.
fn extract_deb_description(text: &str) -> String {
    let mut in_desc = false;
    let mut lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.starts_with("Description:") {
            in_desc = true;
            // The first line after "Description:" is the one-line summary
            if let Some(rest) = line.strip_prefix("Description:") {
                let summary = rest.trim();
                if !summary.is_empty() {
                    lines.push(summary.to_string());
                }
            }
            continue;
        }
        if in_desc {
            if line.starts_with(' ') {
                // Continuation line
                let content = line.trim();
                if content == "." {
                    lines.push(String::new()); // blank paragraph separator
                } else {
                    lines.push(content.to_string());
                }
            } else {
                // New field — end of description
                break;
            }
        }
    }

    lines.join("\n").trim().to_string()
}

/// Try to extract a version string from an AppImage filename.
/// e.g., "Neovim-0.10.1-x86_64" -> "0.10.1"
fn extract_version_from_filename(name: &str) -> String {
    // Look for a segment that looks like a version: starts with digit and contains dots
    for part in name.split('-') {
        let p = part.trim_start_matches('v');
        if p.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && p.contains('.')
        {
            return p.to_string();
        }
    }
    String::new()
}

/// Parse a URI or path argument from the command line into a PathBuf.
/// Handles `file:///path/to/file` and plain paths.
pub fn parse_file_arg(arg: &str) -> Option<PathBuf> {
    if let Some(path_str) = arg.strip_prefix("file://") {
        // URL-decode minimal percent encoding
        let decoded = path_str.replace("%20", " ").replace("%3A", ":").replace("%2F", "/");
        let path = PathBuf::from(decoded);
        if path.exists() {
            return Some(path);
        }
    }

    // Try as a plain path
    let path = PathBuf::from(arg);
    if path.exists() {
        return Some(path);
    }

    None
}

/// Returns true if the arg looks like a package file that should trigger local-file mode
pub fn is_package_file_arg(arg: &str) -> bool {
    let lower = arg.to_lowercase();
    lower.ends_with(".deb")
        || lower.ends_with(".appimage")
        || lower.ends_with(".flatpak")
        || lower.ends_with(".snap")
        || (lower.starts_with("file://")
            && (lower.contains(".deb")
                || lower.contains(".appimage")
                || lower.contains(".flatpak")
                || lower.contains(".snap")))
}
