/**
 * Hot-reload config support for the AnchorKit SDK.
 *
 * Design goals
 * ────────────
 * 1. **Atomic swap** — a `reload()` call either fully commits the new config
 *    or leaves the existing config completely intact (on validation failure).
 *
 * 2. **In-flight safety** — each async operation grabs a *snapshot* of the
 *    config at the moment it starts (`configManager.current()`).  Because
 *    JavaScript is single-threaded, the snapshot is taken synchronously
 *    before the first `await`, so the in-flight operation always works
 *    against the config it started with — never a half-swapped intermediate.
 *
 * 3. **Validation on reload** — an invalid update is rejected with a
 *    `ConfigValidationError`; the currently-loaded config is untouched.
 *
 * 4. **Change notifications** — callers can register a listener that is
 *    invoked (synchronously, on the same tick) whenever a reload succeeds.
 */

// ---------------------------------------------------------------------------
// AnchorConfig — superset of Sep10Config + Sep6StreamConfig
// ---------------------------------------------------------------------------

/**
 * Unified configuration for an AnchorKit SDK instance.
 *
 * This type is the union of all service-level configs (`Sep10Config`,
 * `Sep6StreamConfig`) so that a single `ConfigManager<AnchorConfig>` can
 * cover the whole SDK surface.  Unknown fields are accepted as-is to allow
 * forward compatibility.
 */
export interface AnchorConfig {
  // ── Common ────────────────────────────────────────────────────────────────
  /** Anchor server base URL (no trailing slash). */
  anchorUrl: string;

  // ── SEP-10 ────────────────────────────────────────────────────────────────
  /** Stellar account public key (SEP-10). */
  publicKey?: string;
  /** Optional domain for multi-domain support (SEP-10 extension). */
  domain?: string;
  /** Request timeout in milliseconds. Defaults to 30 000 ms. */
  timeout?: number;

  // ── SEP-6 streaming ───────────────────────────────────────────────────────
  /** SEP-10 JWT token for authenticated requests (SEP-6). */
  token?: string;
  /** Maximum number of transactions to watch simultaneously. Defaults to 50. */
  maxWatched?: number;
  /** Initial reconnect delay in ms. Doubles each failed attempt up to `maxReconnectDelay`. */
  reconnectDelay?: number;
  /** Upper bound on reconnect back-off in ms. */
  maxReconnectDelay?: number;
  /** Polling interval in ms when SSE is unavailable. */
  pollIntervalMs?: number;
  /** Override transport preference. */
  preferSse?: boolean;

  // ── Escape hatch ──────────────────────────────────────────────────────────
  /** Any additional anchor-specific fields. */
  [key: string]: unknown;
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/** Thrown by `ConfigManager.reload()` when the candidate config is invalid. */
export class ConfigValidationError extends Error {
  /** Machine-readable code for the first failing rule. */
  readonly code: string;
  /** The field path that triggered the error, when applicable. */
  readonly field?: string;

  constructor(code: string, message: string, field?: string) {
    super(message);
    this.name = 'ConfigValidationError';
    this.code = code;
    this.field = field;
  }
}

/** Validation result returned by a `ConfigValidator`. */
export type ValidationResult =
  | { valid: true }
  | { valid: false; code: string; message: string; field?: string };

/**
 * A function that inspects a candidate config and returns a `ValidationResult`.
 *
 * Compose multiple validators with `composeValidators`.
 */
export type ConfigValidator<T> = (candidate: T) => ValidationResult;

/**
 * Built-in validator for `AnchorConfig`.
 *
 * Rules:
 * - `anchorUrl` must be a non-empty string.
 * - `anchorUrl` must be a valid absolute URL (http or https).
 * - `timeout`, `maxWatched`, `reconnectDelay`, `maxReconnectDelay`,
 *   `pollIntervalMs` must be positive integers when present.
 */
export function validateAnchorConfig(candidate: AnchorConfig): ValidationResult {
  if (typeof candidate.anchorUrl !== 'string' || candidate.anchorUrl.trim() === '') {
    return {
      valid: false,
      code: 'MISSING_ANCHOR_URL',
      message: '`anchorUrl` is required and must be a non-empty string.',
      field: 'anchorUrl',
    };
  }

  try {
    const parsed = new URL(candidate.anchorUrl);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return {
        valid: false,
        code: 'INVALID_ANCHOR_URL_SCHEME',
        message: '`anchorUrl` must use the http or https scheme.',
        field: 'anchorUrl',
      };
    }
  } catch {
    return {
      valid: false,
      code: 'INVALID_ANCHOR_URL',
      message: '`anchorUrl` must be a valid absolute URL.',
      field: 'anchorUrl',
    };
  }

