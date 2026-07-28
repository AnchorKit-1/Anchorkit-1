use webhook_sdk::{
    idempotency::{derive_idempotency_key, IdempotencyStore},
    processor::IdempotentWebhookProcessor,
    retry::RetryConfig,
    delivery::WebhookDeliverer,
    types::WebhookDelivery,
};
use sqlx::sqlite::SqlitePool;
use std::time::Duration;

#[tokio::test]
async fn test_idempotency_key_derives_from_payload() {
    let payload1 = serde_json::json!({
        "event": "attestation_revoked",
        "subject": "did:stellar:user123",
        "type": "kyc_approved"
    });

    let payload2 = serde_json::json!({
        "event": "attestation_revoked",
        "subject": "did:stellar:user123",
        "type": "kyc_approved"
    });

    let key1 = derive_idempotency_key(&payload1);
    let key2 = derive_idempotency_key(&payload2);

    // Same payload should produce same key
    assert_eq!(key1, key2);
}

#[tokio::test]
async fn test_idempotency_key_differs_for_different_payloads() {
    let payload1 = serde_json::json!({"event": "attestation_revoked", "id": "123"});
    let payload2 = serde_json::json!({"event": "attestation_created", "id": "123"});

    let key1 = derive_idempotency_key(&payload1);
    let key2 = derive_idempotency_key(&payload2);

    // Different payloads should produce different keys
    assert_ne!(key1, key2);
}

#[tokio::test]
async fn test_idempotency_key_is_stable_across_calls() {
    let payload = serde_json::json!({"event": "test", "data": {"nested": "value"}});

    let keys: Vec<String> = (0..10)
        .map(|_| derive_idempotency_key(&payload))
        .collect();

    // All keys should be identical
    assert!(keys.windows(2).all(|w| w[0] == w[1]));
}

#[tokio::test]
async fn test_duplicate_delivery_returns_cached_response() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let idempotency_store = IdempotencyStore::new(pool);
    idempotency_store
        .initialize()
        .await
        .expect("Failed to initialize");

    let payload = serde_json::json!({"event": "test", "id": "abc"});
    let response = serde_json::json!({"status": "processed", "result": "ok"});
    let idempotency_key = derive_idempotency_key(&payload);

    // Record first delivery
    idempotency_store
        .record(&idempotency_key, &payload, &response)
        .await
        .expect("Failed to record");

    // Retrieve cached response (simulating duplicate)
    let cached = idempotency_store
        .get(&idempotency_key)
        .await
        .expect("Failed to retrieve")
        .expect("Response should be cached");

    assert_eq!(cached, response);
}

#[tokio::test]
async fn test_duplicate_detection_works() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let store = IdempotencyStore::new(pool);
    store.initialize().await.expect("Failed to initialize");

    let payload = serde_json::json!({"event": "test"});
    let response = serde_json::json!({"status": "ok"});
    let key = derive_idempotency_key(&payload);

    // First delivery - should not be duplicate
    assert!(
        !store
            .is_duplicate(&key)
            .await
            .expect("Failed to check"),
        "First delivery should not be detected as duplicate"
    );

    // Record it
    store
        .record(&key, &payload, &response)
        .await
        .expect("Failed to record");

    // Second delivery - should be detected as duplicate
    assert!(
        store
            .is_duplicate(&key)
            .await
            .expect("Failed to check"),
        "Second delivery should be detected as duplicate"
    );
}

