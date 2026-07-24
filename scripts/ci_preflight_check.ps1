<#
.SYNOPSIS
    PowerShell equivalent of scripts/ci_preflight_check.sh.

.DESCRIPTION
    Verifies the Rust toolchain and wasm32v1-none target are present and at
    the minimum required version before running the full suite. Mirrors the
    bash script's checks, exit codes, and output messages so either can be
    used interchangeably in CI or locally.

    - wasm32v1-none target support: requires Rust >= 1.84
    - soroban-spec-rust 26.1.1 (transitive dep): requires Rust >= 1.91
    The effective floor is therefore 1.84 for the *target* but 1.91 for a
    full build with current dependencies. This enforces 1.84 here so the
    check stays aligned with the documented requirement; the build itself
    will fail with a clear Cargo error if the dep floor is not met.
#>

$ErrorActionPreference = 'Stop'

$MinRustVersion = [version]'1.84.0'

Write-Host '==> Running CI Preflight Check...'

# 1. Check if rustc is installed
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Host 'ERROR: Rust compiler (rustc) is not found.'
    exit 1
}

$InstalledFullVersion = (rustc --version)
Write-Host "Found Rust version: $InstalledFullVersion"

# Extract semantic version number (e.g., "1.97.1")
$InstalledVersionString = ($InstalledFullVersion -split '\s+')[1]

try {
    $InstalledVersion = [version]$InstalledVersionString
} catch {
    Write-Host "ERROR: Could not parse Rust version from '$InstalledFullVersion'."
    exit 1
}

if ($InstalledVersion -lt $MinRustVersion) {
    Write-Host "ERROR: Rust version $InstalledVersionString is older than the required minimum version $MinRustVersion."
    Write-Host 'Please update Rust using: rustup update'
    exit 1
}

# 2. Check if rustup is installed
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Host 'ERROR: rustup is required to manage toolchains/targets.'
    exit 1
}

# 3. Verify that the wasm32v1-none target is installed
Write-Host 'Checking for target: wasm32v1-none...'
$TargetList = (rustup target list)
if ($TargetList -match 'wasm32v1-none \(installed\)') {
    Write-Host 'SUCCESS: Target wasm32v1-none is installed.'
} else {
    Write-Host "ERROR: Required target 'wasm32v1-none' is missing."
    Write-Host 'Please install it by running: rustup target add wasm32v1-none'
    exit 1
}

Write-Host '==> CI Preflight Check passed successfully!'
exit 0
