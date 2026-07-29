# Maximum TTL Validation Feature

## Overview

This feature implements admin-configurable maximum TTL (time-to-live) limits per attestation type, with sensible defaults, to prevent compromised or careless attestors from issuing arbitrarily long-lived attestations (e.g., "100-year" KYC approvals).

## Problem Statement

Previously, any attestor could submit an attestation with an arbitrarily long TTL with nothing on-chain to prevent it. This posed a security risk where a compromised attestor could grant indefinitely long validity to attestations, potentially undermining the attestation system's integrity.

## Solution

The feature adds:

1. **Default Maximum TTL**: A built-in default (1 year = 365 days in seconds) that applies automatically when no per-type override is configured.

2. **Per-Type Overrides**: Admins can set stricter (or more lenient) max TTLs for specific attestation types (e.g., "kyc" can have 1 year max, but "payment" can have 7 days max).

3. **Validation on Attestation**: The `attest` and `attest_batch` endpoints validate that the requested TTL doesn't exceed the configured maximum, rejecting with `ExceedsMaxTtl` if it does.

4. **Fallback Behavior**: If no per-type override is set, the default maximum TTL applies automatically.

## Implementation Details

### New Error Type

- `ExceedsMaxTtl` (error code 13): Returned when an attestation request exceeds the maximum allowed TTL.

### New Storage Keys (DataKey Variants)

- `MaxAttestationTtl(Symbol)`: Stores the per-type maximum TTL override.
- `DefaultMaxAttestationTtl`: Stores the default maximum TTL used as a fallback.

### Storage Constants

- `DEFAULT_MAX_ATTESTATION_TTL_SECONDS`: Set to 365 days in seconds (31,536,000 seconds).

### Storage Functions

- `get_max_attestation_ttl(env, attestation_type) -> u64`: Returns the effective max TTL for a type (per-type override if set, otherwise default).
- `set_default_max_attestation_ttl(env, max_ttl_seconds)`: Sets the default maximum TTL.
- `set_max_attestation_ttl(env, attestation_type, max_ttl_seconds)`: Sets a per-type maximum TTL override.

### Contract Functions

- `set_default_max_attestation_ttl(env, max_ttl_seconds) -> Result<(), Error>`: Admin-only. Sets the default maximum TTL. Must be greater than zero.
- `set_max_attestation_ttl(env, attestation_type, max_ttl_seconds) -> Result<(), Error>`: Admin-only. Sets a per-type maximum TTL. Must be greater than zero.
- `get_max_attestation_ttl(env, attestation_type) -> u64`: Query endpoint to get the effective maximum TTL for a given type.

### Events

- `DefaultMaxAttestationTtlChanged { max_ttl_seconds: u64 }`: Emitted when the default max TTL is updated.
- `MaxAttestationTtlChanged { attestation_type: Symbol, max_ttl_seconds: u64 }`: Emitted when a per-type max TTL is set or updated.

### Validation Logic

The `record_attestation` function (used by both `attest` and `attest_batch`) now:

1. Checks if TTL is zero (existing check) → rejects with `InvalidExpiration`.
2. Gets the effective max TTL for the attestation type using `storage::get_max_attestation_ttl()`.
3. Checks if the requested TTL exceeds the max → rejects with `ExceedsMaxTtl`.
4. If both checks pass, proceeds to store the attestation as normal.

## Acceptance Criteria

✅ **Default max TTL applies when no per-type override is set**: The `get_max_attestation_ttl` function returns `DEFAULT_MAX_ATTESTATION_TTL_SECONDS` (1 year) if no override is configured.

✅ **attest rejects TTLs exceeding the configured max with a specific error**: The `record_attestation` function validates the TTL and returns `Error::ExceedsMaxTtl` if exceeded.

✅ **Tests cover default, per-type override, and rejection paths**: See `src/max_ttl_tests.rs` with 17 comprehensive test cases.

## Test Coverage (src/max_ttl_tests.rs)

The test suite includes 17 test cases covering:

1. **Default behavior**: Default max TTL applies when no override is set.
2. **Rejection**: attest rejects TTLs exceeding the default max.
3. **Per-type overrides**: Different attestation types can have different max TTLs.
4. **Override precedence**: Per-type override takes precedence over the default.
5. **Custom defaults**: Admin can change the default max TTL.
6. **Batch operations**: `attest_batch` also respects max TTL constraints.
7. **Batch success**: Batch succeeds when all entries respect their type's max TTL.
8. **Zero TTL**: Zero TTL is still rejected (existing behavior).
9. **Zero max TTL rejection**: Setting max TTL to zero is rejected.
10. **Admin-only operations**: Only admin can set max TTL values.
11. **Exact max TTL**: Attestations with exactly the max TTL are allowed.
12. Additional edge cases and integration scenarios.

## Backward Compatibility

- ✅ Existing `attest` and `attest_batch` endpoints continue to work with the same signatures.
- ✅ Existing attestation read functions (`get_attestation`, `is_valid`) are unchanged.
- ✅ The default max TTL (1 year) is permissive enough for most existing use cases.
- ✅ Admins can adjust the default or per-type limits to fit their operational needs.

## Usage Examples

### Setting a Default Max TTL

```
set_default_max_attestation_ttl(env, 90 * 24 * 60 * 60)  // 90 days
```

### Setting a Per-Type Max TTL

```
set_max_attestation_ttl(env, Symbol::new(&env, "kyc"), 365 * 24 * 60 * 60)  // 1 year for KYC
set_max_attestation_ttl(env, Symbol::new(&env, "payment"), 7 * 24 * 60 * 60)  // 7 days for payments
```

### Querying the Effective Max TTL

```
let max_ttl = get_max_attestation_ttl(env, Symbol::new(&env, "kyc"));
// Returns per-type override if set, otherwise returns default.
```

### Attesting with TTL Validation

```
attest(env, attestor, subject, Symbol::new(&env, "kyc"), payload_hash, 30 * 24 * 60 * 60)?
// If 30 days < max TTL for "kyc", succeeds.
// If 30 days > max TTL for "kyc", returns Error::ExceedsMaxTtl.
```

## Files Modified

- `src/errors.rs`: Added `ExceedsMaxTtl` error variant.
- `src/types.rs`: Added `MaxAttestationTtl(Symbol)` and `DefaultMaxAttestationTtl` DataKey variants.
- `src/storage.rs`: Added `get_max_attestation_ttl`, `set_default_max_attestation_ttl`, `set_max_attestation_ttl` functions and `DEFAULT_MAX_ATTESTATION_TTL_SECONDS` constant.
- `src/contract.rs`: Added `set_default_max_attestation_ttl`, `set_max_attestation_ttl`, `get_max_attestation_ttl` contract functions. Updated `record_attestation` to validate TTL against max.
- `src/events.rs`: Added `DefaultMaxAttestationTtlChanged` and `MaxAttestationTtlChanged` event types and emitters.
- `src/lib.rs`: Registered `max_ttl_tests` module.
- `src/max_ttl_tests.rs`: New test module with 17 comprehensive test cases.

## Security Considerations

- Admin-only configuration: Only the contract admin can set max TTL values.
- Immediate effect: Changes to max TTL apply immediately to new attestations.
- Stored attestations unaffected: Existing attestations retain their original TTL; this feature only affects new attestations.
- Zero-TTL protection: Setting max TTL to zero is rejected, ensuring this protection mechanism can't be accidentally disabled.
