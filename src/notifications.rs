// src/notifications.rs - Desktop Notification System
use notify_rust::{Notification, Timeout, Urgency};
// use std::sync::Arc;
use tokio::sync::broadcast;

/// Types of notifications the system can send
#[derive(Debug, Clone)]
pub enum NotificationType {
    /// Package installation completed
    InstallComplete { package_name: String, success: bool },
    /// Package update completed
    UpdateComplete {
        package_name: String,
        old_version: String,
        new_version: String,
    },
    /// Package uninstallation completed
    UninstallComplete { package_name: String },
    /// Updates are available
    UpdatesAvailable { count: usize },
    /// Operation failed with error
    OperationFailed {
        operation: String,
        package_name: String,
        error: String,
    },
    /// Background operation started
    OperationStarted {
        operation: String,
        package_name: String,
    },
    /// Batch operation completed
    BatchComplete {
        operation: String,
        count: usize,
        failed: usize,
    },
    /// Fallback installation succeeded
    FallbackSuccess {
        package_name: String,
        source: String,
        original_source: String,
    },
}

/// Configuration for the notification system
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Show notifications for successful operations
    pub notify_success: bool,
    /// Show notifications for failed operations
    pub notify_failure: bool,
    /// Show notifications for available updates
    pub notify_updates: bool,
    /// Show notifications when operations start
    pub notify_start: bool,
    /// Desktop notification timeout in milliseconds (0 = system default)
    pub timeout_ms: u32,
    /// Application name shown in notifications
    pub app_name: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_success: true,
            notify_failure: true,
            notify_updates: true,
            notify_start: false,
            timeout_ms: 5000,
            app_name: "Offerings".to_string(),
        }
    }
}

