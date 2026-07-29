# Fuzzing Guide

This document describes how to run and manage the AnchorKit fuzzing infrastructure.

## Overview

AnchorKit includes fuzzing harnesses for the `attest` and `revoke` contract functions' argument decoding paths. These harnesses help catch panics and crashes on malformed or adversarial input.

### Fuzz Targets

- **`fuzz_attest_args`** — Tests the argument decoding for the `attest` function:
  - Takes arbitrary bytes as input
  - Tests construction of `attestor` (Address), `subject` (Address), `attestation_type` (Symbol), `payload_hash` (BytesN<32>), and `ttl_seconds` (u64)
  - Ensures no panics on malformed input

- **`fuzz_revoke_args`** — Tests the argument decoding for the `revoke` function:
  - Takes arbitrary bytes as input
  - Tests construction of `caller` (Address), `subject` (Address), and `attestation_type` (Symbol)
  - Ensures no panics on malformed input

## Running Fuzzing

### Prerequisites

Install the Rust nightly toolchain and cargo-fuzz:

```bash
rustup install nightly
cargo install cargo-fuzz
```

### Short Fuzzing Run (Local Testing)

Run a brief fuzzing session (useful for quick validation during development):

```bash
# Fuzz attest for 60 seconds
cd fuzz && cargo fuzz run fuzz_attest_args -- -max_total_time=60

# Fuzz revoke for 60 seconds
cd fuzz && cargo fuzz run fuzz_revoke_args -- -max_total_time=60
```

### Extended Fuzzing Run (Local)

Run longer fuzzing sessions to find deeper bugs:

```bash
# Fuzz attest for 1 hour
cd fuzz && cargo fuzz run fuzz_attest_args -- -max_total_time=3600

# Fuzz revoke for 1 hour
cd fuzz && cargo fuzz run fuzz_revoke_args -- -max_total_time=3600
```

### CI Integration

- **Short runs (60 seconds each)** execute on every PR and push to `main` in the `CI` workflow (see `.github/workflows/ci.yml`)
- **Long runs (600 seconds each)** execute weekly (Mondays at 2 AM UTC) or on manual trigger via the `Fuzzing (long-run)` workflow (see `.github/workflows/fuzz-long.yml`)

To manually trigger the long-run workflow:
1. Go to GitHub Actions → "Fuzzing (long-run)"
2. Click "Run workflow" → "Run workflow"

## Handling Crashes

When the fuzzer finds a crash, it creates a test case file and crashes:

```
thread 'main' panicked at 'fuzz target exited unexpectedly', src/lib.rs:...
```

The crashing input will be saved to `fuzz/artifacts/<target>/<crash-hash>`.

### Minimizing a Crash

Use libfuzzer's `-minimize_crash=1` flag to produce the smallest input that still crashes:

```bash
cd fuzz && cargo fuzz run fuzz_attest_args -- -minimize_crash=fuzz/artifacts/fuzz_attest_args/crash-abc123 -max_total_time=60
```

This produces a minimized crash case that's easier to understand and convert into a regression test.

### Adding a Regression Test

Once you've found and minimized a crash:

1. **Understand the root cause** by examining the minimized input and crash output
2. **Create a new test** in `src/<function>_tests.rs` that exercises the same code path:

Example: If a crash is found in `attest` decoding with malformed input:

```rust
#[test]
#[should_panic]  // or assert with catch_unwind if expecting a panic
fn attest_handles_malformed_input_gracefully() {
    let s = setup();
    let attestor = Address::generate(&s.env);
    // ... construct the malformed input case
    // Ensure the contract either handles it gracefully or returns a clean error
}
```

3. **Run the test** to confirm it reproduces the issue:
```bash
cargo test attest_handles_malformed_input_gracefully
```

4. **Fix the root cause** in the contract code
5. **Verify the fix** by re-running the test and the fuzzer
6. **Commit the test** as part of the fix PR

## Interpreting Fuzzer Output

While running, the fuzzer displays statistics:

```
#7         INITED cov: 1234 ft: 5678 corp: 42 lim: 4096 exec/s: 1000 rss: 64Mb L: 256/4096 MS: 1 CrossOver-
```

- **`#7`** — Iteration count
- **`cov: 1234`** — Code coverage (unique edges hit)
- **`corp: 42`** — Corpus size (number of interesting inputs)
- **`exec/s`** — Iterations per second
- **`rss`** — Memory usage
- **`L`** — Length of input being tested

The fuzzer will continue until it finds a crash, hits the time limit, or hits a resource limit.

## Fuzz Target Development

To add a new fuzz target:

1. Create a new file in `fuzz/fuzz_targets/` named `fuzz_<function>.rs`
2. Implement the `fuzz_target!` macro with arbitrary input bytes
3. Add a `[[bin]]` entry to `fuzz/Cargo.toml`
4. Test locally: `cd fuzz && cargo fuzz run fuzz_<function>`

## Best Practices

- **Review fuzzer crashes carefully** — Not all crashes are bugs; some may be intentional panics on invalid input. Determine if the input is allowed by the contract specification.
- **Minimize first** — Always minimize crashes before investigating; smaller inputs are easier to understand.
- **Add regression tests** — After fixing a crash, commit a regression test to prevent re-introduction.
- **Run fuzzing regularly** — Long-run fuzzing finds bugs that short runs miss.
- **Commit fuzz corpus changes** — If interesting test cases are found, consider committing them to seed the fuzzer for future runs.

## References

- [cargo-fuzz documentation](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [libfuzzer documentation](https://llvm.org/docs/LibFuzzer/)
- [OWASP - Fuzzing](https://owasp.org/www-community/attacks/Fuzzing)
