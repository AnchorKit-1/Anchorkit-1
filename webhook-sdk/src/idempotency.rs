use crate::errors::{Result, WebhookError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePool;

/// Generates a stable idempotency key from a webhook payload.
/// Uses SHA256 hash of the payload to create a deterministic key.
pub fn derive_idempotency_key(payload: &Value) -> String {
    let payload_str = serde_json::to_string(payload).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(payload_str.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Idempotency store ensures duplicate webhook deliveries don't trigger
/// duplicate side effects (at-least-once to exactly-once conversion).
pub struct IdempotencyStore {
    pool: SqlitePool,
}

impl IdempotencyStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Initializes the idempotency storage schema.
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS idempotency_keys (
                idempotency_key TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                response TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Records a processed webhook delivery with its response.
    pub async fn record(&self, idempotency_key: &str, payload: &Value, response: &Value) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO idempotency_keys (idempotency_key, payload, response, created_at)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(idempotency_key)
        .bind(serde_json::to_string(payload)?)
        .bind(serde_json::to_string(response)?)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(())
    }

    /// Retrieves a previously recorded response for an idempotency key.
    /// Returns None if this is the first time seeing this key.
    pub async fn get(&self, idempotency_key: &str) -> Result<Option<Value>> {
        let row = sqlx::query(
            r#"
            SELECT response FROM idempotency_keys
            WHERE idempotency_key = ?
            "#,
        )
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(row.and_then(|r| {
            use sqlx::Row;
            let response: String = r.try_get("response").ok()?;
            serde_json::from_str(&response).ok()
        }))
    }

    /// Checks if a delivery has been processed before (duplicate detection).
    pub async fn is_duplicate(&self, idempotency_key: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 FROM idempotency_keys WHERE idempotency_key = ?")
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(row.is_some())
    }

    /// Cleans up old idempotency records (older than retention_days).
    pub async fn cleanup_old_records(&self, retention_days: i32) -> Result<u64> {
        let days_ago = chrono::Utc::now()
            - chrono::Duration::days(retention_days as i64);

        let result = sqlx::query(
            "DELETE FROM idempotency_keys WHERE created_at < ?"
        )
        .bind(days_ago.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| WebhookError::StorageError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_idempotency_key_deterministic() {
        let payload = serde_json::json!({"event": "test", "id": 123});
        let key1 = derive_idempotency_key(&payload);
        let key2 = derive_idempotency_key(&payload);
        assert_eq!(key1, key2, "Keys should be deterministic");
    }

    #[test]
    fn test_derive_idempotency_key_differs_for_different_payloads() {
        let payload1 = serde_json::json!({"event": "test1"});
        let payload2 = serde_json::json!({"event": "test2"});
        let key1 = derive_idempotency_key(&payload1);
        let key2 = derive_idempotency_key(&payload2);
        assert_ne!(key1, key2, "Different payloads should have different keys");
    }

    #[tokio::test]
    async fn test_idempotency_store_record_and_retrieve() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create database");

        let store = IdempotencyStore::new(pool);
        store
            .initialize()
            .await
            .expect("Failed to initialize");

        let payload = serde_json::json!({"event": "test"});
        let response = serde_json::json!({"status": "ok", "id": "123"});
        let key = derive_idempotency_key(&payload);

        store
            .record(&key, &payload, &response)
            .await
            .expect("Failed to record");

        let retrieved = store
            .get(&key)
            .await
            .expect("Failed to retrieve")
            .expect("Response not found");

        assert_eq!(retrieved, response);
    }

    #[tokio::test]
    async fn test_idempotency_store_duplicate_detection() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create database");

        let store = IdempotencyStore::new(pool);
        store
            .initialize()
            .await
            .expect("Failed to initialize");

        let payload = serde_json::json!({"event": "test"});
        let response = serde_json::json!({"status": "ok"});
        let key = derive_idempotency_key(&payload);

        // First delivery
        assert!(!store
            .is_duplicate(&key)
            .await
            .expect("Failed to check duplicate"));

        store
            .record(&key, &payload, &response)
            .await
            .expect("Failed to record");

        // Second delivery (duplicate)
        assert!(store
            .is_duplicate(&key)
            .await
            .expect("Failed to check duplicate"));
    }

    #[tokio::test]
    async fn test_idempotency_store_cleanup_old_records() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create database");

        let store = IdempotencyStore::new(pool);
        store
            .initialize()
            .await
            .expect("Failed to initialize");

        let payload = serde_json::json!({"event": "test"});
        let response = serde_json::json!({"status": "ok"});
        let key = derive_idempotency_key(&payload);

        store
            .record(&key, &payload, &response)
            .await
            .expect("Failed to record");

        // Cleanup with 0 day retention (should delete old records)
        let deleted = store
            .cleanup_old_records(0)
            .await
            .expect("Failed to cleanup");

        assert_eq!(deleted, 1);
        assert!(
            !store
                .is_duplicate(&key)
                .await
                .expect("Failed to check after cleanup")
        );
    }
}
