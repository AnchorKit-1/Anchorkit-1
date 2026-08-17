//! End-to-end test against a real Stellar testnet, gated behind the
//! `testnet-integration` feature so it never runs as part of a normal
//! `cargo test` -- it needs network access, a funded testnet account, and a
//! deployable wasm artifact, none of which a default test run can assume.
//!
//! Unlike the rest of the test suite, this doesn't use `soroban_sdk`'s
//! mock `Env` -- there's no in-process way to drive a real network. Instead
//! it shells out to the [Stellar CLI](https://developers.stellar.org/docs/tools/cli)
//! (`stellar`), the same tool a contributor would use by hand to deploy and
//! invoke a contract, so this test carries no extra signing/XDR-encoding
//! dependency of its own.
//!
//! ## Prerequisites
//!
//! - The `stellar` CLI installed and on `PATH`.
//! - A funded testnet identity:
//!   ```sh
//!   stellar keys generate anchorkit-testnet --network testnet --fund
//!   ```
//! - The wasm artifact already built:
//!   ```sh
//!   cargo build --target wasm32v1-none --release
//!   ```
//! - The `ANCHORKIT_TESTNET_SOURCE` environment variable set to that
//!   identity's name (or any other identity/secret key the CLI accepts as
//!   `--source`).
//!
//! ## Running
//!
//! ```sh
//! ANCHORKIT_TESTNET_SOURCE=anchorkit-testnet \
//!   cargo test --features testnet-integration testnet_integration -- --nocapture
//! ```
//!
//! ## Testnet state left behind
//!
//! Every run deploys a brand-new contract instance -- Soroban has no way to
//! delete one, so the instance and its wasm upload persist on testnet after
//! the test finishes. That's inherent to testing a real deploy path; use a
//! dedicated, disposable identity rather than one you rely on for other
//! testnet work. The *data* the test writes is cleaned up at the
//! application level: the attestation it creates is issued with a short
//! (120s) TTL and is explicitly revoked as the test's final step, so
//! nothing is left in an active/valid state.

extern crate std;

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::string::{String, ToString};
use std::vec::Vec;

const NETWORK: &str = "testnet";
const ATTESTATION_TYPE: &str = "testnet_e2e";
const TTL_SECONDS: &str = "120";

/// 32 bytes, hex-encoded, standing in for a sha256 payload hash. The
/// contract never inspects its content -- only that it's 32 bytes -- so a
/// fixed pattern is fine for an integration smoke test.
fn payload_hash_hex() -> String {
    "11".repeat(32)
}

fn stellar_bin() -> String {
    env::var("STELLAR_CLI_BIN").unwrap_or_else(|_| "stellar".to_string())
}

fn source_identity() -> String {
    env::var("ANCHORKIT_TESTNET_SOURCE").unwrap_or_else(|_| {
        std::panic!(
            "ANCHORKIT_TESTNET_SOURCE is not set. This test needs a funded testnet identity \
             name (see the module docs on src/testnet_integration_tests.rs for setup)."
        )
    })
}

fn wasm_path() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set by cargo when running tests");
    // The path-joining itself is covered by always-on tests in
    // `wasm_artifact_path.rs` (not gated behind this module's
    // `testnet-integration` feature), so it runs in CI on every OS,
    // including Windows.
    let path = crate::wasm_artifact_path::wasm_artifact_path(&manifest_dir);
    if !path.exists() {
        std::panic!(
            "wasm artifact not found at {}. Build it first: \
             `cargo build --target wasm32v1-none --release`.",
            path.display()
        );
    }
    path
}

