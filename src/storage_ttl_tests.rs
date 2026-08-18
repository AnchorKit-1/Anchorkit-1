use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Bytes, Symbol};

use crate::hash::compute_payload_hash;
use crate::test_util::setup;
use crate::types::DataKey;

const ONE_DAY: u64 = 24 * 60 * 60;
const LEDGERS_PER_DAY: u32 = 17_280;
// Soroban's network-wide max_entry_ttl (see storage::MAX_ENTRY_TTL_LEDGERS
// and soroban_sdk::testutils::ledger::LedgerInfo::default()).
const MAX_ENTRY_TTL_LEDGERS: u32 = 6_312_000;

fn attestation_ttl(s: &crate::test_util::Setup, subject: &Address, kind: &Symbol) -> u32 {
    let key = DataKey::Attestation(subject.clone(), kind.clone());
    s.env
        .as_contract(&s.client.address, || s.env.storage().persistent().get_ttl(&key))
}

fn attestor_ttl(s: &crate::test_util::Setup, attestor: &Address) -> u32 {
    let key = DataKey::Attestor(attestor.clone());
    s.env
        .as_contract(&s.client.address, || s.env.storage().persistent().get_ttl(&key))
}

/// A short-lived attestation (e.g. a payment confirmation expiring in a
/// week) should only buy the minimum rent window, not a full year, since
/// the entry is never going to be alive that long anyway.
#[test]
fn short_lived_attestation_buys_only_the_minimum_rent_window() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "payment_confirmed");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    s.client.attest(&attestor, &subject, &kind, &hash, &(ONE_DAY * 7));

    let ttl = attestation_ttl(&s, &subject, &kind);
    // Old behavior bought a full year (365 * LEDGERS_PER_DAY) regardless of
    // the attestation's own 7-day lifetime; the new floor is 30 days.
    assert!(ttl <= LEDGERS_PER_DAY * 31, "ttl {ttl} should be near the 30-day floor");
    assert!(ttl >= LEDGERS_PER_DAY * 29, "ttl {ttl} should be near the 30-day floor");
}

/// A long-lived attestation (TTL beyond what a single bump can buy, since
/// Soroban clamps `extend_ttl` to its network-wide `max_entry_ttl`) should
/// still get as much storage TTL as the protocol allows in one call,
/// rather than the old flat 365-day window which under-bought relative to
/// this ceiling.
#[test]
fn long_lived_attestation_buys_the_network_maximum_window() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    let two_years = ONE_DAY * 365 * 2;
    // Raise the default max attestation TTL above the two-year request:
    // that business-rule cap (see max_ttl_tests.rs) is orthogonal to the
    // storage-rent ceiling this test exercises, but without this the
    // attest call below would fail with ExceedsMaxTtl before ever reaching
    // the storage-TTL-bump logic.
    s.client.set_default_max_attestation_ttl(&(ONE_DAY * 365 * 3));
    s.client.attest(&attestor, &subject, &kind, &hash, &two_years);

    let ttl = attestation_ttl(&s, &subject, &kind);
    assert!(
        ttl >= MAX_ENTRY_TTL_LEDGERS - LEDGERS_PER_DAY,
        "ttl {ttl} should be pinned near the network's max_entry_ttl ceiling"
    );
}

/// Since a single bump can't cover a multi-year attestation, `renew_attestation`
/// must be able to push the storage TTL back out once time has passed and
/// the entry's remaining TTL has dropped.
#[test]
fn renew_attestation_extends_storage_ttl_again() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, b"payload"));
    let two_years = ONE_DAY * 365 * 2;
    // See the matching comment in `long_lived_attestation_buys_the_network_maximum_window`.
    s.client.set_default_max_attestation_ttl(&(ONE_DAY * 365 * 3));
    s.client.attest(&attestor, &subject, &kind, &hash, &two_years);

    // Advance close to (but before) the storage TTL bought at `attest`
    // time, simulating a maintenance job that renews well ahead of expiry.
    let now = s.env.ledger().timestamp();
    s.env.ledger().set_timestamp(now + ONE_DAY * 300);
    s.env
        .ledger()
        .set_sequence_number(s.env.ledger().sequence() + LEDGERS_PER_DAY * 300);

    let ttl_before = attestation_ttl(&s, &subject, &kind);
    s.client.renew_attestation(&attestor, &subject, &kind);
    let ttl_after = attestation_ttl(&s, &subject, &kind);

    assert!(
        ttl_after > ttl_before,
        "renew should push the storage TTL back out (before {ttl_before}, after {ttl_after})"
    );
}

/// Attestor allow-list entries have no natural expiry, so they should buy
/// the long-lived window rather than the shorter attestation floor.
#[test]
fn attestor_allow_list_entry_buys_the_long_lived_window() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let ttl = attestor_ttl(&s, &attestor);
    assert!(
        ttl >= MAX_ENTRY_TTL_LEDGERS - LEDGERS_PER_DAY,
        "ttl {ttl} should reflect the long-lived attestor window"
    );
}

/// Since attestor membership is indefinite and a single bump can't buy
/// more than `max_entry_ttl`, `renew_attestor` must be able to push the
/// allow-list entry's storage TTL back out once time has passed.
#[test]
fn renew_attestor_extends_storage_ttl_again() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let now = s.env.ledger().timestamp();
    s.env.ledger().set_timestamp(now + ONE_DAY * 300);
    s.env
        .ledger()
        .set_sequence_number(s.env.ledger().sequence() + LEDGERS_PER_DAY * 300);

    let ttl_before = attestor_ttl(&s, &attestor);
    s.client.renew_attestor(&attestor);
    let ttl_after = attestor_ttl(&s, &attestor);

    assert!(
        ttl_after > ttl_before,
        "renew should push the storage TTL back out (before {ttl_before}, after {ttl_after})"
    );
}
