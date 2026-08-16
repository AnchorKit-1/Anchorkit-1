# Threat Model: Single Admin Key in AnchorKit

## Executive Summary

AnchorKit-1's attestation contract currently vests all administrative authority in a single address (the "admin key"). This design creates a critical single point of failure: compromise or loss of this key would prevent legitimate governance operations and enable an attacker to manipulate attestations, disable the contract, or transfer control to a malicious actor. This threat model analyzes these risks and evaluates three mitigation strategies to inform the design of AnchorKit's future multi-signature governance system.

---

## Current Admin Privileges

The admin address controls the following critical operations:

1. **`initialize(admin)`** — Sets up the contract (one-time only)
2. **`set_admin(new_admin)`** — Transfers admin authority to another address
3. **`pause() / unpause()`** — Halts new attestations and revocations (reads still work)
4. **`add_attestor(attestor)` / `remove_attestor(attestor)`** — Manages the allowed attestor list
5. **`renew_attestor(attestor)`** — Extends TTL of attestor registrations
6. **`revoke(caller, subject, attestation_type)`** — Can revoke any attestation (attestors can only revoke their own)

These capabilities allow the admin to:
- Gate which entities can create attestations
- Halt all new credential issuance during emergencies or attacks
- Remove fraudulent or malicious attestations from the system
- Transfer governance to a successor (planned or unplanned)

---

## Risk Analysis

### Impact Classification

**Confidentiality:** Minimal. The contract does not store secrets; attestations are on-chain.

**Integrity:** Critical. A compromised admin key can falsify the state of all attestations, e.g., revoking valid credentials or re-activating revoked ones.

**Availability:** Critical. Loss of the admin key prevents:
- Adding new attestors
- Removing malicious attestors
- Pausing the contract in response to security incidents
- Transferring governance in planned transitions

### Loss Scenarios

**Admin Key Lost (Private Key Destroyed)**
- Cannot perform any admin operation indefinitely
- Contract locks in its current operational state
- If paused (e.g., during an incident), the pause becomes permanent
- Cannot onboard new attestors to replace compromised ones
- Only recourse: contract redeploy (expensive, breaks client integrations, requires fresh initialization)

**Admin Key Compromised (Private Key Stolen)**
- Attacker gains all admin privileges
- Can pause/unpause to disrupt service or hide their actions
- Can add themselves as an attestor or remove legitimate attestors
- Can revoke all attestations for a target subject or re-activate revoked ones
- Can transfer admin to a controlled address, locking out the real owner
- Detection is difficult if compromised silently; damage accumulates

**Admin Address Abandoned (Owner Lost Access)**
- Functionally identical to key loss but due to human error
- May occur if the owner died, lost hardware, forgot passphrases, or transferred the key during vendor migration

---

## Attack Scenarios

### Scenario 1: Targeted Credential Revocation (Integrity Attack)

**Threat Actor:** Competitor, state actor, or disgruntled service operator.

**Objective:** Harm a specific user or organization by revoking their legitimate attestations.

**Attack Flow:**
1. Attacker compromises the admin key through phishing, malware, or supply-chain attack.
2. Attacker calls `revoke(admin, subject, attestation_type)` for all attestations issued to the target.
3. Target's credentials are marked `Revoked` on-chain; all downstream systems see `is_valid()` return false.
4. Target's KYC, professional certifications, or identity proofs vanish, breaking their access to services.
5. Real admin discovers the compromise hours/days later from user complaints or monitoring alerts.

**Impact:**
- Sudden, unexpected loss of service access for affected users
- Reputational damage to affected organizations
- Legal liability if credentials are required for regulatory compliance
- High recovery cost (manual revocation reversal, audit trail reconstruction)

**Deniability:** Attack is fully attributable to the admin key; timeline and nature of changes are auditable from on-chain events.

---

### Scenario 2: Service Disruption via Permanent Pause (Availability Attack)

**Threat Actor:** Disgruntled ex-employee, activist, or insider threat.

**Objective:** Halt all new attestation issuance and revocation operations indefinitely.

**Attack Flow:**
1. Attacker obtains admin key credentials through employee compromise or device theft.
2. Attacker calls `pause()` on the contract.
3. All calls to `attest()`, `attest_batch()`, and `revoke()` immediately fail.
4. Legitimate users cannot issue new credentials or revoke compromised ones.
5. Real admin discovers the pause and calls `unpause()`.
6. **However:** If real admin has also lost the key (e.g., both parties lost access in a key rotation mishap), the pause is permanent.

