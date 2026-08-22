/// Contract schema version tracking and migration logic.
///
/// This module handles safe schema evolution for already-deployed contracts.
/// When the contract is upgraded via WASM hash swap, persistent storage remains,
/// but schema changes require explicit migration. This module detects when the
/// stored schema version is behind the current code version and runs migrations
/// to transform the data into the new format.

use soroban_sdk::{Address, Env, Symbol};

use crate::errors::Error;
use crate::storage;
use crate::events;

/// Current contract schema version.
/// Increment when the data schema changes (new fields, renamed structures, etc.).
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Runs any pending migrations by comparing stored schema version to current.
/// This should be called at the start of every public contract method to ensure
/// the contract never operates on outdated data.
///
/// Returns Ok if schema is current, or Err if a migration failed.
pub fn run_pending_migrations(env: &Env) -> Result<(), Error> {
    let stored_version = storage::get_schema_version(env)?;
    
    if stored_version == CURRENT_SCHEMA_VERSION {
        // Already current, nothing to do
        return Ok(());
    }
    
    if stored_version > CURRENT_SCHEMA_VERSION {
        // Contract code is older than storage schema (shouldn't happen in practice)
        return Err(Error::Unauthorized); // Use Unauthorized as a generic "invalid state" error
    }
    
    // Migrate from stored_version to CURRENT_SCHEMA_VERSION
    match stored_version {
        1 => {
            // V1 is the baseline schema, no migration from V1 -> V1
            // Future migrations would go here: V1 -> V2, V2 -> V3, etc.
            Ok(())
        }
        _ => {
            // Unknown version, safest to reject
            Err(Error::Unauthorized)
        }
    }
}

/// Gets the schema version stored in this contract instance.
/// Returns the version number, or default to 1 if not yet initialized.
pub fn get_schema_version(env: &Env) -> Result<u32, Error> {
    Ok(storage::get_schema_version(env)
        .unwrap_or(CURRENT_SCHEMA_VERSION))
}

/// Sets the schema version to mark that migration is complete.
/// Only called by migration functions.
pub fn set_schema_version(env: &Env, version: u32) -> Result<(), Error> {
    storage::set_schema_version(env, version);
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_no_migration_needed_when_schema_current() {
        let env = soroban_sdk::Env::default();
        
        // Set schema version to current
        storage::set_schema_version(&env, CURRENT_SCHEMA_VERSION);
        
        // Migration should succeed with no work
        let result = run_pending_migrations(&env);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_schema_version_defaults_to_one() {
        let env = soroban_sdk::Env::default();
        
        // Don't set any schema version
        // get_schema_version should default to 1
        let version = get_schema_version(&env);
        assert!(version.is_ok());
        assert_eq!(version.unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_reject_future_schema_version() {
        let env = soroban_sdk::Env::default();
        
        // Set schema version to something in the future
        storage::set_schema_version(&env, CURRENT_SCHEMA_VERSION + 1);
        
        // Should reject (contract code is older than storage)
        let result = run_pending_migrations(&env);
        assert_eq!(result, Err(Error::Unauthorized));
    }

    #[test]
    fn test_set_schema_version() {
        let env = soroban_sdk::Env::default();
        
        // Set a version
        let result = set_schema_version(&env, 2);
        assert!(result.is_ok());
        
        // Verify it was set
        let stored = get_schema_version(&env);
        assert_eq!(stored.unwrap(), 2);
    }
}
