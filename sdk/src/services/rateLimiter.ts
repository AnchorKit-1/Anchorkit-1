/**
 * RateLimiter
 *
 * Sliding-window rate limiter for attestor-facing SDK calls, enforcing
 * independent per-attestor and global request caps.
 *
 * Why sliding window, not fixed window
 * ─────────────────────────────────────
 * A naive fixed-window counter (e.g. "reset the count every 1000ms") lets a
 * caller burst up to 2x its intended limit right at the window boundary:
 * fill the quota in the last millisecond of one window, then immediately
 * fill it again in the first millisecond of the next. This implementation
 * uses the standard sliding-window-counter approximation instead: it keeps
 * two adjacent fixed windows per key (the current one and the one before
 * it) and estimates the request rate as a weighted blend of both, where the
 * weight on the previous window shrinks linearly as the current window
 * progresses. That keeps memory per key at O(1) — no per-request log to
 * store or scan — while still smoothing out edge-of-window bursts.
 *
 * Memory
 * ──────
 * Per-attestor state lives in a `Map` keyed by attestor id. Since attestors
 * come and go over the lifetime of a long-running process, `prune()`
 * removes entries that haven't been touched in `idleRetentionMs` (default:
 * two full windows, by which point a counter's contribution has already
 * decayed to zero). `checkAndConsume()` opportunistically calls `prune()`
 * every `pruneIntervalCalls` calls so callers don't need to wire up their
 * own timer, but `prune()` is also public for callers who want to drive it
 * on their own schedule (e.g. from a periodic housekeeping task).
 */

import type { RateLimitConfig, RateLimitState, RateLimitStatus } from '../types/rateLimit';

const DEFAULT_IDLE_RETENTION_MULTIPLIER = 2;
const DEFAULT_PRUNE_INTERVAL_CALLS = 500;

function alignToWindow(now: number, windowMs: number): number {
  return Math.floor(now / windowMs) * windowMs;
}

function createState(now: number, windowMs: number): RateLimitState {
  return {
    currentWindowStart: alignToWindow(now, windowMs),
    currentCount: 0,
    previousCount: 0,
    lastSeenAt: now,
  };
}

/** Advance `state` to the window containing `now`, rolling counts forward. */
function rollWindow(state: RateLimitState, now: number, windowMs: number): void {
  const windowStart = alignToWindow(now, windowMs);
  const elapsedWindows = Math.round((windowStart - state.currentWindowStart) / windowMs);

  if (elapsedWindows === 1) {
    // Exactly one window has elapsed: what was "current" becomes "previous".
    state.previousCount = state.currentCount;
    state.currentCount = 0;
    state.currentWindowStart = windowStart;
  } else if (elapsedWindows > 1) {
    // Idle for more than a full window: the old window no longer overlaps
    // the sliding window at all.
    state.previousCount = 0;
    state.currentCount = 0;
    state.currentWindowStart = windowStart;
  }
  // elapsedWindows <= 0: still in the same window (or a clock moved
  // backwards), nothing to roll.

  state.lastSeenAt = now;
}

/** Estimate the request count within the trailing `windowMs` sliding window. */
function estimateCount(state: RateLimitState, now: number, windowMs: number): number {
  const elapsedInCurrent = now - state.currentWindowStart;
  const overlap = Math.min(1, Math.max(0, 1 - elapsedInCurrent / windowMs));
  return state.previousCount * overlap + state.currentCount;
}

export class RateLimiter {
  private readonly perAttestorLimit: number;
  private readonly globalLimit: number;
  private readonly windowMs: number;
  private readonly idleRetentionMs: number;
  private readonly pruneIntervalCalls: number;

  private readonly perAttestor = new Map<string, RateLimitState>();
  private global: RateLimitState;
  private callsSincePrune = 0;

