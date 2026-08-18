use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Bytes, Symbol, Vec};

use crate::errors::Error;
use crate::hash::compute_payload_hash;
use crate::test_util::{setup, Setup};
use crate::types::BatchAttestEntry;

const ONE_DAY: u64 = 24 * 60 * 60;

fn entry(
    s: &Setup<'_>,
    subject: &Address,
    kind_name: &str,
    payload: &[u8],
    ttl_seconds: u64,
) -> BatchAttestEntry {
    BatchAttestEntry {
        subject: subject.clone(),
        attestation_type: Symbol::new(&s.env, kind_name),
        payload_hash: compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, payload)),
        ttl_seconds,
    }
}

#[test]
fn attest_batch_stores_every_entry() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let subject_a = Address::generate(&s.env);
    let subject_b = Address::generate(&s.env);
    let subject_c = Address::generate(&s.env);
    let entries = Vec::from_array(
        &s.env,
        [
            entry(&s, &subject_a, "kyc_approved", b"a", ONE_DAY),
            entry(&s, &subject_b, "kyc_approved", b"b", ONE_DAY),
            entry(&s, &subject_c, "payment_confirmed", b"c", ONE_DAY),
        ],
    );

    s.client.attest_batch(&attestor, &entries);

    let kyc = Symbol::new(&s.env, "kyc_approved");
    let payment = Symbol::new(&s.env, "payment_confirmed");
    assert!(s.client.is_valid(&subject_a, &kyc));
    assert!(s.client.is_valid(&subject_b, &kyc));
    assert!(s.client.is_valid(&subject_c, &payment));
    assert_eq!(
        s.client.get_attestation(&subject_a, &kyc).attestor,
        attestor
    );
    assert_eq!(s.client.get_attestation_count(), 3);
}

#[test]
fn attest_batch_emits_one_event_per_entry() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let subject_a = Address::generate(&s.env);
    let subject_b = Address::generate(&s.env);
    let entries = Vec::from_array(
        &s.env,
        [
            entry(&s, &subject_a, "kyc_approved", b"a", ONE_DAY),
            entry(&s, &subject_b, "kyc_approved", b"b", ONE_DAY),
        ],
    );

    s.client.attest_batch(&attestor, &entries);

    assert_eq!(s.env.events().all().events().len(), 2);
}

#[test]
fn invalid_entry_fails_the_whole_batch() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let good_subject = Address::generate(&s.env);
    let bad_subject = Address::generate(&s.env);
    let entries = Vec::from_array(
        &s.env,
        [
            entry(&s, &good_subject, "kyc_approved", b"a", ONE_DAY),
            entry(&s, &bad_subject, "kyc_approved", b"b", 0),
        ],
    );

    assert_eq!(
        s.client.try_attest_batch(&attestor, &entries),
        Err(Ok(Error::InvalidExpiration))
    );

    // Nothing from the batch should have been stored, including the entry
    // that came before the invalid one.
    let kind = Symbol::new(&s.env, "kyc_approved");
    assert_eq!(
        s.client.try_get_attestation(&good_subject, &kind),
        Err(Ok(Error::AttestationNotFound))
    );
    assert_eq!(s.client.get_attestation_count(), 0);
}

#[test]
fn oversized_attestation_type_fails_the_whole_batch() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let good_subject = Address::generate(&s.env);
    let bad_subject = Address::generate(&s.env);
    let entries = Vec::from_array(
        &s.env,
        [
            entry(&s, &good_subject, "kyc_approved", b"a", ONE_DAY),
            entry(
                &s,
                &bad_subject,
                "xxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                b"b",
                ONE_DAY,
            ),
        ],
    );

    assert_eq!(
        s.client.try_attest_batch(&attestor, &entries),
        Err(Ok(Error::AttestationTypeTooLong))
    );

    // Nothing from the batch should have been stored, including the entry
    // that came before the invalid one.
    let kind = Symbol::new(&s.env, "kyc_approved");
    assert_eq!(
        s.client.try_get_attestation(&good_subject, &kind),
        Err(Ok(Error::AttestationNotFound))
    );
    assert_eq!(s.client.get_attestation_count(), 0);
}

#[test]
fn empty_batch_is_rejected() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let entries: Vec<BatchAttestEntry> = Vec::new(&s.env);
    assert_eq!(
        s.client.try_attest_batch(&attestor, &entries),
        Err(Ok(Error::EmptyBatch))
    );
}

#[test]
fn attest_batch_fails_for_unregistered_attestor() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    let entries = Vec::from_array(&s.env, [entry(&s, &subject, "kyc_approved", b"a", ONE_DAY)]);

    assert_eq!(
        s.client.try_attest_batch(&attestor, &entries),
        Err(Ok(Error::AttestorNotRegistered))
    );
}

#[test]
fn attest_batch_fails_when_paused() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    s.client.pause();

    let subject = Address::generate(&s.env);
    let entries = Vec::from_array(&s.env, [entry(&s, &subject, "kyc_approved", b"a", ONE_DAY)]);

    assert_eq!(
        s.client.try_attest_batch(&attestor, &entries),
        Err(Ok(Error::ContractPaused))
    );
}

#[test]
fn unauthenticated_attest_batch_fails() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let subject = Address::generate(&s.env);
    let entries = Vec::from_array(&s.env, [entry(&s, &subject, "kyc_approved", b"a", ONE_DAY)]);

    s.env.set_auths(&[]);
    let result = s.client.try_attest_batch(&attestor, &entries);
    assert!(result.is_err());
}
