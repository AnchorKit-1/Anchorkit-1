use crate::dead_letter::DeadLetterQueue;
use crate::errors::Result;
use crate::types::DeadLetter;

/// Webhook monitor provides inspection capabilities for webhook deliveries,
/// particularly for monitoring dead-lettered deliveries.
pub struct WebhookMonitor {
    dlq: DeadLetterQueue,
}

impl WebhookMonitor {
    pub fn new(dlq: DeadLetterQueue) -> Self {
        Self { dlq }
    }

    /// Stores a dead-lettered delivery.  Provided so callers (including tests)
    /// can populate the queue without needing direct access to the inner DLQ.
    pub async fn store_dead_letter(&self, dead_letter: &DeadLetter) -> Result<()> {
        self.dlq.store(dead_letter).await
    }

    /// Retrieves a dead-lettered delivery by ID.
    pub async fn get_dead_letter(&self, id: &str) -> Result<Option<DeadLetter>> {
        self.dlq.get(id).await
    }

    /// Lists all dead-lettered deliveries with pagination.
    pub async fn list_dead_letters(&self, limit: i64, offset: i64) -> Result<Vec<DeadLetter>> {
        self.dlq.list(limit, offset).await
    }

    /// Gets the total count of dead-lettered deliveries.
    pub async fn dead_letter_count(&self) -> Result<i64> {
        self.dlq.count().await
    }

    /// Generates a monitoring report of dead-lettered deliveries.
    pub async fn generate_report(&self) -> Result<MonitoringReport> {
        let total_dead_letters = self.dlq.count().await?;
        let recent_dead_letters = self.dlq.list(10, 0).await?;

        Ok(MonitoringReport {
            total_dead_letters,
            recent_dead_letters,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MonitoringReport {
    pub total_dead_letters: i64,
    pub recent_dead_letters: Vec<DeadLetter>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePool;

    #[tokio::test]
    async fn test_monitor_report_generation() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        let dlq = DeadLetterQueue::new(pool);
        dlq.initialize()
            .await
            .expect("Failed to initialize dead-letter queue");

        let monitor = WebhookMonitor::new(dlq);

        // Create some test dead-letters
        for i in 0..3 {
            let delivery = crate::types::WebhookDelivery::new(
                format!("https://example.com/webhook/{}", i),
                serde_json::json!({"event": format!("test{}", i)}),
            );
            let dead_letter =
                DeadLetter::from_delivery(delivery, "Max retries exceeded".to_string());
            monitor.store_dead_letter(&dead_letter).await.unwrap();
        }

        let report = monitor
            .generate_report()
            .await
            .expect("Failed to generate report");

        assert_eq!(report.total_dead_letters, 3);
        assert_eq!(report.recent_dead_letters.len(), 3);
    }
}
