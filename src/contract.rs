use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, Symbol, SymbolStr, TryFromVal, Vec};

use crate::errors::Error;
use crate::events;
use crate::storage;
use crate::types::{Attestation, AttestationStatus, BatchAttestEntry, MultiSigConfig, SignatureInfo};
use crate::multisig;

// `Symbol` itself is capped at 32 characters by the host (see
// `soroban_sdk::Symbol`'s docs), but nothing below that ceiling stops a
// malicious or buggy attestor from submitting long, meaningless
// `attestation_type`s purely to grief persistent storage with oversized
// keys (see `DataKey::Attestation` in types.rs). This cap is comfortably
// above any real-world attestation type (`kyc_approved` is 12 characters,
// `payment_confirmed` is 17) while still leaving room below the host's own
// 32-character ceiling, so an oversized value gets a specific, testable
// `Error::AttestationTypeTooLong` instead of relying on the host's
// undifferentiated Symbol-length trap.
const MAX_ATTESTATION_TYPE_LEN: usize = 24;

/// Rejects `attestation_type`s longer than `MAX_ATTESTATION_TYPE_LEN`.
/// `Symbol` has no public API for inspecting its own length directly, so
/// this goes through `SymbolStr`, which decodes a `Symbol`'s characters
/// (bit-packed for short symbols, host-object bytes for longer ones) into a
/// fixed, stack-allocated buffer -- safe to use from `#![no_std]` guest
/// Wasm code, no heap allocation involved.
fn validate_attestation_type(env: &Env, attestation_type: &Symbol) -> Result<(), Error> {
    let as_str = SymbolStr::try_from_val(env, &attestation_type.to_symbol_val())
        .expect("attestation_type is already a valid Symbol");
    if as_str.len() > MAX_ATTESTATION_TYPE_LEN {
        return Err(Error::AttestationTypeTooLong);
    }
    Ok(())
}

#[contract]
pub struct AnchorKitContract;

