use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Symbol};

use crate::errors::Error;
use crate::hash::compute_payload_hash;
use crate::test_util::setup;
use crate::types::AttestationStatus;

const ONE_DAY: u64 = 24 * 60 * 60;

#[test]
fn history_preserved_when_attestation_overwritten() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let first_hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"first"));
    let second_hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"second"));

    // Submit first attestation
    s.client.attest(&attestor, &subject, &kind, &first_hash, &ONE_DAY);

    // Submit second attestation (overwrites in current storage)
    s.client.attest(&attestor, &subject, &kind, &second_hash, &ONE_DAY);

    // get_attestation returns the latest (second)
    let latest = s.client.get_attestation(&subject, &kind);
    assert_eq!(latest.payload_hash, second_hash);

    // But history contains both entries
    let history = s
        .client
        .list_attestation_history(&subject, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().payload_hash, first_hash);
    assert_eq!(history.get(0).unwrap().sequence, 1);
    assert_eq!(history.get(1).unwrap().payload_hash, second_hash);
    assert_eq!(history.get(1).unwrap().sequence, 2);
}

#[test]
fn revoke_creates_new_history_entry() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    // Before revoke, entry is Active
    let history1 = s
        .client
        .list_attestation_history(&subject, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(history1.len(), 1);
    assert_eq!(history1.get(0).unwrap().status, AttestationStatus::Active);

    s.client.revoke(&attestor, &subject, &kind);

    // After revoke, latest entry in current storage is Revoked
    let latest = s.client.get_attestation(&subject, &kind);
    assert_eq!(latest.status, AttestationStatus::Revoked);

    // But history now has two entries: original (Active) and revoked (Revoked)
    let history2 = s
        .client
        .list_attestation_history(&subject, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(history2.len(), 2);
    assert_eq!(history2.get(0).unwrap().status, AttestationStatus::Active);
    assert_eq!(history2.get(0).unwrap().sequence, 1);
    assert_eq!(history2.get(1).unwrap().status, AttestationStatus::Revoked);
    assert_eq!(history2.get(1).unwrap().sequence, 2);
}

#[test]
fn pagination_oldest_first() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");

    // Submit 5 attestations
    for i in 0..5 {
        let payload = format!("payload-{}", i);
        let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, payload.as_bytes()));
        s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }

    // Get first 2 entries (oldest first)
    let page1 = s
        .client
        .list_attestation_history(&subject, &kind, 1, 2, false)
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().sequence, 1);
    assert_eq!(page1.get(1).unwrap().sequence, 2);

    // Get next 2 entries starting from seq 3
    let page2 = s
        .client
        .list_attestation_history(&subject, &kind, 3, 2, false)
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().sequence, 3);
    assert_eq!(page2.get(1).unwrap().sequence, 4);

    // Get remaining entries
    let page3 = s
        .client
        .list_attestation_history(&subject, &kind, 5, 2, false)
        .unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3.get(0).unwrap().sequence, 5);
}

#[test]
fn pagination_newest_first() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");

    // Submit 5 attestations
    for i in 0..5 {
        let payload = format!("payload-{}", i);
        let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, payload.as_bytes()));
        s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }

    // Get first 2 entries from end (newest first)
    let page1 = s
        .client
        .list_attestation_history(&subject, &kind, 5, 2, true)
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().sequence, 5);
    assert_eq!(page1.get(1).unwrap().sequence, 4);

    // Get next 2 entries
    let page2 = s
        .client
        .list_attestation_history(&subject, &kind, 3, 2, true)
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().sequence, 3);
    assert_eq!(page2.get(1).unwrap().sequence, 2);

    // Get remaining
    let page3 = s
        .client
        .list_attestation_history(&subject, &kind, 1, 2, true)
        .unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3.get(0).unwrap().sequence, 1);
}

