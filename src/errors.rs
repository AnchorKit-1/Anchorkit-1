use soroban_sdk::contracterror;

/// Error codes returned by the AnchorKit contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    ContractPaused = 4,
    AttestorAlreadyRegistered = 5,
    AttestorNotRegistered = 6,
    AttestationNotFound = 7,
    AttestationExpired = 8,
    AttestationAlreadyRevoked = 9,
    InvalidExpiration = 10,
    InvalidDomain = 11,
    EmptyBatch = 12,
    InvalidPagination = 13,
    ExceedsMaxTtl = 13,
}