**Impact:**
- Immediate halt to all credential issuance
- Users cannot respond to security incidents (cannot revoke)
- Service reputation damaged
- SLA violations if credential issuance is time-critical
- If pause cannot be lifted, equivalent to total service outage

**Deniability:** The pause event is on-chain; timestamp identifies the attack window.

---

### Scenario 3: Admin Transfer to Attacker (Sovereignty Loss)

**Threat Actor:** Advanced attacker (sophisticated malware, nation-state, supply-chain compromise).

**Objective:** Permanently transfer administrative control to attacker-controlled address.

**Attack Flow:**
1. Attacker compromises the admin key (e.g., through trojanized signing software).
2. Attacker immediately calls `set_admin(attacker_address)`.
3. Event `AdminChanged` is emitted with attacker's address as new admin.
4. Real admin's key is now useless; the contract is under attacker control.
5. Real admin notices the on-chain event via monitoring but cannot revoke the transfer.
6. Attacker then:
   - Pauses the contract to prevent legitimate operations
   - Adds themselves as an attestor
   - Revokes all attestations for competitors or political enemies
   - Issues false attestations to themselves or allies
   - Transfers admin again to a shell company or jurisdiction beyond reach

**Impact:**
- **Permanent loss of governance** without multi-signature or social recovery
- Attacker has unfettered control over all credential state
- Downstream systems relying on AnchorKit cannot trust attestations
- Users may pursue legal action; regulators may investigate
- AnchorKit's credibility is destroyed; migration to competing systems or restart required

**Deniability:** Attack is fully auditable but cannot be reversed after the fact. Real admin must prove compromise in hindsight.

---

## Mitigation Strategies

### Strategy 1: Timelock Mechanisms

**Concept:** Introduce a delay between when an admin initiates a critical action (e.g., `set_admin()` or `pause()`) and when it takes effect. During the delay, any account (not just the admin) can cancel the queued action.

**Implementation Sketch:**
```rust
pub struct PendingAction {
    action: AdminAction,  // enum: SetAdmin(Address), Pause, etc.
    initiator: Address,
    queued_at: u64,       // ledger timestamp
    execute_after: u64,   // delay in seconds
}

pub fn set_admin_pending(env: Env, new_admin: Address, delay: u64) -> Result<(), Error> {
    let admin = storage::get_admin(&env)?;
    admin.require_auth();
    
    let pending = PendingAction {
        action: AdminAction::SetAdmin(new_admin),
        initiator: admin,
        queued_at: env.ledger().timestamp(),
        execute_after: delay,
    };
    storage::set_pending_action(&env, pending);
}

pub fn execute_pending_action(env: Env) -> Result<(), Error> {
    let pending = storage::get_pending_action(&env)?;
    let now = env.ledger().timestamp();
    
    if now < pending.queued_at + pending.execute_after {
        return Err(Error::ActionStillLocked);
    }
    
    match pending.action {
        AdminAction::SetAdmin(new_admin) => {
            storage::set_admin(&env, &new_admin);
            events::admin_changed_executed(&env, &pending.initiator, &new_admin);
        },
        // ... other actions
    }
    storage::clear_pending_action(&env);
}

pub fn cancel_pending_action(env: Env) -> Result<(), Error> {
    // Any caller can cancel a queued action during the delay period
    storage::clear_pending_action(&env);
    events::action_cancelled(&env);
}
```

**Tradeoffs:**

**Advantages:**
- Simple to implement; no external dependencies
- Provides a window to detect and cancel malicious actions
- Preserves single-key simplicity while adding a safety valve
- Can apply to critical actions selectively (e.g., `set_admin` only)
- Non-signatories (e.g., auditors, guardians) can monitor and cancel proactively

**Disadvantages:**
- **Does not prevent loss of the key.** If the only admin key is destroyed, the delay merely postpones the problem; actions still cannot be initiated.
- **Does not prevent key compromise.** An attacker holding the key can initiate and wait out the delay; the cancel mechanism is ineffective if the attacker maintains key access.
- **Delay length tradeoff:** Short delays (e.g., 1 hour) are easy for attackers to outlast; long delays (e.g., 7 days) are operationally painful for legitimate governance (slow incident response, slow transitions).
- **No permission model for cancellers.** If anyone can cancel, a denial-of-service vector exists: attackers can flood the contract with pending actions and immediately cancel them, creating noise and making legitimate cancellations hard to detect.
- **Does not address root cause:** Single point of failure remains unresolved.

**Security Model:** Assumes an attacker either lacks key access (for early detection during the delay) or that the delay window is useful for manual intervention. Provides no protection against a determined attacker with sustained key access.

