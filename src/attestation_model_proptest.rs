//! Model-based (differential) property testing for issue #99.
//!
//! Generates random sequences of `attest`/`revoke`/`pause`/`unpause`/time-advance
//! calls and, after every step, checks that `is_valid` on the real contract
//! agrees with an independent reference model built directly from the public
//! contract semantics -- not by reading `storage.rs` -- so a bug in the real
//! implementation shows up as a mismatch instead of being mirrored by the
//! model. Proptest shrinks any mismatch to a minimal failing sequence and
//! persists it under `proptest-regressions/attestation_model_proptest.txt`,
//! which is replayed first on every subsequent run, turning it into a
//! permanent regression check.

extern crate std;

use std::collections::BTreeMap;
use std::vec::Vec as StdVec;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger as _};
use soroban_sdk::{Address, Bytes, Symbol};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::hash::compute_payload_hash;

const NUM_ATTESTORS: usize = 2;
const NUM_SUBJECTS: usize = 2;
const NUM_TYPES: usize = 2;
// 0 = admin, 1..=NUM_ATTESTORS = attestors, last = an outsider with no role.
const NUM_CALLERS: usize = 1 + NUM_ATTESTORS + 1;

// Kept well under the 365-day default max TTL (`DEFAULT_MAX_ATTESTATION_TTL_SECONDS`)
// so `attest` never fails on `ExceedsMaxTtl` -- that path is exercised by
// `max_ttl_tests.rs` and is out of scope for this attest/revoke/pause/unpause model.
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

/// Independent reference model: no attestation exists until `Attest`
/// succeeds; it is valid exactly while unrevoked and unexpired.
#[derive(Clone, Debug)]
struct RefEntry {
    attestor: usize,
    expires_at: u64,
    revoked: bool,
}

#[derive(Default)]
struct ReferenceModel {
    paused: bool,
    entries: BTreeMap<(usize, usize), RefEntry>,
}

impl ReferenceModel {
    fn is_valid(&self, subject: usize, kind: usize, now: u64) -> bool {
        match self.entries.get(&(subject, kind)) {
            Some(e) => !e.revoked && now < e.expires_at,
            None => false,
        }
    }
}

proptest! {
    // Each case spins up a fresh Soroban `Env` and contract instance, which
    // is far more expensive than a typical proptest case; keep the case
    // count modest so the suite still runs in a reasonable time.
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    #[test]
    fn is_valid_matches_reference_model(
        actions in proptest::collection::vec(action_strategy(), 1..=20)
    ) {
        let mut env = soroban_sdk::Env::default();
        env.mock_all_auths();
        // Each proptest case spins up its own `Env`, and by default every
        // `Env` drop writes a ledger snapshot file under `test_snapshots/`
        // for the repo to commit. With 128 cases per run that's 128 mostly-
        // identical files with no review value (see the same rationale in
        // `attestor_stress_tests.rs`), so this test's value is the property
        // check itself, not a committed snapshot per case.
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

        let mut model = ReferenceModel::default();

        for action in actions {
            match action {
                Action::Attest { attestor, subject, kind, ttl } => {
                    let expect_ok = !model.paused;
                    let result = client.try_attest(
                        &attestors[attestor],
                        &subjects[subject],
                        &kinds[kind],
                        &hash,
                        &ttl,
                    );
                    prop_assert_eq!(result.is_ok(), expect_ok);
                    if expect_ok {
                        let now = env.ledger().timestamp();
                        model.entries.insert(
                            (subject, kind),
                            RefEntry { attestor, expires_at: now.saturating_add(ttl), revoked: false },
                        );
                    }
                }
                Action::Revoke { caller, subject, kind } => {
                    let entry = model.entries.get(&(subject, kind)).cloned();
                    let authorized = entry.as_ref().is_some_and(|e| {
                        callers[caller] == admin || callers[caller] == attestors[e.attestor]
                    });
                    let expect_ok = !model.paused
                        && entry.as_ref().is_some_and(|e| !e.revoked)
                        && authorized;
                    let result = client.try_revoke(&callers[caller], &subjects[subject], &kinds[kind]);
                    prop_assert_eq!(result.is_ok(), expect_ok);
                    if expect_ok {
                        model.entries.get_mut(&(subject, kind)).unwrap().revoked = true;
                    }
                }
                Action::Pause => {
                    client.pause();
                    model.paused = true;
                }
                Action::Unpause => {
                    client.unpause();
                    model.paused = false;
                }
                Action::AdvanceTime(secs) => {
                    let now = env.ledger().timestamp();
                    env.ledger().set_timestamp(now.saturating_add(secs));
                }
            }

            let now = env.ledger().timestamp();
            for (subject, subject_addr) in subjects.iter().enumerate() {
                for (kind, kind_sym) in kinds.iter().enumerate() {
                    let expected = model.is_valid(subject, kind, now);
                    let actual = client.is_valid(subject_addr, kind_sym);
                    prop_assert_eq!(
                        actual, expected,
                        "is_valid mismatch at subject={} kind={}", subject, kind
                    );
                }
            }
        }
    }
}
