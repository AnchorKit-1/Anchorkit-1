//! Criterion benchmarks for the contract's hot paths: `attest`,
//! `get_attestation`, and `is_valid`. These are the three calls a typical
//! integration makes on every request (write an attestation, then read it
//! back or check its validity), so their cost is what most directly shapes
//! perceived latency.
//!
//! Benches run on the native target (never wasm32v1-none) under
//! `[profile.bench]` in Cargo.toml, which is tuned for fastest host
//! execution rather than the on-chain build's wasm-size budget.
//!
//! Run with: cargo bench

use criterion::{criterion_group, criterion_main, Criterion};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Env, Symbol};

use anchorkit::{compute_payload_hash, AnchorKitContract, AnchorKitContractClient};

const ONE_DAY: u64 = 24 * 60 * 60;

/// Spins up a fresh contract instance with mocked auth, an initialized
/// admin, and one registered attestor -- everything `attest` needs to
/// succeed, mirroring `src/test_util.rs`'s `setup()` plus attestor
/// registration.
fn setup() -> (Env, AnchorKitContractClient<'static>, Address, Address, Symbol) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let attestor = Address::generate(&env);
    client.add_attestor(&attestor);

    let subject = Address::generate(&env);
    let kind = Symbol::new(&env, "kyc_approved");

    (env, client, attestor, subject, kind)
}

fn bench_attest(c: &mut Criterion) {
    let (env, client, attestor, subject, kind) = setup();
    let hash = compute_payload_hash(&env, &Bytes::from_slice(&env, b"kyc-decision:approved"));

    c.bench_function("attest", |b| {
        b.iter(|| {
            client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);
        });
    });
}

fn bench_get_attestation(c: &mut Criterion) {
    let (env, client, attestor, subject, kind) = setup();
    let hash = compute_payload_hash(&env, &Bytes::from_slice(&env, b"kyc-decision:approved"));
    client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    c.bench_function("get_attestation", |b| {
        b.iter(|| {
            client.get_attestation(&subject, &kind);
        });
    });
}

fn bench_is_valid(c: &mut Criterion) {
    let (env, client, attestor, subject, kind) = setup();
    let hash = compute_payload_hash(&env, &Bytes::from_slice(&env, b"kyc-decision:approved"));
    client.attest(&attestor, &subject, &kind, &hash, &ONE_DAY);

    c.bench_function("is_valid", |b| {
        b.iter(|| {
            client.is_valid(&subject, &kind);
        });
    });
}

criterion_group!(benches, bench_attest, bench_get_attestation, bench_is_valid);
criterion_main!(benches);
