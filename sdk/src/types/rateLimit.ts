/**
 * Sliding-window rate limiting types.
 *
 * These back `RateLimiter`, which enforces independent per-attestor and
 * global request caps for anchor-facing SDK calls.
 */

/**
 * Configuration for a `RateLimiter` instance.
 */
export interface RateLimitConfig {
  /** Maximum requests a single attestor may make within `windowMs`. */
  perAttestorLimit: number;
  /** Maximum requests across all attestors combined within `windowMs`. */
  globalLimit: number;
  /** Size of the sliding window, in milliseconds. */
  windowMs: number;
  /**
   * How long an attestor's counter is kept after its last request before
   * `prune()` discards it. Beyond one full window past the last request a
   * counter's weighted contribution has already decayed to zero, so the
   * default of `windowMs * 2` is a safe, generous floor.
   */
  idleRetentionMs?: number;
  /**
   * Opportunistically run `prune()` after this many `checkAndConsume()`
   * calls, so long-running processes don't need to schedule pruning
   * themselves. Set to `0` to disable automatic pruning.
   */
  pruneIntervalCalls?: number;
}

/**
 * Result of a single `checkAndConsume()` call.
 */
export interface RateLimitStatus {
  /** Whether the request was allowed (and, if so, counted). */
  allowed: boolean;
  /** Estimated remaining requests for this attestor in the current window. */
  remaining: number;
  /** Estimated remaining requests globally in the current window. */
  remainingGlobal: number;
  /** Milliseconds until capacity is likely to free up; `0` when allowed. */
  retryAfterMs: number;
  /** Which cap was hit, present only when `allowed` is `false`. */
  limitedBy?: 'attestor' | 'global';
}

/**
 * Internal sliding-window counter state for one key (an attestor, or the
 * global counter). Tracks two adjacent fixed windows so the request rate
 * can be estimated by weighting the previous window's count by how much it
 * still overlaps the sliding window — this is what avoids the 2x burst a
 * naive fixed-window counter allows right at a window boundary.
 */
export interface RateLimitState {
  /** Start timestamp (ms, aligned to `windowMs`) of the current window. */
  currentWindowStart: number;
  /** Requests counted in the current window. */
  currentCount: number;
  /** Requests counted in the immediately preceding window. */
  previousCount: number;
  /** Timestamp (ms) of the last request seen for this key. */
  lastSeenAt: number;
}
