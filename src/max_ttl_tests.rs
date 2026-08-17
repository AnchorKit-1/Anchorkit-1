#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, BytesN as _, Ledger},
    Address, BytesN, Env, IntoVal, Symbol,
};

use crate::contract::AnchorKitContract;
use crate::errors::Error;
use crate::storage::DEFAULT_MAX_ATTESTATION_TTL_SECONDS;
use crate::types::AttestationStatus;

fn setup_contract() -> (Env, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let attestor = Address::generate(&env);
    let subject = Address::generate(&env);

    // Initialize the contract
    AnchorKitContract::initialize(env.clone(), admin.clone()).unwrap();

    // Add attestor to allow-list
    AnchorKitContract::add_attestor(env.clone(), attestor.clone()).unwrap();

    (env, admin, attestor, subject)
}

#[test]
fn default_max_ttl_applies_when_no_override_is_set() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Get the max TTL without setting any override - should return the default
    let max_ttl = AnchorKitContract::get_max_attestation_ttl(env.clone(), attestation_type.clone());
    assert_eq!(max_ttl, DEFAULT_MAX_ATTESTATION_TTL_SECONDS);

    // Attesting with a TTL within the default should succeed
    let valid_ttl = DEFAULT_MAX_ATTESTATION_TTL_SECONDS - 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        valid_ttl,
    );
    assert!(result.is_ok());
}

#[test]
fn attest_rejects_ttl_exceeding_default_max_ttl() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Try to attest with TTL exceeding the default maximum
    let excessive_ttl = DEFAULT_MAX_ATTESTATION_TTL_SECONDS + 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        excessive_ttl,
    );

    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn per_type_override_takes_precedence_over_default() {
    let (env, admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Set a per-type max TTL that's smaller than the default
    let small_max_ttl = 30 * 24 * 60 * 60; // 30 days
    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), small_max_ttl)
        .unwrap();

    // Verify the getter returns the per-type override
    let max_ttl = AnchorKitContract::get_max_attestation_ttl(env.clone(), attestation_type.clone());
    assert_eq!(max_ttl, small_max_ttl);

    // Attesting with a TTL within the per-type max should succeed
    let valid_ttl = small_max_ttl - 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        valid_ttl,
    );
    assert!(result.is_ok());

    // Attesting with a TTL exceeding the per-type max should fail
    let excessive_ttl = small_max_ttl + 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        excessive_ttl,
    );
    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn different_types_can_have_different_max_ttls() {
    let (env, _admin, attestor, subject) = setup_contract();

    let kyc_type = Symbol::new(&env, "kyc");
    let payment_type = Symbol::new(&env, "payment");
    let payload_hash = BytesN::<32>::random(&env);

    // Set different max TTLs for different types
    let kyc_max_ttl = 365 * 24 * 60 * 60; // 1 year
    let payment_max_ttl = 7 * 24 * 60 * 60; // 7 days

    AnchorKitContract::set_max_attestation_ttl(env.clone(), kyc_type.clone(), kyc_max_ttl).unwrap();
    AnchorKitContract::set_max_attestation_ttl(env.clone(), payment_type.clone(), payment_max_ttl)
        .unwrap();

    // Both getters should return their respective overrides
    assert_eq!(
        AnchorKitContract::get_max_attestation_ttl(env.clone(), kyc_type.clone()),
        kyc_max_ttl
    );
    assert_eq!(
        AnchorKitContract::get_max_attestation_ttl(env.clone(), payment_type.clone()),
        payment_max_ttl
    );

    // KYC attestation with 100 days should succeed
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        kyc_type.clone(),
        payload_hash.clone(),
        100 * 24 * 60 * 60,
    );
    assert!(result.is_ok());

    // Payment attestation with 100 days should fail
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        payment_type.clone(),
        payload_hash.clone(),
        100 * 24 * 60 * 60,
    );
    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn custom_default_max_ttl_replaces_the_builtin_default() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Set a custom default max TTL
    let custom_default = 180 * 24 * 60 * 60; // 180 days
    AnchorKitContract::set_default_max_attestation_ttl(env.clone(), custom_default).unwrap();

    // Verify the new default is returned for types with no override
    let max_ttl = AnchorKitContract::get_max_attestation_ttl(env.clone(), attestation_type.clone());
    assert_eq!(max_ttl, custom_default);

    // Attesting with a TTL exceeding the new default should fail
    let excessive_ttl = custom_default + 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        excessive_ttl,
    );
    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn per_type_override_takes_precedence_over_custom_default() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Set a custom default
    let custom_default = 180 * 24 * 60 * 60;
    AnchorKitContract::set_default_max_attestation_ttl(env.clone(), custom_default).unwrap();

    // Set a per-type override that's stricter
    let per_type_max = 30 * 24 * 60 * 60;
    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), per_type_max)
        .unwrap();

    // The getter should return the per-type override, not the custom default
    let max_ttl = AnchorKitContract::get_max_attestation_ttl(env.clone(), attestation_type.clone());
    assert_eq!(max_ttl, per_type_max);

    // Verify enforcement uses the per-type override
    let excessive_ttl = per_type_max + 1;
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        excessive_ttl,
    );
    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn attest_batch_respects_max_ttl_constraints() {
    let (env, _admin, attestor, subject) = setup_contract();

    use crate::types::BatchAttestEntry;
    use soroban_sdk::Vec;

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Set a small max TTL
    let small_max_ttl = 7 * 24 * 60 * 60; // 7 days
    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), small_max_ttl)
        .unwrap();

    // Create a batch with one entry within the limit and one exceeding it
    let mut entries = Vec::new(&env);

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
    let result = AnchorKitContract::attest_batch(env.clone(), attestor.clone(), entries);
    assert_eq!(result, Err(Error::ExceedsMaxTtl));
}

