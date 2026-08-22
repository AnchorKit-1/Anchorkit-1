//! Every event this contract emits. Downstream indexers depend on the exact
//! topic and data shape published here, so a change to a struct's fields,
//! `#[topic]` markers, or `topics = [...]` override changes the on-chain
//! event contract. `events_tests.rs` pins the exact wire shape of every
//! event defined below and is the source of truth for what's safe to
//! change without breaking an indexer -- update it deliberately alongside
//! any change here, never by auto-accepting a diff.
use soroban_sdk::{contractevent, Address, BytesN, Env, Symbol};

#[contractevent]
#[derive(Clone, Debug)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
}

#[contractevent(topics = ["admin_set"])]
#[derive(Clone, Debug)]
pub struct AdminChanged {
    #[topic]
    pub previous: Address,
    pub new_admin: Address,
}

#[contractevent(topics = ["attestor_add"])]
#[derive(Clone, Debug)]
pub struct AttestorAdded {
    #[topic]
    pub attestor: Address,
}

#[contractevent(topics = ["attestor_del"])]
#[derive(Clone, Debug)]
pub struct AttestorRemoved {
    #[topic]
    pub attestor: Address,
}

#[contractevent(topics = ["attestor_renew"])]
#[derive(Clone, Debug)]
pub struct AttestorRenewed {
    #[topic]
    pub attestor: Address,
}

#[contractevent(topics = ["attest"])]
#[derive(Clone, Debug)]
pub struct Attested {
    #[topic]
    pub subject: Address,
    #[topic]
    pub attestation_type: Symbol,
    pub attestor: Address,
    pub payload_hash: BytesN<32>,
    pub expires_at: u64,
}

#[contractevent(topics = ["revoke"])]
#[derive(Clone, Debug)]
pub struct Revoked {
    #[topic]
    pub subject: Address,
    #[topic]
    pub attestation_type: Symbol,
    pub revoked_by: Address,
}

#[contractevent(topics = ["renew"])]
#[derive(Clone, Debug)]
pub struct AttestationRenewed {
    #[topic]
    pub subject: Address,
    #[topic]
    pub attestation_type: Symbol,
    pub renewed_by: Address,
}

#[contractevent(topics = ["pause"])]
#[derive(Clone, Debug)]
pub struct PauseToggled {
    pub paused: bool,
}

#[contractevent(topics = ["default_max_ttl"])]
#[derive(Clone, Debug)]
pub struct DefaultMaxAttestationTtlChanged {
    pub max_ttl_seconds: u64,
}

#[contractevent(topics = ["max_ttl"])]
#[derive(Clone, Debug)]
pub struct MaxAttestationTtlChanged {
    #[topic]
    pub attestation_type: Symbol,
    pub max_ttl_seconds: u64,
}

#[contractevent(topics = ["signers_rotate"])]
#[derive(Clone, Debug)]
pub struct SignersRotated {
    pub new_threshold: u32,
}

#[contractevent(topics = ["schema_migrated"])]
#[derive(Clone, Debug)]
pub struct SchemaMigrated {
    pub from_version: u32,
    pub to_version: u32,
}

pub fn initialized(env: &Env, admin: &Address) {
    Initialized {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn admin_changed(env: &Env, previous: &Address, new_admin: &Address) {
    AdminChanged {
        previous: previous.clone(),
        new_admin: new_admin.clone(),
    }
    .publish(env);
}

pub fn attestor_added(env: &Env, attestor: &Address) {
    AttestorAdded {
        attestor: attestor.clone(),
    }
    .publish(env);
}

pub fn attestor_removed(env: &Env, attestor: &Address) {
    AttestorRemoved {
        attestor: attestor.clone(),
    }
    .publish(env);
}

pub fn attestor_renewed(env: &Env, attestor: &Address) {
    AttestorRenewed {
        attestor: attestor.clone(),
    }
    .publish(env);
}

pub fn attested(
    env: &Env,
    attestor: &Address,
    subject: &Address,
    attestation_type: &Symbol,
    payload_hash: &BytesN<32>,
    expires_at: u64,
) {
    Attested {
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        attestor: attestor.clone(),
        payload_hash: payload_hash.clone(),
        expires_at,
    }
    .publish(env);
}

pub fn revoked(env: &Env, subject: &Address, attestation_type: &Symbol, revoked_by: &Address) {
    Revoked {
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        revoked_by: revoked_by.clone(),
    }
    .publish(env);
}

pub fn pause_toggled(env: &Env, paused: bool) {
    PauseToggled { paused }.publish(env);
}

pub fn attestation_renewed(
    env: &Env,
    subject: &Address,
    attestation_type: &Symbol,
    renewed_by: &Address,
) {
    AttestationRenewed {
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        renewed_by: renewed_by.clone(),
    }
    .publish(env);
}

pub fn default_max_attestation_ttl_changed(env: &Env, max_ttl_seconds: u64) {
    DefaultMaxAttestationTtlChanged { max_ttl_seconds }.publish(env);
}

pub fn max_attestation_ttl_changed(env: &Env, attestation_type: &Symbol, max_ttl_seconds: u64) {
    MaxAttestationTtlChanged {
        attestation_type: attestation_type.clone(),
        max_ttl_seconds,
    }
    .publish(env);
}

pub fn signers_rotated(env: &Env, new_threshold: u32) {
    SignersRotated { new_threshold }.publish(env);
}

pub fn schema_migrated(env: &Env, from_version: u32, to_version: u32) {
    SchemaMigrated {
        from_version,
        to_version,
    }
    .publish(env);
}
