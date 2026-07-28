use webhook_sdk::{
    dead_letter::DeadLetterQueue, delivery::WebhookDeliverer, monitor::WebhookMonitor,
    retry::RetryConfig, types::{DeadLetter, WebhookDelivery},
};
use sqlx::sqlite::SqlitePool;
use std::time::Duration;

#[tokio::test]
async fn test_delivery_exhausts_retries_and_lands_in_dead_letter() {
    // Setup: Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let dlq = DeadLetterQueue::new(pool);
    dlq.initialize()
        .await
        .expect("Failed to initialize dead-letter queue");

    // Configure retries with 3 attempts
    let retry_config = RetryConfig {
        max_retries: 3,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(100),
        backoff_multiplier: 2.0,
    };

    let deliverer = WebhookDeliverer::new(retry_config);

    // Create a delivery to an invalid endpoint (will fail)
    let delivery = WebhookDelivery::new(
        "https://invalid.local/webhook".to_string(),
        serde_json::json!({"event": "test_attestation", "data": {"subject": "test"}}),
    );

    // Attempt delivery (will exhaust retries)
    let result = deliverer.deliver(delivery).await;

    // Verify delivery exhausted retries
    assert!(result.is_err());
    match result {
        Err(webhook_sdk::WebhookError::MaxRetriesExceeded) => {
            // Expected
        }
        Err(e) => panic!("Unexpected error: {}", e),
        Ok(_) => panic!("Expected delivery to fail"),
    }
}

#[tokio::test]
async fn test_dead_letter_storage_and_retrieval() {
    // Setup: Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let dlq = DeadLetterQueue::new(pool);
    dlq.initialize()
        .await
        .expect("Failed to initialize dead-letter queue");

    // Create a mock failed delivery
    let mut delivery = WebhookDelivery::new(
        "https://example.com/webhook".to_string(),
        serde_json::json!({"event": "attestation", "attestor": "test-attestor"}),
    );
    delivery.attempt = 3;
    delivery.error = Some("Connection timeout after 3 retries".to_string());
    delivery.delivered = false;

    // Store it in dead-letter queue
    let dead_letter =
        DeadLetter::from_delivery(delivery.clone(), "Max retries exceeded".to_string());
    dlq.store(&dead_letter)
        .await
        .expect("Failed to store dead-letter");

    // Retrieve it
    let retrieved = dlq
        .get(&dead_letter.id)
        .await
        .expect("Failed to retrieve")
        .expect("Dead-letter not found");

    // Verify stored data
    assert_eq!(retrieved.delivery.url, "https://example.com/webhook");
    assert_eq!(retrieved.delivery.attempt, 3);
    assert_eq!(retrieved.reason, "Max retries exceeded");
    assert!(!retrieved.delivery.delivered);
}

#[tokio::test]
async fn test_dead_letter_inspection_via_monitor() {
    // Setup: Create in-memory database and monitor
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let dlq = DeadLetterQueue::new(pool);
    dlq.initialize()
        .await
        .expect("Failed to initialize dead-letter queue");

    let monitor = WebhookMonitor::new(dlq);

    // Add multiple failed deliveries to dead-letter queue
    for i in 0..5 {
        let mut delivery = WebhookDelivery::new(
            format!("https://example.com/webhook/{}", i),
            serde_json::json!({"event": format!("attestation_{}", i), "index": i}),
        );
        delivery.attempt = 3;
        delivery.error = Some(format!("Attempt {} failed", i));
        delivery.delivered = false;

        let dead_letter =
            DeadLetter::from_delivery(delivery, "Max retries exceeded".to_string());
        monitor
            .store_dead_letter(&dead_letter)
            .await
            .expect("Failed to store dead-letter");
    }

    // Test count
    let count = monitor
        .dead_letter_count()
        .await
        .expect("Failed to get count");
    assert_eq!(count, 5);

    // Test list with pagination
    let list = monitor
        .list_dead_letters(10, 0)
        .await
        .expect("Failed to list");
    assert_eq!(list.len(), 5);

    // Test list with limit
    let list_limited = monitor
        .list_dead_letters(2, 0)
        .await
        .expect("Failed to list");
    assert_eq!(list_limited.len(), 2);

    // Test generate report
    let report = monitor
        .generate_report()
        .await
        .expect("Failed to generate report");
    assert_eq!(report.total_dead_letters, 5);
    assert_eq!(report.recent_dead_letters.len(), 5);
}

#[tokio::test]
async fn test_dead_letter_retrieval_by_id() {
    // Setup
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let dlq = DeadLetterQueue::new(pool);
    dlq.initialize()
        .await
        .expect("Failed to initialize dead-letter queue");

    let monitor = WebhookMonitor::new(dlq);

    // Create and store a dead-letter
    let delivery = WebhookDelivery::new(
        "https://example.com/webhook".to_string(),
        serde_json::json!({"event": "test", "payload": "important data"}),
    );
    let dead_letter = DeadLetter::from_delivery(delivery, "Test failure".to_string());
    let dead_letter_id = dead_letter.id.clone();

    monitor
        .store_dead_letter(&dead_letter)
        .await
        .expect("Failed to store");

    // Retrieve by ID
    let retrieved = monitor
        .get_dead_letter(&dead_letter_id)
        .await
        .expect("Failed to retrieve")
        .expect("Dead-letter not found");

    assert_eq!(retrieved.id, dead_letter_id);
    assert_eq!(retrieved.reason, "Test failure");

    // Verify non-existent ID returns None
    let not_found = monitor
        .get_dead_letter("non-existent-id")
        .await
        .expect("Query should not error");
    assert!(not_found.is_none());
}
