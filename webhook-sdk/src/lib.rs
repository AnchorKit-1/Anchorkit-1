//! Webhook SDK for AnchorKit with retry and dead-letter queue support.
//!
//! This crate provides webhook delivery with exponential backoff retry logic
//! and a dead-letter queue for failed deliveries that can be inspected via
//! the webhook monitor tool.
//!
//! # Example
//!
//! ```ignore
//! use webhook_sdk::{
//!     retry::RetryConfig,
//!     delivery::WebhookDeliverer,
//!     dead_letter::DeadLetterQueue,
//!     monitor::WebhookMonitor,
//!     types::WebhookDelivery,
//! };
//! use sqlx::sqlite::SqlitePool;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a database connection
//!     let pool = SqlitePool::connect("sqlite::webhooks.db").await.unwrap();
//!
//!     // Initialize the dead-letter queue
//!     let dlq = DeadLetterQueue::new(pool);
//!     dlq.initialize().await.unwrap();
//!
//!     // Create a monitor for inspecting dead-letters
//!     let monitor = WebhookMonitor::new(dlq.clone());
//!
//!     // Configure retry behavior
//!     let retry_config = RetryConfig {
//!         max_retries: 5,
//!         initial_backoff: Duration::from_millis(100),
//!         max_backoff: Duration::from_secs(60),
//!         backoff_multiplier: 2.0,
//!     };
//!
//!     // Create a deliverer with retry config
//!     let deliverer = WebhookDeliverer::new(retry_config);
//!
//!     // Attempt delivery with automatic retries
//!     let delivery = WebhookDelivery::new(
//!         "https://example.com/webhook".to_string(),
//!         serde_json::json!({"event": "attestation_created"}),
//!     );
//!
//!     match deliverer.deliver(delivery).await {
//!         Ok(result) => println!("Delivered successfully: {:?}", result),
//!         Err(e) => println!("Delivery failed: {}", e),
//!     }
//!
//!     // Inspect dead-lettered deliveries
//!     let report = monitor.generate_report().await.unwrap();
//!     println!("Total dead-letters: {}", report.total_dead_letters);
//! }
//! ```

pub mod dead_letter;
pub mod delivery;
pub mod errors;
pub mod idempotency;
pub mod monitor;
pub mod processor;
pub mod retry;
pub mod types;
pub mod verification;

pub use dead_letter::DeadLetterQueue;
pub use delivery::WebhookDeliverer;
pub use errors::{Result, WebhookError};
pub use idempotency::{derive_idempotency_key, IdempotencyStore};
pub use monitor::WebhookMonitor;
pub use processor::{IdempotentWebhookProcessor, ProcessingResult};
pub use retry::RetryConfig;
pub use types::{DeadLetter, WebhookDelivery};
pub use verification::{compute_signature, verify_signature, WebhookVerifier};
