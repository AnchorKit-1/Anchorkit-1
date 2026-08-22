//! Secret-redacting request/response logging middleware for the transport
//! layer.
//!
//! Request/response logging is invaluable for debugging anchor
//! integrations, but naive logging risks writing `Authorization` headers,
//! JWTs, or other secrets straight into wherever the logs end up -- a file,
//! a log aggregator, a support ticket. This module lets [`WebhookDeliverer`]
//! log every outbound request and its response, with redaction **on by
//! default**. An SDK that defaults to full, unredacted logging is a
//! security foot-gun waiting to happen for whoever integrates it, so
//! redaction must be turned off explicitly rather than turned on.
//!
//! [`WebhookDeliverer`]: crate::delivery::WebhookDeliverer
//!
//! Two things get redacted:
//! - Any header named `Authorization` (case-insensitive), regardless of
//!   its value.
//! - Any JSON string value -- at any depth, under any field name -- that is
//!   shaped like a JWT (`header.payload.signature`, each segment valid
//!   base64url). Field names commonly used for secrets (`token`,
//!   `api_key`, `secret`, `password`, ...) are also redacted even when the
//!   value doesn't happen to look like a JWT.

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The string substituted for anything redacted.
pub const REDACTED: &str = "[REDACTED]";

/// Header names that always get their value replaced, regardless of shape.
const SENSITIVE_HEADER_NAMES: &[&str] = &["authorization", "cookie", "set-cookie", "x-api-key"];

/// JSON object key names that always get their value replaced, regardless
/// of shape -- covers plain API keys and other secrets that aren't
/// JWT-shaped.
const SENSITIVE_KEY_NAMES: &[&str] = &[
    "authorization",
    "token",
    "access_token",
    "refresh_token",
    "api_key",
    "apikey",
    "secret",
    "password",
    "jwt",
    "client_secret",
];

/// Destination for emitted log lines.
///
/// The default is [`StderrSink`]. Tests -- and callers who want to ship
/// these logs somewhere other than stderr -- can inject their own.
pub trait LogSink: Send + Sync {
    fn write_line(&self, line: &str);
}

/// Writes one line per record to stderr.
#[derive(Debug, Default, Clone, Copy)]
pub struct StderrSink;

impl LogSink for StderrSink {
    fn write_line(&self, line: &str) {
        eprintln!("{line}");
    }
}

/// Collects log lines in memory instead of writing them anywhere.
///
/// Useful in tests that need to assert on what was (or wasn't) logged, and
/// for callers who want to buffer output before forwarding it elsewhere.
#[derive(Debug, Default, Clone)]
pub struct CapturingSink {
    lines: Arc<Mutex<Vec<String>>>,
}

impl CapturingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns every line captured so far, in emission order.
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl LogSink for CapturingSink {
    fn write_line(&self, line: &str) {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.to_string());
    }
}

/// Configuration for request/response logging on [`WebhookDeliverer`].
///
/// [`WebhookDeliverer`]: crate::delivery::WebhookDeliverer
#[derive(Clone)]
pub struct LoggingConfig {
    redact: bool,
    sink: Arc<dyn LogSink>,
}

impl Default for LoggingConfig {
    /// Logging to stderr, with redaction on.
    fn default() -> Self {
        Self { redact: true, sink: Arc::new(StderrSink) }
    }
}

impl LoggingConfig {
    /// Logging with redaction on (the safe default), writing to stderr.
    pub fn new() -> Self {
        Self::default()
    }

    /// Same as [`LoggingConfig::new`] but writes to `sink` instead of
    /// stderr.
    pub fn with_sink(sink: impl LogSink + 'static) -> Self {
        Self { redact: true, sink: Arc::new(sink) }
    }

    /// Turns off redaction, so headers and bodies are logged verbatim.
    ///
    /// This must be called explicitly -- there is no constructor that
    /// starts out unredacted -- so that disabling redaction always shows up
    /// as a deliberate line in a diff rather than being the silent default.
    pub fn without_redaction(mut self) -> Self {
        self.redact = false;
        self
    }

    pub fn is_redacting(&self) -> bool {
        self.redact
    }

