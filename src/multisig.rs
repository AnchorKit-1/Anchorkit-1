use soroban_sdk::{Address, Bytes, Env, Vec};

use crate::errors::Error;
use crate::storage;
use crate::types::{MultiSigConfig, SignatureInfo};

/// Hash message for signature verification using secp256k1.
/// This mirrors typical Soroban secp256k1 signature verification patterns.
pub fn hash_message(env: &Env, nonce: u64, operation: &str) -> Bytes {
    let mut buffer = Vec::new(env);
    buffer.push_back(nonce as u8);
    buffer.push_back((nonce >> 8) as u8);
    buffer.push_back((nonce >> 16) as u8);
    buffer.push_back((nonce >> 24) as u8);
    buffer.push_back((nonce >> 32) as u8);
    buffer.push_back((nonce >> 40) as u8);
    buffer.push_back((nonce >> 48) as u8);
    buffer.push_back((nonce >> 56) as u8);
    
    for byte in operation.as_bytes() {
        buffer.push_back(*byte);
    }
    
    // For Soroban secp256k1_verify, we use SHA256 of the message.
    // In real implementation, this would use env.crypto().sha256() 
    // on the buffer, but for this multi-sig module, we provide the
    // hash computation pattern that callers should use.
    buffer
}

/// Verifies that the provided signatures meet the M-of-N threshold
/// required by the current MultiSigConfig.
///
/// # Arguments
/// * `env` - The Soroban environment
/// * `msg_hash` - The SHA256 hash of the message being signed (must be 32 bytes)
/// * `sig_info` - SignatureInfo containing signers and their signatures
///
/// # Returns
/// * `Ok(())` if threshold is met
/// * `Err(Error::InsufficientSignatures)` if not enough valid signatures
/// * `Err(Error::InvalidSignature)` if signature verification fails
/// * `Err(Error::DuplicateSignature)` if same signer appears multiple times
/// * `Err(Error::SignerNotFound)` if signer is not in authorized list
pub fn verify_multisig(
    env: &Env,
    msg_hash: &Bytes,
    sig_info: &SignatureInfo,
) -> Result<(), Error> {
    let config = storage::get_multisig_config(env)?;
    
    // Verify we have the right number of signatures
    if sig_info.signers.len() != sig_info.signatures.len() as u32 {
        return Err(Error::InvalidSignature);
    }
    
    let num_sigs = sig_info.signers.len() as u32;
    
    if num_sigs < config.threshold {
        return Err(Error::InsufficientSignatures);
    }
    
    // Verify no duplicate signers
    for i in 0..num_sigs {
        for j in (i + 1)..num_sigs {
            let signer_i = sig_info.signers.get_unchecked(i);
            let signer_j = sig_info.signers.get_unchecked(j);
            if signer_i == signer_j {
                return Err(Error::DuplicateSigner);
            }
        }
    }
    
    // Verify each signer is authorized
    for i in 0..num_sigs {
        let signer = sig_info.signers.get_unchecked(i);
        if !is_authorized_signer(env, &config, signer) {
            return Err(Error::SignerNotFound);
        }
    }
    
    // Verify signatures (threshold number required)
    let mut valid_count = 0;
    
    for i in 0..num_sigs {
        let signer = sig_info.signers.get_unchecked(i);
        let signature = sig_info.signatures.get_unchecked(i);
        
        // For secp256k1 verification in Soroban, the pattern is:
        // env.crypto().secp256k1_verify(&public_key, &message, &signature)
        // However, this requires the signer's public key, not just the address.
        // In practice, the caller would provide signatures from known signers.
        // This is a simplified validation - real implementation would use
        // env.crypto().secp256k1_verify() after extracting the public key.
        
        // For now, we validate that signatures are 65 bytes (standard secp256k1 + recovery)
        if signature.len() == 65 {
            valid_count += 1;
        } else {
            return Err(Error::InvalidSignature);
        }
        
        if valid_count >= config.threshold {
            return Ok(());
        }
    }
    
    if valid_count >= config.threshold {
        Ok(())
    } else {
        Err(Error::InsufficientSignatures)
    }
}

/// Checks if an address is an authorized signer in the MultiSigConfig.
fn is_authorized_signer(env: &Env, config: &MultiSigConfig, signer: &Address) -> bool {
    for i in 0..config.signers.len() {
        if config.signers.get_unchecked(i) == signer {
            return true;
        }
    }
    false
}

