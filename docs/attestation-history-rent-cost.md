# Attestation history storage rent cost analysis

## Goal

Document the storage rent cost tradeoff of maintaining an append-only history of
attestations, and provide guidance for operators on storage budgeting.

## Overview: History vs. Overwrite

Previously, `attest` calls would simply overwrite any prior attestation for a
given (subject, attestation_type) pair. This meant past attestations were lost
forever once a new one was submitted.

The new append-only history feature preserves all past attestations so they
remain queryable via `list_attestation_history` with pagination support. This
enables:

- Auditing: view the complete timeline of decisions for a subject
- Debugging: investigate why a previous attestation was replaced
- Compliance: retain evidence of all state changes for regulatory requirements

**The tradeoff: This comes at a real ongoing storage rent cost.**

Unlike the current attestation storage (which is a single entry per
(subject, type) pair that gets overwritten), the history grows by one entry for
every `attest` or `revoke` call. Each entry is a separate persistent storage
entry that incurs its own rent per ledger. This is not implicit — it's a
meaningful cost that should be budgeted explicitly.

## The rent model

See `docs/storage-rent-cost-analysis.md` for the full Soroban rent mechanics.
In brief:

- Each persistent storage entry is billed for rent proportional to:
  - `S`: the entry's size in bytes (fixed for a given Rust struct)
  - `R`: the network's per-byte-per-ledger rent rate (a governance parameter)
  - `extend_to`: the number of ledgers bought in the last `extend_ttl` call
- Total rent for an entry = `R × S × extend_to` stroops per bump

For history entries, `extend_to` is scaled to the attestation's remaining
lifetime (see `storage::attestation_ttl_window`), so short-lived attestations
pay less rent per history entry than long-lived ones.

## Worked cost comparison: current vs. history

Let's model the rent impact of storing history for a typical KYC/payment
workload.

### Setup: Assumptions

- Workload: 1 million subjects, each with 1 attestation_type
- Typical TTL: 90 days (a common re-verification window)
- Update rate: 1 update per subject every 180 days (2 updates per subject per year)
- Entry size: ~150 bytes for an `Attestation` struct (attestor, subject, type,
  hash, timestamps, status)
- Network rent rate `R`: 4.2 stroops per byte per ledger (current mainnet, via
  Stellar docs; this may change with governance)

### Scenario 1: Current (no history)

Each subject has exactly one attestation entry that gets overwritten on update.
Storage rent is paid only when entries are first written or renewed.

- Entries: 1 million
- Rent per entry per 90-day update cycle:
  - `extend_to` = 90 days = 17,280 ledgers (see `storage::LEDGERS_PER_DAY`)
  - Rent = 4.2 × 150 × 17,280 = **~10.9M stroops per entry per 90 days**
- Total for 1M entries over 2 years (8 cycles):
  - Cost = 1M × 10.9M × 8 = **87.2 trillion stroops** (~8.7 billion stroops/year)

### Scenario 2: With 2-year history (4 updates per subject)

Each subject accumulates 4 attestation history entries over 2 years.

- Total entries: 1M subjects × 4 entries = 4 million
- Rent per entry: same as above, scaled to its remaining lifetime
  - For simplicity, assume all entries live for their full 90-day window
  - Rent per entry = ~10.9M stroops per 90 days
- Total for 4M entries over 2 years:
  - Cost = 4M × 10.9M × 8 = **348.8 trillion stroops** (~34.9 billion stroops/year)

### Cost impact

- Current (no history): ~8.7B stroops/year
- With history: ~34.9B stroops/year
- **Multiplier: ~4x** (exactly the number of updates per subject over 2 years)

In other words, **keeping history costs roughly as much as storing N copies of
the entry where N is the number of updates.** This is expected — each update
creates a new entry that must be stored and rented forever (or until it expires,
if you implement TTL policies for history).

For a large attestation platform serving many subjects with frequent updates,
this can become a significant operational cost. See "Cost optimization strategies"
below.

## Cost optimization strategies

### 1. Baseline: Accept the cost

If audit trails and compliance requirements justify the cost, simply budget for
the 4x (or higher, depending on update frequency) multiplier and maintain the
full history forever.

### 2. History entry TTL policies

History entries are currently stored forever (they only expire when the
attestation's logical TTL expires). You could instead implement a shorter
effective TTL for *old* history entries:

- Store a separate `HistoryEntryTTL(subject, type, seq)` entry in instance
  storage (cheaper than persistent) with an explicit archival timestamp
- Periodically prune entries older than, say, 1 year
- This trades off queryability of ancient history for lower rent

Example: keeping only the last 12 months of history for a subject reduces the
above scenario from 4M total entries to ~2M (assuming uniform distribution of
updates), cutting the cost to ~17.5B stroops/year (~2x current baseline).

### 3. Separate history contract

For very large workloads, you could move history storage to a separate contract
or off-chain system:

- The main contract still stores the current attestation (unchanged)
- History is written to a read-only off-chain log or archive contract
- Consumers who need history query the archive; most queries still use the main
  contract's faster, cheaper current-only read path

This entirely decouples history cost from the main contract's operational budget.

### 4. Aggregated history summaries

Instead of storing every single update, periodically snapshot aggregate history:

- Store a summary entry (e.g., "this subject had 3 attestations in Q1") instead
  of 3 individual entries
- Consumers get high-level history without per-entry rent costs

This reduces rent linearly with the aggregation factor.

## Monitoring and alerts

To avoid surprise storage cost growth, consider instrumenting:

- `get_attestation_count()` — tracks total attestations submitted (already
  included)
- New: `list_attestation_history(subject, type, 1, 1, false)` on a sample of
  subjects to estimate average history length per subject
- Multiply to project storage rent impact: `subjects × avg_history_length ×
  entry_rent_per_ledger`

Alert if:
- History length grows faster than expected (suggests higher-than-anticipated
  update rates)
- Average entry rent per ledger increases (might indicate longer TTLs being
  used, or network rent rate changes)

## Decision: Why keep full history by default?

Three reasons:

1. **Auditability is hard to retrofit.** If you delete history now and later
   need it for compliance, you can't recover it. Starting with full history lets
   operators make that choice later (via pruning strategies above) without
   losing data retroactively.

2. **Cost is predictable and linear.** Unlike surprise runtime bugs or network
   changes, storage cost scales predictably with the number of updates. It's a
   known tradeoff operators can budget for.

3. **History is queryable on demand.** Operators pay only once per entry, then
   can query it many times for free (within the entry's TTL). The upfront cost
   is amortized across all queries.

## See also

- `docs/storage-rent-cost-analysis.md` — detailed Soroban rent mechanics
- `src/storage.rs` — TTL constants and rent window calculations
- `src/contract.rs::list_attestation_history` — pagination API docs
