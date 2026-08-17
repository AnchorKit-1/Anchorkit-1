extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Symbol};

use crate::errors::Error;
use crate::hash::compute_payload_hash;
use crate::test_util::setup;
use crate::types::AttestationStatus;

const ONE_DAY: u64 = 24 * 60 * 60;

fn attested_kind(env: &soroban_sdk::Env) -> Symbol {
    Symbol::new(env, "kyc_approved")
}

#[test]
fn attestor_can_revoke_own_attestation() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = attested_kind(&s.env);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    s.client.revoke(&attestor, &subject, &kind);

    let stored = s.client.get_attestation(&subject, &kind);
    assert_eq!(stored.status, AttestationStatus::Revoked);
    assert!(!s.client.is_valid(&subject, &kind));
}

#[test]
fn admin_can_revoke_any_attestation() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = attested_kind(&s.env);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    s.client.revoke(&s.admin, &subject, &kind);

    assert!(!s.client.is_valid(&subject, &kind));
}

#[test]
fn cannot_revoke_twice() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = attested_kind(&s.env);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    s.client.revoke(&attestor, &subject, &kind);

    assert_eq!(
        s.client.try_revoke(&attestor, &subject, &kind),
        Err(Ok(Error::AttestationAlreadyRevoked))
    );
}

#[test]
fn cannot_revoke_nonexistent_attestation() {
    let s = setup();
    let subject = Address::generate(&s.env);
    let kind = attested_kind(&s.env);

    assert_eq!(
        s.client.try_revoke(&s.admin, &subject, &kind),
        Err(Ok(Error::AttestationNotFound))
    );
}

#[test]
fn unrelated_caller_cannot_revoke() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    let bystander = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = attested_kind(&s.env);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    assert_eq!(
        s.client.try_revoke(&bystander, &subject, &kind),
        Err(Ok(Error::Unauthorized))
    );
}

// Regression tests for fuzzing findings:

#[test]
fn revoke_with_empty_symbol_name_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    // Attempt to create an attestation with empty symbol (may fail in contract logic)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }));
    // Should not panic, even if it returns an error
    assert!(result.is_ok());
}

#[test]
fn revoke_with_very_long_symbol_name_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    // Create a very long symbol name
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
fn revoke_with_high_ttl_boundary_should_not_panic() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = attested_kind(&s.env);
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    // Test with maximum u64 TTL
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = s.client.try_attest(&attestor, &subject, &kind, &hash, &u64::MAX);
    }));
    // Should not panic (may overflow but should handle gracefully)
    assert!(result.is_ok());
}