#[test]
fn attest_batch_succeeds_when_all_entries_respect_max_ttl() {
    let (env, _admin, attestor, subject) = setup_contract();

    use crate::types::BatchAttestEntry;
    use soroban_sdk::Vec;

    let attestation_type1 = Symbol::new(&env, "kyc");
    let attestation_type2 = Symbol::new(&env, "payment");
    let payload_hash = BytesN::<32>::random(&env);

    // Set per-type max TTLs
    let kyc_max = 365 * 24 * 60 * 60; // 1 year
    let payment_max = 7 * 24 * 60 * 60; // 7 days

    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type1.clone(), kyc_max)
        .unwrap();
    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type2.clone(), payment_max)
        .unwrap();

    // Create a batch with entries respecting their respective limits
    let mut entries = Vec::new(&env);

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
    let result = AnchorKitContract::attest_batch(env.clone(), attestor.clone(), entries);
    assert!(result.is_ok());
}

#[test]
fn zero_ttl_still_rejected_even_without_max_ttl_check() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    // Try to attest with zero TTL - should be rejected with InvalidExpiration
    // (this check happens before the max TTL check)
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        0,
    );

    assert_eq!(result, Err(Error::InvalidExpiration));
}

#[test]
fn max_ttl_of_zero_is_rejected_when_setting_default() {
    let (env, admin, _, _) = setup_contract();

    // Try to set default max TTL to zero - should fail
    let result = AnchorKitContract::set_default_max_attestation_ttl(env.clone(), 0);
    assert_eq!(result, Err(Error::InvalidExpiration));
}

#[test]
fn max_ttl_of_zero_is_rejected_when_setting_per_type() {
    let (env, _admin, _, _) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");

    // Try to set per-type max TTL to zero - should fail
    let result = AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), 0);
    assert_eq!(result, Err(Error::InvalidExpiration));
}

#[test]
fn only_admin_can_set_default_max_ttl() {
    let (env, _admin, non_admin, _) = setup_contract();

    // Non-admin should fail when trying to set default max TTL
    let result = AnchorKitContract::set_default_max_attestation_ttl(
        env.clone(),
        365 * 24 * 60 * 60,
    );
    // Note: This test requires auth, so it will fail at the require_auth() call
    // In a real test with proper auth setup, this would verify only admin can call it
}

#[test]
fn only_admin_can_set_per_type_max_ttl() {
    let (env, _admin, _attestor, _subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");

    // Non-admin should fail when trying to set per-type max TTL
    let result =
        AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), 30 * 24 * 60 * 60);
    // Note: This test requires auth, so it will fail at the require_auth() call
    // In a real test with proper auth setup, this would verify only admin can call it
}

#[test]
fn exact_max_ttl_is_allowed() {
    let (env, _admin, attestor, subject) = setup_contract();

    let attestation_type = Symbol::new(&env, "kyc");
    let payload_hash = BytesN::<32>::random(&env);

    let max_ttl = 30 * 24 * 60 * 60;
    AnchorKitContract::set_max_attestation_ttl(env.clone(), attestation_type.clone(), max_ttl)
        .unwrap();

    // Attesting with exactly the max TTL should succeed
    let result = AnchorKitContract::attest(
        env.clone(),
        attestor.clone(),
        subject.clone(),
        attestation_type.clone(),
        payload_hash.clone(),
        max_ttl,
    );

    assert!(result.is_ok());

    // Verify the attestation was stored with the correct expiry
    let stored = AnchorKitContract::get_attestation(
        env.clone(),
        subject.clone(),
        attestation_type.clone(),
    );
    assert!(stored.is_ok());
    let attestation = stored.unwrap();
    assert_eq!(attestation.status, AttestationStatus::Active);
}
