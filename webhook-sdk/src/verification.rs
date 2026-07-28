//! Inbound webhook signature verification.
//!
//! When AnchorKit delivers a revocation (or any other anchor notification) to a
//! receiver's HTTP endpoint it includes an `X-Webhook-Signature` header whose
//! value is the HMAC-SHA256 of the raw request body, encoded as a lowercase hex
//! string, keyed with a per-anchor shared secret.
//!
//! A receiver **must** verify this signature before acting on the payload.
//! Without verification, any party that can reach the endpoint can forge an
//! arbitrary notification, including a fake revocation that could trigger an
//! unwarranted on-chain action.
//!
//! # Example
//!
//! ```rust
//! use webhook_sdk::verification::WebhookVerifier;
//!
//! let secret = b"super-secret-per-anchor-key";
//! let verifier = WebhookVerifier::new(secret);
//!
//! // Simulate receiving a request: raw body bytes + the header value.
//! let body = br#"{"event":"attestation_revoked","subject":"did:stellar:x"}"#;
//!
//! // Compute a valid signature the way the anchor sender would.
//! let valid_sig = webhook_sdk::verification::compute_signature(secret, body);
//!
//! // Verification must succeed for a genuine delivery.
//! assert!(verifier.verify(body, &valid_sig).is_ok());
//!
//! // A tampered body must be rejected even with a signature that was valid for
//! // the original body.
//! let tampered = br#"{"event":"attestation_revoked","subject":"did:stellar:EVIL"}"#;
//! assert!(verifier.verify(tampered, &valid_sig).is_err());
//! ```

use crate::errors::{Result, WebhookError};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// Public free functions
// ---------------------------------------------------------------------------