/// Desktop notification manager
pub struct NotificationManager {
    config: NotificationConfig,
    event_sender: broadcast::Sender<NotificationType>,
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        let (event_sender, _) = broadcast::channel(50);
        Self {
            config,
            event_sender,
        }
    }

    /// Subscribe to notification events
    pub fn subscribe(&self) -> broadcast::Receiver<NotificationType> {
        self.event_sender.subscribe()
    }

    /// Send a notification
    pub fn notify(
        &self,
        notification_type: NotificationType,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Broadcast event
        let _ = self.event_sender.send(notification_type.clone());

        // Check if we should show this notification based on config
        if !self.should_show(&notification_type) {
            return Ok(());
        }

        let (title, body, urgency, icon) = self.build_notification_content(&notification_type);

        let mut notification = Notification::new();
        notification
            .appname(&self.config.app_name)
            .summary(&title)
            .body(&body)
            .icon(icon)
            .urgency(urgency);

        if self.config.timeout_ms > 0 {
            notification.timeout(Timeout::Milliseconds(self.config.timeout_ms));
        }

        // Show notification (this is non-blocking)
        notification.show()?;

        Ok(())
    }

    /// Send notification asynchronously
    pub async fn notify_async(&self, notification_type: NotificationType) {
        let config = self.config.clone();
        let event_sender = self.event_sender.clone();

        tokio::task::spawn_blocking(move || {
            let _ = event_sender.send(notification_type.clone());

            let manager = NotificationManager {
                config,
                event_sender,
            };

            if let Err(e) = manager.send_notification(&notification_type) {
                eprintln!("Failed to send notification: {}", e);
            }
        });
    }

    fn should_show(&self, notification_type: &NotificationType) -> bool {
        match notification_type {
            NotificationType::InstallComplete { success, .. } => {
                if *success {
                    self.config.notify_success
                } else {
                    self.config.notify_failure
                }
            }
            NotificationType::UpdateComplete { .. } => self.config.notify_success,
            NotificationType::UninstallComplete { .. } => self.config.notify_success,
            NotificationType::UpdatesAvailable { .. } => self.config.notify_updates,
            NotificationType::OperationFailed { .. } => self.config.notify_failure,
            NotificationType::OperationStarted { .. } => self.config.notify_start,
            NotificationType::BatchComplete { failed, .. } => {
                if *failed > 0 {
                    self.config.notify_failure
                } else {
                    self.config.notify_success
                }
            }
            NotificationType::FallbackSuccess { .. } => self.config.notify_success,
        }
    }

    fn build_notification_content(
        &self,
        notification_type: &NotificationType,
    ) -> (String, String, Urgency, &'static str) {
        match notification_type {
            NotificationType::InstallComplete {
                package_name,
                success,
            } => {
                if *success {
                    (
                        "Installation Complete".to_string(),
                        format!("{} has been installed successfully.", package_name),
                        Urgency::Normal,
                        "package-x-generic",
                    )
                } else {
                    (
                        "Installation Failed".to_string(),
                        format!("Failed to install {}.", package_name),
                        Urgency::Critical,
                        "dialog-error",
                    )
                }
            }
            NotificationType::UpdateComplete {
                package_name,
                old_version,
                new_version,
            } => (
                "Update Complete".to_string(),
                format!(
                    "{} updated: {} → {}",
                    package_name, old_version, new_version
                ),
                Urgency::Normal,
                "software-update-available",
            ),
            NotificationType::UninstallComplete { package_name } => (
                "Uninstallation Complete".to_string(),
                format!("{} has been removed.", package_name),
                Urgency::Normal,
                "user-trash",
            ),
            NotificationType::UpdatesAvailable { count } => (
                "Updates Available".to_string(),
                format!(
                    "{} package{} can be updated.",
                    count,
                    if *count == 1 { "" } else { "s" }
                ),
                Urgency::Low,
                "software-update-available",
            ),
            NotificationType::OperationFailed {
                operation,
                package_name,
                error,
            } => (
                format!("{} Failed", Self::capitalize(operation)),
                format!("Failed to {} {}: {}", operation, package_name, error),
                Urgency::Critical,
                "dialog-error",
            ),
            NotificationType::OperationStarted {
                operation,
                package_name,
            } => (
                format!("{}...", Self::capitalize(operation)),
                format!("{} {}...", Self::capitalize(operation), package_name),
                Urgency::Low,
                "system-software-install",
            ),
            NotificationType::BatchComplete {
                operation,
                count,
                failed,
            } => {
                if *failed == 0 {
                    (
                        format!("{} Complete", Self::capitalize(operation)),
                        format!("Successfully {} {} packages.", operation, count),
                        Urgency::Normal,
                        "emblem-ok-symbolic",
                    )
                } else {
                    (
                        format!("{} Complete with Errors", Self::capitalize(operation)),
                        format!(
                            "{} {} packages, {} failed.",
                            Self::capitalize(operation),
                            count,
                            failed
                        ),
                        Urgency::Critical,
                        "dialog-warning",
                    )
                }
            }
            NotificationType::FallbackSuccess {
                package_name,
                source,
                original_source,
            } => (
                "Installation Successful (Fallback)".to_string(),
                format!(
                    "{} was installed from {} after {} failed.",
                    package_name, source, original_source
                ),
                Urgency::Normal,
                "emblem-default",
            ),
        }
    }

    fn send_notification(
        &self,
        notification_type: &NotificationType,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.should_show(notification_type) {
            return Ok(());
        }

        let (title, body, urgency, icon) = self.build_notification_content(notification_type);

        let mut notification = Notification::new();
        notification
            .appname(&self.config.app_name)
            .summary(&title)
            .body(&body)
            .icon(icon)
            .urgency(urgency);

        if self.config.timeout_ms > 0 {
            notification.timeout(Timeout::Milliseconds(self.config.timeout_ms));
        }

        notification.show()?;
        Ok(())
    }

    fn capitalize(s: &str) -> String {
        let mut chars = s.chars();
        match chars.next() {
            None => String::new(),
            Some(c) => c.to_uppercase().chain(chars).collect(),
        }
    }

    /// Notify about install completion
    pub fn notify_install(&self, package_name: &str, success: bool) {
        let _ = self.notify(NotificationType::InstallComplete {
            package_name: package_name.to_string(),
            success,
        });
    }

    /// Notify about update completion
    pub fn notify_update(&self, package_name: &str, old_version: &str, new_version: &str) {
        let _ = self.notify(NotificationType::UpdateComplete {
            package_name: package_name.to_string(),
            old_version: old_version.to_string(),
            new_version: new_version.to_string(),
        });
    }

    /// Notify about uninstall completion
    pub fn notify_uninstall(&self, package_name: &str) {
        let _ = self.notify(NotificationType::UninstallComplete {
            package_name: package_name.to_string(),
        });
    }

    /// Notify about available updates
    pub fn notify_updates_available(&self, count: usize) {
        if count > 0 {
            let _ = self.notify(NotificationType::UpdatesAvailable { count });
        }
    }

    /// Notify about operation failure
    pub fn notify_error(&self, operation: &str, package_name: &str, error: &str) {
        let _ = self.notify(NotificationType::OperationFailed {
            operation: operation.to_string(),
            package_name: package_name.to_string(),
            error: error.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_manager_creation() {
        let config = NotificationConfig::default();
        let manager = NotificationManager::new(config);

        // Test subscription works
        let _receiver = manager.subscribe();
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(NotificationManager::capitalize("install"), "Install");
        assert_eq!(NotificationManager::capitalize("update"), "Update");
        assert_eq!(NotificationManager::capitalize(""), "");
    }
}