---

### Strategy 2: Multi-Signature Governance

**Concept:** Replace the single admin key with an M-of-N multi-signature quorum. Critical actions (e.g., `set_admin`, `pause`, `add_attestor`) require M out of N designated signers to authorize, each signing with their own private key.

**Implementation Sketch:**
```rust
pub struct MultiSigConfig {
    signers: Vec<Address>,  // N authorized signers
    threshold: u32,         // M (quorum size)
}

pub fn set_admin_multisig(
    env: Env,
    new_admin: Address,
    signatures: Vec<(Address, Bytes)>,  // signer address + signature
) -> Result<(), Error> {
    let config = storage::get_multisig_config(&env)?;
    
    // Validate that signatures come from distinct, authorized signers
    let mut signed_addresses = Vec::new();
    for (signer, sig) in signatures.iter() {
        if !config.signers.contains(signer) {
            return Err(Error::UnauthorizedSigner);
        }
        if signed_addresses.contains(signer) {
            return Err(Error::DuplicateSignature);
        }
        
        // Verify signature over the action (e.g., message = ("set_admin", new_admin))
        let message = soroban_sdk::crypto::keccak256(&("set_admin", new_admin.to_bytes()));
        env.crypto().secp256k1_verify(&signer, &message, &sig)?;
        
        signed_addresses.push(signer.clone());
    }
    
    // Check threshold met
    if signed_addresses.len() < config.threshold as usize {
        return Err(Error::InsufficientSignatures);
    }
    
    // Execute the action
    storage::set_admin(&env, &new_admin);
    events::admin_changed_multisig(&env, &signed_addresses, &new_admin);
}

pub fn add_signer(env: Env, new_signer: Address, signatures: Vec<(Address, Bytes)>) -> Result<(), Error> {
    // Same multisig verification as above
    let mut config = storage::get_multisig_config(&env)?;
    config.signers.push(new_signer);
    storage::set_multisig_config(&env, &config);
}
```

**Tradeoffs:**

**Advantages:**
- **Eliminates single point of failure.** Compromise of one signer does not grant full admin access.
- **Distributes trust.** Governance can be held by multiple independent organizations, reducing insider threat risk.
- **Flexible quorum.** Can tune M and N (e.g., 2-of-3, 3-of-5) to balance security vs. operational agility.
- **Prevents unilateral actions.** Malicious signer cannot act alone; collusion of M signers is required.
- **No delays.** Actions execute immediately upon signature collection, enabling fast incident response.
- **Enables signer rotation.** New signers can be added or compromised ones removed via multisig vote.

**Disadvantages:**
- **Coordination overhead.** Every action requires M signers to participate. Scheduling, coordination, and communication tools are needed.
- **M-of-N trust model is strict.** If M signers collude (or are all compromised), governance is subverted. Attack surface increases with N (more potential compromise targets).
- **Key management complexity.** Each signer must secure their private key independently. Increases operational burden (key backup, rotation, custody).
- **Offline signer risk.** If any of the M required signers is unreachable (lost key, downtime, death), actions are blocked until signer is replaced via another multisig vote (chicken-and-egg problem for bootstrapping).
- **On-chain signature verification cost.** Soroban contract invocation must validate M signatures, increasing gas cost per action.
- **Requires signer infrastructure.** Signers must run their own signing infrastructure (e.g., secure key storage, signing automation). Not all organizations have this capability.

**Security Model:** Assumes the M-of-N signers are independent, that no M-subset of signers can be simultaneously compromised, and that signer rotation can occur before an active M-subset is corrupted. Effectiveness depends on operational discipline and the independence/security of the signer pool.

---

### Strategy 3: Social Recovery

**Concept:** Designate a set of "guardians" (trusted individuals or organizations) who can collectively initiate admin key recovery if the key is lost or compromised. Recovery is a two-phase process:
1. A guardian or set of guardians initiates a recovery with a candidate new admin.
2. After a delay (to detect false recoveries), the recovery executes if not cancelled.