  constructor(config: RateLimitConfig) {
    if (!Number.isFinite(config.perAttestorLimit) || config.perAttestorLimit <= 0) {
      throw new RateLimiterConfigError('perAttestorLimit must be a positive number');
    }
    if (!Number.isFinite(config.globalLimit) || config.globalLimit <= 0) {
      throw new RateLimiterConfigError('globalLimit must be a positive number');
    }
    if (!Number.isFinite(config.windowMs) || config.windowMs <= 0) {
      throw new RateLimiterConfigError('windowMs must be a positive number');
    }
    if (config.perAttestorLimit > config.globalLimit) {
      throw new RateLimiterConfigError('perAttestorLimit cannot exceed globalLimit');
    }

    this.perAttestorLimit = config.perAttestorLimit;
    this.globalLimit = config.globalLimit;
    this.windowMs = config.windowMs;
    this.idleRetentionMs =
      config.idleRetentionMs ?? this.windowMs * DEFAULT_IDLE_RETENTION_MULTIPLIER;
    this.pruneIntervalCalls = config.pruneIntervalCalls ?? DEFAULT_PRUNE_INTERVAL_CALLS;

    this.global = createState(Date.now(), this.windowMs);
  }

  /**
   * Check whether `attestorId` may make a request right now, and if so,
   * count it against both the per-attestor and global windows.
   *
   * A denied request is never counted — neither the per-attestor nor the
   * global counter advances for it — so a rejected burst doesn't itself
   * eat into the caller's future quota.
   *
   * @param attestorId - Identifier for the attestor making the request.
   * @param now - Timestamp in ms; defaults to `Date.now()`, overridable for tests.
   */
  checkAndConsume(attestorId: string, now: number = Date.now()): RateLimitStatus {
    if (this.pruneIntervalCalls > 0) {
      this.callsSincePrune += 1;
      if (this.callsSincePrune >= this.pruneIntervalCalls) {
        this.prune(now);
        this.callsSincePrune = 0;
      }
    }

    let attestorState = this.perAttestor.get(attestorId);
    if (!attestorState) {
      attestorState = createState(now, this.windowMs);
      this.perAttestor.set(attestorId, attestorState);
    }

    rollWindow(attestorState, now, this.windowMs);
    rollWindow(this.global, now, this.windowMs);

    const attestorEstimate = estimateCount(attestorState, now, this.windowMs);
    const globalEstimate = estimateCount(this.global, now, this.windowMs);

    const attestorOk = attestorEstimate + 1 <= this.perAttestorLimit;
    const globalOk = globalEstimate + 1 <= this.globalLimit;
    const allowed = attestorOk && globalOk;

    if (allowed) {
      attestorState.currentCount += 1;
      this.global.currentCount += 1;
    }

    const consumedAttestorEstimate = allowed ? attestorEstimate + 1 : attestorEstimate;
    const consumedGlobalEstimate = allowed ? globalEstimate + 1 : globalEstimate;

    const remaining = Math.max(0, Math.floor(this.perAttestorLimit - consumedAttestorEstimate));
    const remainingGlobal = Math.max(0, Math.floor(this.globalLimit - consumedGlobalEstimate));

    let limitedBy: RateLimitStatus['limitedBy'];
    let retryAfterMs = 0;
    if (!allowed) {
      limitedBy = !attestorOk ? 'attestor' : 'global';
      const elapsedInCurrent = now - alignToWindow(now, this.windowMs);
      retryAfterMs = Math.max(0, this.windowMs - elapsedInCurrent);
    }

    return { allowed, remaining, remainingGlobal, retryAfterMs, limitedBy };
  }

  /**
   * Discard tracked attestor state that hasn't been touched in
   * `idleRetentionMs`, so a long-running process doesn't accumulate one
   * entry per attestor ever seen. Returns the number of entries removed.
   */
  prune(now: number = Date.now()): number {
    let removed = 0;
    for (const [id, state] of this.perAttestor) {
      if (now - state.lastSeenAt >= this.idleRetentionMs) {
        this.perAttestor.delete(id);
        removed += 1;
      }
    }
    return removed;
  }

  /** Number of attestors currently tracked (i.e. not yet pruned). */
  get trackedAttestorCount(): number {
    return this.perAttestor.size;
  }

  /** Reset all state. Mainly useful in tests. */
  reset(now: number = Date.now()): void {
    this.perAttestor.clear();
    this.global = createState(now, this.windowMs);
    this.callsSincePrune = 0;
  }
}

export class RateLimiterConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RateLimiterConfigError';
  }
}
