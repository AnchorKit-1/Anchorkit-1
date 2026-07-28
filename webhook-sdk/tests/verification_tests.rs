//! Integration tests for inbound webhook signature verification.
//!
//! These tests exercise `WebhookVerifier` and the free functions
//! `compute_signature` / `verify_signature` from the perspective of a receiver
//! integrating the SDK into an HTTP handler.

use webhook_sdk::{
    verification::{compute_signature, WebhookVerifier},
    WebhookError,
};

const SECRET: &[u8] = b"per-anchor-shared-secret-for-receiver";

// ---------------------------------------------------------------------------
// Happy path: genuine delivery
// ---------------------------------------------------------------------------

#[test]
fn genuine_delivery_is_accepted() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = br#"{"event":"attestation_revoked","subject":"did:stellar:user-abc","type":"kyc_approved"}"#;
    let sig = compute_signature(SECRET, body);

    assert!(
        verifier.verify(body, &sig).is_ok(),
        "a correctly signed delivery must be accepted before any processing"
    );
}

// ---------------------------------------------------------------------------
// Core acceptance criterion: tampered payload is rejected
// ---------------------------------------------------------------------------

#[test]
fn tampered_payload_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);

    // The anchor signs the original notification.
    let original_body = br#"{"event":"attestation_revoked","subject":"did:stellar:legitimate-user","attestation_type":"kyc_approved"}"#;
    let signature_for_original = compute_signature(SECRET, original_body);

    // An attacker modifies the subject field in transit (or constructs a
    // spoofed request with a valid signature copied from a prior delivery).
    let tampered_body = br#"{"event":"attestation_revoked","subject":"did:stellar:attacker-injected","attestation_type":"kyc_approved"}"#;

    let result = verifier.verify(tampered_body, &signature_for_original);

    assert!(
        result.is_err(),
        "tampered payload must NOT be accepted — verify returned Ok unexpectedly"
    );
    assert!(
        matches!(result, Err(WebhookError::InvalidSignature)),
        "expected InvalidSignature error for tampered payload, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Missing signature header — must be rejected before any processing
// ---------------------------------------------------------------------------

#[test]
fn missing_signature_header_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = b"any body";

    let result = verifier.verify_optional_header(body, None);

    assert!(
        result.is_err(),
        "absent X-Webhook-Signature header must be rejected"
    );
    assert!(
        matches!(result, Err(WebhookError::MissingSignatureHeader)),
        "expected MissingSignatureHeader, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Invalid (non-hex) signature header
// ---------------------------------------------------------------------------

#[test]
fn non_hex_signature_header_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = b"some notification body";

    let result = verifier.verify(body, "this-is-not-hex-!!!");

    assert!(result.is_err());
    assert!(
        matches!(result, Err(WebhookError::MalformedSignatureHeader(_))),
        "expected MalformedSignatureHeader, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Wrong secret — delivery signed by a different anchor (or misconfigured
// receiver) must be rejected
// ---------------------------------------------------------------------------

#[test]
fn delivery_signed_with_wrong_secret_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);

    let body = b"notification body";
    // Signed with a different anchor's secret.
    let sig = compute_signature(b"different-anchor-secret", body);

    let result = verifier.verify(body, &sig);

    assert!(
        matches!(result, Err(WebhookError::InvalidSignature)),
        "signature from the wrong secret must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Spoofed request (no signature at all, empty string)
// ---------------------------------------------------------------------------

#[test]
fn empty_signature_header_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = b"notification body";

    // An empty string is valid hex (decodes to zero bytes) but won't match the
    // 32-byte HMAC output.
    let result = verifier.verify(body, "");

    // Empty hex decodes to empty slice; verify_slice will fail on length mismatch.
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Replay: signature valid for one body re-used on a different body
// ---------------------------------------------------------------------------

#[test]
fn replayed_signature_on_different_body_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);

    let first_body = br#"{"event":"attestation_revoked","id":"rev-001"}"#;
    let second_body = br#"{"event":"attestation_revoked","id":"rev-002"}"#;

    let sig_for_first = compute_signature(SECRET, first_body);

    // Attempting to replay the first delivery's signature on a second delivery.
    let result = verifier.verify(second_body, &sig_for_first);

    assert!(
        matches!(result, Err(WebhookError::InvalidSignature)),
        "replayed signature must not be accepted for a different body"
    );
}

// ---------------------------------------------------------------------------
// verify_optional_header: present and valid
// ---------------------------------------------------------------------------

#[test]
fn present_and_valid_optional_header_is_accepted() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = b"anchor notification payload";
    let sig = compute_signature(SECRET, body);

    assert!(verifier.verify_optional_header(body, Some(&sig)).is_ok());
}

// ---------------------------------------------------------------------------
// verify_optional_header: present but invalid
// ---------------------------------------------------------------------------

#[test]
fn present_but_invalid_optional_header_is_rejected() {
    let verifier = WebhookVerifier::new(SECRET);
    let body = b"anchor notification payload";
    let wrong_sig = compute_signature(SECRET, b"completely different body");

    let result = verifier.verify_optional_header(body, Some(&wrong_sig));
    assert!(matches!(result, Err(WebhookError::InvalidSignature)));
}
