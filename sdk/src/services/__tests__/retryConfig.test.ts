import { describe, it, expect } from 'vitest';
import { RetryConfig, RetryConfigError } from '../retryConfig';

describe('RetryConfig', () => {
  describe('defaults', () => {
    it('initializes with documented sensible defaults', () => {
      const config = new RetryConfig();

      expect(config.maxAttempts).toBe(3);
      expect(config.baseDelayMs).toBe(500);
      expect(config.maxDelayMs).toBe(10000);
      expect(config.backoffMultiplier).toBe(2.0);
      expect(config.jitter).toBe(0.1);
    });

    it('RetryConfig.default() factory returns default config', () => {
      const config = RetryConfig.default();

      expect(config.maxAttempts).toBe(3);
      expect(config.baseDelayMs).toBe(500);
      expect(config.maxDelayMs).toBe(10000);
    });
  });

  describe('custom configurations', () => {
    it('accepts valid custom options', () => {
      const config = new RetryConfig({
        maxAttempts: 5,
        baseDelayMs: 250,
        maxDelayMs: 5000,
        backoffMultiplier: 1.5,
        jitter: 0.05,
      });

      expect(config.maxAttempts).toBe(5);
      expect(config.baseDelayMs).toBe(250);
      expect(config.maxDelayMs).toBe(5000);
      expect(config.backoffMultiplier).toBe(1.5);
      expect(config.jitter).toBe(0.05);
    });

    it('rejects invalid maxAttempts', () => {
      expect(() => new RetryConfig({ maxAttempts: 0 })).toThrow(RetryConfigError);
      expect(() => new RetryConfig({ maxAttempts: -2 })).toThrow(RetryConfigError);
      expect(() => new RetryConfig({ maxAttempts: 1.5 })).toThrow(RetryConfigError);
    });

    it('rejects negative baseDelayMs', () => {
      expect(() => new RetryConfig({ baseDelayMs: -100 })).toThrow(RetryConfigError);
    });

    it('rejects maxDelayMs smaller than baseDelayMs', () => {
      expect(() => new RetryConfig({ baseDelayMs: 1000, maxDelayMs: 500 })).toThrow(
        RetryConfigError
      );
    });

    it('rejects backoffMultiplier < 1', () => {
      expect(() => new RetryConfig({ backoffMultiplier: 0.5 })).toThrow(RetryConfigError);
    });

    it('rejects invalid jitter outside [0, 1]', () => {
      expect(() => new RetryConfig({ jitter: -0.1 })).toThrow(RetryConfigError);
      expect(() => new RetryConfig({ jitter: 1.5 })).toThrow(RetryConfigError);
    });
  });

  describe('cloning and multi-client instance isolation', () => {
    it('clones without overrides returning an equivalent independent instance', () => {
      const base = new RetryConfig({ maxAttempts: 4, baseDelayMs: 300 });
      const copy = base.clone();

      expect(copy).not.toBe(base);
      expect(copy.maxAttempts).toBe(300 ? 4 : 4);
      expect(copy.baseDelayMs).toBe(300);
      expect(copy.maxDelayMs).toBe(base.maxDelayMs);
    });

    it('allows multiple client instances to specialize independently without mutating the base', () => {
      const base = new RetryConfig({ maxAttempts: 3, baseDelayMs: 500 });

      // Client A needs aggressive retries
      const clientAConfig = base.clone({ maxAttempts: 6, baseDelayMs: 200 });

      // Client B needs conservative retries with high max delay
      const clientBConfig = base.clone({ maxAttempts: 2, maxDelayMs: 20000 });

      // Base remains untouched
      expect(base.maxAttempts).toBe(3);
      expect(base.baseDelayMs).toBe(500);
      expect(base.maxDelayMs).toBe(10000);

      // Client A has its own knobs
      expect(clientAConfig.maxAttempts).toBe(6);
      expect(clientAConfig.baseDelayMs).toBe(200);
      expect(clientAConfig.maxDelayMs).toBe(10000);

      // Client B has its own knobs
      expect(clientBConfig.maxAttempts).toBe(2);
      expect(clientBConfig.baseDelayMs).toBe(500);
      expect(clientBConfig.maxDelayMs).toBe(20000);
    });
  });

  describe('backoff calculation and retry checks', () => {
    it('computes exponential delay correctly without jitter', () => {
      const config = new RetryConfig({
        baseDelayMs: 100,
        backoffMultiplier: 2.0,
        maxDelayMs: 1000,
      });

      expect(config.computeDelayMs(0)).toBe(100);
      expect(config.computeDelayMs(1)).toBe(200);
      expect(config.computeDelayMs(2)).toBe(400);
      expect(config.computeDelayMs(3)).toBe(800);
      expect(config.computeDelayMs(4)).toBe(1000); // capped at maxDelayMs
    });

    it('evaluates shouldRetry accurately based on maxAttempts', () => {
      const config = new RetryConfig({ maxAttempts: 3 });

      expect(config.shouldRetry(0)).toBe(true);
      expect(config.shouldRetry(1)).toBe(true);
      expect(config.shouldRetry(2)).toBe(true);
      expect(config.shouldRetry(3)).toBe(false);
      expect(config.shouldRetry(4)).toBe(false);
    });
  });
});