**Implementation Sketch:**
```rust
pub struct Guardian {
    address: Address,
    threshold: u32,  // minimum guardians required to initiate recovery
}

pub struct RecoveryRequest {
    new_admin: Address,
    guardians_signed: Vec<Address>,
    initiated_at: u64,
    delay: u64,
}

pub fn initiate_recovery(
    env: Env,
    new_admin: Address,
    guardian_signatures: Vec<(Address, Bytes)>,
) -> Result<(), Error> {
    let guardians = storage::get_guardians(&env)?;
    
    // Validate signatures from distinct guardians
    let mut signed_guardians = Vec::new();
    for (signer, sig) in guardian_signatures.iter() {
        if !guardians.addresses.contains(signer) {
            return Err(Error::UnauthorizedGuardian);
        }
        let message = soroban_sdk::crypto::keccak256(&("recover", new_admin.to_bytes()));
        env.crypto().secp256k1_verify(&signer, &message, &sig)?;
        signed_guardians.push(signer.clone());
    }
    
    // Check guardian threshold
    if signed_guardians.len() < guardians.threshold as usize {
        return Err(Error::InsufficientGuardians);
    }
    
    // Queue the recovery with a delay
    let request = RecoveryRequest {
        new_admin,
        guardians_signed: signed_guardians,
        initiated_at: env.ledger().timestamp(),
        delay: 7 * 24 * 3600,  // 7 days
    };
    storage::set_recovery_request(&env, request);
    events::recovery_initiated(&env, &new_admin);
}

pub fn execute_recovery(env: Env) -> Result<(), Error> {
    let request = storage::get_recovery_request(&env)?;
    let now = env.ledger().timestamp();
    
    if now < request.initiated_at + request.delay {
        return Err(Error::RecoveryStillLocked);
    }
    
    // Execute recovery
    storage::set_admin(&env, &request.new_admin);
    storage::clear_recovery_request(&env);
    events::recovery_executed(&env, &request.new_admin);
    Ok(())
}

pub fn cancel_recovery(env: Env) -> Result<(), Error> {
    // Current admin can cancel recovery (if key is still accessible)
    let admin = storage::get_admin(&env)?;
    admin.require_auth();
    storage::clear_recovery_request(&env);
    events::recovery_cancelled(&env);
}
```

**Tradeoffs:**

**Advantages:**
- **Addresses key loss gracefully.** Guardians can initiate recovery even if the admin key is destroyed, restoring governance.
- **Delay provides detection window.** Unlike pure multisig, social recovery has a delay phase, giving the real admin time to cancel a false recovery attempt.
- **Lower coordination overhead than multisig.** Guardians are called upon only in emergencies; day-to-day operations are unaffected by the admin key alone.
- **Trust model is asymmetric.** Guardians are "emergency contacts," not daily approvers, reducing their attack surface.
- **Can combine with multisig.** Guardians could themselves be a multisig committee (e.g., "3-of-5 guardians can initiate recovery").

**Disadvantages:**
- **Does not prevent loss of the admin key.** Day-to-day operations still depend on a single key; compromise is still catastrophic until recovery is triggered.
- **Guardian compromise is a latent threat.** If M guardians are compromised (but no one notices), they can initiate false recovery and wait out the delay. Real admin must actively monitor and cancel.
- **Delay-based detection is reactive.** Real admin must be actively monitoring the contract and able to act within the delay window. Sleeping, offline, or unavailable admins cannot cancel in time.
- **Guardian trust model.** Guardians must be extremely trusted; they effectively have emergency access to the contract. Vetting, ongoing monitoring, and removal of unfit guardians is critical.
- **Bootstrapping problem.** Selecting trustworthy, independent guardians is difficult. A small, tight-knit group of guardians is easier to compromise; a large group is hard to coordinate.
- **Guardian key management.** Guardians must also protect their keys; a compromised guardian is equivalent to a compromised admin key during the recovery window.

**Security Model:** Assumes guardians are independent, well-vetted, and not simultaneously compromisable. Also assumes the real admin can monitor and cancel false recoveries in time. Effectiveness depends on guardian selection, monitoring infrastructure, and the admin's availability during recovery windows.

---

## Comparative Analysis

| Criterion | Timelock | Multi-Signature | Social Recovery |
|-----------|----------|-----------------|-----------------|
| **Prevents key loss** | ❌ No (actions still blocked) | ❌ No (still single-key for day-to-day) | ✅ Yes (guardians can recover) |
| **Prevents key compromise** | ⚠️ Partial (delay + cancel) | ✅ Yes (needs M-of-N) | ⚠️ Partial (delay + cancel) |
| **Incident response speed** | ❌ Slow (depends on delay) | ✅ Fast (no inherent delay) | ❌ Slow (7-day delay typical) |
| **Operational simplicity** | ✅ High (minimal coordination) | ❌ Low (needs M signers per action) | ✅ High (emergency-only) |
| **Gas cost per action** | ✅ Low (no signatures) | ❌ High (M signature verifications) | ✅ Low (day-to-day) |
| **Requires trusted parties** | ❌ None (on-chain contract logic) | ✅ Yes (M signers) | ✅ Yes (guardians) |
| **Byzantine resilience** | ❌ None (canceller is optional) | ✅ Yes (M-of-N model) | ⚠️ Partial (delay detection) |
| **Single point of failure** | ⚠️ Yes (still one admin key) | ❌ No (distributed) | ⚠️ Yes (day-to-day) |