    pub(crate) fn log_request(&self, method: &str, url: &str, headers: &BTreeMap<String, String>, body: &Value) {
        let (headers, body) = if self.redact {
            (redact_headers(headers), redact_body(body))
        } else {
            (headers.clone(), body.clone())
        };
        self.sink.write_line(&format!(
            "[webhook-sdk] request method={method} url={url} headers={} body={}",
            serde_json::to_string(&headers).unwrap_or_default(),
            serde_json::to_string(&body).unwrap_or_default(),
        ));
    }

    pub(crate) fn log_response(&self, url: &str, status: Option<u16>, body: Option<&str>, error: Option<&str>) {
        let logged_body = body.map(|b| match serde_json::from_str::<Value>(b) {
            Ok(parsed) => {
                let redacted = if self.redact { redact_body(&parsed) } else { parsed };
                serde_json::to_string(&redacted).unwrap_or_default()
            }
            // Not JSON -- log as an opaque string, redacted if it happens to
            // be JWT-shaped on its own (e.g. a bare token as the whole body).
            Err(_) => {
                if self.redact && is_jwt_shaped(b) {
                    REDACTED.to_string()
                } else {
                    b.to_string()
                }
            }
        });
        self.sink.write_line(&format!(
            "[webhook-sdk] response url={url} status={} body={} error={}",
            status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string()),
            logged_body.unwrap_or_else(|| "-".to_string()),
            error.unwrap_or("-"),
        ));
    }
}

/// Redacts sensitive header values.
///
/// A header is redacted if its name (case-insensitive) is one of the known
/// sensitive names (`Authorization`, `Cookie`, ...), or if its value is
/// JWT-shaped regardless of the header name.
pub fn redact_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let redact = SENSITIVE_HEADER_NAMES.contains(&name.to_ascii_lowercase().as_str())
                || is_jwt_shaped(value);
            (name.clone(), if redact { REDACTED.to_string() } else { value.clone() })
        })
        .collect()
}

/// Recursively redacts sensitive values from a JSON body.
///
/// A string value is redacted if it is JWT-shaped, or if the object key it
/// is stored under is a known sensitive name (case-insensitive). Objects
/// and arrays are walked recursively; every other value is left untouched.
pub fn redact_body(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, v)| {
                    let redacted = if SENSITIVE_KEY_NAMES.contains(&key.to_ascii_lowercase().as_str()) {
                        redact_scalar_or_recurse(v)
                    } else {
                        redact_body(v)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_body).collect()),
        Value::String(s) if is_jwt_shaped(s) => Value::String(REDACTED.to_string()),
        other => other.clone(),
    }
}

/// For a value stored under a sensitive key: redact it outright if it's a
/// scalar (string/number/bool), but keep recursing into objects/arrays so
/// e.g. `{"secret": {"nested": "..."}}` doesn't just get blanket-redacted
/// away without a chance to find further JWT-shaped fields inside it.
fn redact_scalar_or_recurse(value: &Value) -> Value {
    match value {
        Value::Object(_) | Value::Array(_) => redact_body(value),
        Value::Null => Value::Null,
        _ => Value::String(REDACTED.to_string()),
    }
}

