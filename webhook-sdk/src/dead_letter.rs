use crate::errors::{Result, WebhookError};
use crate::types::DeadLetter;
use sqlx::sqlite::SqlitePool;

/// Dead-letter queue storage backend.
pub struct DeadLetterQueue {
    pool: SqlitePool,
}

impl DeadLetterQueue {
    /// Creates a new dead-letter queue with the given database pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Initializes the dead-letter queue schema.
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dead_letters (
                id TEXT PRIMARY KEY,
                delivery_id TEXT NOT NULL,
                url TEXT NOT NULL,
                payload TEXT NOT NULL,
                status_code INTEGER,
                response_body TEXT,
                attempt INTEGER NOT NULL,
                error TEXT,
                created_at TEXT NOT NULL,
                last_attempted_at TEXT,
                delivered BOOLEAN NOT NULL,
                reason TEXT NOT NULL,
                dead_lettered_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Stores a delivery in the dead-letter queue.
    pub async fn store(&self, dead_letter: &DeadLetter) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO dead_letters (
                id, delivery_id, url, payload, status_code, response_body,
                attempt, error, created_at, last_attempted_at, delivered,
                reason, dead_lettered_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&dead_letter.id)
        .bind(&dead_letter.delivery.id)
        .bind(&dead_letter.delivery.url)
        .bind(serde_json::to_string(&dead_letter.delivery.payload)?)
        .bind(dead_letter.delivery.status_code)
        .bind(&dead_letter.delivery.response_body)
        .bind(dead_letter.delivery.attempt)
        .bind(&dead_letter.delivery.error)
        .bind(dead_letter.delivery.created_at.to_rfc3339())
        .bind(
            dead_letter
                .delivery
                .last_attempted_at
                .map(|dt| dt.to_rfc3339()),
        )
        .bind(dead_letter.delivery.delivered)
        .bind(&dead_letter.reason)
        .bind(dead_letter.dead_lettered_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Retrieves a dead-lettered delivery by ID.
    pub async fn get(&self, id: &str) -> Result<Option<DeadLetter>> {
        let row = sqlx::query(
            r#"
            SELECT id, delivery_id, url, payload, status_code, response_body,
                   attempt, error, created_at, last_attempted_at, delivered,
                   reason, dead_lettered_at
            FROM dead_letters
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(row.map(|r| self.row_to_dead_letter(&r)))
    }

    /// Lists all dead-lettered deliveries with pagination.
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<DeadLetter>> {
        let rows = sqlx::query(
            r#"
            SELECT id, delivery_id, url, payload, status_code, response_body,
                   attempt, error, created_at, last_attempted_at, delivered,
                   reason, dead_lettered_at
            FROM dead_letters
            ORDER BY dead_lettered_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(rows.iter().map(|r| self.row_to_dead_letter(r)).collect())
    }

    /// Counts total dead-lettered deliveries.
    pub async fn count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM dead_letters")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(row.0)
    }

    fn row_to_dead_letter(&self, row: &sqlx::sqlite::SqliteRow) -> DeadLetter {
        use sqlx::Row;

        let id: String = row.get("id");
        let delivery_id: String = row.get("delivery_id");
        let url: String = row.get("url");
        let payload: String = row.get("payload");
        let status_code: Option<i32> = row.get("status_code");
        let response_body: Option<String> = row.get("response_body");
        let attempt: i32 = row.get("attempt");
        let error: Option<String> = row.get("error");
        let created_at: String = row.get("created_at");
        let last_attempted_at: Option<String> = row.get("last_attempted_at");
        let delivered: bool = row.get("delivered");
        let reason: String = row.get("reason");
        let dead_lettered_at: String = row.get("dead_lettered_at");

        let delivery = crate::types::WebhookDelivery {
            id: delivery_id,
            url,
            payload: serde_json::from_str(&payload).unwrap_or(serde_json::json!({})),
            status_code: status_code.map(|s| s as u16),
            response_body,
            attempt: attempt as u32,
            error,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            last_attempted_at: last_attempted_at.and_then(|dt| {
                chrono::DateTime::parse_from_rfc3339(&dt)
                    .ok()
                    .map(|pdt| pdt.with_timezone(&chrono::Utc))
            }),
            delivered,
        };

        DeadLetter {
            id,
            delivery,
            reason,
            dead_lettered_at: chrono::DateTime::parse_from_rfc3339(&dead_lettered_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dead_letter_store_and_retrieve() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        let dlq = DeadLetterQueue::new(pool);
        dlq.initialize()
            .await
            .expect("Failed to initialize dead-letter queue");

        let delivery = crate::types::WebhookDelivery::new(
            "https://example.com/webhook".to_string(),
            serde_json::json!({"event": "test"}),
        );

        let dead_letter = DeadLetter::from_delivery(delivery, "Max retries exceeded".to_string());

        dlq.store(&dead_letter)
            .await
            .expect("Failed to store dead-letter");

        let retrieved = dlq
            .get(&dead_letter.id)
            .await
            .expect("Failed to retrieve dead-letter")
            .expect("Dead-letter not found");

        assert_eq!(retrieved.id, dead_letter.id);
        assert_eq!(retrieved.delivery.url, dead_letter.delivery.url);
        assert_eq!(retrieved.reason, "Max retries exceeded");
    }

    #[tokio::test]
    async fn test_dead_letter_list_and_count() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        let dlq = DeadLetterQueue::new(pool);
        dlq.initialize()
            .await
            .expect("Failed to initialize dead-letter queue");

        for i in 0..3 {
            let delivery = crate::types::WebhookDelivery::new(
                format!("https://example.com/webhook/{}", i),
                serde_json::json!({"event": format!("test{}", i)}),
            );
            let dead_letter =
                DeadLetter::from_delivery(delivery, "Max retries exceeded".to_string());
            dlq.store(&dead_letter)
                .await
                .expect("Failed to store dead-letter");
        }

        let count = dlq.count().await.expect("Failed to count dead-letters");
        assert_eq!(count, 3);

        let list = dlq
            .list(10, 0)
            .await
            .expect("Failed to list dead-letters");
        assert_eq!(list.len(), 3);
    }
}
