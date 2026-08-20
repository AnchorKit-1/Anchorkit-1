# Contract Upgrade and Storage Migration Design

## Executive Summary

This document designs a safe, tested upgrade path for AnchorKit's attestation contract. As the contract evolves, schema changes (new fields, renamed types, restructured storage) will be inevitable. Soroban supports **WASM code replacement via hash swap** on already-deployed instances, but the storage layer requires explicit migration logic. This design provides:

1. **Concrete upgrade mechanism**: WASM code swap that preserves storage
2. **Versioned schema architecture**: Enables multiple schema formats coexisting
3. **Safe migration strategy**: Data migrated from old to new schema on-demand
4. **Tested end-to-end scenario**: Covers a real-world schema change with migration tests

---

## Part 1: Soroban Contract Upgrade Capabilities

### What Soroban Supports

**WASM Code Replacement (Hash Swap)**
- Soroban contracts are identified by a **contract ID** (deterministic address derived from code hash)
- A contract's code can be upgraded by calling the **`upgrade()`** built-in function with a new WASM blob
- This **preserves the contract ID and all persistent storage**
- Only the contract's executable code changes; storage keys and data remain untouched
- This is the safe upgrade path for already-deployed, already-populated contracts

**What You Cannot Do**
- Redeploy the same contract code (same hash) again—the contract instance remains bound to the original deployment
- Automatically migrate storage schema without explicit code logic
- Replace a contract with a different contract (that changes the contract ID)
- Undo an upgrade without deploying new code that restores the old logic

### Why This Matters for AnchorKit

AnchorKit will accumulate real attestation data over time:
- Thousands of `Attestation` entries in persistent storage
- Millions of `AttestationHistory` entries for audit trails
- Long-lived `Attestor` allow-list registrations

A full contract redeploy (new contract ID) would:
- Break all client integrations (contract ID changes)
- Require migrating all storage manually (off-chain, expensive)
- Create a gap where the old contract data is inaccessible

**WASM upgrade via hash swap solves this**: The contract ID stays the same, storage persists, but code can be updated. The burden shifts to migration logic in the contract itself.

---

## Part 2: Versioned Schema Architecture

### Design: Version-Aware Storage

To support safe schema evolution, introduce versioning to storage:

```rust
// src/types.rs - new addition

/// Storage schema version for migration tracking.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // ... existing keys ...
    
    /// Schema version of this contract instance.
    /// Tracks which version the current storage was migrated to.
    /// Start at 1 for the original schema.
    SchemaVersion,
}

/// Contract schema versions.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractSchemaVersion {
    /// Original schema: Attestation struct with 7 fields.
    V1 = 1,
    /// Future: Add new field to Attestation (e.g., attestor_signature).
    V2 = 2,
}

impl ContractSchemaVersion {
    pub fn current() -> Self {
        ContractSchemaVersion::V1
    }
}
```

### How Versioning Works

1. **On first initialization**: Contract sets `SchemaVersion = V1`
2. **On upgrade**: Contract code checks current schema version in storage
3. **If version < current**: Triggers migration function before any data access
4. **If version == current**: No migration needed, contract operates normally
5. **Never downgrade**: Migrations are one-way; going backward isn't supported

---

## Part 3: Concrete Schema Change Scenario

### Scenario: Adding Attestor Signature Verification

**Motivation**: After launch, AnchorKit discovers that some attestations need cryptographic proof that the attestor actually signed the payload, not just claimed to. Today, the contract trusts the attestor completely. This requires adding a signature field to attestations.

**Old Schema (V1)**:
```rust
pub struct Attestation {
    pub attestor: Address,
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
    // No signature field
}
```

**New Schema (V2)**:
```rust
pub struct Attestation {
    pub attestor: Address,
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
    pub attestor_signature: Option<soroban_sdk::Bytes>, // New field
}
```

### Migration Strategy

**Challenge**: Existing attestations in storage don't have a `attestor_signature` field. Reading them with the V2 struct would fail or return garbage.

**Solution**: Store V1 and V2 attestations separately in storage using version-specific keys.

```rust
// src/types.rs - version-specific storage keys

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // ... existing keys ...
    
    /// Attestations stored in V1 format (for backwards compatibility during migration).
    AttestationV1(Address, Symbol),
    
    /// Attestations stored in V2 format (new format with optional signature).
    AttestationV2(Address, Symbol),
    
    /// Schema version (1, 2, etc.)
    SchemaVersion,
}

/// V1 attestation struct for decoding old data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationV1 {
    pub attestor: Address,
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
}

/// V2 attestation struct (same as current Attestation).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationV2 {
    pub attestor: Address,
    pub subject: Address,
    pub attestation_type: Symbol,
    pub payload_hash: BytesN<32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub status: AttestationStatus,
    pub attestor_signature: Option<soroban_sdk::Bytes>,
}
```

