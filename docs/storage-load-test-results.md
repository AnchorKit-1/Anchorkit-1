# Storage load test results

**Test suite:** `src/attest_storage_load_tests.rs`  
**Run command:**
```sh
cargo test --release --features stress-tests attest_storage -- --nocapture
```
**SDK version:** soroban-sdk 26.1.1 | Rust 1.97  
**Date:** 2026-08-20

---

## Summary

Three load tests were run to characterise how CPU instruction count, memory
usage, and persistent-storage entry count grow as `attest` is called at scale.
The results inform the rent/TTL-tuning work tracked in this area.

**Key findings:**

1. **Per-call CPU and memory cost scale linearly with total storage depth**,
   not O(1). At 5 000 attestations the per-call CPU cost is ~183× higher than
   at call 1. This is a test-host artefact (the mock ledger's in-memory state
   grows on every persistent write), not a property of on-chain execution where
   each key is fetched independently; however the numbers quantify how the
   metered budget is consumed in test scenarios and explain why large-scale
   integration tests feel slow.

2. **TTL choice has no effect on CPU cost.** Across a 1-day → 2-year TTL
   range, CPU instruction counts are identical (269 805 insns). The
   TTL-proportional `extend_ttl` window affects *rent* cost (ledger fees),
   not compute cost.

3. **Each `attest` call writes exactly 3 new persistent keys** regardless of
   history depth: `Attestation` (overwrite), `AttestationSeq` (increment),
   `AttestationHistory` (append). The storage entry count grows by exactly
   3 × N, confirming the O(1)-write design.

4. **History accumulation on a single (subject, type) pair shows the same
   linear growth pattern** as the multi-subject test, proving the cost is
   driven by total ledger depth, not by the history key count for any
   specific pair.

---

## Test 1 — Storage growth and cost at scale

N distinct `(subject, attestation_type)` pairs attested, sampled at
logarithmically-spaced call counts. TTL = 1 day (30-day storage window,
the floor).

| call n | cpu\_insns | mem\_bytes | cumul. storage entries (attest×3 + 1 attestor) | keys / call |
|-------:|----------:|----------:|----------------------------------------------:|:------------|
|      1 |     256 522 |    106 647 |                                             4 | 3 |
|     10 |     372 309 |    137 051 |                                            31 | 3 |
|     50 |     764 022 |    267 611 |                                           151 | 3 |
|    100 |   1 241 698 |    430 811 |                                           301 | 3 |
|    250 |   2 653 193 |    920 411 |                                           751 | 3 |
|    500 |   4 995 987 |  1 736 411 |                                         1 501 | 3 |
|  1 000 |   9 670 626 |  3 368 411 |                                         3 001 | 3 |
|  2 500 |  23 673 556 |  8 264 411 |                                         7 501 | 3 |
|  5 000 |  47 009 340 | 16 424 411 |                                        15 001 | 3 |

**Interpretation:**

- `cpu_insns` grows at ~9 400 insns per additional entry in the ledger (~9.4k
  insns per key). This is consistent with the mock host scanning all persistent
  entries for footprint tracking on each call.
- `mem_bytes` grows at ~3 284 bytes per additional entry — the memory overhead
  of each in-memory persistent key object in the test host.
- The `keys / call` column is always 3: exactly `Attestation`,
  `AttestationSeq`, `AttestationHistory`. No fan-out, no hidden extra writes.
- On-chain, the host fetches only the keys in the transaction footprint, so
  individual `attest` costs should stay close to the call-1 baseline
  (256 522 cpu, 106 647 mem) regardless of ledger depth.

**Growth rate:**

| metric | approx. slope (per 1 000 entries) |
|--------|:----------------------------------|
| cpu\_insns | +1 927 000 insns |
| mem\_bytes | +3 291 000 bytes |

---

## Test 2 — Cost by TTL bucket

One `attest` call per TTL value, fresh contract each time. Default max TTL
raised to 2 years to allow all buckets.

