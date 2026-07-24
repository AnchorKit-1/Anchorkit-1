# AnchorKit-1

AnchorKit is a Soroban-native toolkit for anchoring off-chain attestations to
Stellar. It enables smart contracts to verify real-world events such as KYC
approvals, payment confirmations, and signed claims in a trust-minimized way.

An attestor (an off-chain authority a subject already trusts, e.g. a KYC
provider or a payment processor) submits a sha256 fingerprint of an
off-chain decision to the contract. Any other contract or client can then
check on-chain whether that fingerprint is present, active, and unexpired,
without the anchor ever having to store or expose the underlying payload.

## Status: core contract only
 
This repository currently implements the **on-chain Soroban contract** —
attestation lifecycle, attestor allow-listing, admin control, pause/circuit
breaker, and payload hashing — with a full unit test suite. It intentionally
does **not** yet include the off-chain SDK layer (SEP-10 auth, SEP-6
deposit/withdraw flows, rate limiting/retry helpers, anchor discovery), the
React/Storybook UI components, or the docs site that a complete toolkit
would ship. Those are tracked as GitHub issues (see below) rather than
stubbed out in-tree.

A CLI (`cli/`, see [`docs/cli.md`](docs/cli.md)) has begun landing
alongside the contract, starting with `anchorkit playground` for calling
read-only methods against a deployed instance; more subcommands are tracked
as issues.

## Contract surface

| Method | Description |
|---|---|
| `initialize(admin)` | One-time setup; `admin` must authorize the call. |
| `get_admin()` / `set_admin(new_admin)` | Read/transfer contract administration. |
| `pause()` / `unpause()` / `is_paused()` | Admin circuit breaker; blocks `attest`/`revoke` while active, reads still work. |
| `add_attestor(attestor)` / `remove_attestor(attestor)` / `is_attestor(attestor)` | Admin-managed allow-list of addresses permitted to attest. |
| `renew_attestor(attestor)` | Re-touches an already-registered attestor's persistent storage TTL, admin-only. See "Storage rent & TTL" below for why this exists. |
| `attest(attestor, subject, attestation_type, payload_hash, ttl_seconds)` | Anchors a sha256 payload hash on-chain for `subject` under `attestation_type`. Requires `attestor` to be allow-listed and to authorize the call. |
| `attest_batch(attestor, entries)` | Anchors multiple attestations from one attestor in a single call; each entry is a `(subject, attestation_type, payload_hash, ttl_seconds)` tuple. Atomic — one invalid entry fails the whole batch — and emits one `Attested` event per entry, same as `attest`. See "Batch attestation gas amortization" below for when it's worth using. |
| `get_attestation(subject, attestation_type)` | Fetches a stored attestation, or `AttestationNotFound`. |
| `is_valid(subject, attestation_type)` | `true` iff an attestation exists, is active, and hasn't expired. |
| `revoke(caller, subject, attestation_type)` | Revokes an attestation; `caller` must be the admin or the original attestor. |
| `renew_attestation(caller, subject, attestation_type)` | Re-touches an attestation's persistent storage TTL without changing its content; `caller` must be the admin or the original attestor. See "Storage rent & TTL" below. |
| `get_attestation_count()` | Running count of attestations ever submitted. |

Supporting modules:
- `hash` — `compute_payload_hash` / `verify_payload_hash`, thin wrappers over the host's sha256.
- `domain_validator` — syntactic validation of anchor domain strings (the kind of hostname an attestor would publish a `stellar.toml` under).

## Batch attestation gas amortization

`attest` pays a fixed per-call cost (auth check, allow-list lookup, pause
check) on top of the variable cost of writing one attestation.
`attest_batch` pays that fixed cost once per call no matter how many entries
it carries, so its per-entry cost drops as the batch grows. Measured with
`soroban_sdk`'s CPU instruction metering (`src/batch_gas_benchmark.rs`,
`cargo test batch_gas_benchmark -- --nocapture`):

| Batch size | CPU / entry, individual `attest` | CPU / entry, `attest_batch` | Savings |
|---:|---:|---:|---:|
| 1  | 153,249 | 155,785 | -1.7% (batching overhead costs more than there is to amortize) |
| 2  | 157,664 | 117,415 | 25.5% |
| 5  | 163,453 |  95,316 | 41.7% |
| 10 | 172,847 |  91,813 | 46.9% |
| 20 | 186,799 |  95,535 | 48.9% |
| 40 | 212,585 | 107,091 | 49.6% |

Savings climb fast up to a batch of ~10 (46.9%) and then flatten out,
approaching but never reaching 50% — that's the fixed-cost share of a single
`attest` call. Batching more than ~10-20 entries at once buys very little
extra per-entry savings.

