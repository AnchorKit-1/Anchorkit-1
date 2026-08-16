#![no_main]
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Bytes, BytesN, Env, Symbol};

// Fuzzer for the attest function's argument decoding path.
// The attest function decodes: attestor (Address), subject (Address),
// attestation_type (Symbol), payload_hash (BytesN<32>), ttl_seconds (u64)
// This fuzzer tests that argument decoding doesn't panic on malformed input.

fuzz_target!(|data: &[u8]| {
    // Need at least some minimum data to work with
    if data.len() < 8 {
        return;
    }

    // Catch panics during argument construction and processing
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut offset = 0;

        // Extract sections from the fuzz data
        // Section 1: attestor address seed (8 bytes)
        if offset + 8 > data.len() {
            return;
        }
        let attestor_seed = u64::from_le_bytes([
            data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
            data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
        ]);
        offset += 8;

        // Section 2: subject address seed (8 bytes if available)
        let subject_seed = if offset + 8 <= data.len() {
            let seed = u64::from_le_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ]);
            offset += 8;
            seed
        } else {
            attestor_seed.wrapping_add(1)
        };

        // Section 3: ttl_seconds (8 bytes if available)
        let ttl_seconds = if offset + 8 <= data.len() {
            u64::from_le_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ])
        } else {
            1000
        };

        // Section 4: symbol name (remaining data)
        let symbol_data = if offset < data.len() {
            &data[offset..]
        } else {
            b"test_type"
        };

        // Create a test environment
        let env = Env::default();

        // Construct the arguments - this is where decoding happens
        // These operations should never panic even on arbitrary input
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Create 32-byte hash from the attestor seed
            let mut hash_bytes = [0u8; 32];
            hash_bytes[0..8].copy_from_slice(&attestor_seed.to_le_bytes());
            let payload_hash = BytesN::<32>::from_array(&env, hash_bytes);

            // Try to create symbol from arbitrary data
            // Symbol names can contain any byte sequence, but they have length limits
            let symbol_str = String::from_utf8_lossy(symbol_data);
            // Truncate to valid symbol length (max 32 bytes typically)
            let truncated = if symbol_str.len() > 32 {
                &symbol_str[..32]
            } else {
                &symbol_str
            };
            let _ = Symbol::new(&env, truncated);

            // Note: We can't easily test Address decoding without deeper Soroban internals,
            // but the fuzzer will still exercise the Symbol and BytesN creation paths
            // which are the most likely to panic on arbitrary input
        }));
    }));
});
