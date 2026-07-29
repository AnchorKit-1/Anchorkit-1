#![no_std]

mod contract;
pub mod domain_validator;
mod errors;
mod events;
pub mod hash;
mod storage;
mod types;

pub use contract::{AnchorKitContract, AnchorKitContractClient};
pub use errors::Error;
pub use hash::{compute_payload_hash, verify_payload_hash};
pub use types::{Attestation, AttestationStatus};

#[cfg(test)]
mod test_util;

#[cfg(test)]
mod admin_tests;

#[cfg(test)]
mod attestor_tests;

#[cfg(test)]
mod attest_tests;

#[cfg(test)]
mod attest_batch_tests;

#[cfg(test)]
mod batch_gas_benchmark;

#[cfg(test)]
mod hash_benchmark;

#[cfg(test)]
mod revoke_tests;

#[cfg(test)]
mod storage_ttl_tests;

#[cfg(test)]
mod renew_attestation_tests;

#[cfg(test)]
mod pause_tests;

#[cfg(test)]
mod max_ttl_tests;

#[cfg(all(test, feature = "stress-tests"))]
mod attestor_stress_tests;

#[cfg(all(test, feature = "testnet-integration"))]
mod testnet_integration_tests;
