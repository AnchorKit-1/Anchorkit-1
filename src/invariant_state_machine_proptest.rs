//! State-machine property testing for issue #58.
//!
//! Generates random sequences of `attest`/`revoke`/`pause`/`unpause`/time-advance
//! calls and, after every step, checks four invariants that must hold
//! regardless of how the calls are ordered or interleaved. Unlike
//! `attestation_model_proptest.rs` (issue #99), this test does not predict
//! whether a given call *should* succeed against a parallel reference model
//! -- it only reacts to what the contract actually did (`result.is_ok()`)
//! and checks that the resulting state can never violate an invariant, no
//! matter which random sequence produced it. Any violation proptest finds is
//! shrunk to a minimal failing sequence and persisted under
//! `proptest-regressions/invariant_state_machine_proptest.txt`, which is
//! replayed first on every future run as a permanent regression check.
//!
//! Invariants checked after every step, for every (subject, attestation_type)
//! pair:
//!   1. `is_valid` agrees with `get_attestation`: it is `true` exactly when a
//!      stored attestation exists, is `Active`, and is unexpired.
//!   2. No resurrection: once `revoke` succeeds for a pair and no `attest`
//!      has succeeded for it since, `is_valid` never becomes `true` again --
//!      not through pausing, unpausing, or the passage of time.
//!   3. Pause freezes state: while the contract is paused, a pair's stored
//!      attestation data never changes from what it was the moment pausing
//!      began.
//!   4. Attestation count is monotonic: `get_attestation_count` only ever
//!      increases, by exactly one per successful `attest` call, and is
//!      unaffected by `revoke`/`pause`/`unpause`.

extern crate std;

use std::collections::BTreeMap;
use std::vec::Vec as StdVec;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger as _};
use soroban_sdk::{Address, Bytes, Symbol};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::errors::Error;
use crate::hash::compute_payload_hash;
use crate::types::{Attestation, AttestationStatus};

const NUM_ATTESTORS: usize = 2;
const NUM_SUBJECTS: usize = 2;
const NUM_TYPES: usize = 2;
// 0 = admin, 1..=NUM_ATTESTORS = attestors, last = an outsider with no role.
const NUM_CALLERS: usize = 1 + NUM_ATTESTORS + 1;

// Kept well under the 365-day default max TTL so `attest` never fails on
// `ExceedsMaxTtl` -- that path is exercised by `max_ttl_tests.rs` and is out
// of scope for this attest/revoke/pause/unpause invariant suite.
const MAX_TTL_SECONDS: u64 = 100_000;
const MAX_ADVANCE_SECONDS: u64 = 200_000;

#[derive(Clone, Debug)]
enum Action {
    Attest {
        attestor: usize,
        subject: usize,
        kind: usize,
        ttl: u64,
    },
    Revoke {
        caller: usize,
        subject: usize,
        kind: usize,
    },
    Pause,
    Unpause,
    AdvanceTime(u64),
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        (
            0..NUM_ATTESTORS,
            0..NUM_SUBJECTS,
            0..NUM_TYPES,
            1..=MAX_TTL_SECONDS
        )
            .prop_map(|(attestor, subject, kind, ttl)| Action::Attest {
                attestor,
                subject,
                kind,
                ttl
            }),
        (0..NUM_CALLERS, 0..NUM_SUBJECTS, 0..NUM_TYPES).prop_map(|(caller, subject, kind)| {
            Action::Revoke {
                caller,
                subject,
                kind,
            }
        }),
        Just(Action::Pause),
        Just(Action::Unpause),
        (0..=MAX_ADVANCE_SECONDS).prop_map(Action::AdvanceTime),
    ]
}

/// `Ok(Some(_))` for a stored attestation, `Ok(None)` when none exists --
/// panics on any other error, since that would mean `get_attestation`
/// disagrees with the contract about what errors it can return.
fn fetch_attestation(
    client: &AnchorKitContractClient,
    subject: &Address,
    kind: &Symbol,
) -> Option<Attestation> {
    match client.try_get_attestation(subject, kind) {
        Ok(Ok(a)) => Some(a),
        Err(Ok(Error::AttestationNotFound)) => None,
        other => panic!("unexpected get_attestation result: {:?}", other),
    }
}

