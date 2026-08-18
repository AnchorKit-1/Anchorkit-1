//! Property-based coverage for the `issued_at + ttl_seconds` arithmetic
//! behind `expires_at` in `record_attestation` (see contract.rs). Both
//! operands are `u64`s influenced by outside parties -- `issued_at` is the
//! ledger's timestamp and `ttl_seconds` is caller-supplied -- so near
//! `u64::MAX` a plain `+` would silently wrap to an `expires_at` *smaller*
//! than `issued_at`, which would make `is_valid`'s `timestamp < expires_at`
//! check misbehave (an attestation could look expired the instant it's
//! created, or -- depending on the wrap -- valid in ways it shouldn't be).
//! `saturating_add` is supposed to prevent this by clamping to `u64::MAX`
//! instead of wrapping; this file confirms that holds across the boundary,
//! not just for the hand-picked cases below.
//!
//! `record_attestation` also rejects any `ttl_seconds` above the configured
//! max-TTL (`Error::ExceedsMaxTtl`, default `DEFAULT_MAX_ATTESTATION_TTL_SECONDS`
//! -- see `max_ttl_tests.rs`), which would otherwise short-circuit every
//! case here before the arithmetic under test even runs. Each case below
//! raises the default max TTL to `u64::MAX` first so the boundary itself is
//! actually exercised; that cap is out of scope for this file.

extern crate std;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger as _};
use soroban_sdk::{Address, Bytes, Symbol};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::hash::compute_payload_hash;

fn assert_expires_at_matches_saturating_add(issued_at: u64, ttl_seconds: u64) {
    let mut env = soroban_sdk::Env::default();
    env.mock_all_auths();
    // As in `attestation_model_proptest.rs`: skip the per-case ledger
    // snapshot file this crate's `Env` writes on drop by default -- with
    // many cases per run that's many mostly-identical files with no review
    // value; the property check itself is what matters here.
    env.set_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });

    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let attestor = Address::generate(&env);
    let subject = Address::generate(&env);
    client.add_attestor(&attestor);
    client.set_default_max_attestation_ttl(&u64::MAX);

    env.ledger().set_timestamp(issued_at);

    let kind = Symbol::new(&env, "kyc_approved");
    let hash = compute_payload_hash(&env, &Bytes::from_slice(&env, b"payload"));
    client.attest(&attestor, &subject, &kind, &hash, &ttl_seconds);

    let stored = client.get_attestation(&subject, &kind);
    assert!(
        stored.expires_at >= issued_at,
        "expires_at ({}) wrapped below issued_at ({})",
        stored.expires_at,
        issued_at
    );
    assert_eq!(stored.expires_at, issued_at.saturating_add(ttl_seconds));
}

proptest! {
    // Each case spins up a fresh Soroban `Env` and contract instance (see
    // the same rationale in `attestation_model_proptest.rs`); keep the case
    // count modest so the suite still runs in a reasonable time.
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    /// `issued_at` ranges over the last 10M values below `u64::MAX` and
    /// `ttl_seconds` over the full `u64` range, so every generated pair
    /// lands squarely in the "would overflow a plain `+`" zone.
    #[test]
    fn expires_at_never_wraps_below_issued_at(
        issued_at in (u64::MAX - 10_000_000)..=u64::MAX,
        ttl_seconds in 1..=u64::MAX,
    ) {
        assert_expires_at_matches_saturating_add(issued_at, ttl_seconds);
    }
}

// Explicit boundary cases, kept alongside the property test as fast,
// always-run regression coverage for exactly the inputs that would trip a
// non-saturating implementation.
#[test]
fn expires_at_saturates_at_u64_max_with_timestamp_already_at_max() {
    assert_expires_at_matches_saturating_add(u64::MAX, 1);
}

#[test]
fn expires_at_saturates_when_ttl_alone_would_overflow() {
    assert_expires_at_matches_saturating_add(u64::MAX - 1, u64::MAX);
}

#[test]
fn expires_at_does_not_saturate_when_sum_fits() {
    assert_expires_at_matches_saturating_add(u64::MAX - 100, 50);
}
