# Contributing Guidelines

Thank you for contributing to AnchorKit!

## Local Development Requirements

Before running tests or submitting a Pull Request, ensure your environment meets the toolchain requirements.

### Toolchain versions

There are two version floors in play:

- **Rust 1.84** — the minimum for `wasm32v1-none` target support (when the target was stabilised).
- **Rust 1.91** — the effective minimum with the current dependency set, since `soroban-spec-rust 26.1.1` requires it.

In practice, just keep your toolchain current:

```bash
rustup update
```

A `rust-toolchain.toml` at the repo root pins the channel to `stable` and auto-installs the `wasm32v1-none` target, so `rustup` picks up the right toolchain automatically when you run any `cargo` command in this repo — no manual `rustup override` needed.

If you want to install the target explicitly:

```bash
rustup target add wasm32v1-none
```

> **Windows users:** see [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for toolchain installation via PowerShell, plus known gotchas (path length limits, line endings, and a `cargo test` limitation on the `gnu` toolchain).

### Commands

```bash
# Run the unit test suite (native target)
cargo test

# Build the deployable contract (WASM target)
cargo build --target wasm32v1-none --release

# Run the gated allow-list stress benchmark (excluded from normal `cargo test`)
cargo test --release --features stress-tests allow_list_scaling -- --nocapture
```

### Windows note

`cargo test` fails on the `x86_64-pc-windows-gnu` toolchain with an `export ordinal too large` linker error — a PE/COFF format limitation triggered by the `soroban-sdk` test harness, not a bug in this project. It does **not** affect the `wasm32v1-none` build, and CI's Windows runner uses the unaffected `msvc` toolchain by default. See [`docs/platform-quirks.md`](docs/platform-quirks.md) for the full analysis, and [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for local workarounds.

## Pre-commit hooks (recommended)

The repo ships a `.pre-commit-config.yaml` that runs `cargo fmt --check` and
`cargo clippy` before every commit. These are the same checks enforced in CI,
so you catch formatting and lint issues locally before a PR is opened.

### Install (one-time)

```bash
pip install pre-commit
pre-commit install
```

That's it. The hooks run automatically on `git commit` from that point on.

### What runs on each commit

| Hook | Command | Speed |
|---|---|---|
| `cargo-fmt` | `cargo fmt --check` on staged `.rs` files | Fast — rustfmt only parses, never compiles |
| `cargo-clippy` | `cargo clippy --workspace --no-deps -- -D warnings` | Incremental — slow on a cold build, fast once Cargo's cache is warm |

`-D warnings` promotes every Clippy warning to an error, matching the CI
behaviour exactly so there are no surprises at PR time.

### Run manually

```bash
# Check all files in the repo (useful before opening a PR)
pre-commit run --all-files

# Run a single hook
pre-commit run cargo-fmt --all-files
pre-commit run cargo-clippy --all-files
```

### Skip hooks when needed

If you need to commit work-in-progress that isn't yet lint-clean:

```bash
git commit --no-verify -m "wip: ..."
```

Use this sparingly — CI will still enforce the same checks.

> **Windows users:** `pre-commit` requires Python. Run the commands above in
> Git Bash or WSL. See [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for details.

## Local CI Preflight Check

To verify that your toolchain and target are correctly set up before submitting a PR, run the preflight script for your shell:

| Shell | Command |
|---|---|
| bash (Linux, macOS, Git Bash, WSL) | `./scripts/ci_preflight_check.sh` |
| PowerShell (Windows) | `./scripts/ci_preflight_check.ps1` |

Both scripts run the same checks — installed Rust version against the minimum floor, and that the `wasm32v1-none` target is installed — and exit non-zero with the same error messages on failure, so either is a valid local signal regardless of which shell you use. CI runs the bash variant on Linux/macOS and the PowerShell variant on Windows (`.github/workflows/ci.yml`), so both are exercised on every push/PR. See [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for PowerShell-specific notes.

## Test Conventions

- Tests live alongside the code they cover (e.g. `admin_tests.rs`, `attest_tests.rs`, `attestor_tests.rs`, `pause_tests.rs`, `revoke_tests.rs`) and use `soroban_sdk`'s `testutils` feature, already wired up under `[dev-dependencies]` in `Cargo.toml`.
- Each test has a corresponding fixture under `test_snapshots/<area>_tests/<test_name>.1.json`. When you add or rename a test, make sure a matching snapshot is generated and committed — reviewers will expect the snapshot directory to reflect the test suite 1:1.
- Name tests descriptively for what they assert, not just what they call (e.g. `cannot_revoke_twice`, `pause_blocks_attest_and_unpause_restores_it`) — this keeps `test_snapshots/` self-documenting.
- Benchmarks (like `batch_gas_benchmark`) and other expensive/gated tests should use a Cargo feature flag (see `stress-tests` in `Cargo.toml`) so they don't run as part of a normal `cargo test`.
- Run the full suite (`cargo test`) before opening a PR, and re-run it after any rebase.

## Code Quality Expectations

- **Test coverage:** new functionality should ship with tests covering both the success path and the realistic failure/edge cases (unauthorized caller, paused state, missing/expired data, double actions, etc.) — follow the pattern already established in the `*_tests.rs` files for the module you're touching.
- **Clippy cleanliness:** run `cargo clippy` locally and resolve warnings before opening a PR:

  ```bash
  cargo clippy --all-targets --all-features
  ```

  Note: clippy is not currently enforced as a required CI check (see `.github/workflows/ci.yml`), so this is a contributor expectation rather than an automated gate for now — please still run it. If you believe a specific lint should be allowed for good reason, use a targeted `#[allow(...)]` with a short comment explaining why, rather than suppressing it project-wide.

## Pull Request Expectations

- Keep PRs focused on a single issue/concern where possible; this makes review and the resulting `test_snapshots/` diff easier to reason about.
- Reference the issue you're addressing in the PR description (e.g. `Closes #123`).
- Ensure `cargo test` and `./scripts/ci_preflight_check.sh` both pass locally before requesting review.
- The CI matrix (`.github/workflows/ci.yml`) runs on Linux, macOS, and Windows for every push/PR to `main` — a green run on your branch is expected before merge.
- Update relevant documentation (`README.md`, `WINDOWS_SETUP.md`, `docs/platform-quirks.md`, etc.) alongside code changes when behavior, setup steps, or known limitations change.