/// Runs `stellar <args>`, panicking with stdout/stderr on a non-zero exit
/// so a failure here reads as a normal test failure instead of a hang.
fn run_stellar(args: &[&str]) -> String {
    let bin = stellar_bin();
    let output = Command::new(&bin).args(args).output().unwrap_or_else(|e| {
        std::panic!(
            "failed to execute `{bin}`: {e}. Is the Stellar CLI installed and on PATH? \
             See the module docs on src/testnet_integration_tests.rs."
        )
    });

    if !output.status.success() {
        std::panic!(
            "`{bin} {}` failed (exit {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            args.join(" "),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn invoke(contract_id: &str, source: &str, method: &str, method_args: &[&str]) -> String {
    let mut args: Vec<&str> = std::vec![
        "contract",
        "invoke",
        "--id",
        contract_id,
        "--source",
        source,
        "--network",
        NETWORK,
        "--",
        method,
    ];
    args.extend_from_slice(method_args);
    run_stellar(&args)
}

/// Deploys a fresh contract instance and drives a full
/// `attest` -> `get_attestation` -> `revoke` cycle against it on real
/// Stellar testnet, using the Stellar CLI for every signed/submitted
/// transaction.
#[test]
fn attest_get_revoke_full_cycle_on_testnet() {
    let source = source_identity();
    let wasm = wasm_path();
    let wasm_str = wasm.to_str().expect("wasm path is valid UTF-8");

    let address = run_stellar(&["keys", "address", &source]);
    std::println!("Using testnet identity `{source}` ({address})");

    let contract_id = run_stellar(&[
        "contract",
        "deploy",
        "--wasm",
        wasm_str,
        "--source",
        &source,
        "--network",
        NETWORK,
    ]);
    std::println!(
        "Deployed contract {contract_id} on {NETWORK} -- this instance is not deletable \
         and will remain on testnet after this test finishes."
    );

    // Setup: initialize the instance and allow-list our own address as the
    // sole attestor. Both calls are auth'd by `source`, which the CLI signs
    // automatically since it matches `--source`.
    invoke(&contract_id, &source, "initialize", &["--admin", &address]);
    invoke(
        &contract_id,
        &source,
        "add_attestor",
        &["--attestor", &address],
    );

    let hash = payload_hash_hex();

    // attest: use the same address as attestor and subject -- nothing in
    // the contract requires them to differ, and it avoids provisioning a
    // second funded identity for a role that never needs to sign anything.
    invoke(
        &contract_id,
        &source,
        "attest",
        &[
            "--attestor",
            &address,
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
            "--payload_hash",
            &hash,
            "--ttl_seconds",
            TTL_SECONDS,
        ],
    );

    // get_attestation: confirm it landed, active, with the hash we sent.
    let stored = invoke(
        &contract_id,
        &source,
        "get_attestation",
        &[
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
        ],
    );
    assert!(
        stored.contains("Active"),
        "expected freshly attested record to be Active, got: {stored}"
    );
    assert!(
        stored.contains(&hash),
        "expected stored payload_hash to match what was submitted, got: {stored}"
    );

    let is_valid = invoke(
        &contract_id,
        &source,
        "is_valid",
        &[
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
        ],
    );
    assert!(
        is_valid.contains("true"),
        "expected is_valid to report true, got: {is_valid}"
    );

    // revoke: this is also the cleanup step -- it drives the attestation to
    // its terminal Revoked state so no active/valid state is left behind
    // beyond the (undeletable) contract instance itself.
    invoke(
        &contract_id,
        &source,
        "revoke",
        &[
            "--caller",
            &address,
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
        ],
    );

    let revoked = invoke(
        &contract_id,
        &source,
        "get_attestation",
        &[
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
        ],
    );
    assert!(
        revoked.contains("Revoked"),
        "expected attestation to be Revoked after revoke, got: {revoked}"
    );

    let is_valid_after_revoke = invoke(
        &contract_id,
        &source,
        "is_valid",
        &[
            "--subject",
            &address,
            "--attestation_type",
            ATTESTATION_TYPE,
        ],
    );
    assert!(
        is_valid_after_revoke.contains("false"),
        "expected is_valid to report false after revoke, got: {is_valid_after_revoke}"
    );

    std::println!("Full attest -> get_attestation -> revoke cycle passed on {NETWORK}.");
}
