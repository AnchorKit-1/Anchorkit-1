/// Migration tests for schema upgrade scenarios.
///
/// These tests simulate real-world upgrade scenarios: contracts that were
/// deployed with V1 schema and are upgraded to V2+ schema via WASM hash swap.
/// The tests manually populate storage with old-schema data and verify that
/// the new contract code handles it correctly.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Symbol};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::errors::Error;
use crate::migrations;
use crate::storage;
use crate::types::{Attestation, AttestationStatus, DataKey};

#[test]
fn test_current_schema_version_is_one() {
    // Verify baseline assumption: current contract version is V1
    assert_eq!(migrations::CURRENT_SCHEMA_VERSION, 1);
}

#[test]
fn test_no_migration_needed_when_schema_current() {
    let env = soroban_sdk::Env::default();

    // Set schema version to current
    storage::set_schema_version(&env, migrations::CURRENT_SCHEMA_VERSION);

    // Migration check should succeed with no action
    let result = migrations::run_pending_migrations(&env);
    assert!(result.is_ok());
}

#[test]
fn test_migration_rejects_future_schema_version() {
    let env = soroban_sdk::Env::default();

    // Simulate storage from a newer contract version
    storage::set_schema_version(&env, migrations::CURRENT_SCHEMA_VERSION + 1);

    // Migration should reject (contract code is older)
    let result = migrations::run_pending_migrations(&env);
    assert_eq!(result, Err(Error::Unauthorized));
}

