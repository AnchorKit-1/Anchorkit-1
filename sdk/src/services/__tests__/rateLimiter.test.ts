/**
 * Tests for RateLimiter
 *
 * Acceptance criteria covered:
 *   ✓ AC1 – Sliding window correctly prevents edge-of-window bursting
 *   ✓ AC2 – Per-attestor and global limits are independently configurable
 *   ✓ AC3 – State is efficiently prunable so long-running processes don't leak memory
 *
 * All timing is driven by an explicit `now` argument rather than real or
 * fake timers, so the tests are deterministic and exercise exact window
 * boundaries.
 */

import { describe, it, expect } from 'vitest';
import { RateLimiter, RateLimiterConfigError } from '../rateLimiter';

const WINDOW_MS = 1000;

function makeLimiter(overrides: Partial<ConstructorParameters<typeof RateLimiter>[0]> = {}) {
  return new RateLimiter({
    perAttestorLimit: 5,
    globalLimit: 100,
    windowMs: WINDOW_MS,
    ...overrides,
  });
}

describe('RateLimiter construction', () => {
  it('rejects a non-positive perAttestorLimit', () => {
    expect(() => makeLimiter({ perAttestorLimit: 0 })).toThrow(RateLimiterConfigError);
  });

  it('rejects a non-positive globalLimit', () => {
    expect(() => makeLimiter({ globalLimit: -1 })).toThrow(RateLimiterConfigError);
  });

  it('rejects a non-positive windowMs', () => {
    expect(() => makeLimiter({ windowMs: 0 })).toThrow(RateLimiterConfigError);
  });

  it('rejects a perAttestorLimit greater than globalLimit', () => {
    expect(() => makeLimiter({ perAttestorLimit: 10, globalLimit: 5 })).toThrow(
      RateLimiterConfigError
    );
  });
});

describe('basic allow/deny within a single window', () => {
  it('allows requests up to the per-attestor limit and denies the next one', () => {
    const limiter = makeLimiter({ perAttestorLimit: 3, globalLimit: 100 });
    const now = 10_000; // mid-window, well away from a boundary

    for (let i = 0; i < 3; i++) {
      const status = limiter.checkAndConsume('alice', now);
      expect(status.allowed).toBe(true);
    }

    const denied = limiter.checkAndConsume('alice', now);
    expect(denied.allowed).toBe(false);
    expect(denied.limitedBy).toBe('attestor');
    expect(denied.remaining).toBe(0);
  });

  it('reports decreasing remaining count as requests are consumed', () => {
    const limiter = makeLimiter({ perAttestorLimit: 5, globalLimit: 100 });
    const now = 10_000;

    const first = limiter.checkAndConsume('alice', now);
    expect(first.remaining).toBe(4);
    const second = limiter.checkAndConsume('alice', now);
    expect(second.remaining).toBe(3);
  });

  it('does not consume quota on a denied request', () => {
    const limiter = makeLimiter({ perAttestorLimit: 1, globalLimit: 100 });
    const now = 10_000;

    expect(limiter.checkAndConsume('alice', now).allowed).toBe(true);
    // Fire several denied requests in the same instant.
    for (let i = 0; i < 5; i++) {
      expect(limiter.checkAndConsume('alice', now).allowed).toBe(false);
    }

    // A true sliding window keeps a request live for a full `windowMs`
    // after it happened, so capacity only fully frees up two fixed
    // windows after the original request's window began. If the denied
    // attempts above had been (wrongly) counted, capacity would never
    // free up at all.
    const fullyDecayed = now + 2 * WINDOW_MS;
    const status = limiter.checkAndConsume('alice', fullyDecayed);
    expect(status.allowed).toBe(true);
  });
});

describe('sliding window prevents edge-of-window bursting', () => {
  it('does not allow 2x the limit across a fixed-window boundary', () => {
    const limit = 10;
    const limiter = makeLimiter({ perAttestorLimit: limit, globalLimit: 1000 });

    // Window 0 covers [0, 1000). Fill the quota right at the end of it.
    const lastMomentOfWindow0 = 999;
    for (let i = 0; i < limit; i++) {
      expect(limiter.checkAndConsume('alice', lastMomentOfWindow0).allowed).toBe(true);
    }
    expect(limiter.checkAndConsume('alice', lastMomentOfWindow0).allowed).toBe(false);

    // A naive fixed-window counter would reset to 0 the instant window 1
    // begins and allow another `limit` requests immediately. The sliding
    // window must not: at now=1000, window 0 still overlaps ~100% of the
    // trailing 1000ms window, so it should still be fully denied.
    const firstMomentOfWindow1 = 1000;
    const attemptsAllowedRightAfterBoundary = Array.from({ length: limit }, () =>
      limiter.checkAndConsume('alice', firstMomentOfWindow1).allowed
    ).filter(Boolean).length;

    expect(attemptsAllowedRightAfterBoundary).toBe(0);
  });

  it('gradually admits requests as the previous window decays out of the trailing view', () => {
    const limit = 10;
    const limiter = makeLimiter({ perAttestorLimit: limit, globalLimit: 1000 });

    const lastMomentOfWindow0 = 999;
    for (let i = 0; i < limit; i++) {
      limiter.checkAndConsume('alice', lastMomentOfWindow0);
    }

    // Half way through window 1: previous window's weight has decayed to
    // ~50%, so roughly half the original quota should have freed up.
    const halfwayThroughWindow1 = 1500;
    const status = limiter.checkAndConsume('alice', halfwayThroughWindow1);
    expect(status.allowed).toBe(true);

    // Fully past two windows: previous window no longer overlaps at all,
    // full fresh quota is available.
    const wellIntoWindow2 = 2500;
    const freshLimiter = makeLimiter({ perAttestorLimit: limit, globalLimit: 1000 });
    for (let i = 0; i < limit; i++) {
      freshLimiter.checkAndConsume('alice', lastMomentOfWindow0);
    }
    let allowedCount = 0;
    for (let i = 0; i < limit; i++) {
      if (freshLimiter.checkAndConsume('alice', wellIntoWindow2).allowed) allowedCount += 1;
    }
    expect(allowedCount).toBe(limit);
  });
});

