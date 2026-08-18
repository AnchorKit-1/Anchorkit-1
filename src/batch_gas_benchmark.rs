//! Measures how much `attest_batch` saves over N individual `attest` calls,
//! across a range of batch sizes, to find where the savings level off (and
//! the hard batch-size ceiling imposed by Soroban's per-invocation ledger
//! footprint limits).
//!
//! Run with `cargo test batch_gas_benchmark -- --nocapture` to see the
//! printed tables; results are summarized in the README's "Batch
//! attestation gas amortization" section.
//!
//! Each `attest` call pays a fixed per-invocation cost (auth check,
//! allow-list lookup, pause check) plus a variable per-entry cost (storage
//! write, TTL math, event). `attest_batch` pays the fixed cost once and the
//! variable cost N times, so its per-entry cost should approach the
//! variable-only cost as batch size grows, while individual calls always
//! pay fixed+variable per entry.

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Symbol, Vec};

use crate::hash::compute_payload_hash;
use crate::test_util::setup;
use crate::types::BatchAttestEntry;

const ONE_DAY: u64 = 24 * 60 * 60;
// Kept under the ledger-footprint ceiling documented on
// `attest_batch_stays_under_the_ledger_write_ceiling` below (currently 15
// entries) -- a larger size here would hit the same host-level footprint
// trap this benchmark is trying to measure around, not a benchmark result.
const BATCH_SIZES: &[u32] = &[1, 2, 5, 10, 15];

fn batch_entries(s: &crate::test_util::Setup<'_>, n: u32, kind: &Symbol) -> Vec<BatchAttestEntry> {
    let mut entries = Vec::new(&s.env);
    for i in 0..n {
        let subject = Address::generate(&s.env);
        let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, &i.to_be_bytes()));
        entries.push_back(BatchAttestEntry {
            subject,
            attestation_type: kind.clone(),
            payload_hash: hash,
            ttl_seconds: ONE_DAY,
        });
    }
    entries
}

/// Sum of `n` separately-measured `attest` calls: the budget is reset
/// before *each* call so cross-call host caching effects can't leak one
/// call's cost into another's, then the per-call costs are summed in plain
/// Rust arithmetic.
#[allow(deprecated)]
fn measure_individual(n: u32) -> (u64, u64) {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    let kind = Symbol::new(&s.env, "kyc_approved");

    let calls: std::vec::Vec<_> = (0..n)
        .map(|i| {
            let subject = Address::generate(&s.env);
            let hash = compute_payload_hash(&s.env, &Bytes::from_slice(&s.env, &i.to_be_bytes()));
            (subject, hash)
        })
        .collect();

    let mut budget = s.env.budget();
    let mut total_cpu = 0u64;
    let mut total_mem = 0u64;
    for (subject, hash) in calls.iter() {
        budget.reset_unlimited();
        s.client.attest(&attestor, subject, &kind, hash, &ONE_DAY);
        total_cpu += budget.cpu_instruction_cost();
        total_mem += budget.memory_bytes_cost();
    }
    (total_cpu, total_mem)
}

/// Metered cost of one `attest_batch` call carrying `n` entries. Entry
/// construction happens before the budget is reset, matching
/// `measure_individual` so both isolate just the contract invocation cost.
#[allow(deprecated)]
fn measure_batch(n: u32) -> (u64, u64) {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    let kind = Symbol::new(&s.env, "kyc_approved");
    let entries = batch_entries(&s, n, &kind);

    let mut budget = s.env.budget();
    budget.reset_unlimited();
    s.client.attest_batch(&attestor, &entries);
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

#[test]
fn batch_amortizes_fixed_overhead() {
    std::println!(
        "{:>10} | {:>14} | {:>14} | {:>12} | {:>12} | {:>9}",
        "batch_size",
        "individual_cpu",
        "batch_cpu",
        "indiv/entry",
        "batch/entry",
        "savings"
    );

    for &n in BATCH_SIZES {
        let (individual_cpu, _individual_mem) = measure_individual(n);
        let (batch_cpu, _batch_mem) = measure_batch(n);

        let individual_per_entry = individual_cpu as f64 / n as f64;
        let batch_per_entry = batch_cpu as f64 / n as f64;
        let savings_pct = 100.0 * (1.0 - batch_per_entry / individual_per_entry);

        std::println!(
            "{:>10} | {:>14} | {:>14} | {:>12.0} | {:>12.0} | {:>8.1}%",
            n,
            individual_cpu,
            batch_cpu,
            individual_per_entry,
            batch_per_entry,
            savings_pct
        );
    }
}

/// Each attestation entry writes three separate persistent keys --
/// `Attestation`, `AttestationSeq`, and `AttestationHistory` (see
/// `record_attestation` in contract.rs, which both stores the attestation
/// and appends a history entry for it) -- plus a handful of fixed slots
/// shared by the whole invocation (the attestor allow-list read, the
/// contract instance, etc.), and Soroban caps total footprint entries per
/// invocation (the test host defaults to the mainnet-equivalent limit of
/// 100). That makes batch size a hard wall, not just a cost curve: past
/// some N, no amount of budget helps because the invocation is rejected
/// before it runs, with a host trap rather than an `Error` the caller can
/// handle gracefully (`try_attest_batch` doesn't catch it either --
/// footprint limits are enforced by the host below the contract, not by
/// anything `attest_batch` itself returns).
///
/// Bisecting locally found the wall at exactly 16 entries under the test
/// host's default limits: 15 succeeds, 16 fails ("total footprint ledger
/// entries: 102 > 100"). Real transactions add their own footprint (fee
/// bump, source account, etc.), so treat 15 as an optimistic upper bound,
/// not a safe operating ceiling -- see the README note.
#[test]
fn attest_batch_stays_under_the_ledger_write_ceiling() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    s.client.add_attestor(&attestor);
    let kind = Symbol::new(&s.env, "kyc_approved");

    let entries = batch_entries(&s, 15, &kind);
    s.client.attest_batch(&attestor, &entries);
    assert_eq!(s.client.get_attestation_count(), 15);
}
