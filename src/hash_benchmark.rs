//! Benchmarks SHA-256 hashing performance under the Soroban SDK.
//! Run with `cargo test benchmark_sha256_hashing -- --nocapture`.

extern crate std;

use soroban_sdk::{Bytes, Env};
use crate::hash::compute_payload_hash;
use std::time::Instant;

#[test]
fn benchmark_sha256_hashing() {
    let sizes = [32, 64, 1024, 16384, 32768, 65536];

    std::println!(
        "\n| Payload Size | Iterations | CPU Instructions | Memory (Bytes) | Avg Time (ns) | Throughput (MB/s) |"
    );
    std::println!(
        "|-------------:|-----------:|-----------------:|---------------:|--------------:|------------------:|"
    );

    for &size in sizes.iter() {
        let env = Env::default();
        let mut budget = env.cost_estimate().budget();

        // Construct input payload data
        let data_vec = std::vec![0u8; size];
        let data = Bytes::from_slice(&env, &data_vec);

        // Measure CPU and memory overhead by isolating the compute call
        budget.reset_unlimited();
        let _hash = compute_payload_hash(&env, &data);
        let cpu_cost = budget.cpu_instruction_cost();
        let mem_cost = budget.memory_bytes_cost();

        // Run loop iterations to calculate stable elapsed wall-clock time
        let iterations = match size {
            s if s <= 64 => 50000,
            1024 => 10000,
            _ => 1000,
        };

        budget.reset_unlimited();
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = compute_payload_hash(&env, &data);
        }
        let duration = start.elapsed();

        let avg_time_ns = duration.as_nanos() as f64 / iterations as f64;
        let size_mb = size as f64 / (1024.0 * 1024.0);
        let throughput = if duration.as_secs_f64() > 0.0 {
            (size_mb * iterations as f64) / duration.as_secs_f64()
        } else {
            0.0
        };

        let size_str = if size < 1024 {
            std::format!("{} B", size)
        } else {
            std::format!("{} KB", size / 1024)
        };

        std::println!(
            "| {:>12} | {:>10} | {:>16} | {:>14} | {:>13.1} | {:>17.2} |",
            size_str,
            iterations,
            cpu_cost,
            mem_cost,
            avg_time_ns,
            throughput
        );
    }
    std::println!();
}