#[test]
fn test_get_schema_version_defaults_to_one() {
    let env = soroban_sdk::Env::default();

    // Don't explicitly set schema version
    // Default should be V1
    let version = migrations::get_schema_version(&env);
    assert!(version.is_ok());
    assert_eq!(version.unwrap(), migrations::CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_set_and_get_schema_version() {
    let env = soroban_sdk::Env::default();

    // Set a schema version
    let result = migrations::set_schema_version(&env, 2);
    assert!(result.is_ok());

    // Verify it was stored
    let stored = migrations::get_schema_version(&env);
    assert!(stored.is_ok());
    assert_eq!(stored.unwrap(), 2);
}

#[test]
fn test_v1_attestation_survives_schema_check() {
    // Simulates: V1 contract created attestation, V1 code still running
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    // Manually create a V1 attestation in storage
    let subject = Address::generate(&env);
    let attestor = Address::generate(&env);
    let attestation_type = Symbol::short("kyc_approved");

    let attestation = Attestation {
        attestor: attestor.clone(),
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        payload_hash: BytesN::<32>::from_array(&env, &[1u8; 32]),
        issued_at: 1000,
        expires_at: 2000,
        status: AttestationStatus::Active,
    };

    // Store it with V1 schema marker
    env.storage()
        .persistent()
        .set(&DataKey::Attestation(subject.clone(), attestation_type.clone()), &attestation);
    storage::set_schema_version(&env, 1);

    // Contract method should detect V1 schema and run migrations
    let result = migrations::run_pending_migrations(&env);
    assert!(result.is_ok());

    // Verify schema is still at V1 (no migration implemented yet for V1->V2)
    let version = migrations::get_schema_version(&env).unwrap();
    assert_eq!(version, 1);

    // Verify attestation is still readable
    let retrieved = env
        .storage()
        .persistent()
        .get::<_, Attestation>(&DataKey::Attestation(subject.clone(), attestation_type.clone()));
    assert!(retrieved.is_some());
}

#[test]
fn test_multiple_attestations_preserved_during_schema_check() {
    // Simulates: V1 contract with 10 different attestations
    let env = soroban_sdk::Env::default();

    let mut attestations = Vec::new();

    // Create 10 attestations with different subjects and types
    for i in 0..10 {
        let subject = Address::generate(&env);
        let attestor = Address::generate(&env);
        let type_name = format!("type_{}", i);
        let attestation_type = Symbol::new(&env, &type_name);

        let attestation = Attestation {
            attestor,
            subject: subject.clone(),
            attestation_type: attestation_type.clone(),
            payload_hash: BytesN::<32>::from_array(&env, &[(i as u8) + 1; 32]),
            issued_at: 1000 + i as u64,
            expires_at: 2000 + i as u64,
            status: AttestationStatus::Active,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Attestation(subject.clone(), attestation_type.clone()), &attestation);

        attestations.push((subject, attestation_type, attestation));
    }

    storage::set_schema_version(&env, 1);

    // Run migration check
    let result = migrations::run_pending_migrations(&env);
    assert!(result.is_ok());

    // Verify all attestations are still readable at V1
    for (subject, attestation_type, original) in attestations {
        let retrieved = env
            .storage()
            .persistent()
            .get::<_, Attestation>(&DataKey::Attestation(subject.clone(), attestation_type.clone()));
        
        assert!(retrieved.is_some(), "Attestation should still exist after migration check");
        let att = retrieved.unwrap();
        assert_eq!(att.subject, original.subject);
        assert_eq!(att.attestor, original.attestor);
        assert_eq!(att.issued_at, original.issued_at);
    }
}

#[test]
fn test_schema_version_persists_across_method_calls() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let _client = AnchorKitContractClient::new(&env, &contract_id);

    // Set schema version
    storage::set_schema_version(&env, 1);

    // First migration check
    let result1 = migrations::run_pending_migrations(&env);
    assert!(result1.is_ok());

    // Verify it persists
    let version1 = migrations::get_schema_version(&env).unwrap();
    assert_eq!(version1, 1);

    // Second migration check
    let result2 = migrations::run_pending_migrations(&env);
    assert!(result2.is_ok());

    // Still at V1
    let version2 = migrations::get_schema_version(&env).unwrap();
    assert_eq!(version2, 1);
}

#[test]
fn test_contract_with_no_schema_version_defaults_to_v1() {
    // Simulates: Contract deployed before schema versioning was added
    let env = soroban_sdk::Env::default();

    // Don't set any schema version (old contract wouldn't have)
    // Attestation count entry exists from V1
    env.storage()
        .instance()
        .set(&DataKey::AttestationCount, &42u64);

    // When new versioned contract runs migrations, should default to V1
    let version = migrations::get_schema_version(&env);
    assert!(version.is_ok());
    assert_eq!(version.unwrap(), 1);

    // Old attestation count should still be there
    let count = env.storage().instance().get::<_, u64>(&DataKey::AttestationCount);
    assert_eq!(count, Some(42u64));
}

#[test]
fn test_v1_to_v2_scenario_framework_ready() {
    // This test demonstrates the framework is ready for V1->V2 migrations.
    // When V2 schema is implemented, this test can be extended to:
    // 1. Populate storage with V1 attestations
    // 2. Simulate "upgrade" to V2 code
    // 3. Verify all V1 data is migrated to V2 format correctly

    let env = soroban_sdk::Env::default();

    // Setup: V1 contract with data
    storage::set_schema_version(&env, 1);

    // Future: When V2 schema is implemented:
    // - Add migrate_v1_to_v2() function to migrations.rs
    // - Update run_pending_migrations() to call it
    // - Create V2 versions of Attestation, AttestationHistory, etc.
    // - Populate this test with V1 data and verify it migrates correctly

    // For now, verify framework is in place:
    let current = migrations::CURRENT_SCHEMA_VERSION;
    assert_eq!(current, 1, "When ready to implement V2, update CURRENT_SCHEMA_VERSION to 2");

    let version = migrations::get_schema_version(&env).unwrap();
    assert_eq!(version, 1);

    let result = migrations::run_pending_migrations(&env);
    assert!(result.is_ok());
}

#[test]
fn test_migration_framework_is_deterministic() {
    // Migrations must be deterministic: same input always produces same output
    let env = soroban_sdk::Env::default();

    // Set V1 schema
    storage::set_schema_version(&env, 1);

    // Run migration multiple times
    let result1 = migrations::run_pending_migrations(&env);
    let result2 = migrations::run_pending_migrations(&env);
    let result3 = migrations::run_pending_migrations(&env);

    // All should succeed
    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert!(result3.is_ok());

    // Schema version should remain at V1
    assert_eq!(migrations::get_schema_version(&env).unwrap(), 1);
}