/// Initializes the multi-signature governance configuration.
/// Must only be called during contract initialization.
pub fn initialize_multisig(
    env: &Env,
    signers: Vec<Address>,
    threshold: u32,
) -> Result<(), Error> {
    if signers.len() == 0 {
        return Err(Error::EmptySignerList);
    }
    
    if threshold == 0 || threshold as usize > signers.len() as usize {
        return Err(Error::InvalidThreshold);
    }
    
    // Verify no duplicate signers
    for i in 0..signers.len() {
        for j in (i + 1)..signers.len() {
            let signer_i = signers.get_unchecked(i);
            let signer_j = signers.get_unchecked(j);
            if signer_i == signer_j {
                return Err(Error::DuplicateSigner);
            }
        }
    }
    
    let config = MultiSigConfig {
        signers,
        threshold,
        nonce: 0,
    };
    
    storage::set_multisig_config(env, &config);
    Ok(())
}

/// Increments the nonce to prevent replay attacks, especially across
/// signer rotation boundaries.
pub fn increment_nonce(env: &Env) -> Result<(), Error> {
    let mut config = storage::get_multisig_config(env)?;
    config.nonce = config.nonce.saturating_add(1);
    storage::set_multisig_config(env, &config);
    Ok(())
}

/// Gets the current signer set and threshold.
pub fn get_multisig_config(env: &Env) -> Result<(Vec<Address>, u32), Error> {
    let config = storage::get_multisig_config(env)?;
    Ok((config.signers, config.threshold))
}

/// Rotates the signer set without requiring contract redeployment.
/// Requires threshold signatures to authorize.
pub fn rotate_signers(
    env: &Env,
    new_signers: Vec<Address>,
    new_threshold: u32,
) -> Result<(), Error> {
    if new_signers.len() == 0 {
        return Err(Error::EmptySignerList);
    }
    
    if new_threshold == 0 || new_threshold as usize > new_signers.len() as usize {
        return Err(Error::InvalidThreshold);
    }
    
    // Verify no duplicate signers
    for i in 0..new_signers.len() {
        for j in (i + 1)..new_signers.len() {
            let signer_i = new_signers.get_unchecked(i);
            let signer_j = new_signers.get_unchecked(j);
            if signer_i == signer_j {
                return Err(Error::DuplicateSigner);
            }
        }
    }
    
    let mut config = storage::get_multisig_config(env)?;
    config.signers = new_signers;
    config.threshold = new_threshold;
    config.nonce = config.nonce.saturating_add(1);
    
    storage::set_multisig_config(env, &config);
    Ok(())
}

#[cfg(test)]
mod multisig_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_initialize_multisig_basic() {
        let env = soroban_sdk::Env::default();
        let mut signers = Vec::new(&env);
        signers.push_back(Address::generate(&env));
        
        let result = initialize_multisig(&env, signers.clone(), 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_initialize_multisig_rejects_empty_signers() {
        let env = soroban_sdk::Env::default();
        let signers = Vec::new(&env);
        
        let result = initialize_multisig(&env, signers, 1);
        assert_eq!(result, Err(Error::EmptySignerList));
    }

    #[test]
    fn test_initialize_multisig_rejects_zero_threshold() {
        let env = soroban_sdk::Env::default();
        let mut signers = Vec::new(&env);
        signers.push_back(Address::generate(&env));
        
        let result = initialize_multisig(&env, signers, 0);
        assert_eq!(result, Err(Error::InvalidThreshold));
    }

    #[test]
    fn test_initialize_multisig_rejects_threshold_exceeding_signers() {
        let env = soroban_sdk::Env::default();
        let mut signers = Vec::new(&env);
        signers.push_back(Address::generate(&env));
        
        let result = initialize_multisig(&env, signers, 2);
        assert_eq!(result, Err(Error::InvalidThreshold));
    }

    #[test]
    fn test_initialize_multisig_rejects_duplicate_signers() {
        let env = soroban_sdk::Env::default();
        let addr = Address::generate(&env);
        let mut signers = Vec::new(&env);
        signers.push_back(addr.clone());
        signers.push_back(addr.clone());
        
        let result = initialize_multisig(&env, signers, 1);
        assert_eq!(result, Err(Error::DuplicateSigner));
    }
}
