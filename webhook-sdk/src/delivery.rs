use crate::circuit_breaker::CircuitBreakerRegistry;
use crate::errors::{Result, WebhookError};
use crate::retry::RetryConfig;
use crate::types::WebhookDelivery;
use chrono::Utc;
use reqwest::Client;
use std::sync::Arc;

/// Handles webhook delivery with retry logic.
///
/// A circuit breaker can be attached with
/// [`WebhookDeliverer::with_circuit_breaker`] to short-circuit calls to a
/// destination that has been failing consistently, instead of continuing
/// to hammer it.
pub struct WebhookDeliverer {
    client: Client,
    retry_config: RetryConfig,
    circuit_breakers: Option<Arc<CircuitBreakerRegistry>>,
}

impl WebhookDeliverer {
    pub fn new(retry_config: RetryConfig) -> Self {
        Self { client: Client::new(), retry_config, circuit_breakers: None }
    }

    /// Attaches a circuit breaker, keyed per destination host, that
    /// short-circuits calls to a destination which has failed
    /// `config.failure_threshold` times in a row until `config.cooldown`
    /// has elapsed.
    pub fn with_circuit_breaker(mut self, config: crate::circuit_breaker::CircuitBreakerConfig) -> Self {
        self.circuit_breakers = Some(Arc::new(CircuitBreakerRegistry::new(config)));
        self
    }

    /// Current circuit state for `url`'s host, if a circuit breaker is
    /// attached and a call has been made against that host before.
    pub fn circuit_state_for(&self, url: &str) -> Option<crate::circuit_breaker::CircuitState> {
        let host = circuit_key(url)?;
        self.circuit_breakers.as_ref()?.state_for(&host)
    }