  const positiveIntFields = [
    'timeout',
    'maxWatched',
    'reconnectDelay',
    'maxReconnectDelay',
    'pollIntervalMs',
  ] as const;

  for (const field of positiveIntFields) {
    const value = candidate[field];
    if (value !== undefined) {
      if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
        return {
          valid: false,
          code: 'INVALID_FIELD_VALUE',
          message: `\`${field}\` must be a positive finite number when provided (got ${String(value)}).`,
          field,
        };
      }
    }
  }

  return { valid: true };
}

/**
 * Compose multiple validators into one.  Validators are run in order;
 * the first failure short-circuits and is returned.
 */
export function composeValidators<T>(...validators: ConfigValidator<T>[]): ConfigValidator<T> {
  return (candidate: T): ValidationResult => {
    for (const validate of validators) {
      const result = validate(candidate);
      if (!result.valid) return result;
    }
    return { valid: true };
  };
}

// ---------------------------------------------------------------------------
// ConfigManager
// ---------------------------------------------------------------------------

/** Called synchronously after every successful `reload()`. */
export type ConfigChangeListener<T> = (next: T, previous: T) => void;

/**
 * Manages a config value with hot-reload support.
 *
 * ```ts
 * const mgr = new ConfigManager(initialConfig, validateAnchorConfig);
 *
 * // Operations: grab a snapshot at the start of each async call.
 * async function doRequest() {
 *   const cfg = mgr.current();   // synchronous, O(1) reference read
 *   const resp = await fetch(`${cfg.anchorUrl}/auth`);
 *   // … uses cfg throughout; immune to concurrent reloads
 * }
 *
 * // Reload: validates before committing.
 * mgr.reload({ ...mgr.current(), anchorUrl: 'https://new.anchor.example.com' });
 * // If validation fails, a ConfigValidationError is thrown and the current
 * // config is left untouched.
 * ```
 */
export class ConfigManager<T> {
  private _current: Readonly<T>;
  private readonly _validator: ConfigValidator<T>;
  private readonly _listeners: Array<ConfigChangeListener<T>> = [];

  /**
   * @param initial   The starting configuration; validated at construction.
   * @param validator A function that returns `{ valid: true }` or an error
   *                  descriptor for invalid configs.  `validateAnchorConfig`
   *                  is the built-in choice for `AnchorConfig`.
   */
  constructor(initial: T, validator: ConfigValidator<T>) {
    this._validator = validator;

    // Validate the initial config eagerly so callers discover misconfiguration
    // at construction time rather than silently accepting a broken initial state.
    const result = validator(initial);
    if (!result.valid) {
      throw new ConfigValidationError(result.code, result.message, result.field);
    }

    // Deep-freeze so that in-flight operations cannot accidentally mutate it.
    this._current = Object.freeze({ ...initial });
  }

  // -------------------------------------------------------------------------
  // Reading
  // -------------------------------------------------------------------------

  /**
   * Return the current config snapshot.
   *
   * This is a synchronous O(1) read.  Callers should call this once at the
   * **start** of each async operation and hold the returned reference for the
   * duration of that operation — this is the "snapshot" pattern that gives
   * in-flight requests immunity from concurrent reloads.
   */
  current(): Readonly<T> {
    return this._current;
  }

  // -------------------------------------------------------------------------
  // Reloading
  // -------------------------------------------------------------------------

  /**
   * Atomically validate and swap in a new config.
   *
   * - If `candidate` passes validation, the swap happens synchronously before
   *   this method returns, and all registered listeners are notified.
   * - If `candidate` fails validation, a `ConfigValidationError` is thrown and
   *   the current config is **not** modified — not even partially.
   *
   * @throws {ConfigValidationError} When the candidate config is invalid.
   */
  reload(candidate: T): void {
    const result = this._validator(candidate);
    if (!result.valid) {
      throw new ConfigValidationError(result.code, result.message, result.field);
    }

    const previous = this._current;
    // Freeze a shallow copy so in-flight holders still see their own snapshot
    // and cannot mutate the new canonical config.
    this._current = Object.freeze({ ...candidate });

    // Notify listeners synchronously on the same tick.
    for (const listener of this._listeners) {
      try {
        listener(this._current, previous);
      } catch {
        // Never let a listener crash the reload path.
      }
    }
  }

  // -------------------------------------------------------------------------
  // Change listeners
  // -------------------------------------------------------------------------

  /**
   * Register a callback that is invoked (synchronously) after every
   * successful `reload()`.
   *
   * @returns An unsubscribe function.
   */
  onChange(listener: ConfigChangeListener<T>): () => void {
    this._listeners.push(listener);
    return () => {
      const idx = this._listeners.indexOf(listener);
      if (idx !== -1) this._listeners.splice(idx, 1);
    };
  }
}