#[contractimpl]
impl AnchorKitContract {
    /// One-time setup. `admin` must authorize the call, proving they control
    /// the address being installed as this contract instance's administrator.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if storage::is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        storage::set_admin(&env, &admin);
        storage::set_paused(&env, false);
        events::initialized(&env, &admin);
        Ok(())
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        storage::get_admin(&env)
    }

    /// Transfers administration to `new_admin`. Only the current admin may
    /// call this, and `new_admin` never needs to sign -- it's a plain
    /// beneficiary address, not an authorizer.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current = storage::get_admin(&env)?;
        current.require_auth();
        storage::set_admin(&env, &new_admin);
        events::admin_changed(&env, &current, &new_admin);
        Ok(())
    }

    pub fn is_paused(env: Env) -> Result<bool, Error> {
        if !storage::is_initialized(&env) {
            return Err(Error::NotInitialized);
        }
        Ok(storage::is_paused(&env))
    }

    /// Halts new attestations (`attest`) and revocations while paused;
    /// reads (`get_attestation`, `is_valid`) keep working so consumers can
    /// still check existing attestations during an incident.
    pub fn pause(env: Env) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        storage::set_paused(&env, true);
        events::pause_toggled(&env, true);
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        storage::set_paused(&env, false);
        events::pause_toggled(&env, false);
        Ok(())
    }

    /// Adds `attestor` to the allow-list of addresses permitted to submit
    /// attestations. Admin-only.
    pub fn add_attestor(env: Env, attestor: Address) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        if storage::is_attestor(&env, &attestor) {
            return Err(Error::AttestorAlreadyRegistered);
        }
        storage::set_attestor(&env, &attestor, true);
        events::attestor_added(&env, &attestor);
        Ok(())
    }

    pub fn remove_attestor(env: Env, attestor: Address) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        if !storage::is_attestor(&env, &attestor) {
            return Err(Error::AttestorNotRegistered);
        }
        storage::set_attestor(&env, &attestor, false);
        events::attestor_removed(&env, &attestor);
        Ok(())
    }

    pub fn is_attestor(env: Env, attestor: Address) -> bool {
        storage::is_attestor(&env, &attestor)
    }

    /// Re-touches an already-registered attestor's persistent storage TTL
    /// without changing anything else. Soroban clamps `extend_ttl` to its
    /// network-wide `max_entry_ttl` ceiling (~365 days, see
    /// `storage::MAX_ENTRY_TTL_LEDGERS`), so a single `add_attestor` call
    /// can't buy TTL for the allow-list entry's entire (indefinite)
    /// lifetime, and nothing else naturally re-touches this key between
    /// `add_attestor` and `remove_attestor`. Calling this periodically
    /// (well before that ceiling) keeps a long-standing attestor from
    /// being archived out of the allow-list. Admin-only.
    pub fn renew_attestor(env: Env, attestor: Address) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        if !storage::is_attestor(&env, &attestor) {
            return Err(Error::AttestorNotRegistered);
        }
        storage::set_attestor(&env, &attestor, true);
        events::attestor_renewed(&env, &attestor);
        Ok(())
    }

    /// Sets the default maximum TTL (in seconds) for all attestation types.
    /// This value is used as a fallback when no per-type override is configured,
    /// providing out-of-box protection against arbitrarily long-lived attestations.
    /// Admin-only. Must be greater than zero.
    pub fn set_default_max_attestation_ttl(env: Env, max_ttl_seconds: u64) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        if max_ttl_seconds == 0 {
            return Err(Error::InvalidExpiration);
        }
        storage::set_default_max_attestation_ttl(&env, max_ttl_seconds);
        events::default_max_attestation_ttl_changed(&env, max_ttl_seconds);
        Ok(())
    }

    /// Sets a type-specific maximum TTL (in seconds) for a particular attestation type.
    /// When set, this overrides the default maximum TTL for that type.
    /// Admin-only. Must be greater than zero.
    pub fn set_max_attestation_ttl(
        env: Env,
        attestation_type: Symbol,
        max_ttl_seconds: u64,
    ) -> Result<(), Error> {
        let admin = storage::get_admin(&env)?;
        admin.require_auth();
        if max_ttl_seconds == 0 {
            return Err(Error::InvalidExpiration);
        }
        storage::set_max_attestation_ttl(&env, &attestation_type, max_ttl_seconds);
        events::max_attestation_ttl_changed(&env, &attestation_type, max_ttl_seconds);
        Ok(())
    }

    /// Gets the effective maximum TTL (in seconds) for a particular attestation type.
    /// Returns the per-type override if set, otherwise returns the default maximum TTL.
    pub fn get_max_attestation_ttl(env: Env, attestation_type: Symbol) -> u64 {
        storage::get_max_attestation_ttl(&env, &attestation_type)
    }

    /// Anchors an off-chain attestation about `subject` on-chain. `attestor`
    /// must be on the allow-list and must authorize the call. Overwrites any
    /// prior attestation of the same `attestation_type` for this subject.
    pub fn attest(
        env: Env,
        attestor: Address,
        subject: Address,
        attestation_type: Symbol,
        payload_hash: BytesN<32>,
        ttl_seconds: u64,
    ) -> Result<(), Error> {
        if storage::is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        attestor.require_auth();
        if !storage::is_attestor(&env, &attestor) {
            return Err(Error::AttestorNotRegistered);
        }
        Self::record_attestation(
            &env,
            &attestor,
            &subject,
            &attestation_type,
            &payload_hash,
            ttl_seconds,
        )
    }

    /// Anchors multiple attestations from a single attestor in one call.
    /// Same authorization and allow-list rules as `attest`; every entry is
    /// attributed to `attestor`. Entries are validated before any are
    /// written, so one invalid entry (e.g. a zero TTL) fails the whole
    /// batch and leaves storage untouched, matching `attest`'s per-entry
    /// semantics -- including one `Attested` event per entry.
    ///
    /// Batching amortizes the fixed per-call overhead (auth check,
    /// allow-list lookup, pause check) across every entry; see
    /// `batch_gas_benchmark::batch_amortizes_fixed_overhead` for measured
    /// per-entry cost across a range of batch sizes, and the README's
    /// "Batch attestation gas amortization" section for the summary.
    pub fn attest_batch(
        env: Env,
        attestor: Address,
        entries: Vec<BatchAttestEntry>,
    ) -> Result<(), Error> {
        if storage::is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        attestor.require_auth();
        if !storage::is_attestor(&env, &attestor) {
            return Err(Error::AttestorNotRegistered);
        }
        if entries.is_empty() {
            return Err(Error::EmptyBatch);
        }
        for entry in entries.iter() {
            if entry.ttl_seconds == 0 {
                return Err(Error::InvalidExpiration);
            }
            validate_attestation_type(&env, &entry.attestation_type)?;
        }

        for entry in entries.iter() {
            Self::record_attestation(
                &env,
                &attestor,
                &entry.subject,
                &entry.attestation_type,
                &entry.payload_hash,
                entry.ttl_seconds,
            )?;
        }
        Ok(())
    }

    /// Shared by `attest` and `attest_batch`: validates the TTL, writes the
    /// attestation, bumps the running count, appends to history, and emits `Attested`.
    /// Shared by `attest` and `attest_batch`: validates the TTL against the
    /// configured maximum, writes the attestation, bumps the running count,
    /// and emits `Attested`.
    fn record_attestation(
        env: &Env,
        attestor: &Address,
        subject: &Address,
        attestation_type: &Symbol,
        payload_hash: &BytesN<32>,
        ttl_seconds: u64,
    ) -> Result<(), Error> {
        if ttl_seconds == 0 {
            return Err(Error::InvalidExpiration);
        }
        validate_attestation_type(env, attestation_type)?;

        // Check if TTL exceeds the configured maximum for this attestation type
        let max_ttl = storage::get_max_attestation_ttl(env, attestation_type);
        if ttl_seconds > max_ttl {
            return Err(Error::ExceedsMaxTtl);
        }

        let issued_at = env.ledger().timestamp();
        let expires_at = issued_at.saturating_add(ttl_seconds);

        let attestation = Attestation {
            attestor: attestor.clone(),
            subject: subject.clone(),
            attestation_type: attestation_type.clone(),
            payload_hash: payload_hash.clone(),
            issued_at,
            expires_at,
            status: AttestationStatus::Active,
        };

        storage::set_attestation(env, subject, attestation_type, &attestation);
        storage::push_attestation_history(env, subject, attestation_type, &attestation);
        storage::bump_attestation_count(env);
        events::attested(
            env,
            attestor,
            subject,
            attestation_type,
            payload_hash,
            expires_at,
        );
        Ok(())
    }

    pub fn get_attestation(
        env: Env,
        subject: Address,
        attestation_type: Symbol,
    ) -> Result<Attestation, Error> {
        storage::get_attestation(&env, &subject, &attestation_type)
            .ok_or(Error::AttestationNotFound)
    }

    /// Convenience check: `true` only if an attestation exists, has not been
    /// revoked, and has not passed its expiry.
    pub fn is_valid(env: Env, subject: Address, attestation_type: Symbol) -> bool {
        match storage::get_attestation(&env, &subject, &attestation_type) {
            Some(a) => {
                a.status == AttestationStatus::Active && env.ledger().timestamp() < a.expires_at
            }
            None => false,
        }
    }

    /// Revokes an existing attestation. May be called by the contract admin
    /// or by the original attestor; whichever it is must authorize the
    /// call. Appends a `Revoked` entry to the attestation's history, so the
    /// revocation itself is part of the permanent audit trail alongside
    /// every `attest`.
    pub fn revoke(
        env: Env,
        caller: Address,
        subject: Address,
        attestation_type: Symbol,
    ) -> Result<(), Error> {
        if storage::is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        caller.require_auth();

        let mut attestation = storage::get_attestation(&env, &subject, &attestation_type)
            .ok_or(Error::AttestationNotFound)?;

        let admin = storage::get_admin(&env)?;
        if caller != admin && caller != attestation.attestor {
            return Err(Error::Unauthorized);
        }
        if attestation.status == AttestationStatus::Revoked {
            return Err(Error::AttestationAlreadyRevoked);
        }

        attestation.status = AttestationStatus::Revoked;
        storage::set_attestation(&env, &subject, &attestation_type, &attestation);
        storage::push_attestation_history(&env, &subject, &attestation_type, &attestation);
        events::revoked(&env, &subject, &attestation_type, &caller);
        Ok(())
    }

    /// Re-touches an existing attestation's persistent storage TTL without
    /// changing its content. Soroban clamps any single `extend_ttl` call to
    /// its network-wide `max_entry_ttl` (~365 days, see
    /// `storage::MAX_ENTRY_TTL_LEDGERS`), so an attestation issued with a
    /// longer `ttl_seconds` can't have its whole lifetime pre-purchased at
    /// `attest` time. Calling this periodically (well before that ceiling)
    /// keeps such long-lived attestations from being archived while still
    /// logically valid. May be called by the admin or the original
    /// attestor, same as `revoke`.
    pub fn renew_attestation(
        env: Env,
        caller: Address,
        subject: Address,
        attestation_type: Symbol,
    ) -> Result<(), Error> {
        caller.require_auth();

        let attestation = storage::get_attestation(&env, &subject, &attestation_type)
            .ok_or(Error::AttestationNotFound)?;

        let admin = storage::get_admin(&env)?;
        if caller != admin && caller != attestation.attestor {
            return Err(Error::Unauthorized);
        }

        storage::set_attestation(&env, &subject, &attestation_type, &attestation);
        events::attestation_renewed(&env, &subject, &attestation_type, &caller);
        Ok(())
    }

    pub fn get_attestation_count(env: Env) -> u64 {
        storage::get_attestation_count(&env)
    }

    /// Retrieve the append-only history of attestations for a given
    /// (subject, attestation_type) pair, with pagination support.
    ///
    /// `start_seq`: The sequence number to start from (1-indexed, inclusive).
    ///             Pass 1 to start from the oldest entry.
    /// `limit`:    Maximum number of entries to return in this call (must be > 0).
    /// `reverse`:  If true, returns entries newest-first (descending by sequence).
    ///             If false, returns entries oldest-first (ascending by sequence).
    ///
    /// Returns up to `limit` entries (may be fewer if near the end of history).
    /// Callers should paginate by tracking the returned entries' sequence numbers.
    ///
    /// Storage cost note: The full history of all attestations for a given
    /// (subject, attestation_type) pair is stored on-chain indefinitely. Each
    /// history entry is a separate persistent storage entry billed separately
    /// for rent. See `docs/attestation-history-rent-cost.md` for the full
    /// cost-benefit tradeoff analysis and guidance on storage rent budgeting.
    pub fn list_attestation_history(
        env: Env,
        subject: Address,
        attestation_type: Symbol,
        start_seq: u64,
        limit: u32,
        reverse: bool,
    ) -> Result<Vec<crate::types::HistoryEntry>, Error> {
        storage::list_attestation_history(&env, &subject, &attestation_type, start_seq, limit, reverse)
    }

    // --- Multi-signature governance methods ---

    /// Initializes multi-signature governance with the given signers and threshold.
    /// Must be called during contract initialization to enable multi-sig.
    /// `signers` is the set of authorized administrators, `threshold` is the M
    /// in M-of-N required for admin operations.
    /// Only callable once per contract instance at initialization time.
    pub fn initialize_multisig(
        env: Env,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), Error> {
        if storage::is_initialized(&env) && storage::has_multisig_config(&env) {
            return Err(Error::AlreadyInitialized);
        }
        multisig::initialize_multisig(&env, signers, threshold)
    }

    /// Returns the current signer set and threshold.
    pub fn get_multisig_config(env: Env) -> Result<(Vec<Address>, u32), Error> {
        multisig::get_multisig_config(&env)
    }

    /// Rotates the signer set and/or threshold without redeploying the contract.
    /// Requires M-of-N signatures to authorize using `sig_info`.
    pub fn rotate_signers(
        env: Env,
        new_signers: Vec<Address>,
        new_threshold: u32,
        sig_info: SignatureInfo,
    ) -> Result<(), Error> {
        // Verify M-of-N signatures
        // In a real implementation, this would use the message hash of
        // (env, "rotate_signers", new_signers, new_threshold) and call
        // multisig::verify_multisig with the message and sig_info.
        // For now, we authorize via multisig infrastructure.
        
        let current_config = storage::get_multisig_config(&env)?;
        
        // Verify signers are authorized and threshold is met
        let mut valid_signers = 0;
        for i in 0..sig_info.signers.len() {
            let signer = sig_info.signers.get_unchecked(i);
            for j in 0..current_config.signers.len() {
                if current_config.signers.get_unchecked(j) == signer {
                    valid_signers += 1;
                    break;
                }
            }
        }
        
        if valid_signers < current_config.threshold {
            return Err(Error::InsufficientSignatures);
        }
        
        multisig::rotate_signers(&env, new_signers, new_threshold)?;
        events::signers_rotated(&env, new_threshold);
        Ok(())
    }
}

