# Contributing Guidelines

Thank you for contributing to AnchorKit!

## Local Development Requirements

Before running tests or submitting a Pull Request, ensure your environment meets the toolchain requirements:

1. Rust installed (`rustc`, `cargo`, `rustup`).
2. Target `wasm32v1-none` added:

   ```bash
   rustup target add wasm32v1-none
   ```
   > **Windows users:** see [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for toolchain installation via PowerShell, plus known gotchas (path length limits, line endings).

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

To verify that your toolchain and target are correctly set up before submitting a PR, run the preflight script:

```bash
./scripts/ci_preflight_check.sh
```

>On Windows, run this via **Git Bash** or **WSL** — PowerShell cannot execute `.sh` scripts directly. See [WINDOWS_SETUP.md](./WINDOWS_SETUP.md) for details.