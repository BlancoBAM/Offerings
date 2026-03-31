// src/adapters/mod.rs - Module exports for all package adapters
mod appimage;
mod custom;
mod flatpak;
mod github;
mod homebrew;
mod lilith;
mod snap;
mod soar;

pub use appimage::AppImageAdapter;
pub use custom::CustomAdapter;
pub use flatpak::FlatpakAdapter;
pub use github::GitHubReleaseAdapter;
pub use homebrew::HomebrewAdapter;
pub use lilith::LilithCatalogAdapter;
pub use snap::SnapAdapter;
pub use soar::SoarAdapter;

use crate::model::{OperationResult, Package, PackageSource};
use async_trait::async_trait;
use std::error::Error;
use std::time::Duration;

/// Progress callback for long-running operations.
/// Receives a value from 0.0 to 1.0.
pub type ProgressCallback = std::sync::Arc<dyn Fn(f32) + Send + Sync>;

pub fn emit_progress(callback: &Option<ProgressCallback>, progress: f32) {
    if let Some(cb) = callback {
        cb(progress.clamp(0.0, 1.0));
    }
}

pub fn start_staged_progress(
    callback: Option<ProgressCallback>,
    start: f32,
    max_before_finish: f32,
    step: f32,
    interval: Duration,
) -> Option<tokio::task::JoinHandle<()>> {
    emit_progress(&callback, start);
    callback.map(move |cb| {
        tokio::spawn(async move {
            let mut progress = start;
            loop {
                tokio::time::sleep(interval).await;
                progress = (progress + step).min(max_before_finish);
                cb(progress);
            }
        })
    })
}

/// Common trait for all package adapters
#[async_trait]
pub trait PackageAdapter: Send + Sync {
    /// Get the package source type
    fn source(&self) -> PackageSource;

    /// Check if this adapter is available on the system
    async fn is_available(&self) -> bool;

    /// List all available packages from this source
    async fn list_available(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>>;

    /// List all installed packages from this source
    async fn list_installed(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>>;

    /// Get detailed info for a specific package
    #[allow(dead_code)]
    async fn get_package(&self, id: &str) -> Result<Option<Package>, Box<dyn Error + Send + Sync>>;

    /// Check for available updates
    async fn check_updates(&self) -> Result<Vec<Package>, Box<dyn Error + Send + Sync>>;

    /// Install a package
    async fn install(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;

    /// Install a package with real-time progress
    async fn install_with_progress(
        &self,
        package_id: &str,
        _callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.install(package_id).await
    }

    /// Update a package
    async fn update(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;

    /// Update a package with real-time progress
    async fn update_with_progress(
        &self,
        package_id: &str,
        _callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.update(package_id).await
    }

    /// Uninstall a package
    async fn uninstall(
        &self,
        package_id: &str,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;

    /// Uninstall a package with real-time progress
    async fn uninstall_with_progress(
        &self,
        package_id: &str,
        _callback: Option<ProgressCallback>,
    ) -> Result<OperationResult, Box<dyn Error + Send + Sync>> {
        self.uninstall(package_id).await
    }

    /// Get dependencies of a package
    #[allow(dead_code)]
    async fn get_dependencies(
        &self,
        package_id: &str,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>>;

    /// Refresh the package cache/index
    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(()) // Default: no-op
    }

    /// Launch a package
    async fn launch(&self, _package_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("Launching not supported for this source".into())
    }
}

/// Helper to strip ANSI sequences from output strings
pub fn strip_ansi_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Helper function to run a shell command and capture output
pub async fn run_command(cmd: &str, args: &[&str]) -> Result<String, Box<dyn Error + Send + Sync>> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .env("NO_COLOR", "1")
        .env("HOMEBREW_NO_COLOR", "1")
        .env("TERM", "dumb")
        .output()
        .await?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout);
        Ok(strip_ansi_escapes(&raw))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Command failed: {}", strip_ansi_escapes(&stderr)).into())
    }
}

/// Helper function to run a command and stream its output to a progress callback.
/// The callback receives (line, is_stderr).
pub async fn run_command_with_stream<F>(
    cmd: &str,
    args: &[&str],
    mut callback: F,
) -> Result<String, Box<dyn Error + Send + Sync>>
where
    F: FnMut(&str, bool) + Send + Sync + 'static,
{
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new(cmd)
        .args(args)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let mut full_output = String::new();
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut stdout_read_buf = [0u8; 1024];
    let mut stderr_read_buf = [0u8; 1024];
    let mut stdout_done = false;
    let mut stderr_done = false;

    while !stdout_done || !stderr_done {
        tokio::select! {
            result = stdout.read(&mut stdout_read_buf), if !stdout_done => {
                match result {
                    Ok(0) => {
                        stdout_done = true;
                        if !stdout_buf.is_empty() {
                            let line = String::from_utf8_lossy(&stdout_buf);
                            let cleaned = strip_ansi_escapes(&line);
                            callback(&cleaned, false);
                            full_output.push_str(&cleaned);
                            full_output.push('\n');
                            stdout_buf.clear();
                        }
                    }
                    Ok(n) => {
                        for &byte in &stdout_read_buf[..n] {
                            if byte == b'\n' || byte == b'\r' {
                                if !stdout_buf.is_empty() {
                                    let line = String::from_utf8_lossy(&stdout_buf);
                                    let cleaned = strip_ansi_escapes(&line);
                                    callback(&cleaned, false);
                                    full_output.push_str(&cleaned);
                                    full_output.push('\n');
                                    stdout_buf.clear();
                                }
                            } else {
                                stdout_buf.push(byte);
                            }
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            result = stderr.read(&mut stderr_read_buf), if !stderr_done => {
                match result {
                    Ok(0) => {
                        stderr_done = true;
                        if !stderr_buf.is_empty() {
                            let line = String::from_utf8_lossy(&stderr_buf);
                            let cleaned = strip_ansi_escapes(&line);
                            callback(&cleaned, true);
                            stderr_buf.clear();
                        }
                    }
                    Ok(n) => {
                        for &byte in &stderr_read_buf[..n] {
                            if byte == b'\n' || byte == b'\r' {
                                if !stderr_buf.is_empty() {
                                    let line = String::from_utf8_lossy(&stderr_buf);
                                    let cleaned = strip_ansi_escapes(&line);
                                    callback(&cleaned, true);
                                    stderr_buf.clear();
                                }
                            } else {
                                stderr_buf.push(byte);
                            }
                        }
                    }
                    Err(_) => (),
                }
            }
        }
    }

    let status = child.wait().await?;
    if status.success() {
        Ok(full_output)
    } else {
        Err(format!("Command failed with status: {}", status).into())
    }
}

/// Helper function to run a command with sudo
pub async fn run_sudo_command(
    cmd: &str,
    args: &[&str],
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let mut sudo_args = vec![cmd];
    sudo_args.extend(args);

    let output = tokio::process::Command::new("pkexec")
        .args(&sudo_args)
        .env("NO_COLOR", "1")
        .env("HOMEBREW_NO_COLOR", "1")
        .env("TERM", "dumb")
        .output()
        .await?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout);
        Ok(strip_ansi_escapes(&raw))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Command failed: {}", strip_ansi_escapes(&stderr)).into())
    }
}

/// Check if a command exists on the system
pub async fn command_exists(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
