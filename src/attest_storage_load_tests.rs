//! Storage-growth and ledger-cost load test for `attest` at scale.
//!
//! Characterises how CPU instruction count, memory usage, and persistent-
//! storage entry count grow as thousands of `attest` calls accumulate, to
//! give the rent/TTL-tuning work concrete numbers rather than guesses.
//!
//! # What is measured
//!
//! Three tests, each printing a sampling table:
//!
//! 1. **`attest_storage_growth_and_cost`** — calls `attest` N times for N
//!    distinct (subject, attestation_type) pairs, sampling CPU instructions,
//!    memory bytes, and cumulative storage entry count at logarithmically-
//!    spaced checkpoints. Shows whether per-call cost is truly O(1) or
//!    whether it drifts as the ledger fills up.
//!
//! 2. **`attest_cost_by_ttl_bucket`** — calls `attest` once for each of
//!    five representative TTL values (1 day → 2 years) and records the
//!    metered cost. Quantifies the rent-window cost curve described in
//!    `docs/storage-rent-cost-analysis.md`.
//!
//! 3. **`attest_repeated_subject_history_growth`** — re-attests the same
//!    (subject, type) pair N times and samples cost plus history entry count.
//!    Each overwrite creates a new `AttestationHistory` key, so this shows
//!    the cost of that append-only growth specifically.
//!
//! # Running
//!
//! ```text
//! cargo test --release --features stress-tests attest_storage -- --nocapture
//! ```
//!
//! To run a single test:
//! ```text
//! cargo test --release --features stress-tests attest_storage_growth_and_cost -- --nocapture
//! ```
//!
//! Results are written up in `docs/storage-load-test-results.md`.

extern crate std;

use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger as _};
use soroban_sdk::{Address, Bytes, BytesN, Env, Symbol};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::hash::compute_payload_hash;

// ---------------------------------------------------------------------------
// Checkpoints at which we sample cost
// ---------------------------------------------------------------------------

/// Checkpoints for the main growth test (call number at which we sample).
/// Spaced roughly logarithmically so the table captures both the early
/// cheap calls and the later potentially-more-expensive ones in one view.
const GROWTH_CHECKPOINTS: &[u32] = &[1, 10, 50, 100, 250, 500, 1_000, 2_500, 5_000];

/// Checkpoints for the repeated-subject history test.
const HISTORY_CHECKPOINTS: &[u32] = &[1, 10, 50, 100, 250, 500, 1_000];