proptest! {
    // Each case spins up a fresh Soroban `Env`, which is far more expensive
    // than a typical proptest case; keep the case count modest so the suite
    // still runs in a reasonable time.
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    #[test]
    fn invariants_hold_across_random_sequences(
        actions in proptest::collection::vec(action_strategy(), 1..=20)
    ) {
        let mut env = soroban_sdk::Env::default();
        env.mock_all_auths();
        // A committed ledger snapshot per proptest case (128 of them) has no
        // review value -- see the same rationale in `attestor_stress_tests.rs`.
        env.set_config(EnvTestConfig {
            capture_snapshot_at_drop: false,
        });

        let contract_id = env.register(AnchorKitContract, ());
        let client = AnchorKitContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);

        let attestors: StdVec<Address> = (0..NUM_ATTESTORS)
            .map(|_| Address::generate(&env))
            .collect();
        for a in &attestors {
            client.add_attestor(a);
        }
        let outsider = Address::generate(&env);
        let mut callers = StdVec::new();
        callers.push(admin.clone());
        callers.extend(attestors.iter().cloned());
        callers.push(outsider);

        let subjects: StdVec<Address> = (0..NUM_SUBJECTS)
            .map(|_| Address::generate(&env))
            .collect();
        let kinds: StdVec<Symbol> = (0..NUM_TYPES)
            .map(|i| Symbol::new(&env, if i == 0 { "kyc_approved" } else { "payment_confirmed" }))
            .collect();
        let hash = compute_payload_hash(&env, &Bytes::from_slice(&env, b"payload"));

        // Minimal bookkeeping, built only from what the contract actually
        // did -- never a prediction of what it *should* do.
        let mut revoked_since_attest: BTreeMap<(usize, usize), bool> = BTreeMap::new();
        let mut paused = false;
        let mut paused_snapshot: Option<BTreeMap<(usize, usize), Option<Attestation>>> = None;
        let mut expected_count: u64 = 0;

        for action in actions {
            match action {
                Action::Attest { attestor, subject, kind, ttl } => {
                    let result = client.try_attest(
                        &attestors[attestor],
                        &subjects[subject],
                        &kinds[kind],
                        &hash,
                        &ttl,
                    );
                    if result.is_ok() {
                        revoked_since_attest.insert((subject, kind), false);
                        expected_count += 1;
                    }
                }
                Action::Revoke { caller, subject, kind } => {
                    let result = client.try_revoke(&callers[caller], &subjects[subject], &kinds[kind]);
                    if result.is_ok() {
                        revoked_since_attest.insert((subject, kind), true);
                    }
                }
                Action::Pause => {
                    client.pause();
                    if !paused {
                        let mut snap = BTreeMap::new();
                        for (si, s_addr) in subjects.iter().enumerate() {
                            for (ki, k_sym) in kinds.iter().enumerate() {
                                snap.insert((si, ki), fetch_attestation(&client, s_addr, k_sym));
                            }
                        }
                        paused_snapshot = Some(snap);
                    }
                    paused = true;
                }
                Action::Unpause => {
                    client.unpause();
                    paused = false;
                    paused_snapshot = None;
                }
                Action::AdvanceTime(secs) => {
                    let now = env.ledger().timestamp();
                    env.ledger().set_timestamp(now.saturating_add(secs));
                }
            }

            let now = env.ledger().timestamp();

            for (si, s_addr) in subjects.iter().enumerate() {
                for (ki, k_sym) in kinds.iter().enumerate() {
                    let stored = fetch_attestation(&client, s_addr, k_sym);
                    let valid = client.is_valid(s_addr, k_sym);

                    // Invariant 1: is_valid agrees with get_attestation's own fields.
                    let expected_valid = stored
                        .as_ref()
                        .is_some_and(|a| a.status == AttestationStatus::Active && now < a.expires_at);
                    prop_assert_eq!(
                        valid, expected_valid,
                        "is_valid/get_attestation disagree at subject={} kind={}", si, ki
                    );

                    // Invariant 2: revocation is never undone by anything but a fresh attest.
                    if *revoked_since_attest.get(&(si, ki)).unwrap_or(&false) {
                        prop_assert!(
                            !valid,
                            "revoked attestation became valid again at subject={} kind={}", si, ki
                        );
                        if let Some(a) = &stored {
                            prop_assert_eq!(
                                a.status.clone(), AttestationStatus::Revoked,
                                "revoked attestation's stored status flipped back to Active at subject={} kind={}", si, ki
                            );
                        }
                    }

                    // Invariant 3: pausing freezes every pair's stored data.
                    if let Some(snap) = &paused_snapshot {
                        prop_assert_eq!(
                            &stored, snap.get(&(si, ki)).unwrap(),
                            "attestation data changed while contract paused at subject={} kind={}", si, ki
                        );
                    }
                }
            }

            // Invariant 4: attestation count is monotonic and tracks successful attests exactly.
            let count = client.get_attestation_count();
            prop_assert_eq!(
                count, expected_count,
                "attestation count diverged from the number of successful attest calls"
            );
        }
    }
}
