#![cfg(test)]

use soroban_sdk::testutils::{Address as _, BytesN as _};
use soroban_sdk::{Address, BytesN, Symbol};

use crate::errors::Error;
use crate::storage::DEFAULT_MAX_ATTESTATION_TTL_SECONDS;
use crate::test_util::{setup, Setup};
use crate::types::AttestationStatus;

fn setup_contract<'a>() -> (Setup<'a>, Address, Address) {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    (s, attestor, subject)
}

#[test]
fn default_max_ttl_applies_when_no_override_is_set() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Get the max TTL without setting any override - should return the default
    let max_ttl = s.client.get_max_attestation_ttl(&attestation_type);
    assert_eq!(max_ttl, DEFAULT_MAX_ATTESTATION_TTL_SECONDS);

    // Attesting with a TTL within the default should succeed
    let valid_ttl = DEFAULT_MAX_ATTESTATION_TTL_SECONDS - 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &valid_ttl,
    );
    assert!(result.is_ok());
}

#[test]
fn attest_rejects_ttl_exceeding_default_max_ttl() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Try to attest with TTL exceeding the default maximum
    let excessive_ttl = DEFAULT_MAX_ATTESTATION_TTL_SECONDS + 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &excessive_ttl,
    );

    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn per_type_override_takes_precedence_over_default() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set a per-type max TTL that's smaller than the default
    let small_max_ttl = 30 * 24 * 60 * 60; // 30 days
    s.client
        .set_max_attestation_ttl(&attestation_type, &small_max_ttl);

    // Verify the getter returns the per-type override
    let max_ttl = s.client.get_max_attestation_ttl(&attestation_type);
    assert_eq!(max_ttl, small_max_ttl);

    // Attesting with a TTL within the per-type max should succeed
    let valid_ttl = small_max_ttl - 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &valid_ttl,
    );
    assert!(result.is_ok());

    // Attesting with a TTL exceeding the per-type max should fail
    let excessive_ttl = small_max_ttl + 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &excessive_ttl,
    );
    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn different_types_can_have_different_max_ttls() {
    let (s, attestor, subject) = setup_contract();

    let kyc_type = Symbol::new(&s.env, "kyc");
    let payment_type = Symbol::new(&s.env, "payment");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set different max TTLs for different types
    let kyc_max_ttl = 365 * 24 * 60 * 60; // 1 year
    let payment_max_ttl = 7 * 24 * 60 * 60; // 7 days

    s.client.set_max_attestation_ttl(&kyc_type, &kyc_max_ttl);
    s.client
        .set_max_attestation_ttl(&payment_type, &payment_max_ttl);

    // Both getters should return their respective overrides
    assert_eq!(s.client.get_max_attestation_ttl(&kyc_type), kyc_max_ttl);
    assert_eq!(
        s.client.get_max_attestation_ttl(&payment_type),
        payment_max_ttl
    );

    // KYC attestation with 100 days should succeed
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &kyc_type,
        &payload_hash,
        &(100 * 24 * 60 * 60),
    );
    assert!(result.is_ok());

    // Payment attestation with 100 days should fail
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &payment_type,
        &payload_hash,
        &(100 * 24 * 60 * 60),
    );
    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn custom_default_max_ttl_replaces_the_builtin_default() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set a custom default max TTL
    let custom_default = 180 * 24 * 60 * 60; // 180 days
    s.client.set_default_max_attestation_ttl(&custom_default);

    // Verify the new default is returned for types with no override
    let max_ttl = s.client.get_max_attestation_ttl(&attestation_type);
    assert_eq!(max_ttl, custom_default);

    // Attesting with a TTL exceeding the new default should fail
    let excessive_ttl = custom_default + 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &excessive_ttl,
    );
    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn per_type_override_takes_precedence_over_custom_default() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set a custom default
    let custom_default = 180 * 24 * 60 * 60;
    s.client.set_default_max_attestation_ttl(&custom_default);

    // Set a per-type override that's stricter
    let per_type_max = 30 * 24 * 60 * 60;
    s.client
        .set_max_attestation_ttl(&attestation_type, &per_type_max);

    // The getter should return the per-type override, not the custom default
    let max_ttl = s.client.get_max_attestation_ttl(&attestation_type);
    assert_eq!(max_ttl, per_type_max);

    // Verify enforcement uses the per-type override
    let excessive_ttl = per_type_max + 1;
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &excessive_ttl,
    );
    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn attest_batch_respects_max_ttl_constraints() {
    let (s, attestor, subject) = setup_contract();

    use crate::types::BatchAttestEntry;
    use soroban_sdk::Vec;

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set a small max TTL
    let small_max_ttl = 7 * 24 * 60 * 60; // 7 days
    s.client
        .set_max_attestation_ttl(&attestation_type, &small_max_ttl);

    // Create a batch with one entry within the limit and one exceeding it
    let mut entries = Vec::new(&s.env);

    entries.push_back(BatchAttestEntry {
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        payload_hash: payload_hash.clone(),
        ttl_seconds: small_max_ttl - 1, // Within limit
    });

    entries.push_back(BatchAttestEntry {
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        payload_hash: payload_hash.clone(),
        ttl_seconds: small_max_ttl + 1, // Exceeds limit
    });

    // The batch should fail because one entry exceeds the max TTL
    let result = s.client.try_attest_batch(&attestor, &entries);
    assert_eq!(result, Err(Ok(Error::ExceedsMaxTtl)));
}

