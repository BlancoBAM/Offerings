// src/adapters/mod.rs - Module exports for all package adapters
mod flatpak;
mod snap;
mod appimage;
mod pacstall;
mod custom;

pub use flatpak::FlatpakAdapter;
pub use snap::SnapAdapter;
pub use appimage::AppImageAdapter;
pub use pacstall::PacstallAdapter;
pub use custom::CustomAdapter;

use crate::model::{Package, PackageSource, OperationResult};
use async_trait::async_trait;
use std::error::Error;

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
    async fn install(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;
    
    /// Update a package
    async fn update(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;
    
    /// Uninstall a package
    async fn uninstall(&self, package_id: &str) -> Result<OperationResult, Box<dyn Error + Send + Sync>>;
    
    /// Get dependencies of a package
    #[allow(dead_code)]
    async fn get_dependencies(&self, package_id: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>>;

    /// Refresh the package cache/index
    async fn refresh_cache(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(()) // Default: no-op
    }
}

/// Helper function to run a shell command and capture output
pub async fn run_command(cmd: &str, args: &[&str]) -> Result<String, Box<dyn Error + Send + Sync>> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Command failed: {}", stderr).into())
    }
}

/// Helper function to run a command with sudo
pub async fn run_sudo_command(cmd: &str, args: &[&str]) -> Result<String, Box<dyn Error + Send + Sync>> {
    let mut sudo_args = vec![cmd];
    sudo_args.extend(args);

    let output = tokio::process::Command::new("pkexec")
        .args(&sudo_args)
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Command failed: {}", stderr).into())
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
