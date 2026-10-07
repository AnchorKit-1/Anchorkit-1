/**
 * Retry configuration and option types for off-chain anchor HTTP requests.
 *
 * This configuration type serves as the shared foundation for backoff,
 * rate limiting, circuit breaking, and transport wrappers across the SDK.
 */

/**
 * Options used to initialize or override a `RetryConfig` instance.
 */
export interface RetryOptions {
  /**
   * Maximum number of request attempts (including the initial call).
   * Must be an integer >= 1.
   * Defaults to 3.
   */
  maxAttempts?: number;

  /**
   * Base delay before the first retry attempt, in milliseconds.
   * Must be a non-negative number >= 0.
   * Defaults to 500 ms.
   */
  baseDelayMs?: number;

  /**
   * Maximum delay ceiling between attempts, in milliseconds.
   * Prevents exponential backoff from expanding indefinitely.
   * Must be >= `baseDelayMs`.
   * Defaults to 10 000 ms (10 seconds).
   */
  maxDelayMs?: number;

  /**
   * Backoff multiplier applied per retry attempt.
   * Must be >= 1.0.
   * Defaults to 2.0.
   */
  backoffMultiplier?: number;

  /**
   * Random jitter ratio in range [0, 1] to prevent synchronized retry storms.
   * Defaults to 0.1 (±10% jitter).
   */
  jitter?: number;
}
