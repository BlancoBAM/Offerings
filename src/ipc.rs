// src/ipc.rs - IPC Server for External Control
use crate::model::{OperationResult, Package};
// use crate::model::{Package, PackageOperation, OperationResult};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
// use std::error::Error; // Removed unused import
// use std::sync::Mutex;
// use std::sync::Arc;
use tokio::sync::mpsc;

/// IPC request from external clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "params")]
pub enum IpcRequest {
    /// Get current status
    Status,
    /// Search for packages
    Search { query: String },
    /// Get package details
    GetPackage { id: String },
    /// Install a package
    Install { id: String },
    /// Uninstall a package
    Uninstall { id: String },
    /// Update a package
    Update { id: String },
    /// Update all packages
    UpdateAll,
    /// List installed packages
    ListInstalled,
    /// List available updates
    ListUpdates,
    /// Refresh package cache
    Refresh,
    /// Quit the application
    Quit,
}

/// IPC response to clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<IpcResponseData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IpcResponseData {
    Status(StatusData),
    Packages(Vec<PackageSummary>),
    Package(PackageSummary),
    Operation(OperationResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    pub running: bool,
    pub installed_count: usize,
    pub updates_available: usize,
    pub active_operations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub source: String,
    pub version: Option<String>,
    pub is_installed: bool,
    pub has_update: bool,
}

impl From<&Package> for PackageSummary {
    fn from(pkg: &Package) -> Self {
        Self {
            id: pkg.identity.id.clone(),
            name: pkg.identity.name.clone(),
            source: pkg.identity.source.label().to_string(),
            version: pkg.version.installed.clone().or(pkg.version.latest.clone()),
            is_installed: pkg.is_installed,
            has_update: pkg.version.has_update(),
        }
    }
}

/// Command sent from IPC server to the main application
#[derive(Debug)]
pub enum IpcCommand {
    Request(IpcRequest, mpsc::Sender<IpcResponse>),
    Shutdown,
}

/// IPC Server for handling external control requests
pub struct IpcServer {
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new() -> Self {
        Self {
            socket_path: Self::default_socket_path(),
        }
    }

    /// Create IPC server with custom socket path
    pub fn with_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Get the default socket path
    fn default_socket_path() -> PathBuf {
        let uid = unsafe { libc::getuid() };
        let runtime_dir = PathBuf::from(format!("/run/user/{}", uid));
        if runtime_dir.is_dir() {
            runtime_dir.join("offerings.sock")
        } else {
            std::env::temp_dir().join(format!("offerings-{}.sock", uid))
        }
    }

    /// Start the IPC server
    pub fn start(
        &mut self,
        runtime_handle: tokio::runtime::Handle,
    ) -> Result<mpsc::Receiver<IpcCommand>, Box<dyn std::error::Error>> {
        // Remove existing socket if present
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        // Ensure parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let listener = UnixListener::bind(&self.socket_path)?;

        // Set socket permissions (user only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.socket_path, std::fs::Permissions::from_mode(0o600))?;
        }

        let (command_sender, command_receiver) = mpsc::channel::<IpcCommand>(100);

        // Clone for the listener thread
        let socket_path = self.socket_path.clone();
        let runtime_handle = runtime_handle.clone();

        // Spawn listener thread
        std::thread::spawn(move || {
            Self::accept_connections(listener, command_sender, socket_path, runtime_handle);
        });

        Ok(command_receiver)
    }

    fn accept_connections(
        listener: UnixListener,
        command_sender: mpsc::Sender<IpcCommand>,
        socket_path: PathBuf,
        runtime_handle: tokio::runtime::Handle,
    ) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let sender = command_sender.clone();
                    let runtime_handle = runtime_handle.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = Self::handle_client(stream, sender, runtime_handle) {
                            eprintln!("IPC client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("IPC accept error: {}", e);
                    break;
                }
            }
        }

        // Cleanup socket on exit
        std::fs::remove_file(&socket_path).ok();
    }

    fn handle_client(
        mut stream: UnixStream,
        command_sender: mpsc::Sender<IpcCommand>,
        runtime_handle: tokio::runtime::Handle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let reader = BufReader::new(stream.try_clone()?);

        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            // Parse request
            let request: IpcRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    let error_response = IpcResponse {
                        success: false,
                        message: format!("Invalid request: {}", e),
                        data: None,
                    };
                    let response_json = serde_json::to_string(&error_response)?;
                    writeln!(stream, "{}", response_json)?;
                    continue;
                }
            };

            // Check for quit command
            let is_quit = matches!(request, IpcRequest::Quit);

            // Create response channel
            let (response_sender, mut response_receiver) = mpsc::channel::<IpcResponse>(1);

            // Send command to main app
            runtime_handle.block_on(async {
                if command_sender
                    .send(IpcCommand::Request(request, response_sender))
                    .await
                    .is_err()
                {
                    return;
                }

                // Wait for response
                if let Some(response) = response_receiver.recv().await {
                    let response_json = serde_json::to_string(&response).unwrap_or_default();
                    let _ = writeln!(stream, "{}", response_json);
                }
            });

            if is_quit {
                let _ = command_sender.blocking_send(IpcCommand::Shutdown);
                break;
            }
        }

        Ok(())
    }

    /// Stop the IPC server and cleanup
    pub fn stop(&mut self) {
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).ok();
        }
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// IPC Client for sending commands to the running application
pub struct IpcClient {
    socket_path: PathBuf,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new() -> Self {
        Self {
            socket_path: IpcServer::default_socket_path(),
        }
    }

    /// Create client with custom socket path
    pub fn with_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Check if the server is running
    pub fn is_server_running(&self) -> bool {
        self.socket_path.exists() && UnixStream::connect(&self.socket_path).is_ok()
    }

    /// Send a request and get a response
    pub fn send(&self, request: IpcRequest) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        let mut stream = UnixStream::connect(&self.socket_path)?;

        let request_json = serde_json::to_string(&request)?;
        writeln!(stream, "{}", request_json)?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response: IpcResponse = serde_json::from_str(&response_line)?;
        Ok(response)
    }

    /// Convenience methods for common operations
    pub fn status(&self) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Status)
    }

    pub fn search(&self, query: &str) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Search {
            query: query.to_string(),
        })
    }

    pub fn install(&self, package_id: &str) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Install {
            id: package_id.to_string(),
        })
    }

    pub fn uninstall(&self, package_id: &str) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Uninstall {
            id: package_id.to_string(),
        })
    }

    pub fn update(&self, package_id: &str) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Update {
            id: package_id.to_string(),
        })
    }

    pub fn update_all(&self) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::UpdateAll)
    }

    pub fn list_installed(&self) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::ListInstalled)
    }

    pub fn list_updates(&self) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::ListUpdates)
    }

    pub fn refresh(&self) -> Result<IpcResponse, Box<dyn std::error::Error>> {
        self.send(IpcRequest::Refresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = IpcRequest::Install {
            id: "firefox".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Install"));
        assert!(json.contains("firefox"));
    }

    #[test]
    fn test_response_serialization() {
        let response = IpcResponse {
            success: true,
            message: "OK".to_string(),
            data: Some(IpcResponseData::Status(StatusData {
                running: true,
                installed_count: 100,
                updates_available: 5,
                active_operations: 0,
            })),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("running"));
    }
}
