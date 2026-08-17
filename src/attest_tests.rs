extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Bytes, BytesN, Symbol};

use crate::errors::Error;
use crate::hash::compute_payload_hash;
use crate::test_util::setup;
use crate::types::AttestationStatus;

const ONE_DAY: u64 = 24 * 60 * 60;

#[test]
fn attest_stores_attestation_and_is_valid() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let payload = Bytes::from_slice(&s.env, b"kyc-decision:approved");
    let hash = compute_payload_hash(&s.env, &payload);

    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    assert!(s.client.is_valid(&subject, &kind));
    let stored = s.client.get_attestation(&subject, &kind);
    assert_eq!(stored.attestor, attestor);
    assert_eq!(stored.subject, subject);
    assert_eq!(stored.payload_hash, hash);
    assert_eq!(stored.status, AttestationStatus::Active);
    assert_eq!(s.client.get_attestation_count(), 1);
}

#[test]
fn attest_fails_for_unregistered_attestor() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    assert_eq!(
        s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY),
        Err(Ok(Error::AttestorNotRegistered))
    );
}

#[test]
fn attest_fails_when_paused() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    s.client.pause();

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    assert_eq!(
        s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY),
        Err(Ok(Error::ContractPaused))
    );
}

#[test]
fn attest_fails_with_zero_ttl() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    assert_eq!(
        s.client.try_attest(&attestor, &subject, &kind, &hash, &0),
        Err(Ok(Error::InvalidExpiration))
    );
}

#[test]
fn get_attestation_returns_not_found_for_missing() {
    let s = setup();
    let subject = Address::generate(&s.env);
    let kind = Symbol::new(&s.env, "kyc_approved");
    assert_eq!(
        s.client.try_get_attestation(&subject, &kind),
        Err(Ok(Error::AttestationNotFound))
    );
    assert!(!s.client.is_valid(&subject, &kind));
}

#[test]
fn is_valid_false_after_expiry() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "payment_confirmed");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    assert!(s.client.is_valid(&subject, &kind));

    let now = s.env.ledger().timestamp();
    s.env.ledger().set_timestamp(now + ONE_DAY + 1);

    assert!(!s.client.is_valid(&subject, &kind));
}

#[test]
fn is_valid_false_exactly_at_expires_at() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "payment_confirmed");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    // expires_at is exclusive: at the exact expiry timestamp the
    // attestation must already read as invalid, not one second later.
    let stored = s.client.get_attestation(&subject, &kind);
    s.env.ledger().set_timestamp(stored.expires_at);

    assert!(!s.client.is_valid(&subject, &kind));
}

#[test]
fn a_new_attest_call_overwrites_the_prior_attestation_for_the_same_type() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let first_hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"first"));
    let second_hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"second"));

    s.client.attest(&attestor, &subject, &kind, &first_hash, &ONE_DAY);
    s.client.attest(&attestor, &subject, &kind, &second_hash, &ONE_DAY);

    let stored = s.client.get_attestation(&subject, &kind);
    assert_eq!(stored.payload_hash, second_hash);
    assert_eq!(s.client.get_attestation_count(), 2);
}

#[test]
fn unauthenticated_attest_fails() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    s.env.set_auths(&[]);
    let result = s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    assert!(result.is_err());
}

// Regression tests for fuzzing findings:

#[test]
fn attest_with_empty_symbol_name_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    // Attempt with empty symbol name (may fail in contract logic, but should not panic)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }));
    // Should not panic
    assert!(result.is_ok());
}

#[test]
fn attest_with_very_long_symbol_name_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    // Create a very long symbol name (exceeds typical limits)
    let long_name = "x".repeat(1000);
    let kind = Symbol::new(&s.env, &long_name);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    // Attempt with overly long symbol
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }));
    // Should not panic
    assert!(result.is_ok());
}

#[test]
fn attest_with_u64_max_ttl_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    // Test with maximum u64 value for TTL (may overflow timestamp calculation)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &u64::MAX);
    }));
    // Should not panic (saturating_add should handle overflow gracefully)
    assert!(result.is_ok());
}

#[test]
fn attest_with_malformed_payload_hash_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    // Create various malformed hash values
    let mut hash_bytes = [0u8; 32];
    // Fill with pattern that might trigger issues
    for i in 0..32 {
        hash_bytes[i] = (i as u8).wrapping_mul(0xFF);
    }
    let hash = BytesN::from_array(&s.env, &hash_bytes);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }));
    // Should not panic
    assert!(result.is_ok());
}
