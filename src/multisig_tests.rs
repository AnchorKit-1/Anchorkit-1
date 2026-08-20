use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Vec};

use crate::contract::{AnchorKitContract, AnchorKitContractClient};
use crate::errors::Error;
use crate::types::SignatureInfo;
use crate::test_util::setup;

#[test]
fn initialize_multisig_basic() {
    let s = setup();
    let env = &s.env;
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(env, &contract_id);

    let mut signers = Vec::new(env);
    signers.push_back(Address::generate(env));
    signers.push_back(Address::generate(env));
    signers.push_back(Address::generate(env));

    let result = client.initialize_multisig(&signers, 2);
    assert!(result.is_ok());

    let (retrieved_signers, threshold) = client.get_multisig_config().unwrap();
    assert_eq!(threshold, 2);
    assert_eq!(retrieved_signers.len(), 3);
}

#[test]
fn initialize_multisig_rejects_empty_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let signers = Vec::new(&env);
    let result = client.try_initialize_multisig(&signers, 1);
    assert_eq!(result, Err(Ok(Error::EmptySignerList)));
}

#[test]
fn initialize_multisig_rejects_zero_threshold() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    signers.push_back(Address::generate(&env));

    let result = client.try_initialize_multisig(&signers, 0);
    assert_eq!(result, Err(Ok(Error::InvalidThreshold)));
}

#[test]
fn initialize_multisig_rejects_threshold_exceeding_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    signers.push_back(Address::generate(&env));

    let result = client.try_initialize_multisig(&signers, 2);
    assert_eq!(result, Err(Ok(Error::InvalidThreshold)));
}

#[test]
fn initialize_multisig_rejects_duplicate_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let addr = Address::generate(&env);
    let mut signers = Vec::new(&env);
    signers.push_back(addr.clone());
    signers.push_back(addr.clone());

    let result = client.try_initialize_multisig(&signers, 1);
    assert_eq!(result, Err(Ok(Error::DuplicateSigner)));
}

#[test]
fn initialize_multisig_1_of_1() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    signers.push_back(Address::generate(&env));

    let result = client.initialize_multisig(&signers, 1);
    assert!(result.is_ok());

    let (retrieved_signers, threshold) = client.get_multisig_config().unwrap();
    assert_eq!(threshold, 1);
    assert_eq!(retrieved_signers.len(), 1);
}

#[test]
fn initialize_multisig_3_of_5() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    for _ in 0..5 {
        signers.push_back(Address::generate(&env));
    }

    let result = client.initialize_multisig(&signers, 3);
    assert!(result.is_ok());

    let (retrieved_signers, threshold) = client.get_multisig_config().unwrap();
    assert_eq!(threshold, 3);
    assert_eq!(retrieved_signers.len(), 5);
}

#[test]
fn rotate_signers_requires_sufficient_signatures() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signer3 = Address::generate(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());
    signers.push_back(signer3.clone());

    // Initialize with 2-of-3
    client.initialize_multisig(&signers, 2);

    // Try rotation with insufficient signatures (only 1)
    let mut new_signers = Vec::new(&env);
    new_signers.push_back(Address::generate(&env));

    let mut sig_signers = Vec::new(&env);
    sig_signers.push_back(signer1.clone());

    let mut signatures = Vec::new(&env);
    signatures.push_back(Bytes::new(&env)); // Empty signature placeholder

    let sig_info = SignatureInfo {
        signers: sig_signers,
        signatures,
    };

    let result = client.try_rotate_signers(&new_signers, 1, &sig_info);
    assert_eq!(result, Err(Ok(Error::InsufficientSignatures)));
}

#[test]
fn rotate_signers_rejects_empty_new_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    let signer1 = Address::generate(&env);
    signers.push_back(signer1.clone());

    // Initialize with 1-of-1
    client.initialize_multisig(&signers, 1);

    // Try rotation to empty signers
    let new_signers = Vec::new(&env);

    let mut sig_signers = Vec::new(&env);
    sig_signers.push_back(signer1.clone());

    let mut signatures = Vec::new(&env);
    signatures.push_back(Bytes::new(&env));

    let sig_info = SignatureInfo {
        signers: sig_signers,
        signatures,
    };

    let result = client.try_rotate_signers(&new_signers, 1, &sig_info);
    assert_eq!(result, Err(Ok(Error::EmptySignerList)));
}

#[test]
fn rotate_signers_rejects_invalid_threshold() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    let signer1 = Address::generate(&env);
    signers.push_back(signer1.clone());

    // Initialize with 1-of-1
    client.initialize_multisig(&signers, 1);

    // Try rotation with threshold exceeding signers
    let mut new_signers = Vec::new(&env);
    new_signers.push_back(Address::generate(&env));

    let mut sig_signers = Vec::new(&env);
    sig_signers.push_back(signer1.clone());

    let mut signatures = Vec::new(&env);
    signatures.push_back(Bytes::new(&env));

    let sig_info = SignatureInfo {
        signers: sig_signers,
        signatures,
    };

    let result = client.try_rotate_signers(&new_signers, 2, &sig_info);
    assert_eq!(result, Err(Ok(Error::InvalidThreshold)));
}

#[test]
fn rotate_signers_rejects_duplicate_new_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    let signer1 = Address::generate(&env);
    signers.push_back(signer1.clone());

    // Initialize with 1-of-1
    client.initialize_multisig(&signers, 1);

    // Try rotation with duplicate new signers
    let addr = Address::generate(&env);
    let mut new_signers = Vec::new(&env);
    new_signers.push_back(addr.clone());
    new_signers.push_back(addr.clone());

    let mut sig_signers = Vec::new(&env);
    sig_signers.push_back(signer1.clone());

    let mut signatures = Vec::new(&env);
    signatures.push_back(Bytes::new(&env));

    let sig_info = SignatureInfo {
        signers: sig_signers,
        signatures,
    };

    let result = client.try_rotate_signers(&new_signers, 1, &sig_info);
    assert_eq!(result, Err(Ok(Error::DuplicateSigner)));
}

#[test]
fn rotate_signers_rejects_unauthorized_signers() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    let signer1 = Address::generate(&env);
    signers.push_back(signer1.clone());

    // Initialize with 1-of-1
    client.initialize_multisig(&signers, 1);

    // Try rotation with unauthorized signer
    let mut new_signers = Vec::new(&env);
    new_signers.push_back(Address::generate(&env));

    let unauthorized_signer = Address::generate(&env);
    let mut sig_signers = Vec::new(&env);
    sig_signers.push_back(unauthorized_signer);

    let mut signatures = Vec::new(&env);
    signatures.push_back(Bytes::new(&env));

    let sig_info = SignatureInfo {
        signers: sig_signers,
        signatures,
    };

    let result = client.try_rotate_signers(&new_signers, 1, &sig_info);
    assert_eq!(result, Err(Ok(Error::InsufficientSignatures)));
}

#[test]
fn multisig_threshold_enforcement() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let mut signers = Vec::new(&env);
    signers.push_back(Address::generate(&env));
    signers.push_back(Address::generate(&env));
    signers.push_back(Address::generate(&env));

    // Initialize with 3-of-3 (all signers required)
    client.initialize_multisig(&signers, 3);

    let (retrieved_signers, threshold) = client.get_multisig_config().unwrap();
    assert_eq!(threshold, 3);
    assert_eq!(retrieved_signers.len(), 3);
}

#[test]
fn get_multisig_config_before_initialization() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);

    let result = client.try_get_multisig_config();
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}
