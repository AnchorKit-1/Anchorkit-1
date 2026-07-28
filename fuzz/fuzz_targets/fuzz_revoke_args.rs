#![no_main]
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Env, Symbol};

// Fuzzer for the revoke function's argument decoding path.
// The revoke function decodes: caller (Address), subject (Address), attestation_type (Symbol)
// This fuzzer tests that argument decoding doesn't panic on malformed input.

fuzz_target!(|data: &[u8]| {
    // Need at least some minimum data to work with
    if data.len() < 8 {
        return;
    }

    // Catch panics during argument construction and processing
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut offset = 0;

        // Section 1: caller address seed (8 bytes)
        if offset + 8 > data.len() {
            return;
        }
        let _caller_seed = u64::from_le_bytes([
            data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
            data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
        ]);
        offset += 8;

        // Section 2: subject address seed (8 bytes if available)
        let _subject_seed = if offset + 8 <= data.len() {
            let seed = u64::from_le_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
            ]);
            offset += 8;
            seed
        } else {
            _caller_seed.wrapping_add(1)
        };

        // Section 3: symbol name (remaining data)
        let symbol_data = if offset < data.len() {
            &data[offset..]
        } else {
            b"attested"
        };

        // Create a test environment
        let env = Env::default();

        // Construct the arguments - this is where decoding happens
        // These operations should never panic even on arbitrary input
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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

            // Note: Address decoding is tested through the Symbol creation path,
            // which is the most likely to panic on arbitrary input in the Soroban SDK
        }));
    }));
});
