# Windows Setup Guide

This guide walks through setting up the Rust/wasm toolchain needed to build and test AnchorKit-1 on Windows.

## Prerequisites

- **Git for Windows** — [git-scm.com/download/win](https://git-scm.com/download/win)
- **Visual Studio Build Tools** (required for the `msvc` Rust toolchain) — install the "Desktop development with C++" workload from the [Visual Studio Build Tools installer](https://visualstudio.microsoft.com/visual-cpp-build-tools/)

## 1. Install rustup

Download and run `rustup-init.exe` from [rustup.rs](https://rustup.rs/), or install via PowerShell:

```powershell
winget install Rustlang.Rustup
```

When prompted, choose the default installation (`stable-x86_64-pc-windows-msvc` host toolchain). After installation, restart your terminal and verify:

```powershell
rustc --version
cargo --version
rustup --version
```

## 2. Add the `wasm32v1-none` target

```powershell
rustup target add wasm32v1-none
```

Confirm it installed correctly:

```powershell
rustup target list --installed
```

You should see `wasm32v1-none` in the output.

## 3. Clone and build the project

```powershell
git clone https://github.com/AnchorKit-1/Anchorkit-1.git
cd Anchorkit-1
cargo build --release --target wasm32v1-none
```

This project is a Cargo workspace with two members (the root `anchorkit` contract crate and `webhook-sdk`), so `cargo build`/`cargo test` from the repo root will operate on both by default.

## 4. Running tests

```powershell
cargo test
```

Tests use `soroban-sdk`'s `testutils` feature (already wired up via `[dev-dependencies]` in `Cargo.toml`) and run on your **host** target, not `wasm32v1-none` — you don't need to pass `--target` for `cargo test`.

> **Known limitation:** `cargo test` is currently skipped on Windows in CI. See [Known Windows Gotchas](#cargo-test-is-not-run-on-windows-in-ci) below before relying on it as your primary test signal.

To run the gated allow-list stress benchmark (skipped by default):

```powershell
cargo test --release --features stress-tests allow_list_scaling -- --nocapture
```

If you're working from PowerShell (not WSL or Git Bash), note the following PowerShell-specific points:

- Use `;` instead of `&&` to chain commands, e.g. `cargo build; cargo test` (PowerShell doesn't support `&&` the same way as bash on older Windows PowerShell versions; PowerShell 7+ does support it, but `;` works everywhere).
- If a script in `scripts/` only has a `.sh` file (no `.ps1` counterpart), run it via Git Bash or WSL — PowerShell cannot execute shell scripts directly. Scripts with a `.ps1` counterpart (e.g. `ci_preflight_check`) run natively — see [Running the CI preflight check](#running-the-ci-preflight-check) below.
- Environment variables are set differently: use `$env:VAR_NAME = "value"` instead of `export VAR_NAME=value`.

## Known Windows Gotchas

### `cargo test` is not run on Windows in CI

CI (`.github/workflows/ci.yml`) skips `cargo test` entirely on Windows runners:

```yaml
- name: Run Cargo Tests (Linux / macOS only)
  if: runner.os != 'Windows'
  run: cargo test
```

This is because `cargo test` fails on the `x86_64-pc-windows-gnu` toolchain with
`ld: error: export ordinal too large` — the `soroban-sdk` test harness generates
more DLL exports than the PE/COFF format allows. The WASM contract build
(`--target wasm32v1-none`) is unaffected. See
[`docs/platform-quirks.md`](docs/platform-quirks.md) and issue #WIN-1 for the
full analysis.

This guide recommends the `msvc` toolchain (see [Prerequisites](#prerequisites)),
and it has **not yet been confirmed** whether this failure also affects `msvc` —
only `gnu` has been verified to fail. Until that's confirmed, don't treat a
local `cargo test` pass on Windows as equivalent to CI: push your branch and
let the Linux/macOS CI jobs validate your changes, or run tests via WSL if you
want a same-machine signal that matches CI.

### Path length limits (MAX_PATH)

Windows historically limits paths to 260 characters, which can cause build failures with deeply nested `target/` directories (common with Cargo's dependency trees). To fix:

1. Enable long path support (requires admin PowerShell):

   ```powershell
   New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
     -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force
   ```

2. Also enable long paths in Git:

   ```powershell
   git config --system core.longpaths true
   ```

3. Clone the repo close to your drive root (e.g. `C:\dev\Anchorkit-1`) rather than deep inside nested folders, to reduce the chance of hitting the limit even with long paths enabled.

### Line endings (CRLF vs LF)

Windows Git defaults to converting `LF` to `CRLF` on checkout, which can cause noisy diffs or test snapshot mismatches (relevant here since this project uses `test_snapshots/*.json` fixtures).

Set Git to preserve `LF` line endings for this repo:

```powershell
git config --global core.autocrlf input
```

If you've already cloned the repo before setting this, re-normalize line endings:

```powershell
git rm --cached -r .
git reset --hard
```

Check that the repo's `.gitattributes` (if present) enforces `LF` for source and JSON files; if it's missing one, consider adding:

```
* text=auto eol=lf
```

### Slow release builds

The `[profile.release]` settings in `Cargo.toml` (`lto = true`, `codegen-units = 1`, `opt-level = "z"`) are tuned for a small wasm binary, not fast compilation. A `cargo build --release --target wasm32v1-none` can take noticeably longer on Windows than a debug build — this is expected and not a Windows-specific bug, but combined with antivirus scanning (below) it can feel much slower than on macOS/Linux.

### Antivirus / Windows Defender slowdowns

Real-time scanning can significantly slow down `cargo build`/`cargo test` due to the large number of files Cargo writes to `target/`. Consider adding an exclusion for your project's `target/` directory in Windows Security settings if build times seem unusually slow.

### Symlinks

Some Cargo/wasm tooling relies on symlinks, which require either Developer Mode enabled (Settings → Update & Security → For Developers) or running your terminal as Administrator. Without one of these, symlink creation will fail silently or with a permissions error.

## Verifying your setup

Run the full test suite to confirm everything is working:

```powershell
cargo test
```

Tests under `test_snapshots/` (admin, attest, attestor, pause, revoke, smoke) should pass locally with no `wasm32v1-none` target or toolchain errors on `msvc`. Note that this local run isn't currently mirrored by CI on Windows — see [`cargo test` is not run on Windows in CI](#cargo-test-is-not-run-on-windows-in-ci) above.

## Running the CI preflight check

Before submitting a PR, the project asks you to run the preflight check. On Windows, run the native PowerShell version — it checks the same Rust version and `wasm32v1-none` target requirements as the bash script, with matching exit codes and messages, and is what CI's Windows job runs:

```powershell
./scripts/ci_preflight_check.ps1
```

If PowerShell blocks the script with an "execution of scripts is disabled" error, your execution policy needs loosening for locally-authored scripts:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

You can still run the bash version (`scripts/ci_preflight_check.sh`) via Git Bash or WSL if you prefer:

```bash
./scripts/ci_preflight_check.sh      # Git Bash
wsl bash scripts/ci_preflight_check.sh  # WSL
```

If the bash script fails immediately with something like `bad interpreter` or `\r: command not found`, it's almost always the CRLF line-ending issue described above — make sure `core.autocrlf` is set to `input` and re-clone or re-normalize the repo.
