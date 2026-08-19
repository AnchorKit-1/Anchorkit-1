# Pre-audit self-review checklist

This is a working checklist of this contract's actual attack surface,
method by method, covering the three risk classes most relevant to a
Soroban contract: **reentrancy via cross-contract calls**, **auth bypass**,
and **storage-exhaustion / rent-griefing**. It is meant to be the starting
point for a real internal review before any mainnet deployment -- not a
box-ticking exercise -- so every finding below is tied to specific lines in
`src/contract.rs`, and every method is marked either **✅ reviewed and
safe** or **⚠️ needs mitigation**.

This project already has several deep, single-topic security writeups
(admin-key threat modeling, replay protection, max-TTL validation, storage
rent). This checklist doesn't repeat that analysis -- it links to it from
the relevant method and spends its own words on what those documents
don't cover: reentrancy, and a method-by-method sweep that surfaces a few
gaps none of the existing docs mention.

If you're doing that review: start with [Open findings](#open-findings),
then use the per-method sections for the reasoning behind every verdict
(including the "safe" ones -- re-derive them, don't just trust them).

## Threat model

Three actor classes, in increasing order of trust:

- **Public callers** -- anyone. Can call any method; most either require no
  auth (reads) or require auth as a specific party (writes).
- **Attestors** -- addresses the admin has explicitly allow-listed via
  `add_attestor`. Trusted to submit honest attestations, but this checklist
  treats a given attestor as a potential bad actor within the bounds of
  what an attestor is allowed to do (the "malicious or buggy attestor"
  framing used elsewhere in this project's issue tracker). An attestor
  cannot add itself; only the admin can do that.
- **Admin** -- one address, set at `initialize` and transferable via
  `set_admin`. Fully trusted for the purposes of this checklist -- "what
  could a malicious admin do" is out of scope here, it's the entire
  subject of [`docs/threat-model-admin-key.md`](threat-model-admin-key.md).
  This checklist treats "the admin makes an honest mistake" or "the
  admin's control is bypassed by someone else" as in-scope, not "the
  admin acts maliciously."

## A note on reentrancy in Soroban

This contract makes **no cross-contract calls of its own** -- there is no
`env.invoke_contract` (or equivalent) anywhere in `src/contract.rs`. The
only place execution can leave this contract mid-call is `require_auth()`
on a caller-supplied `Address`: if that address is a custom-account
contract (rather than a plain keypair), `require_auth()` synchronously
invokes that contract's `__check_auth`.

That invocation goes through the host's standard contract-call path, which
always sets `ContractReentryMode::Prohibited` for the caller (confirmed by
reading `host.rs`/`frame.rs` in `soroban-env-host` 26.1.4, the version this
project currently depends on via `soroban-sdk 26.1.1`): a contract that's
already on the call stack cannot be re-entered unless it explicitly opts
in (`#[contractimpl]` here does not), so a hostile `__check_auth` cannot
call back into `AnchorKitContract` while a public method of this contract
is still executing. **This is a platform guarantee, not something this
contract's code does itself** -- it should be re-confirmed against
whatever protocol version is actually live at mainnet-deployment time,
since host behavior here is exactly the kind of thing worth an auditor
independently verifying rather than taking this document's word for it.

Given that, "reentrancy" below is addressed per-method only where a
`require_auth()` call happens *before* a state-changing operation that a
reentrant call (if it were possible) could interleave with -- i.e. where
the ordering would matter if the platform guarantee above ever changed or
turned out to be narrower than assumed.

## Open findings

Everything marked ⚠️ below, gathered in one place:

| # | Method | Finding | Category |
|---|--------|---------|----------|
| 1 | `renew_attestation` | Not blocked by `pause()`, unlike `attest`/`attest_batch`/`revoke` -- an attestor the admin is actively trying to contain via pause can still keep their existing attestations alive. | Auth / incident-response gap |
| 2 | `list_attestation_history` | In `reverse` mode, iteration is bounded by caller-supplied `limit`/`start_seq` alone, not by how much history actually exists -- unlike forward mode, which self-limits against the real `total_seq`. A caller-controlled `(start_seq, limit)` pair can force a huge iteration count on a pair with little or no real history. | Storage-exhaustion / resource-griefing |
| 3 | `attest_batch` | `entries: Vec<BatchAttestEntry>` has no application-level size cap; only implicit network-level resource limits bound a single call. | Storage-exhaustion / rent-griefing |
| 4 | `attest` / `attest_batch` | No cap on the *number* of distinct `(subject, attestation_type)` keys -- and, since attestation history was added, the number of *history* entries -- a single allow-listed attestor can create over time. | Storage-exhaustion / rent-griefing |
| 5 | `attest` / `attest_batch` | `attestation_type` has no application-level length cap (only the host's own 32-character `Symbol` ceiling applies), so a malicious/buggy attestor can still pick maximally-sized keys within that ceiling. Tracked separately as issue #57. | Storage-exhaustion / rent-griefing |
| 6 | `initialize` | First caller to submit a valid `initialize` transaction wins the admin seat; nothing enforces that deployment and initialization happen atomically. See also [`docs/threat-model-admin-key.md`](threat-model-admin-key.md). | Auth (front-running) |
| 7 | `set_admin` | Single-step transfer: an admin who authorizes a transfer to a mistyped/uncontrolled `new_admin` cannot recover. This is the central problem [`docs/threat-model-admin-key.md`](threat-model-admin-key.md) is about -- see that document for the full analysis and proposed mitigations (timelock / multisig / social recovery) rather than re-deriving it here. | Auth (irrecoverable-loss risk) |

None of these are exploitable by an unprivileged public caller to bypass
authorization outright -- they're gaps in defense-in-depth and
incident-response completeness, which is exactly the kind of thing meant
to be caught here rather than in a real incident.

## Admin & lifecycle

### `initialize(env, admin)`
- **Auth bypass:** ✅ safe. Guarded by `storage::is_initialized` before
  `admin.require_auth()`, so it can only succeed once, and only with an
  `admin` address that itself authorized the call -- nobody can install
  themselves as admin without `admin`'s signature.
- ⚠️ **Needs mitigation (front-running, finding #6):** the *first*
  transaction to call `initialize` with a validly-authorizing `admin` wins,
  and there's no atomicity between contract deployment and this call. On a
  public network, a still-uninitialized contract is a race. Mitigate
  operationally: submit deployment (upload + create) and `initialize` in
  the same transaction, or at minimum the same submission batch, rather
  than as separate steps.
- **Storage-exhaustion:** ✅ safe. Writes exactly two fixed-size instance
  keys (`Admin`, `Paused`); no caller-controlled growth.

### `get_admin(env)`
Read-only, no auth. ✅ safe on all three axes -- the admin address isn't
sensitive (it's the counterparty for every admin-gated auth check already
visible on-chain).

### `set_admin(env, new_admin)`
- **Auth bypass:** ✅ safe against third parties -- only the *current*
  admin's authorization can move the seat (`current.require_auth()`
  against the value already in storage, not against caller input).
- ⚠️ **Needs mitigation (irrecoverable loss, finding #7):** single-step
  transfer with no accept/claim step by `new_admin`. See
  [`docs/threat-model-admin-key.md`](threat-model-admin-key.md) ("Scenario
  3: Admin Transfer to Attacker" and the multisig/timelock/social-recovery
  strategies) for the full treatment -- this checklist just flags that the
  gap exists at the `set_admin` call site specifically.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban) --
  `current.require_auth()` happens before the write, so even a hostile
  `__check_auth` on the *current* admin's address can't observe or race
  the update.
- **Storage-exhaustion:** ✅ safe. Overwrites the single `Admin` key.

### `is_paused(env)`
Read-only; requires `is_initialized` first (`NotInitialized` otherwise).
✅ safe on all three axes.

### `pause(env)` / `unpause(env)`
- **Auth bypass:** ✅ safe. Admin-only via `storage::get_admin` +
  `require_auth()`; `non_admin_cannot_pause` covers this. See also
  [`docs/threat-model-admin-key.md`](threat-model-admin-key.md) ("Scenario
  2: Service Disruption via Permanent Pause") for what an admin-key
  compromise does with this method specifically -- out of scope here since
  that's an admin-trust question, not an auth-bypass one.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
- **Storage-exhaustion:** ✅ safe. Single `Paused` boolean.
- Both are the mechanism finding #1 (`renew_attestation`) is evaluated
  against.

## Attestor allow-list

All four methods below share one property that matters for every risk
category: **they're admin-only.** An external, non-admin caller cannot
reach any of them, which removes most of the griefing surface a
public-facing "add a key to persistent storage" method would otherwise
have. The remaining risk is entirely about the admin's own trust boundary
(out of scope -- see [Threat model](#threat-model)).

### `add_attestor(env, attestor)`
- **Auth bypass:** ✅ safe. Admin-only; `AttestorAlreadyRegistered` guards
  double-registration.
- **Storage-exhaustion:** ✅ safe *given the admin-only gate*. An
  unprivileged caller cannot create `Attestor` keys at all.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban)
  (`admin`'s `require_auth()` precedes the write).

### `remove_attestor(env, attestor)`
Symmetric to `add_attestor`; guarded by `AttestorNotRegistered`. ✅ safe on
all three axes for the same reasons.

### `is_attestor(env, attestor)`
Read-only, no auth. ✅ safe.

### `renew_attestor(env, attestor)`
Admin-only re-touch of an existing key's TTL (no new key created). ✅ safe
on all three axes. Unlike `renew_attestation` below, there's no incident
scenario where blocking this during a pause matters -- it's purely
housekeeping the admin controls, and is not reachable by the party (an
attestor) a pause might be trying to contain.

## Max-TTL configuration

`set_default_max_attestation_ttl`, `set_max_attestation_ttl`, and
`get_max_attestation_ttl` are covered in depth by
[`docs/max-ttl-validation.md`](max-ttl-validation.md) (that document is
the design writeup for this whole feature); this checklist adds only the
reentrancy/auth-bypass/storage angle that document doesn't focus on.

### `set_default_max_attestation_ttl(env, max_ttl_seconds)`
- **Auth bypass:** ✅ safe. Admin-only (`only_admin_can_set_default_max_ttl`);
  rejects `0` with `InvalidExpiration`.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
- **Storage-exhaustion:** ✅ safe. Single `DefaultMaxAttestationTtl`
  instance key, admin-only.

### `set_max_attestation_ttl(env, attestation_type, max_ttl_seconds)`
- **Auth bypass:** ✅ safe. Admin-only (`only_admin_can_set_per_type_max_ttl`);
  same zero-rejection as the default setter.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
- **Storage-exhaustion:** ✅ safe *given the admin-only gate* -- same
  reasoning as `add_attestor`. `attestation_type` is unbounded in length
  here too (see finding #5), but since this method is admin-only, an
  external caller can't use it to create keys at all, only the admin can
  (and that's already the accepted trust boundary for the allow-list
  above).

### `get_max_attestation_ttl(env, attestation_type)`
Read-only, no auth. ✅ safe.

## Attestations

### `attest(env, attestor, subject, attestation_type, payload_hash, ttl_seconds)`
This is the main public write path and the highest-risk method in the
contract.
- **Auth bypass:** ✅ safe. Blocked while paused; `attestor.require_auth()`
  plus an `AttestorNotRegistered` allow-list check gate every write. An
  attestor can only ever attribute attestations to itself (`attestor` is
  both the authorizing party and the stored value -- there's no way to
  submit on another attestor's behalf).
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
  `storage::is_paused` is checked before `require_auth()`, and
  `storage::is_attestor` after it, but since a reentrant call back into
  this contract is rejected by the host regardless, the ordering doesn't
  create a window today.
- ⚠️ **Needs mitigation (storage-exhaustion, findings #4 and #5, shared
  with `attest_batch`):** the allow-list gate bounds *who* can write, not
  *how many* distinct entries they can write, nor how large each
  `attestation_type` key is. Every `(subject, attestation_type)` pair not
  previously used creates a new persistent key, and -- since attestation
  history was added -- *every* `attest` call, including one that
  overwrites an existing pair, appends a new, permanent
  `AttestationHistory` entry (see
  [`docs/attestation-history-rent-cost.md`](attestation-history-rent-cost.md),
  which documents the cost of this tradeoff in detail but doesn't itself
  propose a cap). So growth is no longer bounded by distinct pairs the way
  it would be without history -- it's effectively bounded only by how many
  times an allow-listed attestor is willing to pay to call `attest`. This
  is partially self-limiting economically (the calling attestor's
  transaction pays the rent for every entry it creates), but that's a
  cost, not a hard limit, and doesn't protect against a buggy (rather than
  economically-rational-malicious) attestor generating large volumes of
  entries. Consider an explicit per-attestor rate limit or outstanding-
  attestation cap if this needs a hard bound rather than an economic one.
  `attestation_type`'s own length is unbounded up to the host's 32-
  character `Symbol` ceiling; issue #57 tracks adding an explicit,
  smaller, application-level cap with a dedicated error.

### `attest_batch(env, attestor, entries)`
Shares `record_attestation` (and therefore the findings above) with
`attest`, amplified by batching:
- **Auth bypass:** ✅ safe. Same pause/auth/allow-list gates as `attest`,
  checked once for the whole call (correctly -- see
  `attest_batch_fails_for_unregistered_attestor`,
  `attest_batch_fails_when_paused`).
- **Atomicity:** ✅ safe and already tested
  (`invalid_entry_fails_the_whole_batch`,
  `attest_batch_respects_max_ttl_constraints`). TTLs (and the max-TTL
  check) are validated for every entry *before* any entry is written, so
  one invalid entry can't leave partial state.
- ⚠️ **Needs mitigation (storage-exhaustion, finding #3):** `entries` is
  an unbounded `Vec<BatchAttestEntry>`. There's no application-level
  `MAX_BATCH_SIZE`-style check; the only backstop is the network's own
  per-transaction resource limits (max instructions, max ledger footprint
  entries -- see the "Hard ceiling, not just a cost curve" section of
  `README.md`, which documents that a *15-entry* batch is already close to
  the observed footprint ceiling under the test host's default limits).
  That existing ceiling is real, but it's an artifact of network config
  the contract inherits silently rather than something the contract
  itself asserts or documents as a limit. Consider an explicit
  `Error::BatchTooLarge`-style cap sized well under the observed ceiling,
  both for predictable gas costs and so the contract's own limits don't
  depend on inheriting network config unchanged.
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).

### `record_attestation(env, attestor, subject, attestation_type, payload_hash, ttl_seconds)` *(private helper, shared by `attest`/`attest_batch`)*
- **Correctness:** ✅ safe. `expires_at = issued_at.saturating_add(ttl_seconds)`
  is proptest-covered at the `u64::MAX` boundary; see issue #55 for the
  dedicated property test confirming `saturating_add` doesn't silently
  wrap even when the newer `ExceedsMaxTtl` check (see
  [`docs/max-ttl-validation.md`](max-ttl-validation.md)) is raised out of
  the way so the arithmetic under test actually runs.
- **Storage-exhaustion:** see the `attest` finding above; this is where
  the actual `storage::set_attestation` / `storage::push_attestation_history`
  writes happen, but the risk is about the callers' lack of a cap, not
  this function's own logic.
- Not independently reachable (no `pub` visibility), so auth bypass /
  reentrancy are addressed at the `attest` / `attest_batch` call sites.

### `get_attestation(env, subject, attestation_type)`
Read-only, no auth. ✅ safe.

### `is_valid(env, subject, attestation_type)`
Read-only, no auth. The `expires_at` boundary is exclusive
(`timestamp < expires_at`) and explicitly tested
(`is_valid_false_exactly_at_expires_at`). ✅ safe.

### `revoke(env, caller, subject, attestation_type)`
- **Auth bypass:** ✅ safe. Blocked while paused
  (`pause_blocks_revoke`); restricted to `caller == admin || caller ==
  attestation.attestor` (`Error::Unauthorized` otherwise), and double-
  revocation is guarded (`AttestationAlreadyRevoked`). `caller.require_auth()`
  runs before the attestation lookup, so an unrelated caller still pays to
  prove their own identity before being rejected -- a minor gas
  inefficiency, not a security issue (they can't influence anything by
  doing so, and can't skip proving who they are to find out whether
  they'd be allowed).
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
- **Storage-exhaustion:** ✅ safe as far as the `Attestation` entry itself
  (overwritten in place, `status` field only), but note it *also* appends
  a permanent `AttestationHistory` entry via `push_attestation_history`,
  same as `attest` -- see finding #4. A caller with a valid attestation to
  revoke can't create *new* `(subject, attestation_type)` pairs this way
  (only the admin/original attestor can call `revoke`, and only on a
  pair that already exists), so this doesn't independently widen the
  griefing surface beyond what `attest` already established.

### `renew_attestation(env, caller, subject, attestation_type)`
- **Auth bypass:** ⚠️ **needs mitigation (finding #1).** Restricted to
  `caller == admin || caller == attestation.attestor`, same as `revoke` --
  that part is correctly enforced. But unlike `attest`, `attest_batch`,
  and `revoke`, this method has **no `storage::is_paused` check at all**.
  `pause`'s own doc comment says it "halts new attestations (`attest`)
  and revocations while paused" and lists reads as the only thing that
  keeps working -- `renew_attestation` isn't mentioned either way, and as
  written it silently falls outside that guarantee. Concretely: if the
  admin pauses the contract specifically because a given attestor is
  behaving badly, that same attestor can still call `renew_attestation`
  on their own attestations and keep them alive, undermining the intent
  of pausing. Fix: add the same `if storage::is_paused(&env) { return
  Err(Error::ContractPaused); }` guard used in `attest` and `revoke`, or
  -- if letting attestors keep their own entries alive during a pause is
  actually intended -- document that explicitly in `pause`'s doc comment
  so it's a decision, not a gap. (`renew_attestor`, the allow-list
  equivalent, doesn't have this problem: it's admin-only, so there's no
  scenario where the party being contained by a pause is also the one
  calling it.)
- **Reentrancy:** ✅ safe, per the [reentrancy note](#a-note-on-reentrancy-in-soroban).
- **Storage-exhaustion:** ✅ safe. Re-touches an existing key's TTL only;
  creates no new entries (unlike `attest`/`revoke`, this one does *not*
  call `push_attestation_history`).

### `get_attestation_count(env)`
Read-only, no auth. ✅ safe.

### `list_attestation_history(env, subject, attestation_type, start_seq, limit, reverse)`
- **Auth bypass:** ✅ safe. Read-only, no auth required or expected --
  history is exactly as public as `get_attestation`.
- **Reentrancy:** N/A -- no state mutation.
- ⚠️ **Needs mitigation (storage/resource-exhaustion, finding #2):**
  `storage::list_attestation_history` validates `limit != 0`
  (`Error::InvalidPagination`) but never caps `limit`'s *upper* bound, and
  the two pagination directions aren't symmetric in how they're bounded:
  - **Forward** (`reverse: false`): `for current in (start_seq..).take(limit
    as usize) { if current > total_seq { break; } ... }` -- self-limits
    against the pair's real `total_seq`, so a caller can't force more
    lookups than history actually has entries, regardless of how large
    `limit` is.
  - **Reverse** (`reverse: true`): `for _ in 0..limit { if current == 0 {
    break; } ...; current = current.saturating_sub(1); }` -- has no
    equivalent check against `total_seq`. A caller who passes a large
    `start_seq` together with a large `limit` (both fully caller-
    controlled, u64/u32 respectively) forces up to `min(limit, start_seq)`
    storage lookups, the large majority of which will be misses on a pair
    with little or no real history.
  This is a read-only call, so the immediate cost lands on the caller's
  own transaction budget rather than corrupting contract state -- but if
  this method is ever called by another contract on a caller-influenced
  `limit`/`start_seq` (a plausible integration pattern for a pagination
  API like this one), an untrusted end user could use it to waste *that
  integrator's* resource budget. Mitigate by capping `limit` to a small
  constant (`Error::InvalidPagination` or a new, more specific error for
  values above it) and/or adding the same `current > total_seq`-style
  bound to the reverse branch that the forward branch already has.

## Re-running this review

Everything above is tied to the code as of this file's introduction. If
`src/contract.rs` gains a method, changes an auth/pause check, or this
project's `soroban-sdk`/`soroban-env-host` dependency is bumped (which
could change the reentrancy-guarantee assumption this checklist leans on),
re-derive the affected entries rather than assuming they still hold.