---

## Recommendations & Hybrid Approach

### Primary Recommendation: Multi-Signature Governance + Social Recovery (Hybrid)

Combining multi-signature governance with social recovery provides the strongest security posture:

1. **Day-to-Day Operations (Multi-Signature):**
   - Establish a 2-of-3 or 3-of-5 multisig committee for all critical admin actions.
   - Signers should be geographically distributed, organizationally independent, and held to high security standards.
   - This eliminates the single point of failure for normal operations.

2. **Emergency Recovery (Social Recovery Layer):**
   - Designate a separate set of 3-5 well-trusted guardians (not overlapping with signers).
   - Guardians can initiate recovery if the multisig committee is incapacitated (e.g., all signers lose keys simultaneously).
   - Recovery includes a 7-day delay to allow cancellation by recovered signers.

3. **Rationale:**
   - Multisig is fast and prevents day-to-day compromise; social recovery handles the "all signers compromised or lost" scenario.
   - Separating signers and guardians raises the bar for total takeover.
   - The delay in social recovery gives a window for the multisig committee to cancel false recoveries.

### Implementation Roadmap

**Phase 1: Multi-Signature Governance (Immediate)**
- Implement 3-of-5 multisig for `set_admin`, `pause`, `unpause`, and `add_attestor` / `remove_attestor`.
- Migrate existing admin key to one of the 5 signers.
- Preserve `renew_attestor` as an admin-only, non-multisig action (lower-risk, operational convenience).
- Test extensively with production-like replay scenarios.

**Phase 2: Social Recovery (Follow-up)**
- Design and implement guardian-based recovery as described above.
- Enlist guardians; establish their roles and responsibilities in a formal charter.
- Integrate recovery initiation and execution into monitoring and alerting.
- Conduct incident simulations (e.g., "what if 3 signers are hacked simultaneously?").

### Operational Considerations

1. **Signer Recruitment:** Identify 5 independent, well-resourced signers with strong security practices. Avoid vendor lock-in; signers should be from different organizations/jurisdictions.

2. **Key Management:** Require each signer to use hardware security modules (HSMs) or multi-party computation (MPC) wallets to reduce key compromise risk.

3. **Monitoring & Alerting:** Implement off-chain monitoring to detect and alert on any pending admin actions, failed signature collections, or recovery initiations.

4. **Testing:** Conduct quarterly drills:
   - Test signature collection and execution of a legitimate multisig action.
   - Simulate signer unavailability; test if remaining signers can still operate.
   - Simulate a false recovery initiation; verify the admin can cancel in time.

5. **Governance Documentation:** Create a charter documenting signers' roles, conflict-of-interest policies, compensation (if any), and removal procedures.

---

## Security Hardening Roadmap (Beyond Admin Key)

While this threat model focuses on the admin key, consider these related hardening efforts:

1. **Attestor Attestation:** Extend attestation to cover attestors themselves (e.g., attest to an attestor's identity and jurisdiction compliance).

2. **Timelock on High-Risk Operations:** Even with multisig, introduce a shorter timelock (e.g., 1 hour) for the most critical actions like `pause()`, allowing time for off-chain coordination and double-checks.

3. **Event Audit Logging:** Ensure all admin actions emit distinct, detailed events for off-chain monitoring and forensic analysis.

4. **Attestor Revocation Caps:** Limit the number of attestations that can be bulk-revoked in a single transaction to reduce the blast radius of a compromised admin key.

5. **Governance Incentive Alignment:** If AnchorKit becomes protocol-critical, consider incentivizing signers to secure their keys (e.g., slashing for security breaches, rewards for diligence).

---

## Conclusion

The single admin key is a critical vulnerability that must be addressed before AnchorKit is widely deployed. Key loss incurs operational friction (contract redeploy), and key compromise is catastrophic (complete governance takeover). A hybrid approach combining multi-signature governance for day-to-day operations with social recovery for emergency scenarios provides strong protection against both loss and compromise while maintaining reasonable operational efficiency.

Immediate next steps:
1. Recruit and vet 3-5 multisig signers.
2. Implement 3-of-5 multisig for critical actions.
3. Deploy and test in staging before production rollout.
4. Plan and implement social recovery as a follow-up phase.