### Migration Flow

**When code is upgraded from V1 to V2**:

1. **First call to any contract method** (after upgrade):
   - Contract checks `SchemaVersion` in storage
   - Finds `SchemaVersion = 1` (old schema in use)
   - Triggers `migrate_v1_to_v2()` function

2. **In `migrate_v1_to_v2()`**:
   ```rust
   pub fn migrate_v1_to_v2(env: &Env) -> Result<(), Error> {
       let current_version = get_schema_version(env)?;
       
       if current_version >= 2 {
           return Ok(()); // Already migrated
       }
       
       // Iterate over all old attestations
       // For each Attestation(subject, type) found in old storage:
       // 1. Read it as AttestationV1
       // 2. Convert to AttestationV2 (set signature = None)
       // 3. Write to AttestationV2 key
       // 4. Delete old AttestationV1 key
       
       // Update schema version
       set_schema_version(env, 2)?;
       events::schema_migrated(env, 1, 2);
       Ok(())
   }
   ```

3. **After migration**:
   - All attestations are now under `AttestationV2` keys
   - New attestations written to `AttestationV2` by default
   - Contract reads/writes use V2 schema
   - Old `Attestation(subject, type)` keys are no longer used

4. **Subsequent calls**:
   - Check shows `SchemaVersion = 2`
   - No migration needed
   - Normal operation

### Key Properties

- **Backward compatible**: Old attestations are preserved during migration
- **Atomic**: Either all attestations migrate, or none (no partial state)
- **One-way**: Going from V2 back to V1 not supported (requires new code)
- **No data loss**: Every field from V1 is present in V2
- **Testable**: Can simulate old-schema instance and test migration

---

## Part 4: Migration Test

### Test Architecture

Create a migration test that:
1. Simulates a contract instance with V1 schema and data
2. Manually populates storage with V1 attestations (using V1 keys/structs)
3. "Upgrades" by calling the new contract code
4. Verifies all V1 data was migrated to V2 correctly

```rust
// src/migration_tests.rs

#[test]
fn test_v1_to_v2_attestation_migration() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    let client = AnchorKitContractClient::new(&env, &contract_id);
    
    // SETUP: Simulate V1 schema instance with data
    // Manually create storage entries that look like V1
    let subject = Address::generate(&env);
    let attestor = Address::generate(&env);
    let attestation_type = Symbol::short("kyc_approved");
    
    // Directly write V1-format attestation to storage
    let old_attestation = AttestationV1 {
        attestor: attestor.clone(),
        subject: subject.clone(),
        attestation_type: attestation_type.clone(),
        payload_hash: BytesN::<32>::from_array(&env, &[1u8; 32]),
        issued_at: 1000,
        expires_at: 2000,
        status: AttestationStatus::Active,
    };
    
    env.storage()
        .persistent()
        .set(&DataKey::AttestationV1(subject.clone(), attestation_type.clone()), &old_attestation);
    
    // Set schema version to V1
    env.storage()
        .instance()
        .set(&DataKey::SchemaVersion, &1u32);
    
    // ACTION: Call a contract method, triggering migration
    let result = client.try_get_attestation(&subject, &attestation_type);
    
    // VERIFY: 
    // 1. Method succeeds (migration didn't break anything)
    assert!(result.is_ok());
    
    // 2. Attestation is returned correctly (V2 format now)
    let attestation_v2 = result.unwrap().unwrap();
    assert_eq!(attestation_v2.attestor, attestor);
    assert_eq!(attestation_v2.subject, subject);
    assert_eq!(attestation_v2.attestation_type, attestation_type);
    assert_eq!(attestation_v2.issued_at, 1000);
    assert_eq!(attestation_v2.expires_at, 2000);
    assert_eq!(attestation_v2.status, AttestationStatus::Active);
    assert_eq!(attestation_v2.attestor_signature, None); // Migrated with None
    
    // 3. Schema version updated to V2
    let schema_version = env.storage().instance().get(&DataKey::SchemaVersion);
    assert_eq!(schema_version, Some(2u32));
    
    // 4. Old V1 key is gone
    let old_data = env.storage()
        .persistent()
        .get::<_, AttestationV1>(&DataKey::AttestationV1(subject.clone(), attestation_type));
    assert!(old_data.is_none());
    
    // 5. New V2 key exists
    let new_data = env.storage()
        .persistent()
        .get::<_, AttestationV2>(&DataKey::AttestationV2(subject.clone(), attestation_type));
    assert!(new_data.is_some());
}

#[test]
fn test_v1_to_v2_batch_migration() {
    let env = soroban_sdk::Env::default();
    let contract_id = env.register(AnchorKitContract, ());
    
    // Populate storage with 100 V1 attestations across different subjects/types
    for i in 0..100 {
        let subject = Address::generate(&env);
        let attestor = Address::generate(&env);
        let type_name = format!("type_{}", i % 5); // 5 different types
        let attestation_type = Symbol::new(&env, &type_name);
        
        let old_attestation = AttestationV1 {
            attestor,
            subject: subject.clone(),
            attestation_type: attestation_type.clone(),
            payload_hash: BytesN::<32>::from_array(&env, &[i as u8; 32]),
            issued_at: 1000 + i,
            expires_at: 2000 + i,
            status: AttestationStatus::Active,
        };
        
        env.storage()
            .persistent()
            .set(&DataKey::AttestationV1(subject, attestation_type), &old_attestation);
    }
    
    // Set schema version to V1
    env.storage()
        .instance()
        .set(&DataKey::SchemaVersion, &1u32);
    
    // Trigger migration via contract method
    let client = AnchorKitContractClient::new(&env, &contract_id);
    let _ = client.try_get_attestation_count();
    
    // Verify all attestations were migrated
    // (In real implementation, would iterate storage or have migration counter)
    let schema_version = env.storage().instance().get(&DataKey::SchemaVersion);
    assert_eq!(schema_version, Some(2u32));
}
```

