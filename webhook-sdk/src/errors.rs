use thiserror::Error;

/// Errors that can occur during webhook operations.
#[derive(Error, Debug)]
pub enum WebhookError {
    #[error("Delivery failed: {0}")]
    DeliveryFailed(String),

    #[error("Max retries exceeded")]
    MaxRetriesExceeded,

    #[error("Invalid webhook URL: {0}")]
    InvalidWebhookUrl(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    /// The `X-Webhook-Signature` header was absent from the inbound request.
    #[error("Missing signature header")]
    MissingSignatureHeader,

    /// The `X-Webhook-Signature` header was present but could not be decoded
    /// (e.g. not valid hex, wrong length).
    #[error("Malformed signature header: {0}")]
    MalformedSignatureHeader(String),

    /// The decoded signature did not match the HMAC-SHA256 of the request body
    /// computed with the shared secret.  The delivery is rejected.
    #[error("Invalid signature: payload may have been tampered with")]
    InvalidSignature,
}

pub type Result<T> = std::result::Result<T, WebhookError>;