## Storage rent & TTL

Persistent storage TTL bumps (`extend_ttl`) are sized per entry rather than
a single flat constant: attestations buy a rent window proportional to
their own `ttl_seconds` (floored at 30 days, capped at Soroban's network-wide
`max_entry_ttl`), and attestor allow-list entries buy that same network
maximum up front since they have no natural expiry. This cuts rent 51-92%
for the common days-to-months attestation case relative to the old flat
365-day window, with no change in safety.

Soroban clamps `extend_ttl` to `max_entry_ttl` (~365 days) regardless of what's
requested, so a single call can never cover a multi-year attestation or an
indefinitely-long-lived attestor. `renew_attestation` and `renew_attestor`
exist to re-touch those entries' TTL well before that ceiling, so long-lived
data doesn't archive out from under a still-valid attestation or attestor.
See [`docs/storage-rent-cost-analysis.md`](docs/storage-rent-cost-analysis.md)
for the full worked cost comparison and rationale.

**Hard ceiling, not just a cost curve:** each attestation occupies two
ledger footprint slots (its data entry and its TTL/rent entry), and Soroban
caps total footprint entries per invocation (100 under the test host's
mainnet-equivalent default). That puts a hard wall on batch size regardless
of gas budget: **47 entries succeeds, 48 fails** with a host-level
`"total footprint ledger entries: 102 > 100"` trap that `attest_batch`
can't turn into a graceful `Error` — it's enforced by the host below the
contract. Real transactions carry additional footprint of their own (fee
bump, source account, etc.), so treat 47 as an optimistic upper bound, not
a safe one; callers should batch in chunks well under it (the ~10-20 range
above captures nearly all the gas savings anyway). See
`batch_gas_benchmark::attest_batch_stays_under_the_ledger_write_ceiling` and
`batch_gas_benchmark::batch_amortizes_fixed_overhead`.

## Building and testing

> For full contributor guidelines (test conventions, clippy expectations, PR checklist), see [CONTRIBUTING.md](CONTRIBUTING.md).

### Minimum toolchain

There are two version floors in play:

- **Rust 1.84** — when `wasm32v1-none` was stabilised (the target itself becomes available)
- **Rust 1.91** — effective minimum with the current dependency set (`soroban-spec-rust 26.1.1` requires it)

Running `rustup update` (which gives you current stable, well above both) is the simplest path. A `rust-toolchain.toml` at the repo root pins the channel to `stable` and auto-installs the `wasm32v1-none` target via `rustup`.

Verified on (CI matrix, `.github/workflows/ci.yml`, runs on every push/PR to `main`):
- Linux (ubuntu-latest, Rust stable ≥ 1.84) — full build + test suite ✅
- macOS (macos-latest, Rust stable ≥ 1.84) — full build + test suite ✅
- Windows (windows-latest, `x86_64-pc-windows-msvc`, Rust stable ≥ 1.84) — full build + test suite ✅

### Commands

```sh
# Install the WASM target if not already present
# (rust-toolchain.toml handles this automatically)
rustup target add wasm32v1-none

# Run the unit test suite
cargo test

# Build the deployable contract
cargo build --target wasm32v1-none --release
```

### Windows note

`cargo test` passes on Windows in CI (`windows-latest`'s default
`x86_64-pc-windows-msvc` toolchain). A local Windows install using the
`x86_64-pc-windows-gnu`/MinGW toolchain instead will hit
`ld: error: export ordinal too large`, because the soroban-sdk test harness
generates more DLL exports than the PE/COFF format allows under that linker —
this is specific to the GNU toolchain, not Windows generally. Switch to the
MSVC toolchain or run the test suite on Linux/macOS/WSL instead. See
[`docs/platform-quirks.md`](docs/platform-quirks.md) for the full analysis
(#WIN-1).

## Documentation

- [CLI](docs/cli.md) — `anchorkit` subcommands, including a sample `playground` session.
- [Revocation notification design](docs/revocation-notification-design.md) — proposed payload and delivery semantics for notifying subscribers when an attestation should be revoked.
- [Contract wasm size](docs/wasm-size.md) — before/after size profiling and what `strip = true` and dropping an unused dependency bought us.

## Roadmap

The gap between this ~50%-built core and a complete toolkit is tracked as
GitHub issues, labeled by area and difficulty. Expect issues covering:
off-chain SEP-10/SEP-6 flows, rate limiting and retry/backoff, anchor
discovery and health scoring, attestation pagination and audit logging,
replay-window protection, further CLI subcommands, UI components, and
end-to-end docs.

## License

MIT