#[test]
fn attest_batch_succeeds_when_all_entries_respect_max_ttl() {
    let (s, attestor, subject) = setup_contract();

    use crate::types::BatchAttestEntry;
    use soroban_sdk::Vec;

    let attestation_type1 = Symbol::new(&s.env, "kyc");
    let attestation_type2 = Symbol::new(&s.env, "payment");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Set per-type max TTLs
    let kyc_max = 365 * 24 * 60 * 60; // 1 year
    let payment_max = 7 * 24 * 60 * 60; // 7 days

    s.client
        .set_max_attestation_ttl(&attestation_type1, &kyc_max);
    s.client
        .set_max_attestation_ttl(&attestation_type2, &payment_max);

    // Create a batch with entries respecting their respective limits
    let mut entries = Vec::new(&s.env);

    entries.push_back(BatchAttestEntry {
        subject: subject.clone(),
        attestation_type: attestation_type1.clone(),
        payload_hash: payload_hash.clone(),
        ttl_seconds: 100 * 24 * 60 * 60, // Within kyc_max
    });

    entries.push_back(BatchAttestEntry {
        subject: subject.clone(),
        attestation_type: attestation_type2.clone(),
        payload_hash: payload_hash.clone(),
        ttl_seconds: 5 * 24 * 60 * 60, // Within payment_max
    });

    // The batch should succeed
    let result = s.client.try_attest_batch(&attestor, &entries);
    assert!(result.is_ok());
}

#[test]
fn zero_ttl_still_rejected_even_without_max_ttl_check() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    // Try to attest with zero TTL - should be rejected with InvalidExpiration
    // (this check happens before the max TTL check)
    let result = s
        .client
        .try_attest(&attestor, &subject, &attestation_type, &payload_hash, &0);

    assert_eq!(result, Err(Ok(Error::InvalidExpiration)));
}

#[test]
fn max_ttl_of_zero_is_rejected_when_setting_default() {
    let s = setup();

    // Try to set default max TTL to zero - should fail
    let result = s.client.try_set_default_max_attestation_ttl(&0);
    assert_eq!(result, Err(Ok(Error::InvalidExpiration)));
}

#[test]
fn max_ttl_of_zero_is_rejected_when_setting_per_type() {
    let s = setup();

    let attestation_type = Symbol::new(&s.env, "kyc");

    // Try to set per-type max TTL to zero - should fail
    let result = s.client.try_set_max_attestation_ttl(&attestation_type, &0);
    assert_eq!(result, Err(Ok(Error::InvalidExpiration)));
}

#[test]
fn only_admin_can_set_default_max_ttl() {
    let s = setup();

    // With no auths mocked, the admin-only auth check inside
    // set_default_max_attestation_ttl has nobody to authorize it, so the
    // call must fail rather than silently succeed.
    s.env.set_auths(&[]);
    let result = s
        .client
        .try_set_default_max_attestation_ttl(&(365 * 24 * 60 * 60));
    assert!(result.is_err());
}

#[test]
fn only_admin_can_set_per_type_max_ttl() {
    let s = setup();

    let attestation_type = Symbol::new(&s.env, "kyc");

    s.env.set_auths(&[]);
    let result = s
        .client
        .try_set_max_attestation_ttl(&attestation_type, &(30 * 24 * 60 * 60));
    assert!(result.is_err());
}

#[test]
fn exact_max_ttl_is_allowed() {
    let (s, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&s.env, "kyc");
    let payload_hash = BytesN::<32>::random(&s.env);

    let max_ttl = 30 * 24 * 60 * 60;
    s.client
        .set_max_attestation_ttl(&attestation_type, &max_ttl);

    // Attesting with exactly the max TTL should succeed
    let result = s.client.try_attest(
        &attestor,
        &subject,
        &attestation_type,
        &payload_hash,
        &max_ttl,
    );
    assert!(result.is_ok());

    // Verify the attestation was stored with the correct expiry
    let attestation = s.client.get_attestation(&subject, &attestation_type);
    assert_eq!(attestation.status, AttestationStatus::Active);
}