    /// Delivers a webhook with automatic retry-with-backoff on failure.
    /// Returns the delivery result including attempt count and any errors.
    ///
    /// If a circuit breaker is attached and the destination's circuit is
    /// open, the call is short-circuited immediately -- without touching
    /// the network and without consuming a retry -- and
    /// [`WebhookError::CircuitOpen`] is returned.
    pub async fn deliver(&self, mut delivery: WebhookDelivery) -> Result<WebhookDelivery> {
        let breaker = match (&self.circuit_breakers, circuit_key(&delivery.url)) {
            (Some(registry), Some(key)) => Some(registry.breaker_for(&key)),
            _ => None,
        };

        loop {
            delivery.attempt += 1;
            delivery.last_attempted_at = Some(Utc::now());

            let permit = match &breaker {
                Some(breaker) => match breaker.try_acquire() {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        delivery.error = Some(WebhookError::CircuitOpen.to_string());
                        return Err(WebhookError::CircuitOpen);
                    }
                },
                None => None,
            };

            match self.send(&delivery).await {
                Ok((status, response_body)) => {
                    if let Some(permit) = permit {
                        permit.success();
                    }
                    delivery.status_code = Some(status);
                    delivery.response_body = response_body;
                    delivery.delivered = true;
                    return Ok(delivery);
                }
                Err(e) => {
                    if let Some(permit) = permit {
                        permit.failure();
                    }
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
        let response = self
            .client
            .post(&delivery.url)
            .json(&delivery.payload)
            .send()
            .await?;

        let status = response.status().as_u16();
        let body = response.text().await.ok();

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

/// Derives the circuit-breaker registry key (the destination host) from a
/// delivery URL. Returns `None` for a URL that doesn't parse or has no
/// host, in which case no circuit breaker tracking is applied for that
/// call -- an unparseable URL will fail in `send()` on its own.
fn circuit_key(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(|h| h.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit_breaker::{CircuitBreakerConfig, CircuitState};
    use std::net::TcpListener as StdTcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Spawns a tiny local HTTP server that replies `500` to the first
    /// `fail_count` requests it receives and `200` to every one after that,
    /// so tests can simulate "a down anchor that later recovers" without
    /// any real network access. Returns the server's base URL, a counter of
    /// requests received so far, and the task handle.
    fn spawn_fake_server(fail_count: usize) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
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
                let count = received_clone.fetch_add(1, Ordering::SeqCst);

                // Drain (and ignore) the request so the client's write
                // doesn't block/reset on a full socket buffer.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let response_body = format!("{{\"seen\":{count}}}");
                let response = if count < fail_count {
                    format!(
                        "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        (format!("http://{addr}"), received, handle)
    }

    #[tokio::test]
    async fn test_delivery_success_on_first_attempt() {
        let (base_url, received, _server) = spawn_fake_server(0);
        let deliverer = WebhookDeliverer::new(RetryConfig::default());
        let delivery = WebhookDelivery::new(base_url, serde_json::json!({"test": "data"}));

        let result = deliverer.deliver(delivery).await.expect("delivery should succeed");

        assert_eq!(result.status_code, Some(200));
        assert_eq!(received.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_circuit_breaker_short_circuits_after_threshold_and_does_not_hit_the_network() {
        let (base_url, received, _server) = spawn_fake_server(usize::MAX); // always fails
        let deliverer = WebhookDeliverer::new(RetryConfig {
            max_retries: 1,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            backoff_multiplier: 2.0,
        })
        .with_circuit_breaker(CircuitBreakerConfig { failure_threshold: 2, cooldown: Duration::from_secs(60) });

        // Two failing deliveries trip the breaker (max_retries=1 means each
        // `deliver` call makes exactly one network attempt).
        for _ in 0..2 {
            let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
            let err = deliverer.deliver(delivery).await.unwrap_err();
            assert!(matches!(err, WebhookError::MaxRetriesExceeded));
        }
        assert_eq!(received.load(Ordering::SeqCst), 2);
        assert_eq!(deliverer.circuit_state_for(&base_url), Some(CircuitState::Open));

        // A third call must be short-circuited: no new connection reaches
        // the server, and the error is CircuitOpen rather than a delivery
        // failure.
        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
        let err = deliverer.deliver(delivery).await.unwrap_err();
        assert!(matches!(err, WebhookError::CircuitOpen));
        assert_eq!(received.load(Ordering::SeqCst), 2, "short-circuited call must not reach the network");
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_probe_recovers_after_cooldown() {
        // Server fails exactly once, then recovers -- so the half-open
        // probe (the 2nd request) succeeds.
        let (base_url, received, _server) = spawn_fake_server(1);
        let deliverer = WebhookDeliverer::new(RetryConfig {
            max_retries: 1,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            backoff_multiplier: 2.0,
        })
        .with_circuit_breaker(CircuitBreakerConfig { failure_threshold: 1, cooldown: Duration::from_millis(30) });

        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
        let err = deliverer.deliver(delivery).await.unwrap_err();
        assert!(matches!(err, WebhookError::MaxRetriesExceeded));
        assert_eq!(deliverer.circuit_state_for(&base_url), Some(CircuitState::Open));

        // Immediately retrying is short-circuited.
        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
        assert!(matches!(deliverer.deliver(delivery).await.unwrap_err(), WebhookError::CircuitOpen));
        assert_eq!(received.load(Ordering::SeqCst), 1);

        tokio::time::sleep(Duration::from_millis(50)).await;

        // After the cooldown, the next call is let through as the probe and
        // succeeds (server has recovered), closing the circuit.
        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
        let result = deliverer.deliver(delivery).await.expect("probe should succeed");
        assert_eq!(result.status_code, Some(200));
        assert_eq!(received.load(Ordering::SeqCst), 2);
        assert_eq!(deliverer.circuit_state_for(&base_url), Some(CircuitState::Closed));

        // Normal traffic resumes.
        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({}));
        deliverer.deliver(delivery).await.expect("delivery after recovery should succeed");
        assert_eq!(received.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_no_circuit_breaker_attached_behaves_as_before() {
        let (base_url, _received, _server) = spawn_fake_server(0);
        let deliverer = WebhookDeliverer::new(RetryConfig::default());
        let delivery = WebhookDelivery::new(base_url.clone(), serde_json::json!({"test": "data"}));

        deliverer.deliver(delivery).await.expect("delivery should succeed");
        assert_eq!(deliverer.circuit_state_for(&base_url), None);
    }
}