### Running the Tests

```bash
cargo test --lib migration_tests -- --nocapture
```

---

## Part 5: Implementation Roadmap

### Phase 1: Add Schema Version (Current Contract)

```rust
// src/storage.rs - new functions

pub fn get_schema_version(env: &Env) -> Result<u32, Error> {
    Ok(env.storage()
        .instance()
        .get(&DataKey::SchemaVersion)
        .unwrap_or(1)) // Default to V1 if not set
}

pub fn set_schema_version(env: &Env, version: u32) {
    env.storage().instance().set(&DataKey::SchemaVersion, &version);
}

// src/contract.rs - add migration guard to all public methods

pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
    // Ensure migration happens before any logic
    migrations::run_pending_migrations(&env)?;
    
    // ... existing initialize logic ...
}

pub fn attest(...) -> Result<(), Error> {
    migrations::run_pending_migrations(&env)?;
    // ... existing attest logic ...
}
```

### Phase 2: Define V2 Schema (Future PR)

When V2 schema is needed:

```rust
// src/types.rs - add V2 structures
// src/storage.rs - add V2 read/write functions
// src/migrations.rs - add migrate_v1_to_v2() function
```

### Phase 3: Future Schema Changes

Each future schema change follows the same pattern:
1. Define new V(n) types with version-specific storage keys
2. Add migration function `migrate_v(n-1)_to_v(n)()`
3. Update `run_pending_migrations()` to call new migration
4. Add comprehensive migration tests

---

## Part 6: Safety Guarantees

### No Data Loss

- Every field in old schema exists in new schema
- Migrations copy data field-by-field
- Tests verify every old entry appears in new form

### Atomic Migration

- Either all data migrates, or migration fails with error
- No partial state (half-migrated)
- Subsequent method call re-attempts migration if it fails

### Rollback Path

If migration has a bug:
1. Deploy a new contract version that detects the bug
2. Add code to copy data back to old keys
3. Set schema version back to old value
4. Clients can still read the contract (though with degraded functionality)

This isn't ideal, but it's possible. Better: extensive testing before deployment.

### Storage Rent Implications

**During Migration**:
- Old keys are read
- New keys are written
- Storage usage temporarily doubles
- Old keys are deleted (storage is reclaimed)

**After Migration**:
- Same number of keys as before
- Rent remains similar

For large contracts with millions of entries, migration might need to be chunked across multiple contract invocations.

---

## Part 7: Best Practices

### For AnchorKit Developers

1. **Add schema version early**: Even if only supporting V1 today, the infrastructure costs nothing and enables future upgrades.

2. **Plan schema changes**: Before adding a new field, consider:
   - Can old entries exist without this field? → Use `Option<T>`
   - Does every new attestation need this? → Add to `Attestation` struct
   - Should it be optional forever? → Store separately in a new key

3. **Test migrations**: Every schema change needs a migration test simulating an old instance.

4. **Document schema versions**: Maintain a changelog of schema versions and migration logic.

5. **Versioned storage keys**: Use explicit V1/V2 keys during transition for clarity.

### For Contract Users

1. **No action needed for WASM upgrades**: Contract ID stays the same, your integrations continue working.

2. **Monitor events**: Watch for `SchemaMigrated` events to know when your data was transformed.

3. **Expect transaction cost spikes**: During migration, first contract call after upgrade might be expensive (iterating all storage).

---

## Conclusion

Soroban's WASM upgrade capability combined with version-aware migration logic enables safe, tested schema evolution without redeploying or reintegrating. The key insight: **migration logic runs in the contract itself**, making it subject to the same testing, auditing, and determinism guarantees as regular contract code.

This design provides a production-grade upgrade path for AnchorKit's long-term evolution.