#[tokio::test]
async fn test_same_delivery_twice_only_triggers_once() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let idempotency_store = IdempotencyStore::new(pool);
    idempotency_store
        .initialize()
        .await
        .expect("Failed to initialize");

    let retry_config = RetryConfig {
        max_retries: 1,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(100),
        backoff_multiplier: 2.0,
    };

    let deliverer = WebhookDeliverer::new(retry_config);
    let processor = IdempotentWebhookProcessor::new(deliverer, idempotency_store);

    let payload = serde_json::json!({
        "event": "attestation_revoked",
        "attestation_id": "revoke-123"
    });

    // Derive the expected idempotency key directly from the payload — this is
    // the stable, deterministic key the processor uses internally.  We compute
    // it here so we can verify both calls map to the same key regardless of
    // whether each call returns Ok or Err (the invalid URL means both attempts
    // fail at the network layer, but the second is cached as a duplicate).
    let expected_key = derive_idempotency_key(&payload);

    // First delivery attempt (will fail — invalid domain).
    let delivery1 = WebhookDelivery::new(
        "https://invalid.example.local/webhook".to_string(),
        payload.clone(),
    );
    let result1 = processor.process(delivery1).await;

    // The first attempt is NOT a duplicate; it either returns Ok with
    // is_duplicate=false, or Err (network failure after caching the failure).
    let is_duplicate1 = result1.as_ref().map(|r| r.is_duplicate).unwrap_or(false);
    assert!(!is_duplicate1, "first delivery must not be flagged as duplicate");

    // Second delivery attempt with the same payload (different URL, same key).
    let delivery2 = WebhookDelivery::new(
        "https://another-invalid.example.local/webhook".to_string(),
        payload,
    );
    let result2 = processor.process(delivery2).await;

    // The second attempt must be detected as a duplicate (the failure response
    // was cached after the first attempt).
    let is_duplicate2 = result2.as_ref().map(|r| r.is_duplicate).unwrap_or(false);
    assert!(is_duplicate2, "second delivery with same payload must be flagged as duplicate");

    // The idempotency key exposed when the call returns Ok must equal the key
    // derived directly from the payload.
    if let Ok(ref r) = result2 {
        assert_eq!(
            r.idempotency_key, expected_key,
            "idempotency key in result must match derive_idempotency_key(payload)"
        );
    }
}

#[tokio::test]
async fn test_idempotency_store_cleanup_old_records() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let store = IdempotencyStore::new(pool);
    store.initialize().await.expect("Failed to initialize");

    let payload = serde_json::json!({"event": "test"});
    let response = serde_json::json!({"status": "ok"});
    let key = derive_idempotency_key(&payload);

    store
        .record(&key, &payload, &response)
        .await
        .expect("Failed to record");

    // Verify it exists
    assert!(
        store
            .is_duplicate(&key)
            .await
            .expect("Failed to check"),
        "Record should exist after creation"
    );

    // Cleanup with 0 day retention
    let deleted = store
        .cleanup_old_records(0)
        .await
        .expect("Failed to cleanup");

    assert_eq!(deleted, 1);

    // Verify it's gone
    assert!(
        !store
            .is_duplicate(&key)
            .await
            .expect("Failed to check"),
        "Record should be deleted after cleanup"
    );
}

#[tokio::test]
async fn test_different_urls_same_payload_are_duplicate() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create database");

    let idempotency_store = IdempotencyStore::new(pool);
    idempotency_store
        .initialize()
        .await
        .expect("Failed to initialize");

    let retry_config = RetryConfig::default();
    let deliverer = WebhookDeliverer::new(retry_config);
    let processor = IdempotentWebhookProcessor::new(deliverer, idempotency_store);

    let payload = serde_json::json!({"event": "on_chain_revocation", "id": "rev-001"});

    // Two deliveries with same payload but different URLs
    let delivery1 = WebhookDelivery::new(
        "https://service1.example.com/webhook".to_string(),
        payload.clone(),
    );

    let delivery2 = WebhookDelivery::new(
        "https://service2.example.com/webhook".to_string(),
        payload,
    );

    // Process first one (will fail with invalid URL, but that's ok)
    let result1 = processor.process(delivery1).await;
    let is_dup1 = result1.as_ref().map(|r| r.is_duplicate).unwrap_or(false);

    // Process second one with same payload
    let result2 = processor.process(delivery2).await;
    let is_dup2 = result2.as_ref().map(|r| r.is_duplicate).unwrap_or(false);

    // First should not be duplicate
    assert!(!is_dup1);
    // Second should be duplicate despite different URL
    assert!(is_dup2);
}
