use soroban_sdk::{Address, Env, Symbol};

use crate::errors::Error;
use crate::types::{Attestation, DataKey};

// --- Persistent storage rent model ---------------------------------------
//
// Soroban bills rent for persistent entries in proportion to entry size and
// to the number of ledgers `extend_ttl`'s `extend_to` buys; `extend_to` is
// what's charged, while `threshold` only gates *whether* a given call pays
// anything (a no-op if the entry is already live past `threshold`). See
// docs/storage-rent-cost-analysis.md for the full worked cost comparison
// behind the values below.
const SECONDS_PER_LEDGER: u64 = 5;
const LEDGERS_PER_DAY: u32 = 17_280; // (24 * 60 * 60) / SECONDS_PER_LEDGER

// Soroban clamps any `extend_ttl` request to this network-wide ceiling
// regardless of the `extend_to` requested -- there is no way to buy a
// longer-lived persistent entry in a single bump. Confirmed against
// `soroban_sdk::testutils::ledger::LedgerInfo::default().max_entry_ttl`
// (soroban-sdk 26.1.1), which mirrors the current network config: asking
// `set_max_entry_ttl` for more than this in a test, or `extend_ttl` for
// more than this on-chain, silently caps down to it rather than erroring.
// At ~365 days, it's already almost exactly what the old flat
// `ATTESTATION_TTL_EXTEND_TO` constant assumed -- so entries with a
// caller-supplied lifetime longer than this (see `MAX_ATTESTATION_TTL_EXTEND_TO`
// below) cannot have their whole lifetime pre-purchased in one call and
// need periodic renewal; see `renew_attestation` in contract.rs.
const MAX_ENTRY_TTL_LEDGERS: u32 = 6_312_000;

// Attestor allow-list entries have no natural expiry -- membership lasts
// until an admin explicitly calls `remove_attestor` -- and are rewritten
// only on `add_attestor`/`remove_attestor`, calls rare enough that they
// won't reliably land before the old 1-year window ran out. Buying the
// longest window the network allows doesn't raise the per-ledger rent
// rate, so it's strictly cheaper per day of liveness than re-buying a
// shorter window more often, and it removes the risk of the entry
// archiving quietly between admin actions.
const ATTESTOR_TTL_THRESHOLD: u32 = LEDGERS_PER_DAY * 90;
const ATTESTOR_TTL_EXTEND_TO: u32 = MAX_ENTRY_TTL_LEDGERS;

// Attestation entries carry a caller-supplied logical lifetime
// (`ttl_seconds`, tracked as `expires_at`) that the old flat 365-day rent
// window ignored. That flat window overpaid for the common case (short
// KYC/payment attestations expiring in days-to-months) and underpaid for
// attestations with a stated TTL beyond a year, whose storage entry could
// archive before its logical expiry since nothing re-touches the key
// between `attest` and `revoke`. Sizing the rent window to the
// attestation's own remaining lifetime fixes the common case; the
// uncommon case (TTL beyond the network's own ceiling) is bounded by
// `MAX_ENTRY_TTL_LEDGERS` and needs `renew_attestation` to stay alive.
const MIN_ATTESTATION_TTL_EXTEND_TO: u32 = LEDGERS_PER_DAY * 30;
const MAX_ATTESTATION_TTL_EXTEND_TO: u32 = MAX_ENTRY_TTL_LEDGERS;

// Default maximum TTL for attestations: 1 year (365 days in seconds).
// This applies when no per-type override is configured, providing out-of-box
// protection against arbitrarily long-lived attestations.
pub const DEFAULT_MAX_ATTESTATION_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Converts an attestation's remaining lifetime into a bounded persistent
/// storage `extend_to` (in ledgers), clamped to
/// `[MIN_ATTESTATION_TTL_EXTEND_TO, MAX_ATTESTATION_TTL_EXTEND_TO]`.
fn attestation_extend_to(remaining_seconds: u64) -> u32 {
    let ledgers = remaining_seconds / SECONDS_PER_LEDGER;
    let ledgers = u32::try_from(ledgers).unwrap_or(u32::MAX);
    ledgers.clamp(MIN_ATTESTATION_TTL_EXTEND_TO, MAX_ATTESTATION_TTL_EXTEND_TO)
}

/// Bump when less than half of the purchased window remains, giving a wide
/// safety margin against irregular write patterns without re-bumping (and
/// re-billing) on every touch.
fn attestation_ttl_window(remaining_seconds: u64) -> (u32, u32) {
    let extend_to = attestation_extend_to(remaining_seconds);
    (extend_to / 2, extend_to)
}