#[test]
fn empty_history_for_nonexistent_attestation() {
    let s = setup();
    let subject = Address::generate(&s.env);
    let kind = Symbol::new(&s.env, "kyc_approved");

    let history = s
        .client
        .list_attestation_history(&subject, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(history.len(), 0);
}

#[test]
fn pagination_limit_zero_fails() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    // Limit of 0 should fail
    assert_eq!(
        s.client
            .try_list_attestation_history(&subject, &kind, 1, 0, false),
        Err(Ok(Error::InvalidPagination))
    );
}

#[test]
fn pagination_start_seq_beyond_end_returns_empty() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    // Start from seq 100 when only 1 entry exists
    let history = s
        .client
        .list_attestation_history(&subject, &kind, 100, 10, false)
        .unwrap();
    assert_eq!(history.len(), 0);
}

#[test]
fn backward_compatibility_get_attestation_returns_latest() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");

    // Submit multiple attestations
    for i in 0..3 {
        let payload = format!("payload-{}", i);
        let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, payload.as_bytes()));
        s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    }

    // get_attestation should return the latest (last one submitted)
    let latest = s.client.get_attestation(&subject, &kind);
    let hash_final =
        compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload-2"));
    assert_eq!(latest.payload_hash, hash_final);
}

#[test]
fn backward_compatibility_is_valid_checks_latest() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    // Submit first attestation (Active)
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
    assert!(s.client.is_valid(&subject, &kind));

    // Revoke it (marks current storage as Revoked)
    s.client.revoke(&attestor, &subject, &kind);

    // is_valid should return false (checks the latest storage entry)
    assert!(!s.client.is_valid(&subject, &kind));

    // But history still contains the original Active entry
    let history = s
        .client
        .list_attestation_history(&subject, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().status, AttestationStatus::Active);
}

#[test]
fn history_sequence_increments_across_multiple_subjects() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject1 = Address::generate(&s.env);
    let subject2 = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));

    // Submit attestations for different subjects
    s.client.attest(&attestor, &subject1, &kind, &hash, &ONE_DAY);
    s.client.attest(&attestor, &subject2, &kind, &hash, &ONE_DAY);
    s.client.attest(&attestor, &subject1, &kind, &hash, &ONE_DAY);

    // Each (subject, type) pair has its own sequence
    let hist1 = s
        .client
        .list_attestation_history(&subject1, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(hist1.len(), 2);
    assert_eq!(hist1.get(0).unwrap().sequence, 1);
    assert_eq!(hist1.get(1).unwrap().sequence, 2);

    let hist2 = s
        .client
        .list_attestation_history(&subject2, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(hist2.len(), 1);
    assert_eq!(hist2.get(0).unwrap().sequence, 1);
}

#[test]
fn batch_attest_creates_history_entries_for_each() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject1 = Address::generate(&s.env);
    let subject2 = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash1 = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"first"));
    let hash2 = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"second"));

    let entries = soroban_sdk::Vec::from_array(
        &s.env,
        [
            crate::types::BatchAttestEntry {
                subject: subject1.clone(),
                attestation_type: kind.clone(),
                payload_hash: hash1.clone(),
                ttl_seconds: ONE_DAY,
            },
            crate::types::BatchAttestEntry {
                subject: subject2.clone(),
                attestation_type: kind.clone(),
                payload_hash: hash2.clone(),
                ttl_seconds: ONE_DAY,
            },
        ],
    );

    s.client.attest_batch(&attestor, &entries);

    // Each subject has a history entry
    let hist1 = s
        .client
        .list_attestation_history(&subject1, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(hist1.len(), 1);
    assert_eq!(hist1.get(0).unwrap().payload_hash, hash1);

    let hist2 = s
        .client
        .list_attestation_history(&subject2, &kind, 1, 10, false)
        .unwrap();
    assert_eq!(hist2.len(), 1);
    assert_eq!(hist2.get(0).unwrap().payload_hash, hash2);
}