/// A JWT is three base64url segments (header, payload, signature) joined by
/// dots. This is a shape check, not a signature/claims validation -- it's
/// meant to catch "this looks like a bearer token" for redaction purposes,
/// not to validate tokens.
pub fn is_jwt_shaped(s: &str) -> bool {
    let segments: Vec<&str> = s.split('.').collect();
    segments.len() == 3
        && segments.iter().all(|seg| {
            !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE_JWT: &str =
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbmNob3IifQ.dGhpc19pc19ub3RfYV9yZWFsX3NpZ25hdHVyZQ";

    #[test]
    fn is_jwt_shaped_accepts_real_jwt_shape() {
        assert!(is_jwt_shaped(SAMPLE_JWT));
    }

    #[test]
    fn is_jwt_shaped_rejects_non_jwt_strings() {
        assert!(!is_jwt_shaped("hello world"));
        assert!(!is_jwt_shaped("a.b"));
        assert!(!is_jwt_shaped("a.b.c.d"));
        assert!(!is_jwt_shaped(""));
        assert!(!is_jwt_shaped("a..c"));
    }

    #[test]
    fn redact_headers_redacts_authorization_case_insensitively() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer sekret-value".to_string());
        headers.insert("authorization".to_string(), "Bearer sekret-value".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let redacted = redact_headers(&headers);

        assert_eq!(redacted["Authorization"], REDACTED);
        assert_eq!(redacted["authorization"], REDACTED);
        assert_eq!(redacted["Content-Type"], "application/json");
    }

    #[test]
    fn redact_headers_redacts_jwt_shaped_value_under_any_header_name() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Auth-Token".to_string(), SAMPLE_JWT.to_string());

        let redacted = redact_headers(&headers);

        assert_eq!(redacted["X-Auth-Token"], REDACTED);
    }

    #[test]
    fn redact_body_redacts_jwt_shaped_fields_at_any_depth() {
        let body = json!({
            "event": "attestation_revoked",
            "auth": { "access_token": SAMPLE_JWT },
            "nested": { "list": [ { "id_token": SAMPLE_JWT } ] }
        });

        let redacted = redact_body(&body);

        assert_eq!(redacted["auth"]["access_token"], REDACTED);
        assert_eq!(redacted["nested"]["list"][0]["id_token"], REDACTED);
        assert_eq!(redacted["event"], "attestation_revoked");
    }

    #[test]
    fn redact_body_redacts_sensitive_key_names_even_when_not_jwt_shaped() {
        let body = json!({ "api_key": "plain-string-secret", "password": "hunter2" });

        let redacted = redact_body(&body);

        assert_eq!(redacted["api_key"], REDACTED);
        assert_eq!(redacted["password"], REDACTED);
    }

    #[test]
    fn redact_body_leaves_non_sensitive_fields_untouched() {
        let body = json!({ "event": "attestation_created", "subject": "did:stellar:x", "count": 3 });

        let redacted = redact_body(&body);

        assert_eq!(redacted, body);
    }

    #[test]
    fn log_request_with_redaction_never_contains_the_secret() {
        let sink = CapturingSink::new();
        let config = LoggingConfig::with_sink(sink.clone());

        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer top-secret".to_string());
        let body = json!({ "access_token": SAMPLE_JWT, "event": "ping" });

        config.log_request("POST", "https://anchor.example/webhook", &headers, &body);

        let lines = sink.lines();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("top-secret"));
        assert!(!lines[0].contains(SAMPLE_JWT));
        assert!(lines[0].contains(REDACTED));
        assert!(lines[0].contains("ping"));
    }

    #[test]
    fn log_request_without_redaction_logs_verbatim() {
        let sink = CapturingSink::new();
        let config = LoggingConfig::with_sink(sink.clone()).without_redaction();

        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer top-secret".to_string());
        let body = json!({ "access_token": SAMPLE_JWT });

        config.log_request("POST", "https://anchor.example/webhook", &headers, &body);

        let lines = sink.lines();
        assert!(lines[0].contains("top-secret"));
        assert!(lines[0].contains(SAMPLE_JWT));
    }

    #[test]
    fn log_response_redacts_jwt_in_json_body() {
        let sink = CapturingSink::new();
        let config = LoggingConfig::with_sink(sink.clone());

        let body = format!("{{\"refresh_token\":\"{SAMPLE_JWT}\"}}");
        config.log_response("https://anchor.example/webhook", Some(200), Some(&body), None);

        let lines = sink.lines();
        assert!(!lines[0].contains(SAMPLE_JWT));
        assert!(lines[0].contains(REDACTED));
    }

    #[test]
    fn log_response_redacts_bare_jwt_body() {
        let sink = CapturingSink::new();
        let config = LoggingConfig::with_sink(sink.clone());

        config.log_response("https://anchor.example/webhook", Some(200), Some(SAMPLE_JWT), None);

        let lines = sink.lines();
        assert!(!lines[0].contains(SAMPLE_JWT));
        assert!(lines[0].contains(REDACTED));
    }

    #[test]
    fn default_logging_config_redacts() {
        assert!(LoggingConfig::default().is_redacting());
        assert!(LoggingConfig::new().is_redacting());
    }
}