describe('per-attestor and global limits are independently enforced', () => {
  it('denies one attestor exceeding its own cap even though global has headroom', () => {
    const limiter = makeLimiter({ perAttestorLimit: 2, globalLimit: 1000 });
    const now = 10_000;

    expect(limiter.checkAndConsume('alice', now).allowed).toBe(true);
    expect(limiter.checkAndConsume('alice', now).allowed).toBe(true);
    const denied = limiter.checkAndConsume('alice', now);
    expect(denied.allowed).toBe(false);
    expect(denied.limitedBy).toBe('attestor');

    // A different attestor is unaffected.
    expect(limiter.checkAndConsume('bob', now).allowed).toBe(true);
  });

  it('denies further requests once the global cap is hit, even under per-attestor limits', () => {
    const limiter = makeLimiter({ perAttestorLimit: 3, globalLimit: 3 });
    const now = 10_000;

    expect(limiter.checkAndConsume('alice', now).allowed).toBe(true);
    expect(limiter.checkAndConsume('bob', now).allowed).toBe(true);
    expect(limiter.checkAndConsume('carol', now).allowed).toBe(true);

    const denied = limiter.checkAndConsume('dave', now);
    expect(denied.allowed).toBe(false);
    expect(denied.limitedBy).toBe('global');
    expect(denied.remainingGlobal).toBe(0);
  });
});

describe('state pruning', () => {
  it('removes idle attestor entries once idleRetentionMs has passed', () => {
    const limiter = makeLimiter({
      perAttestorLimit: 5,
      globalLimit: 100,
      windowMs: WINDOW_MS,
      idleRetentionMs: 2000,
      pruneIntervalCalls: 0, // disable auto-prune; test manual prune()
    });

    limiter.checkAndConsume('alice', 0);
    limiter.checkAndConsume('bob', 500);
    expect(limiter.trackedAttestorCount).toBe(2);

    // Not yet past idleRetentionMs for either.
    limiter.prune(1000);
    expect(limiter.trackedAttestorCount).toBe(2);

    // alice's last activity (t=0) is now >= 2000ms stale; bob's (t=500) is not.
    limiter.prune(2000);
    expect(limiter.trackedAttestorCount).toBe(1);

    limiter.prune(2500);
    expect(limiter.trackedAttestorCount).toBe(0);
  });

  it('prunes automatically every pruneIntervalCalls calls', () => {
    const limiter = makeLimiter({
      perAttestorLimit: 5,
      globalLimit: 1000,
      windowMs: WINDOW_MS,
      idleRetentionMs: 100,
      pruneIntervalCalls: 3,
    });

    limiter.checkAndConsume('alice', 0);
    expect(limiter.trackedAttestorCount).toBe(1);

    // Three more calls (all far enough past alice's idle retention) should
    // trigger an internal prune sweep without the caller invoking prune().
    limiter.checkAndConsume('bob', 10_000);
    limiter.checkAndConsume('bob', 10_000);
    limiter.checkAndConsume('bob', 10_000);

    // alice should have been swept away by the automatic prune, leaving
    // only bob.
    expect(limiter.trackedAttestorCount).toBe(1);
  });

  it('does not prune an attestor that keeps making requests', () => {
    const limiter = makeLimiter({
      perAttestorLimit: 1000,
      globalLimit: 100_000,
      windowMs: WINDOW_MS,
      idleRetentionMs: 500,
      pruneIntervalCalls: 0,
    });

    for (let t = 0; t <= 5000; t += 100) {
      limiter.checkAndConsume('alice', t);
      limiter.prune(t);
    }

    expect(limiter.trackedAttestorCount).toBe(1);
  });

  it('reset() clears all tracked state', () => {
    const limiter = makeLimiter();
    limiter.checkAndConsume('alice', 0);
    limiter.checkAndConsume('bob', 0);
    expect(limiter.trackedAttestorCount).toBe(2);

    limiter.reset(0);
    expect(limiter.trackedAttestorCount).toBe(0);
  });
});

describe('retryAfterMs', () => {
  it('is zero when allowed and positive when denied', () => {
    const limiter = makeLimiter({ perAttestorLimit: 1, globalLimit: 100 });
    const now = 1500;

    const allowed = limiter.checkAndConsume('alice', now);
    expect(allowed.retryAfterMs).toBe(0);

    const denied = limiter.checkAndConsume('alice', now);
    expect(denied.allowed).toBe(false);
    expect(denied.retryAfterMs).toBeGreaterThan(0);
    expect(denied.retryAfterMs).toBeLessThanOrEqual(WINDOW_MS);
  });
});