pub fn get_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn is_attestor(env: &Env, attestor: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Attestor(attestor.clone()))
        .unwrap_or(false)
}

pub fn set_attestor(env: &Env, attestor: &Address, allowed: bool) {
    let key = DataKey::Attestor(attestor.clone());
    if allowed {
        env.storage().persistent().set(&key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&key, ATTESTOR_TTL_THRESHOLD, ATTESTOR_TTL_EXTEND_TO);
    } else {
        env.storage().persistent().remove(&key);
    }
}

pub fn get_attestation(
    env: &Env,
    subject: &Address,
    attestation_type: &Symbol,
) -> Option<Attestation> {
    let key = DataKey::Attestation(subject.clone(), attestation_type.clone());
    env.storage().persistent().get(&key)
}

pub fn set_attestation(
    env: &Env,
    subject: &Address,
    attestation_type: &Symbol,
    attestation: &Attestation,
) {
    let key = DataKey::Attestation(subject.clone(), attestation_type.clone());
    env.storage().persistent().set(&key, attestation);

    let remaining_seconds = attestation
        .expires_at
        .saturating_sub(env.ledger().timestamp());
    let (threshold, extend_to) = attestation_ttl_window(remaining_seconds);
    env.storage().persistent().extend_ttl(&key, threshold, extend_to);
}

pub fn bump_attestation_count(env: &Env) -> u64 {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::AttestationCount)
        .unwrap_or(0);
    let next = count + 1;
    env.storage()
        .instance()
        .set(&DataKey::AttestationCount, &next);
    next
}

pub fn get_attestation_count(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::AttestationCount)
        .unwrap_or(0)
}

/// Gets the maximum allowed TTL for a specific attestation type.
/// Returns the per-type override if set, otherwise returns the default maximum TTL.
pub fn get_max_attestation_ttl(env: &Env, attestation_type: &Symbol) -> u64 {
    let type_specific_key = DataKey::MaxAttestationTtl(attestation_type.clone());
    env.storage()
        .instance()
        .get(&type_specific_key)
        .unwrap_or_else(|| {
            // Fall back to default if no per-type override is set
            env.storage()
                .instance()
                .get(&DataKey::DefaultMaxAttestationTtl)
                .unwrap_or(DEFAULT_MAX_ATTESTATION_TTL_SECONDS)
        })
}

/// Sets the default maximum TTL for all attestation types.
/// This is the fallback value used when no per-type override is configured.
pub fn set_default_max_attestation_ttl(env: &Env, max_ttl_seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::DefaultMaxAttestationTtl, &max_ttl_seconds);
}

/// Sets a type-specific maximum TTL for attestations of a particular type.
/// Takes precedence over the default maximum TTL for that type.
pub fn set_max_attestation_ttl(env: &Env, attestation_type: &Symbol, max_ttl_seconds: u64) {
    let key = DataKey::MaxAttestationTtl(attestation_type.clone());
    env.storage()
        .instance()
        .set(&key, &max_ttl_seconds);
}

#[cfg(test)]
mod ttl_window_tests {
    use super::*;

    const ONE_DAY_SECONDS: u64 = 24 * 60 * 60;

    #[test]
    fn floors_short_lived_attestations_to_the_minimum_window() {
        let extend_to = attestation_extend_to(ONE_DAY_SECONDS);
        assert_eq!(extend_to, MIN_ATTESTATION_TTL_EXTEND_TO);
    }

    #[test]
    fn tracks_a_mid_range_lifetime_proportionally() {
        let ninety_days = ONE_DAY_SECONDS * 90;
        let extend_to = attestation_extend_to(ninety_days);
        assert_eq!(extend_to, LEDGERS_PER_DAY * 90);
    }

    #[test]
    fn caps_long_lived_attestations_to_the_maximum_window() {
        let ten_years = ONE_DAY_SECONDS * 365 * 10;
        let extend_to = attestation_extend_to(ten_years);
        assert_eq!(extend_to, MAX_ATTESTATION_TTL_EXTEND_TO);
    }

    #[test]
    fn threshold_is_half_of_the_purchased_window() {
        let (threshold, extend_to) = attestation_ttl_window(ONE_DAY_SECONDS * 90);
        assert_eq!(threshold, extend_to / 2);
    }
}
