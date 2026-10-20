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
use std::sync::{Arc, Mutex};
/// Progress callback for long-running operations.
/// Receives a value from 0.0 to 1.0.
pub type ProgressCallback = std::sync::Arc<dyn Fn(f32) + Send + Sync>;

/// Extended progress callback carrying a status message alongside the fraction.
/// Used for database refresh and multi-stage operations that report real
/// measured progress ("2140 of 6830 rows written").
pub type DbProgressCallback = std::sync::Arc<dyn Fn(f32, String) + Send + Sync>;

/// Monotonic real-progress window.
///
/// Wraps an optional `ProgressCallback` and a `[start, end]` window of the
/// overall operation. Values pushed into it MUST represent actually observed
/// facts (tool-reported percentages, completed real sub-steps); the window
/// clamps and monotonically advances them, so the UI bars never jump backwards.
pub struct RealProgress {
    callback: Option<ProgressCallback>,
    start: f32,
    end: f32,
    last: Mutex<f32>,
}

impl RealProgress {
    /// Create the tracker for the `[start, end]` window of the whole operation.
    /// `start` is emitted immediately so the UI shows a live bar right away.
    pub fn new(callback: Option<ProgressCallback>, start: f32, end: f32) -> Self {
        let rp = Self {
            callback,
            start,
            end,
            last: Mutex::new(start),
        };
        rp.emit_raw(start);
        rp
    }

    fn emit_raw(&self, value: f32) {
        if let Some(cb) = &self.callback {
            cb(value.clamp(0.0, 1.0));
        }
    }

    /// Push an observed absolute progress value; clamped to [start, end] and
    /// monotonic (never moves backwards).
    pub fn set(&self, value: f32) {
        let clamped = value.clamp(self.start, self.end);
        let mut last = self.last.lock().unwrap();
        if clamped > *last {
            *last = clamped;
            self.emit_raw(clamped);
        }
    }

    /// Push a tool-reported percentage (0.0..=1.0) mapped into the window.
    pub fn set_percent(&self, percent: f32) {
        let p = percent.clamp(0.0, 1.0);
        self.set(self.start + (self.end - self.start) * p);
    }

    /// A real sub-operation completed ahead of the expected window fraction.
    pub fn milestone(&self, fraction_of_window: f32) {
        self.set(self.start + (self.end - self.start) * fraction_of_window.clamp(0.0, 1.0));
    }

    /// Operation finished successfully.
    pub fn finish(&self) {
        self.set(1.0);
        self.emit_raw(1.0);
    }

    /// Returns a closure for `run_command_with_stream` that lifts tool-published
    /// percentages (e.g. "Downloading 45%", "Setup snap ... 78%") into the window.
    pub fn percent_mapper(
        &self,
    ) -> impl Fn(&str, bool) + Send + Sync + use<> {
        let this = Arc::new(Self {
            callback: self.callback.clone(),
            start: self.start,
            end: self.end,
            last: Mutex::new(*self.last.lock().unwrap()),
        });
        move |line: &str, _is_stderr: bool| {
            if let Some(p) = extract_percent(line) {
                this.set_percent(p);
            }
        }
    }

    /// The last emitted raw value.
    pub fn current(&self) -> f32 {
        *self.last.lock().unwrap()
    }
}

/// Extract a percentage published in a tool output chunk ("45%", "12.5 %").
/// Returns the fraction (0.0..=1.0) or `None`.
pub fn extract_percent(line: &str) -> Option<f32> {
    let chars: Vec<char> = line.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if *ch == '%' {
            let mut start = i;
            let mut saw_digit = false;
            let mut dot_used = false;
            while start > 0 {
                let c = chars[start - 1];
                if c.is_ascii_digit() {
                    saw_digit = true;
                    start -= 1;
                } else if c == '.' && !dot_used && start > 1 && chars[start - 2].is_ascii_digit() {
                    dot_used = true;
                    start -= 1;
                } else {
                    break;
                }
            }
            if saw_digit {
                let s: String = chars[start..i].iter().collect();
                let s = s.replace(',', "");
                if let Ok(v) = s.trim().parse::<f32>() {
                    if (0.0..=100.0).contains(&v) {
                        return Some(v / 100.0);
                    }
                }
            }
        }
    }
    None
}

pub fn emit_progress(callback: &Option<ProgressCallback>, progress: f32) {
    if let Some(cb) = callback {
        cb(progress.clamp(0.0, 1.0));
    }
}

/// Common trait for all package adapters
#[async_trait]
pub trait PackageAdapter: Send + Sync {
    /// Get the package source type
    fn source(&self) -> PackageSource;

    /// Downcast hook so the backend can reach concrete adapter extensions
    /// (e.g. the Direct Release adapter's "add any repo" flow).
    fn as_any(&self) -> &dyn std::any::Any;

    /// Whether this adapter's `list_available()` belongs in the browse
    /// catalogue. The store's central concept: the OFFLINE unified catalogue
    /// (one deduplicated desktop-app entry per app) is the single browse
    /// source. Runtime package managers (flatpak/snap/am/brew) still serve
    /// installed-state tracking, install/uninstall and update checks — they
    /// just must not re-flood the browse grid with their own raw listings
    /// (which re-introduce CLI plumbing and duplicates that the catalogue
    /// builder already filtered out).
    fn contributes_browse(&self) -> bool {
        true
    }

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
    // Errors are reported from stderr; keep the last meaningful lines so the
    // failure message says WHAT failed ("No installed refs found for 'X'")
    // instead of a bare "exit status: 1".
    let mut stderr_tail: Vec<String> = Vec::new();
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
                            push_stderr_tail(&mut stderr_tail, &cleaned);
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
                                    push_stderr_tail(&mut stderr_tail, &cleaned);
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
        let detail = stderr_tail.join(" ").trim().to_string();
        if detail.is_empty() {
            Err(format!("Command failed with status: {}", status).into())
        } else {
            Err(format!("Command failed: {}", detail).into())
        }
    }
}

/// Ring-buffer of the last few stderr lines used for error reporting.
fn push_stderr_tail(tail: &mut Vec<String>, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    tail.push(trimmed.to_string());
    if tail.len() > 5 {
        let drop_n = tail.len() - 5;
        tail.drain(..drop_n);
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
