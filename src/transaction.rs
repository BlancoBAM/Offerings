// src/transaction.rs - Transaction Management and Safety Mechanisms
use crate::db::Database;
use crate::model::{OperationResult, Package, PackageOperation, TransactionLog, TransactionStatus};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Mutex as TokioMutex};

/// Transaction event for subscribers
#[derive(Clone, Debug)]
pub enum TransactionEvent {
    Started {
        transaction_id: i64,
        operation: String,
        package_id: String,
    },
    Progress {
        transaction_id: i64,
        percent: f32,
        message: String,
    },
    Completed {
        transaction_id: i64,
        success: bool,
        message: String,
    },
    RolledBack {
        transaction_id: i64,
    },
}

/// Transaction manager for safe package operations
pub struct TransactionManager {
    db: Arc<Database>,
    operation_lock: Arc<TokioMutex<()>>,
    event_sender: broadcast::Sender<TransactionEvent>,
    pending_operations: Arc<Mutex<VecDeque<PendingOperation>>>,
}

#[derive(Clone, Debug)]
struct PendingOperation {
    operation: PackageOperation,
    priority: i32,
}

impl TransactionManager {
    pub fn new(db: Arc<Database>) -> Self {
        let (event_sender, _) = broadcast::channel(100);
        Self {
            db,
            operation_lock: Arc::new(TokioMutex::new(())),
            event_sender,
            pending_operations: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Subscribe to transaction events
    pub fn subscribe(&self) -> broadcast::Receiver<TransactionEvent> {
        self.event_sender.subscribe()
    }

    /// Execute an operation with full transaction logging
    pub async fn execute<F, Fut>(
        &self,
        operation: &PackageOperation,
        package: Option<&Package>,
        executor: F,
    ) -> Result<OperationResult, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<
            Output = Result<OperationResult, Box<dyn std::error::Error + Send + Sync>>,
        >,
    {
        // Acquire operation lock
        let _lock = self.operation_lock.lock().await;

        // Get operation details
        let (op_name, pkg_id) = match operation {
            PackageOperation::Install(id) => ("install", id.clone()),
            PackageOperation::Update(id) => ("update", id.clone()),
            PackageOperation::Uninstall(id) => ("uninstall", id.clone()),
            PackageOperation::UpdateAll => ("update_all", "all".to_string()),
        };

        // Serialize previous state if package exists
        let previous_state = package.map(|p| serde_json::to_string(p).unwrap_or_default());
        let source = package
            .map(|p| format!("{:?}", p.identity.source))
            .unwrap_or_else(|| "unknown".to_string());

        // Start transaction
        let transaction_id =
            self.db
                .start_transaction(op_name, &pkg_id, &source, previous_state.as_deref())?;

        // Notify subscribers
        let _ = self.event_sender.send(TransactionEvent::Started {
            transaction_id,
            operation: op_name.to_string(),
            package_id: pkg_id.clone(),
        });

        // Execute the operation
        let result = executor().await;

        // Complete transaction based on result
        match &result {
            Ok(op_result) => {
                let new_state = if op_result.success {
                    Some(serde_json::to_string(op_result).unwrap_or_default())
                } else {
                    None
                };

                self.db.complete_transaction(
                    transaction_id,
                    op_result.success,
                    new_state.as_deref(),
                    if op_result.success {
                        None
                    } else {
                        Some(&op_result.message)
                    },
                )?;

                let _ = self.event_sender.send(TransactionEvent::Completed {
                    transaction_id,
                    success: op_result.success,
                    message: op_result.message.clone(),
                });
            }
            Err(e) => {
                self.db
                    .complete_transaction(transaction_id, false, None, Some(&e.to_string()))?;

                let _ = self.event_sender.send(TransactionEvent::Completed {
                    transaction_id,
                    success: false,
                    message: e.to_string(),
                });
            }
        }

        result
    }

    /// Send progress update for a transaction
    pub fn report_progress(&self, transaction_id: i64, percent: f32, message: &str) {
        let _ = self.event_sender.send(TransactionEvent::Progress {
            transaction_id,
            percent,
            message: message.to_string(),
        });
    }

    /// Attempt to rollback a transaction
    pub async fn rollback(&self, transaction_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let transactions = self.db.get_recent_transactions(100)?;

        let transaction = transactions
            .iter()
            .find(|t| t.id == transaction_id)
            .ok_or("Transaction not found")?;

        if transaction.status != TransactionStatus::Completed {
            return Err("Can only rollback completed transactions".into());
        }

        // Mark as rolled back
        self.db.rollback_transaction(transaction_id)?;

        let _ = self
            .event_sender
            .send(TransactionEvent::RolledBack { transaction_id });

        Ok(())
    }

    /// Queue an operation for later execution
    pub fn queue_operation(&self, operation: PackageOperation, priority: i32) {
        let mut queue = self.pending_operations.lock().unwrap();

        let pending = PendingOperation {
            operation,
            priority,
        };

        // Insert based on priority (higher priority first)
        let pos = queue
            .iter()
            .position(|op| op.priority < priority)
            .unwrap_or(queue.len());
        queue.insert(pos, pending);
    }

    /// Get the next pending operation
    pub fn next_pending(&self) -> Option<PackageOperation> {
        let mut queue = self.pending_operations.lock().unwrap();
        queue.pop_front().map(|p| p.operation)
    }

    /// Get recent transaction history
    pub fn get_history(
        &self,
        limit: i32,
    ) -> Result<Vec<TransactionLog>, Box<dyn std::error::Error>> {
        Ok(self.db.get_recent_transactions(limit)?)
    }

    /// Check if there are any failed transactions that might need attention
    pub fn get_failed_transactions(
        &self,
    ) -> Result<Vec<TransactionLog>, Box<dyn std::error::Error>> {
        let all = self.db.get_recent_transactions(50)?;
        Ok(all
            .into_iter()
            .filter(|t| t.status == TransactionStatus::Failed)
            .collect())
    }
}

/// Rollback engine for undoing package operations
pub struct RollbackEngine {
    db: Arc<Database>,
}

impl RollbackEngine {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Get rollback points (completed transactions that can be undone)
    pub fn get_rollback_points(&self) -> Result<Vec<RollbackPoint>, Box<dyn std::error::Error>> {
        let transactions = self.db.get_recent_transactions(20)?;

        Ok(transactions
            .into_iter()
            .filter(|t| t.status == TransactionStatus::Completed && t.previous_state.is_some())
            .map(|t| RollbackPoint {
                transaction_id: t.id,
                operation: t.operation.clone(),
                package_id: t.package_id.clone(),
                timestamp: t.started_at,
                can_rollback: Self::can_rollback(&t),
            })
            .collect())
    }

    fn can_rollback(transaction: &TransactionLog) -> bool {
        // Only install and update operations can be rolled back
        matches!(transaction.operation.as_str(), "install" | "update")
    }

    /// Get the previous state for a transaction
    pub fn get_previous_state(
        &self,
        transaction_id: i64,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let transactions = self.db.get_recent_transactions(100)?;
        Ok(transactions
            .into_iter()
            .find(|t| t.id == transaction_id)
            .and_then(|t| t.previous_state))
    }
}

/// Represents a point in time that can be rolled back to
#[derive(Debug, Clone)]
pub struct RollbackPoint {
    pub transaction_id: i64,
    pub operation: String,
    pub package_id: String,
    pub timestamp: i64,
    pub can_rollback: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_transaction_manager() {
        let db = Arc::new(Database::open(&PathBuf::from(":memory:")).unwrap());
        let manager = TransactionManager::new(db);

        let op = PackageOperation::Install("test-pkg".to_string());

        let result = manager
            .execute(&op, None, || async {
                Ok(OperationResult {
                    success: true,
                    message: "Installed successfully".to_string(),
                    updated_packages: vec!["test-pkg".to_string()],
                })
            })
            .await
            .unwrap();

        assert!(result.success);

        let history = manager.get_history(10).unwrap();
        assert!(!history.is_empty());
        assert_eq!(history[0].operation, "install");
    }
}
