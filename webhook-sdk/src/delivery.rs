use crate::errors::{Result, WebhookError};
use crate::logging::LoggingConfig;
use crate::retry::RetryConfig;
use crate::types::WebhookDelivery;
use chrono::Utc;
use reqwest::Client;
use std::collections::BTreeMap;

/// Handles webhook delivery with retry logic.
///
/// Optional request/response logging middleware can be attached with
/// [`WebhookDeliverer::with_logging`] -- secrets are redacted by default.
pub struct WebhookDeliverer {
    client: Client,
    retry_config: RetryConfig,
    logging: Option<LoggingConfig>,
}

impl WebhookDeliverer {
    pub fn new(retry_config: RetryConfig) -> Self {
        Self { client: Client::new(), retry_config, logging: None }
    }

    /// Attaches request/response logging middleware. See [`LoggingConfig`]
    /// -- redaction is on by default.
    pub fn with_logging(mut self, logging: LoggingConfig) -> Self {
        self.logging = Some(logging);
        self
    }

    /// Delivers a webhook with automatic retry-with-backoff on failure.
    /// Returns the delivery result including attempt count and any errors.
    pub async fn deliver(&self, mut delivery: WebhookDelivery) -> Result<WebhookDelivery> {
        loop {
            delivery.attempt += 1;
            delivery.last_attempted_at = Some(Utc::now());

            match self.send(&delivery).await {
                Ok((status, response_body)) => {
                    delivery.status_code = Some(status);
                    delivery.response_body = response_body;
                    delivery.delivered = true;
                    return Ok(delivery);
                }
                Err(e) => {
                    delivery.error = Some(e.to_string());

                    if !self.retry_config.should_retry(delivery.attempt) {
                        // Max retries exceeded, return delivery with error
                        return Err(WebhookError::MaxRetriesExceeded);
                    }

                    // Wait before retrying
                    let backoff = self.retry_config.backoff_duration(delivery.attempt - 1);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    async fn send(&self, delivery: &WebhookDelivery) -> Result<(u16, Option<String>)> {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        if let Some(logging) = &self.logging {
            logging.log_request("POST", &delivery.url, &headers, &delivery.payload);
        }

        let result = self.client.post(&delivery.url).json(&delivery.payload).send().await;

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                if let Some(logging) = &self.logging {
                    logging.log_response(&delivery.url, None, None, Some(&e.to_string()));
                }
                return Err(e.into());
            }
        };

        let status = response.status().as_u16();
        let body = response.text().await.ok();

        if let Some(logging) = &self.logging {
            logging.log_response(&delivery.url, Some(status), body.as_deref(), None);
        }

        if (200..300).contains(&status) {
            Ok((status, body))
        } else {
            Err(WebhookError::DeliveryFailed(format!(
                "HTTP {}: {}",
                status,
                body.unwrap_or_default()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::CapturingSink;
    use std::net::TcpListener as StdTcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Spawns a tiny local HTTP server (loopback only, no real network
    /// access needed) that always replies `200 OK` with a small JSON body,
    /// so tests can exercise a full request/response round trip
    /// deterministically.
    fn spawn_fake_server() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let std_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        std_listener.set_nonblocking(true).expect("set nonblocking");
        let addr = std_listener.local_addr().expect("local addr");
        let listener = TcpListener::from_std(std_listener).expect("tokio listener from std");
        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = received.clone();

        let handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };
                received_clone.fetch_add(1, Ordering::SeqCst);

                // Drain (and ignore) the request so the client's write
                // doesn't block/reset on a full socket buffer.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let response_body = "{\"ok\":true}";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        (format!("http://{addr}"), received, handle)
    }

    #[tokio::test]
    async fn test_delivery_success_on_first_attempt() {
        let (base_url, received, _server) = spawn_fake_server();
        let deliverer = WebhookDeliverer::new(RetryConfig::default());
        let delivery = WebhookDelivery::new(base_url, serde_json::json!({"test": "data"}));

        let result = deliverer.deliver(delivery).await.expect("delivery should succeed");

        assert_eq!(result.status_code, Some(200));
        assert_eq!(received.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_logging_middleware_redacts_secrets_in_captured_output() {
        let (base_url, _received, _server) = spawn_fake_server();
        let sink = CapturingSink::new();
        let deliverer = WebhookDeliverer::new(RetryConfig::default()).with_logging(LoggingConfig::with_sink(sink.clone()));

        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbmNob3IifQ.dGhpc19pc19ub3RfYV9yZWFsX3NpZ25hdHVyZQ";
        let delivery = WebhookDelivery::new(base_url, serde_json::json!({ "access_token": jwt, "event": "ping" }));

        deliverer.deliver(delivery).await.expect("delivery should succeed");

        let lines = sink.lines();
        assert!(lines.iter().any(|l| l.contains("request")));
        assert!(lines.iter().any(|l| l.contains("response")));
        for line in &lines {
            assert!(!line.contains(jwt), "log line leaked the JWT: {line}");
        }
        assert!(lines.iter().any(|l| l.contains("[REDACTED]")));
        assert!(lines.iter().any(|l| l.contains("ping")), "non-secret fields should still be logged");
    }

    #[tokio::test]
    async fn test_logging_middleware_off_by_default() {
        // With no `.with_logging(...)` call, delivery must behave exactly
        // as before -- nothing to assert on output, just that it still
        // works with no logging attached.
        let (base_url, _received, _server) = spawn_fake_server();
        let deliverer = WebhookDeliverer::new(RetryConfig::default());
        let delivery = WebhookDelivery::new(base_url, serde_json::json!({"test": "data"}));

        deliverer.deliver(delivery).await.expect("delivery should succeed");
    }

    #[tokio::test]
    async fn test_logging_middleware_without_redaction_logs_secrets_verbatim() {
        let (base_url, _received, _server) = spawn_fake_server();
        let sink = CapturingSink::new();
        let deliverer = WebhookDeliverer::new(RetryConfig::default())
            .with_logging(LoggingConfig::with_sink(sink.clone()).without_redaction());

        let delivery = WebhookDelivery::new(base_url, serde_json::json!({ "api_key": "plain-secret" }));
        deliverer.deliver(delivery).await.expect("delivery should succeed");

        let lines = sink.lines();
        assert!(lines.iter().any(|l| l.contains("plain-secret")));
    }
}
