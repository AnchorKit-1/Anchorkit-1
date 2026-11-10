import type { RetryOptions } from '../types/retry';

/**
 * Error thrown when an invalid RetryConfig option is supplied.
 */
export class RetryConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RetryConfigError';
  }
}

/**
 * Configuration for off-chain anchor HTTP request retries.
 *
 * Ships with sensible defaults:
 * - `maxAttempts`: 3 (1 initial call + 2 retries)
 * - `baseDelayMs`: 500 ms
 * - `maxDelayMs`: 10 000 ms (10 seconds)
 * - `backoffMultiplier`: 2.0
 * - `jitter`: 0.1 (±10%)
 *
 * Provides an immutable `.clone()` method allowing multiple client instances
 * to independently share, branch, or override base configurations.
 */
export class RetryConfig {
  /** Default maximum attempts (1 initial call + 2 retries = 3 attempts total). */
  public static readonly DEFAULT_MAX_ATTEMPTS = 3;

  /** Default initial base delay: 500 milliseconds. */
  public static readonly DEFAULT_BASE_DELAY_MS = 500;

  /** Default maximum delay ceiling: 10,000 milliseconds (10 seconds). */
  public static readonly DEFAULT_MAX_DELAY_MS = 10000;

  /** Default exponential backoff multiplier: 2.0. */
  public static readonly DEFAULT_BACKOFF_MULTIPLIER = 2.0;

  /** Default jitter ratio: 0.1 (±10%). */
  public static readonly DEFAULT_JITTER = 0.1;

  public readonly maxAttempts: number;
  public readonly baseDelayMs: number;
  public readonly maxDelayMs: number;
  public readonly backoffMultiplier: number;
  public readonly jitter: number;

  constructor(options: RetryOptions = {}) {
    const maxAttempts = options.maxAttempts ?? RetryConfig.DEFAULT_MAX_ATTEMPTS;
    const baseDelayMs = options.baseDelayMs ?? RetryConfig.DEFAULT_BASE_DELAY_MS;
    const maxDelayMs = options.maxDelayMs ?? RetryConfig.DEFAULT_MAX_DELAY_MS;
    const backoffMultiplier = options.backoffMultiplier ?? RetryConfig.DEFAULT_BACKOFF_MULTIPLIER;
    const jitter = options.jitter ?? RetryConfig.DEFAULT_JITTER;

    if (!Number.isInteger(maxAttempts) || maxAttempts < 1) {
      throw new RetryConfigError('maxAttempts must be an integer >= 1');
    }
    if (typeof baseDelayMs !== 'number' || isNaN(baseDelayMs) || baseDelayMs < 0) {
      throw new RetryConfigError('baseDelayMs must be a number >= 0');
    }
    if (typeof maxDelayMs !== 'number' || isNaN(maxDelayMs) || maxDelayMs < baseDelayMs) {
      throw new RetryConfigError('maxDelayMs must be a number >= baseDelayMs');
    }
    if (typeof backoffMultiplier !== 'number' || isNaN(backoffMultiplier) || backoffMultiplier < 1) {
      throw new RetryConfigError('backoffMultiplier must be a number >= 1');
    }
    if (typeof jitter !== 'number' || isNaN(jitter) || jitter < 0 || jitter > 1) {
      throw new RetryConfigError('jitter must be a number between 0 and 1');
    }

    this.maxAttempts = maxAttempts;
    this.baseDelayMs = baseDelayMs;
    this.maxDelayMs = maxDelayMs;
    this.backoffMultiplier = backoffMultiplier;
    this.jitter = jitter;
  }

  /**
   * Creates a default RetryConfig instance with standard sensible defaults.
   */
  public static default(): RetryConfig {
    return new RetryConfig();
  }

  /**
   * Produces an independent clone of this RetryConfig, optionally overriding
   * specific properties. Usable across multiple client instances so each can share
   * or specialize configuration without mutating the base instance.
   */
  public clone(overrides: Partial<RetryOptions> = {}): RetryConfig {
    return new RetryConfig({
      maxAttempts: overrides.maxAttempts ?? this.maxAttempts,
      baseDelayMs: overrides.baseDelayMs ?? this.baseDelayMs,
      maxDelayMs: overrides.maxDelayMs ?? this.maxDelayMs,
      backoffMultiplier: overrides.backoffMultiplier ?? this.backoffMultiplier,
      jitter: overrides.jitter ?? this.jitter,
    });
  }

  /**
   * Calculates the delay for a given attempt index (0-indexed completed retries).
   * Applies exponential backoff clamped to `maxDelayMs`.
   */
  public computeDelayMs(attemptIndex: number, applyJitter = false): number {
    if (attemptIndex <= 0) {
      return this.baseDelayMs;
    }
    const rawDelay = this.baseDelayMs * Math.pow(this.backoffMultiplier, attemptIndex);
    const clampedDelay = Math.min(rawDelay, this.maxDelayMs);

    if (!applyJitter || this.jitter === 0) {
      return clampedDelay;
    }

    const min = clampedDelay * (1 - this.jitter);
    const max = clampedDelay * (1 + this.jitter);
    return Math.min(Math.round(min + Math.random() * (max - min)), this.maxDelayMs);
  }

  /**
   * Determines whether another attempt should be made given the number of completed attempts.
   */
  public shouldRetry(completedAttempts: number): boolean {
    return completedAttempts < this.maxAttempts;
  }
}
