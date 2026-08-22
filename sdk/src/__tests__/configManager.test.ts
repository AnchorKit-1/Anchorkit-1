/**
 * Tests for the hot-reload ConfigManager.
 *
 * Acceptance criteria covered:
 *   ✓ AC1 – Invalid updates are rejected without disrupting the currently-loaded config.
 *   ✓ AC2 – In-flight requests using the old config complete against the old config,
 *            not a half-swapped state.
 *   ✓ AC3 – Valid reload swaps in the new config and notifies listeners.
 *
 * All tests are deterministic and require no real network I/O.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  ConfigManager,
  ConfigValidationError,
  validateAnchorConfig,
  composeValidators,
} from '../config/index';
import type { AnchorConfig, ConfigValidator } from '../config/index';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const VALID_CONFIG: AnchorConfig = {
  anchorUrl: 'https://anchor.example.com',
  publicKey: 'GSTELLARKEY1',
  timeout: 30_000,
};

const VALID_CONFIG_2: AnchorConfig = {
  anchorUrl: 'https://other.anchor.example.com',
  publicKey: 'GSTELLARKEY2',
  timeout: 15_000,
};

// ---------------------------------------------------------------------------
// ConfigManager – construction
// ---------------------------------------------------------------------------

describe('ConfigManager – construction', () => {
  it('accepts a valid initial config', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    expect(mgr.current().anchorUrl).toBe('https://anchor.example.com');
  });

  it('throws ConfigValidationError when the initial config is invalid', () => {
    expect(
      () =>
        new ConfigManager(
          { anchorUrl: '' } as AnchorConfig,
          validateAnchorConfig
        )
    ).toThrow(ConfigValidationError);
  });

  it('includes the field name in the validation error for missing anchorUrl', () => {
    try {
      new ConfigManager({ anchorUrl: '' } as AnchorConfig, validateAnchorConfig);
      expect.fail('should have thrown');
    } catch (err) {
      expect(err).toBeInstanceOf(ConfigValidationError);
      expect((err as ConfigValidationError).field).toBe('anchorUrl');
    }
  });
});

// ---------------------------------------------------------------------------
// AC3 – Valid reload
// ---------------------------------------------------------------------------

describe('ConfigManager – valid reload (AC3)', () => {
  it('swaps the config when the new config is valid', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    mgr.reload(VALID_CONFIG_2);
    expect(mgr.current().anchorUrl).toBe('https://other.anchor.example.com');
  });

  it('invokes onChange listeners synchronously after a successful reload', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    const calls: Array<{ next: AnchorConfig; previous: AnchorConfig }> = [];
    mgr.onChange((next, previous) => calls.push({ next, previous }));

    mgr.reload(VALID_CONFIG_2);

    expect(calls).toHaveLength(1);
    expect(calls[0].next.anchorUrl).toBe('https://other.anchor.example.com');
    expect(calls[0].previous.anchorUrl).toBe('https://anchor.example.com');
  });

  it('unsubscribe() stops further notifications', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    const calls: number[] = [];
    const unsub = mgr.onChange(() => calls.push(1));

    mgr.reload(VALID_CONFIG_2);
    expect(calls).toHaveLength(1);

    unsub();
    mgr.reload(VALID_CONFIG);
    expect(calls).toHaveLength(1); // no new call
  });

  it('multiple listeners all receive the change', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    const a: string[] = [];
    const b: string[] = [];
    mgr.onChange((next) => a.push(next.anchorUrl));
    mgr.onChange((next) => b.push(next.anchorUrl));

    mgr.reload(VALID_CONFIG_2);

    expect(a).toEqual(['https://other.anchor.example.com']);
    expect(b).toEqual(['https://other.anchor.example.com']);
  });

  it('a listener that throws does not prevent other listeners from running', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    const safe: string[] = [];
    mgr.onChange(() => { throw new Error('boom'); });
    mgr.onChange((next) => safe.push(next.anchorUrl));

    expect(() => mgr.reload(VALID_CONFIG_2)).not.toThrow();
    expect(safe).toEqual(['https://other.anchor.example.com']);
  });

  it('the returned snapshot is frozen (immutable)', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    const snapshot = mgr.current();
    expect(() => {
      (snapshot as Record<string, unknown>).anchorUrl = 'https://mutated.example.com';
    }).toThrow();
    expect(mgr.current().anchorUrl).toBe('https://anchor.example.com');
  });
});

// ---------------------------------------------------------------------------
// AC1 – Rejected invalid reload
// ---------------------------------------------------------------------------

describe('ConfigManager – rejected invalid reload (AC1)', () => {
  it('throws ConfigValidationError when anchorUrl is missing', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    expect(() => mgr.reload({ anchorUrl: '' })).toThrow(ConfigValidationError);
  });

  it('leaves the current config unchanged after a rejected reload', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    expect(() => mgr.reload({ anchorUrl: 'not-a-url' })).toThrow(ConfigValidationError);

    // Config must be the original, untouched.
    expect(mgr.current().anchorUrl).toBe('https://anchor.example.com');
    expect(mgr.current().publicKey).toBe('GSTELLARKEY1');
  });

  it('does NOT notify listeners on a rejected reload', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    const calls: number[] = [];
    mgr.onChange(() => calls.push(1));

    expect(() => mgr.reload({ anchorUrl: '' })).toThrow(ConfigValidationError);
    expect(calls).toHaveLength(0);
  });

  it('rejects a config with an invalid URL scheme (ftp://)', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    expect(() =>
      mgr.reload({ anchorUrl: 'ftp://anchor.example.com' })
    ).toThrow(ConfigValidationError);
    expect(mgr.current().anchorUrl).toBe('https://anchor.example.com');
  });

  it('rejects a negative timeout value', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    expect(() =>
      mgr.reload({ ...VALID_CONFIG, timeout: -1 })
    ).toThrow(ConfigValidationError);
    expect(mgr.current().timeout).toBe(30_000);
  });

  it('rejects a zero maxWatched value', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    expect(() =>
      mgr.reload({ ...VALID_CONFIG, maxWatched: 0 })
    ).toThrow(ConfigValidationError);
  });

  it('error includes the failing field name', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    try {
      mgr.reload({ ...VALID_CONFIG, pollIntervalMs: -100 });
      expect.fail('should have thrown');
    } catch (err) {
      expect(err).toBeInstanceOf(ConfigValidationError);
      expect((err as ConfigValidationError).field).toBe('pollIntervalMs');
      expect((err as ConfigValidationError).code).toBe('INVALID_FIELD_VALUE');
    }
  });

  it('multiple sequential invalid reloads never corrupt the current config', () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    for (let i = 0; i < 5; i++) {
      expect(() => mgr.reload({ anchorUrl: '' })).toThrow(ConfigValidationError);
    }

    expect(mgr.current().anchorUrl).toBe('https://anchor.example.com');
  });
});

// ---------------------------------------------------------------------------
// AC2 – In-flight safety
// ---------------------------------------------------------------------------

describe('ConfigManager – in-flight request safety (AC2)', () => {
  it('an in-flight operation that captured a snapshot sees the OLD config after a reload', async () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);

    // Simulate an async operation that grabs the config snapshot *before*
    // the reload happens, then yields, then reads from its snapshot.
    let snapshotUrl: string | undefined;

    const inFlight = (async () => {
      // 1. Grab snapshot synchronously at operation start.
      const cfg = mgr.current();

      // 2. Yield — this is the "in-flight" window where a reload could occur.
      await Promise.resolve();

      // 3. Use the snapshot — must still reflect the original config.
      snapshotUrl = cfg.anchorUrl;
    })();

    // Reload happens while the operation is awaited.
    mgr.reload(VALID_CONFIG_2);

    // Let the in-flight op finish.
    await inFlight;

    // The in-flight op saw the ORIGINAL config, not the new one.
    expect(snapshotUrl).toBe('https://anchor.example.com');
    // But the manager now serves the NEW config.
    expect(mgr.current().anchorUrl).toBe('https://other.anchor.example.com');
  });

  it('a second operation started after reload sees the NEW config', async () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    mgr.reload(VALID_CONFIG_2);

    // Simulate operation started after reload.
    const cfg = mgr.current();
    await Promise.resolve();
    expect(cfg.anchorUrl).toBe('https://other.anchor.example.com');
  });

  it('multiple concurrent in-flight operations each see their own snapshot', async () => {
    const mgr = new ConfigManager(VALID_CONFIG, validateAnchorConfig);
    const urlsSeenByOps: string[] = [];

    const makeOp = async () => {
      const cfg = mgr.current();      // snapshot taken now
      await Promise.resolve();        // yield — reload may happen here
      urlsSeenByOps.push(cfg.anchorUrl);
    };

    // Start two ops before any reload.
    const op1 = makeOp();
    const op2 = makeOp();

    // Reload mid-flight.
    mgr.reload(VALID_CONFIG_2);

    // Start a third op AFTER the reload.
    const op3 = makeOp();

    await Promise.all([op1, op2, op3]);

    // First two ops snapshotted before reload → original URL.
    expect(urlsSeenByOps[0]).toBe('https://anchor.example.com');
    expect(urlsSeenByOps[1]).toBe('https://anchor.example.com');
    // Third op snapshotted after reload → new URL.
    expect(urlsSeenByOps[2]).toBe('https://other.anchor.example.com');
  });
});

// ---------------------------------------------------------------------------
// composeValidators
// ---------------------------------------------------------------------------

describe('composeValidators', () => {
  it('passes when all validators pass', () => {
    const alwaysOk: ConfigValidator<AnchorConfig> = () => ({ valid: true });
    const composed = composeValidators(validateAnchorConfig, alwaysOk);
    expect(composed(VALID_CONFIG)).toEqual({ valid: true });
  });

  it('returns the first failure and stops checking further validators', () => {
    const calls: string[] = [];

    const v1: ConfigValidator<AnchorConfig> = () => {
      calls.push('v1');
      return { valid: false, code: 'E1', message: 'fail1' };
    };
    const v2: ConfigValidator<AnchorConfig> = () => {
      calls.push('v2');
      return { valid: false, code: 'E2', message: 'fail2' };
    };

    const composed = composeValidators(v1, v2);
    const result = composed(VALID_CONFIG);

    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.code).toBe('E1');
    expect(calls).toEqual(['v1']); // v2 not reached
  });

  it('custom validator can enforce additional constraints', () => {
    const requirePublicKey: ConfigValidator<AnchorConfig> = (cfg) => {
      if (!cfg.publicKey) {
        return { valid: false, code: 'MISSING_PUBLIC_KEY', message: '`publicKey` is required', field: 'publicKey' };
      }
      return { valid: true };
    };

    const composed = composeValidators(validateAnchorConfig, requirePublicKey);

    // Missing publicKey but otherwise valid URL → fails on custom rule.
    const result = composed({ anchorUrl: 'https://anchor.example.com' });
    expect(result.valid).toBe(false);
    if (!result.valid) {
      expect(result.code).toBe('MISSING_PUBLIC_KEY');
      expect(result.field).toBe('publicKey');
    }

    // All fields present → passes.
    expect(composed(VALID_CONFIG)).toEqual({ valid: true });
  });
});

// ---------------------------------------------------------------------------
// validateAnchorConfig edge cases
// ---------------------------------------------------------------------------

describe('validateAnchorConfig', () => {
  it('accepts http:// URLs', () => {
    expect(validateAnchorConfig({ anchorUrl: 'http://localhost:8000' })).toEqual({ valid: true });
  });

  it('rejects a plain hostname without a scheme', () => {
    const result = validateAnchorConfig({ anchorUrl: 'anchor.example.com' });
    expect(result.valid).toBe(false);
  });

  it('accepts all optional numeric fields when positive', () => {
    const result = validateAnchorConfig({
      anchorUrl: 'https://anchor.example.com',
      timeout: 10_000,
      maxWatched: 100,
      reconnectDelay: 500,
      maxReconnectDelay: 30_000,
      pollIntervalMs: 5_000,
    });
    expect(result.valid).toBe(true);
  });

  it('rejects NaN for a numeric field', () => {
    const result = validateAnchorConfig({ anchorUrl: 'https://a.example.com', timeout: NaN });
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.field).toBe('timeout');
  });

  it('rejects Infinity for a numeric field', () => {
    const result = validateAnchorConfig({ anchorUrl: 'https://a.example.com', reconnectDelay: Infinity });
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.field).toBe('reconnectDelay');
  });
});
