use soroban_sdk::{contracttype, Address, BytesN, Symbol};

/// Storage keys for all persistent and instance data the contract manages.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// The contract administrator.
    Admin,
    /// Whether new attestations are currently accepted.
    Paused,
    /// Allow-list membership for an attestor address.
    Attestor(Address),
    /// A single attestation, keyed by the subject it describes and the
    /// attestation type (e.g. `kyc_approved`, `payment_confirmed`).
    Attestation(Address, Symbol),
    /// Running count of attestations ever submitted, for basic observability.
    AttestationCount,
    /// The next sequence number to assign for a (subject, attestation_type) pair's
    /// history. Incremented each time an attestation is recorded for that pair.
    AttestationSeq(Address, Symbol),
    /// One entry in the append-only history of attestations for a given
    /// (subject, attestation_type) pair. Indexed by sequence number (starting at 1).
    /// The history is queryable via `list_attestation_history` for pagination.
    AttestationHistory(Address, Symbol, u64), // (subject, type, sequence)
}

/// Lifecycle state of an attestation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttestationStatus {
    Active,
    Revoked,
}

/// One entry of an `attest_batch` call: everything about an individual
/// attestation except the attestor, which is shared by the whole batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchAttestEntry {
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub ttl_seconds: u64,
}

/// A single off-chain attestation anchored on-chain.
///
/// `payload_hash` is a sha256 digest of the off-chain payload the attestor
/// vouches for (e.g. a KYC decision or a signed payment confirmation); the
/// contract never stores the payload itself, only its fingerprint.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    pub attestor: Address,
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
}

/// One entry from the append-only history of attestations for a given
/// (subject, attestation_type) pair. The sequence number indicates its
/// position in the history (1-indexed). History entries are immutable once
/// written and queryable via `list_attestation_history` with pagination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub sequence: u64,
    pub attestor: Address,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
}