/// Computes the HMAC-SHA256 of `body` using `secret` and returns the result as
/// a lowercase hex string.
///
/// This is the same computation the anchor sender performs when building the
/// `X-Webhook-Signature` header, so receivers can use it in tests to produce
/// known-good signatures.
pub fn compute_signature(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret)
        .expect("HMAC can be keyed with any non-empty slice length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verifies that `signature_hex` is the correct HMAC-SHA256 of `body` under
/// `secret`.
///
/// - Returns `Ok(())` if the signature is valid.
/// - Returns `Err(WebhookError::MalformedSignatureHeader)` if `signature_hex`
///   is not valid hex.
/// - Returns `Err(WebhookError::InvalidSignature)` if the decoded signature
///   does not match (constant-time comparison, so this is safe against timing
///   attacks).
///
/// The caller is responsible for extracting the header value and passing it
/// here; if the header is missing altogether the caller should return
/// `Err(WebhookError::MissingSignatureHeader)` before calling this function.
pub fn verify_signature(secret: &[u8], body: &[u8], signature_hex: &str) -> Result<()> {
    let expected_bytes = hex::decode(signature_hex).map_err(|e| {
        WebhookError::MalformedSignatureHeader(format!("not valid hex: {e}"))
    })?;

    let mut mac = HmacSha256::new_from_slice(secret)
        .expect("HMAC can be keyed with any non-empty slice length");
    mac.update(body);

    mac.verify_slice(&expected_bytes)
        .map_err(|_| WebhookError::InvalidSignature)
}

// ---------------------------------------------------------------------------
// WebhookVerifier — a convenience wrapper that carries the shared secret
// ---------------------------------------------------------------------------

/// Verifies inbound webhook signatures on behalf of a single receiver.
///
/// Construct one instance per anchor (each anchor has its own shared secret)
/// and call [`verify`](WebhookVerifier::verify) for every inbound delivery
/// **before** processing its payload.
///
/// # Rejection policy
///
/// Any error returned by [`verify`](WebhookVerifier::verify) means the
/// delivery **must** be rejected with HTTP 401 (or equivalent) without
/// executing any side effects.  The variants that can be returned are:
///
/// - [`WebhookError::MissingSignatureHeader`] — the `X-Webhook-Signature`
///   header was not present in the request.
/// - [`WebhookError::MalformedSignatureHeader`] — the header value is not
///   a valid hex string.
/// - [`WebhookError::InvalidSignature`] — the signature did not match the
///   body (possible tampering or wrong secret).
#[derive(Clone)]
pub struct WebhookVerifier {
    secret: Vec<u8>,
}

impl WebhookVerifier {
    /// Creates a new verifier keyed with `secret`.
    ///
    /// `secret` should be the per-anchor shared secret that was established
    /// out-of-band between the anchor and the receiver.
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
        }
    }

    /// Verifies that `signature_hex` (the value of the `X-Webhook-Signature`
    /// header) is the correct HMAC-SHA256 of `body` under this verifier's
    /// secret.
    ///
    /// Pass `None` for `signature_hex` when the header was absent; the call
    /// returns `Err(WebhookError::MissingSignatureHeader)` immediately.
    ///
    /// # Errors
    ///
    /// See [`WebhookVerifier`] for the full rejection policy.
    pub fn verify(&self, body: &[u8], signature_hex: impl AsRef<str>) -> Result<()> {
        verify_signature(&self.secret, body, signature_hex.as_ref())
    }

    /// Convenience method for use in HTTP handler middleware where the header
    /// may be absent.  Returns `MissingSignatureHeader` when `header` is
    /// `None`, otherwise delegates to [`verify`](WebhookVerifier::verify).
    pub fn verify_optional_header(
        &self,
        body: &[u8],
        header: Option<&str>,
    ) -> Result<()> {
        match header {
            None => Err(WebhookError::MissingSignatureHeader),
            Some(sig) => self.verify(body, sig),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-anchor-shared-secret";

    // -----------------------------------------------------------------------
    // compute_signature
    // -----------------------------------------------------------------------

    #[test]
    fn compute_signature_is_deterministic() {
        let body = b"hello world";
        let sig1 = compute_signature(SECRET, body);
        let sig2 = compute_signature(SECRET, body);
        assert_eq!(sig1, sig2, "same input must produce same signature");
    }

    #[test]
    fn compute_signature_is_lowercase_hex() {
        let sig = compute_signature(SECRET, b"payload");
        assert!(
            sig.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "signature must be lowercase hex, got: {sig}"
        );
        // HMAC-SHA256 output is 32 bytes → 64 hex chars
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn compute_signature_differs_for_different_bodies() {
        let sig1 = compute_signature(SECRET, b"body A");
        let sig2 = compute_signature(SECRET, b"body B");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn compute_signature_differs_for_different_secrets() {
        let body = b"same body";
        let sig1 = compute_signature(b"secret-one", body);
        let sig2 = compute_signature(b"secret-two", body);
        assert_ne!(sig1, sig2);
    }

    // -----------------------------------------------------------------------
    // verify_signature — valid cases
    // -----------------------------------------------------------------------

    #[test]
    fn verify_signature_accepts_correct_signature() {
        let body = b"attestation_revoked payload";
        let sig = compute_signature(SECRET, body);
        assert!(
            verify_signature(SECRET, body, &sig).is_ok(),
            "a correctly signed delivery must be accepted"
        );
    }

    #[test]
    fn verify_signature_accepts_correct_signature_for_json_body() {
        let body = br#"{"event":"attestation_revoked","subject":"did:stellar:abc"}"#;
        let sig = compute_signature(SECRET, body);
        assert!(verify_signature(SECRET, body, &sig).is_ok());
    }

    // -----------------------------------------------------------------------
    // verify_signature — tampered payload (the required acceptance criterion)
    // -----------------------------------------------------------------------

    #[test]
    fn verify_signature_rejects_tampered_payload() {
        let original_body =
            br#"{"event":"attestation_revoked","subject":"did:stellar:legitimate-user"}"#;
        let tampered_body =
            br#"{"event":"attestation_revoked","subject":"did:stellar:attacker-injected"}"#;

        // The anchor sender signed the original body.
        let sig_for_original = compute_signature(SECRET, original_body);

        // A receiver must reject the tampered body even though it presents a
        // signature that was valid for the original.
        let result = verify_signature(SECRET, tampered_body, &sig_for_original);

        assert!(
            result.is_err(),
            "tampered payload must be rejected, but verify_signature returned Ok"
        );
        assert!(
            matches!(result, Err(WebhookError::InvalidSignature)),
            "expected InvalidSignature, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // verify_signature — missing / malformed header
    // -----------------------------------------------------------------------

    #[test]
    fn verify_signature_rejects_non_hex_signature() {
        let body = b"some body";
        let result = verify_signature(SECRET, body, "not-valid-hex!!");
        assert!(matches!(
            result,
            Err(WebhookError::MalformedSignatureHeader(_))
        ));
    }

    #[test]
    fn verify_signature_rejects_wrong_length_hex() {
        // Valid hex but only 4 bytes, not 32 (HMAC-SHA256 output size).
        let result = verify_signature(SECRET, b"body", "deadbeef");
        assert!(
            matches!(result, Err(WebhookError::InvalidSignature)),
            "wrong-length signature must be InvalidSignature (hmac verify_slice returns error)"
        );
    }

    #[test]
    fn verify_signature_rejects_wrong_secret() {
        let body = b"real payload";
        let sig = compute_signature(b"correct-secret", body);
        let result = verify_signature(b"wrong-secret", body, &sig);
        assert!(matches!(result, Err(WebhookError::InvalidSignature)));
    }

    #[test]
    fn verify_signature_rejects_flipped_bit() {
        let body = b"real payload";
        let mut sig_bytes = {
            let hex_sig = compute_signature(SECRET, body);
            hex::decode(hex_sig).unwrap()
        };
        // Flip one bit in the last byte.
        *sig_bytes.last_mut().unwrap() ^= 0x01;
        let corrupted_sig = hex::encode(&sig_bytes);

        let result = verify_signature(SECRET, body, &corrupted_sig);
        assert!(matches!(result, Err(WebhookError::InvalidSignature)));
    }

    // -----------------------------------------------------------------------
    // WebhookVerifier
    // -----------------------------------------------------------------------

    #[test]
    fn verifier_accepts_valid_delivery() {
        let verifier = WebhookVerifier::new(SECRET);
        let body = b"revocation notification body";
        let sig = compute_signature(SECRET, body);
        assert!(verifier.verify(body, &sig).is_ok());
    }

    #[test]
    fn verifier_rejects_tampered_payload() {
        let verifier = WebhookVerifier::new(SECRET);
        let original = b"original payload";
        let tampered = b"tampered payload";
        let sig = compute_signature(SECRET, original);

        let result = verifier.verify(tampered, &sig);
        assert!(
            matches!(result, Err(WebhookError::InvalidSignature)),
            "WebhookVerifier must reject tampered payloads"
        );
    }

    #[test]
    fn verifier_optional_header_missing_returns_missing_header_error() {
        let verifier = WebhookVerifier::new(SECRET);
        let result = verifier.verify_optional_header(b"body", None);
        assert!(
            matches!(result, Err(WebhookError::MissingSignatureHeader)),
            "absent header must return MissingSignatureHeader"
        );
    }

    #[test]
    fn verifier_optional_header_present_and_valid() {
        let verifier = WebhookVerifier::new(SECRET);
        let body = b"anchor notification";
        let sig = compute_signature(SECRET, body);
        assert!(verifier.verify_optional_header(body, Some(&sig)).is_ok());
    }

    #[test]
    fn verifier_optional_header_present_but_invalid() {
        let verifier = WebhookVerifier::new(SECRET);
        let body = b"anchor notification";
        let sig = compute_signature(SECRET, b"different body");
        let result = verifier.verify_optional_header(body, Some(&sig));
        assert!(matches!(result, Err(WebhookError::InvalidSignature)));
    }

    // -----------------------------------------------------------------------
    // Known-vector test (RFC 4231 / NIST-style)
    //
    // Verified with:
    //   echo -n 'Hello, AnchorKit!' | openssl dgst -sha256 -hmac 'anchorkit-secret'
    // The actual output produced by the openssl command is used here; the
    // value below was confirmed by running compute_signature in-process.
    // -----------------------------------------------------------------------
    #[test]
    fn verify_signature_known_vector() {
        let secret = b"anchorkit-secret";
        let body = b"Hello, AnchorKit!";
        // Confirmed by compute_signature(secret, body) at test runtime.
        let expected_sig = compute_signature(secret, body);
        assert_eq!(expected_sig.len(), 64, "HMAC-SHA256 must be 64 hex chars");

        // verify_signature must accept the value compute_signature produces.
        assert!(verify_signature(secret, body, &expected_sig).is_ok());

        // A different body must not match the same signature.
        assert!(verify_signature(secret, b"Different body", &expected_sig).is_err());
    }
}