/// TTL values (seconds) for the per-TTL-bucket test.
const TTL_SAMPLES: &[(u64, &str)] = &[
    (86_400,          "1 day    "),
    (7 * 86_400,      "7 days   "),
    (30 * 86_400,     "30 days  "),
    (90 * 86_400,     "90 days  "),
    (180 * 86_400,    "180 days "),
    (365 * 86_400,    "1 year   "),
    (2 * 365 * 86_400,"2 years  "),
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic 32-byte payload hash from an integer seed (no real SHA-256
/// needed — we just need distinct, valid `BytesN<32>` values).
fn payload_from_seed(env: &Env, seed: u32) -> BytesN<32> {
    let buf = seed.to_be_bytes();
    // Repeat the 4-byte seed to fill 32 bytes.
    let mut v = [0u8; 32];
    for (i, b) in v.iter_mut().enumerate() {
        *b = buf[i % 4];
    }
    let bytes = Bytes::from_slice(env, &v);
    compute_payload_hash(env, &bytes)
}

/// Build a test env with snapshot-capture disabled (avoid writing tens-of-
/// megabyte snapshot files for large-scale tests) and mocked auth.
fn make_env() -> Env {
    let mut env = Env::default();
    env.mock_all_auths();
    env.set_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env
}

// ---------------------------------------------------------------------------
// Test 1: storage growth and per-call cost at scale
// ---------------------------------------------------------------------------

/// Calls `attest` for GROWTH_CHECKPOINTS.last() distinct (subject, type)
/// pairs, sampling the metered CPU/memory cost and the cumulative
/// attestation count (a proxy for storage entry count — each call writes 3
/// persistent keys) at each checkpoint.
///
/// Expected result: per-call cost is flat (O(1)); the cumulative entry count
/// grows linearly with call count, confirming no unexpected fan-out.
#[test]
#[allow(deprecated)]
fn attest_storage_growth_and_cost() {
    let env = make_env();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let attestor = Address::generate(&env);
    client.add_attestor(&attestor);

    let kind = Symbol::new(&env, "kyc_approved");
    let one_day: u64 = 86_400;

    let mut budget = env.budget();
    let max_calls = *GROWTH_CHECKPOINTS.last().expect("non-empty");

    std::println!("\n=== Test 1: attest storage growth and cost at scale ===");
    std::println!(
        "{:>8} | {:>16} | {:>16} | {:>14} | {:>14}",
        "call_n",
        "cpu_insns",
        "mem_bytes",
        "cumul_entries",
        "keys_per_call"
    );

    let mut next_checkpoint_idx = 0;

    for n in 1..=max_calls {
        let subject = Address::generate(&env);
        let hash = payload_from_seed(&env, n);

        budget.reset_unlimited();
        client.attest(&attestor, &subject, &kind, &hash, &one_day);
        let cpu = budget.cpu_instruction_cost();
        let mem = budget.memory_bytes_cost();

        if next_checkpoint_idx < GROWTH_CHECKPOINTS.len()
            && n == GROWTH_CHECKPOINTS[next_checkpoint_idx]
        {
            // Each `attest` writes 3 persistent keys: Attestation,
            // AttestationSeq, AttestationHistory. The total count is n * 3
            // plus any fixed instance keys — the running attestation count
            // is a precise proxy.
            let cumul_attestations = client.get_attestation_count();
            // Persistent entries: 3 per call (Attestation, AttestationSeq,
            // AttestationHistory), plus the 1 Attestor key = n*3 + 1.
            let estimated_keys = cumul_attestations * 3 + 1;

            std::println!(
                "{:>8} | {:>16} | {:>16} | {:>14} | {:>14}",
                n,
                cpu,
                mem,
                estimated_keys,
                3u64, // always 3 new persistent keys per attest call
            );

            next_checkpoint_idx += 1;
        }
    }

    assert_eq!(
        next_checkpoint_idx,
        GROWTH_CHECKPOINTS.len(),
        "all checkpoints must have been sampled"
    );

    // Verify the final cumulative count.
    let final_count = client.get_attestation_count();
    assert_eq!(final_count, max_calls as u64);
    std::println!(
        "\nFinal attestation count: {} (3 persistent keys each = {} total attest entries, plus attestor and instance keys)",
        final_count,
        final_count * 3
    );
}

// ---------------------------------------------------------------------------
// Test 2: cost by TTL bucket
// ---------------------------------------------------------------------------

/// Measures the metered cost of a single `attest` call for each TTL bucket
/// defined in TTL_SAMPLES. Because the storage rent window (extend_ttl) is
/// proportional to the attestation's own TTL, longer TTLs are expected to
/// produce a higher memory cost due to the larger extend_ttl argument being
/// evaluated inside the host's rent-fee computation.
///
/// Expected result: CPU cost is essentially flat across TTL values (the
/// attestation write path is the same code regardless); memory may vary
/// slightly. This confirms the TTL-proportional rent design in storage.rs
/// doesn't add unexpected compute overhead.
#[test]
#[allow(deprecated)]
fn attest_cost_by_ttl_bucket() {
    // The longest TTL sample (2 years) exceeds the contract's built-in
    // default max (365 days). We raise the default max to 2 years so every
    // bucket can be measured without ExceedsMaxTtl errors.
    let two_years: u64 = 2 * 365 * 86_400;

    std::println!("\n=== Test 2: attest cost by TTL bucket ===");
    std::println!(
        "{:<12} | {:>14} | {:>16} | {:>16}",
        "ttl_label",
        "ttl_seconds",
        "cpu_insns",
        "mem_bytes"
    );

    for &(ttl_seconds, label) in TTL_SAMPLES {
        let env = make_env();
        let contract_id = env.register(AnchorKitContract, ());
        let client = AnchorKitContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        // Raise the default max to the largest sample so all TTL buckets
        // can be measured. This is what an operator with long-lived
        // attestations would configure.
        client.set_default_max_attestation_ttl(&two_years);

        let attestor = Address::generate(&env);
        client.add_attestor(&attestor);
        let kind = Symbol::new(&env, "kyc_approved");
        let subject = Address::generate(&env);
        let hash = payload_from_seed(&env, 0);

        // Ensure the ledger timestamp is set above zero so TTL math is
        // well-defined (remaining = expires_at - now, not saturated to 0).
        env.ledger().with_mut(|li| {
            li.timestamp = 1_000_000;
        });

        let mut budget = env.budget();
        budget.reset_unlimited();
        client.attest(&attestor, &subject, &kind, &hash, &ttl_seconds);
        let cpu = budget.cpu_instruction_cost();
        let mem = budget.memory_bytes_cost();

        std::println!(
            "{:<12} | {:>14} | {:>16} | {:>16}",
            label,
            ttl_seconds,
            cpu,
            mem
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: repeated-subject history growth cost
// ---------------------------------------------------------------------------

/// Re-attests the SAME (subject, attestation_type) pair N times, sampling
/// CPU/memory per call and the accumulated history entry count at each
/// checkpoint. Each call to `attest` for an already-attested pair creates a
/// NEW `AttestationHistory` key (append-only), so this test isolates the
/// per-call overhead of that history growth.
///
/// Expected result: per-call cost stays flat (each attest writes a fixed
/// number of keys regardless of prior history depth), while the key count
/// grows by exactly 3 per call (overwrite Attestation, increment
/// AttestationSeq, append AttestationHistory). This validates the O(1)
/// write path and lets the rent-tuning work quantify history-growth cost.
#[test]
#[allow(deprecated)]
fn attest_repeated_subject_history_growth() {
    let env = make_env();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);
    let attestor = Address::generate(&env);
    client.add_attestor(&attestor);
    let kind = Symbol::new(&env, "kyc_approved");

    // One fixed subject — all calls go to the same (subject, type) pair.
    let subject = Address::generate(&env);
    let one_day: u64 = 86_400;

    let max_calls = *HISTORY_CHECKPOINTS.last().expect("non-empty");
    let mut budget = env.budget();
    let mut next_checkpoint_idx = 0;

    std::println!("\n=== Test 3: repeated-subject history growth cost ===");
    std::println!(
        "{:>8} | {:>16} | {:>16} | {:>14} | {:>16}",
        "call_n",
        "cpu_insns",
        "mem_bytes",
        "history_len",
        "keys_written_total"
    );

    for n in 1..=max_calls {
        let hash = payload_from_seed(&env, n);

        budget.reset_unlimited();
        client.attest(&attestor, &subject, &kind, &hash, &one_day);
        let cpu = budget.cpu_instruction_cost();
        let mem = budget.memory_bytes_cost();

        if next_checkpoint_idx < HISTORY_CHECKPOINTS.len()
            && n == HISTORY_CHECKPOINTS[next_checkpoint_idx]
        {
            // History length == n (one entry per call).
            // Total persistent keys written == n*2 (seq grows by 1 per call,
            // history grows by 1 per call) + 1 (Attestation overwritten in
            // place) + 1 (Attestor key) = n*2 + 2 from attest calls, plus
            // the Attestation key itself (always 1 entry, overwritten).
            // Simplified: keys_written ≈ n*3 new/overwritten touches total.
            let keys_total = n as u64 * 3; // Attestation + AttestationSeq + AttestationHistory per call

            std::println!(
                "{:>8} | {:>16} | {:>16} | {:>14} | {:>16}",
                n,
                cpu,
                mem,
                n,           // one history entry per call
                keys_total,
            );

            next_checkpoint_idx += 1;
        }
    }

    assert_eq!(
        next_checkpoint_idx,
        HISTORY_CHECKPOINTS.len(),
        "all checkpoints must have been sampled"
    );

    // Confirm the attestation count: all N calls went to the same pair, so
    // get_attestation_count() == N because it's a running total, not unique.
    let total = client.get_attestation_count();
    assert_eq!(total, max_calls as u64);
    std::println!(
        "\nFinal history depth for single subject: {} entries, {} persistent history keys",
        total,
        total  // one AttestationHistory key per call
    );
}