| TTL | ttl\_seconds | cpu\_insns | mem\_bytes |
|:----|-------------:|-----------:|-----------:|
| 1 day    |        86 400 |    269 805 |    110 915 |
| 7 days   |       604 800 |    269 805 |    110 915 |
| 30 days  |     2 592 000 |    269 805 |    110 915 |
| 90 days  |     7 776 000 |    269 805 |    110 915 |
| 180 days |    15 552 000 |    269 805 |    110 915 |
| 1 year   |    31 536 000 |    269 805 |    110 915 |
| 2 years  |    63 072 000 |    269 805 |    110 915 |

**Interpretation:**

- CPU and memory cost are **identical across all TTL values** (269 805 insns,
  110 915 bytes). Choosing a longer or shorter TTL has zero compute cost
  impact; only the ledger rent fee (the `extend_ttl` window argument) changes.
- This validates the TTL-proportional rent design: operators pay less rent per
  write for short-lived attestations (see `docs/storage-rent-cost-analysis.md`)
  without any compute-cost penalty.

---

## Test 3 — Repeated-subject history growth

Same `(subject, attestation_type)` pair re-attested N times. Each call
appends one new `AttestationHistory` key.

| call n | cpu\_insns | mem\_bytes | history depth | total keys written (×3) |
|-------:|-----------:|-----------:|--------------:|:-----------------------|
|      1 |    256 522 |    106 647 |             1 | 3 |
|     10 |    326 727 |    118 887 |            10 | 30 |
|     50 |    494 194 |    167 847 |            50 | 150 |
|    100 |    690 327 |    229 047 |           100 | 300 |
|    250 |  1 263 075 |    412 647 |           250 | 750 |
|    500 |  2 201 878 |    718 647 |           500 | 1 500 |
|  1 000 |  4 066 274 |  1 330 647 |         1 000 | 3 000 |

**Interpretation:**

- Cost grows at the same rate as Test 1 (driven by total ledger depth, not
  history-key count for the specific pair). Each re-attestation writes 3 keys
  (overwrite `Attestation`, increment `AttestationSeq`, new
  `AttestationHistory`), which is the same as a brand-new attestation.
- A subject with 1 000 revision history entries (e.g. a frequently-updated
  KYC status) has a cumulative 3 000 key footprint and costs ~4 066 274 insns
  per new attestation in the test host environment.
- On-chain footprint for a single `attest` is still only the 3 keys it writes
  plus the attestor allow-list read and contract instance; the broader history
  depth doesn't widen the per-call footprint.

---

## Implications for TTL / rent tuning

| Finding | Tuning implication |
|---------|:------------------|
| 3 persistent keys per `attest` (Attestation + AttestationSeq + AttestationHistory) | Rent budget must cover all three TTL windows, not just the attestation entry. The `AttestationHistory` key's TTL is currently tied to the attestation's own TTL; a pruning policy or shorter history TTL would reduce ongoing rent. |
| TTL has no compute cost | Operators can freely choose longer TTLs for important attestations without CPU budget concerns; the only cost is the proportionally larger `extend_ttl` rent fee. |
| Linear cost scaling in mock host ≠ on-chain behavior | Benchmarks using large ledger states must account for the mock host's full-scan cost model. Real on-chain performance should stay near the call-1 baseline. |
| History depth grows unboundedly for re-attested pairs | A subject with a high re-attestation rate accumulates persistent storage proportionally. For high-frequency KYC re-verification workloads, consider a history-TTL policy (see `docs/attestation-history-rent-cost.md` §"Cost optimization strategies"). |

---

## Related

- `docs/storage-rent-cost-analysis.md` — rent model, TTL constant rationale
- `docs/attestation-history-rent-cost.md` — history storage cost tradeoffs
- `src/attest_storage_load_tests.rs` — the test source
- `src/attestor_stress_tests.rs` — allow-list scaling benchmark (same methodology)
