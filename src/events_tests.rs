//! Snapshot-style tests that pin the exact wire shape (topics + data) of
//! every event defined in `events.rs`.
//!
//! These deliberately do NOT construct the expected value from the event
//! structs themselves (e.g. via `MyEvent { .. }.to_xdr(..)`) -- doing so
//! would make the assertion track whatever `events.rs` currently does,
//! silently passing even if a `#[topic]` attribute, a `topics = [...]`
//! override, or a field were changed or removed. Instead each expected
//! topics/data value is hand-built from primitives (`Symbol`, `Map`,
//! `.into_val`), independent of the `#[contractevent]` derive, so a change
//! to the on-chain event contract makes the corresponding test fail rather
//! than silently follow along. See `src/events.rs` for the source of truth
//! this file is pinning.
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{vec, Address, Bytes, Env, IntoVal, Map, Symbol, Val};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::hash::compute_payload_hash;
use crate::test_util::setup;

const ONE_DAY: u64 = 24 * 60 * 60;

fn empty_data(env: &Env) -> Val {
    Map::<Symbol, Val>::from_array(env, []).into_val(env)
}

#[test]
fn initialized_event_shape() {
    // `Initialized` fires from `initialize`, which `test_util::setup()`
    // already calls -- so this test registers the contract itself rather
    // than going through the shared helper, to observe that one event in
    // isolation.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id.clone(),
                (Symbol::new(&env, "initialized"), admin.clone()).into_val(&env),
                empty_data(&env),
            ),
        ]
    );
}

#[test]
fn admin_changed_event_shape() {
    let s = setup();
    let new_admin = Address::generate(&s.env);

    s.client.set_admin(&new_admin);

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(Symbol::new(&s.env, "new_admin"), new_admin.into_val(&s.env))],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "admin_set"), s.admin.clone()).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn attestor_added_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);

    s.client.add_attestor(&attestor);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "attestor_add"), attestor.clone()).into_val(&s.env),
                empty_data(&s.env),
            ),
        ]
    );
}

#[test]
fn attestor_removed_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    s.client.remove_attestor(&attestor);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "attestor_del"), attestor.clone()).into_val(&s.env),
                empty_data(&s.env),
            ),
        ]
    );
}

#[test]
fn attestor_renewed_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    s.client.renew_attestor(&attestor);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "attestor_renew"), attestor.clone()).into_val(&s.env),
                empty_data(&s.env),
            ),
        ]
    );
}

#[test]
fn attested_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);

    let kind = Symbol::new(&s.env, "kyc_approved");
    let payload = Bytes::from_slice(&s.env, b"kyc-decision:approved");
    let hash = compute_payload_hash(&s.env, &payload);
    let issued_at = s.env.ledger().timestamp();

    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    let expires_at = issued_at + ONE_DAY;
    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [
            (
                Symbol::new(&s.env, "attestor"),
                attestor.clone().into_val(&s.env),
            ),
            (Symbol::new(&s.env, "payload_hash"), hash.into_val(&s.env)),
            (
                Symbol::new(&s.env, "expires_at"),
                expires_at.into_val(&s.env),
            ),
        ],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "attest"), subject.clone(), kind.clone()).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn revoked_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    let kind = Symbol::new(&s.env, "kyc_approved");
    let payload = Bytes::from_slice(&s.env, b"kyc-decision:approved");
    let hash = compute_payload_hash(&s.env, &payload);
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    s.client.revoke(&attestor, &subject, &kind);

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(
            Symbol::new(&s.env, "revoked_by"),
            attestor.clone().into_val(&s.env),
        )],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "revoke"), subject.clone(), kind.clone()).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn attestation_renewed_event_shape() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    let subject = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    let kind = Symbol::new(&s.env, "kyc_approved");
    let payload = Bytes::from_slice(&s.env, b"kyc-decision:approved");
    let hash = compute_payload_hash(&s.env, &payload);
    s.client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    s.client.renew_attestation(&attestor, &subject, &kind);

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(
            Symbol::new(&s.env, "renewed_by"),
            attestor.clone().into_val(&s.env),
        )],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "renew"), subject.clone(), kind.clone()).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn pause_toggled_event_shape_on_pause() {
    let s = setup();

    s.client.pause();

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(Symbol::new(&s.env, "paused"), true.into_val(&s.env))],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "pause"),).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn pause_toggled_event_shape_on_unpause() {
    let s = setup();
    s.client.pause();

    s.client.unpause();

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(Symbol::new(&s.env, "paused"), false.into_val(&s.env))],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "pause"),).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn default_max_attestation_ttl_changed_event_shape() {
    let s = setup();
    let max_ttl_seconds: u64 = 42 * ONE_DAY;

    s.client.set_default_max_attestation_ttl(&max_ttl_seconds);

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(
            Symbol::new(&s.env, "max_ttl_seconds"),
            max_ttl_seconds.into_val(&s.env),
        )],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "default_max_ttl"),).into_val(&s.env),
                data,
            ),
        ]
    );
}

#[test]
fn max_attestation_ttl_changed_event_shape() {
    let s = setup();
    let kind = Symbol::new(&s.env, "kyc_approved");
    let max_ttl_seconds: u64 = 7 * ONE_DAY;

    s.client.set_max_attestation_ttl(&kind, &max_ttl_seconds);

    let data = Map::<Symbol, Val>::from_array(
        &s.env,
        [(
            Symbol::new(&s.env, "max_ttl_seconds"),
            max_ttl_seconds.into_val(&s.env),
        )],
    )
    .into_val(&s.env);

    assert_eq!(
        s.env.events().all(),
        vec![
            &s.env,
            (
                s.client.address.clone(),
                (Symbol::new(&s.env, "max_ttl"), kind.clone()).into_val(&s.env),
                data,
            ),
        ]
    );
}
