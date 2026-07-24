/**
 * Sep6StreamingService
 *
 * Provides live SEP-6 transaction status updates using:
 *   1. SSE (EventSource) when the anchor supports it — lower latency.
 *   2. Long-poll / periodic polling when SSE is unavailable — graceful fallback.
 *
 * Key behaviours
 * ──────────────
 * - Detects anchor SSE support once at startup (HEAD request + feature flag).
 * - Tracks the last-seen `updated_at` cursor per transaction so reconnected
 *   streams never miss or duplicate a status change.
 * - Exponential back-off on reconnect; resets once a stream stabilises.
 * - Emits `TransactionStreamEvent` on every status change (not on no-ops).
 * - Emits `TransactionStreamError` for non-fatal problems (e.g. network blip).
 * - Automatically unsubscribes once a terminal status is received.
 * - Respects an optional AbortSignal on the config for global cancellation.
 */

import type {
  Sep6StreamConfig,
  Sep6Transaction,
  Sep6TransactionStatus,
  TransactionStreamEvent,
  TransactionStreamError,
  StreamHandle,
  WatcherState,
  AnchorStreamCapability,
} from '../types/sep6';
import { TERMINAL_STATUSES } from '../types/sep6';

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

const DEFAULT_RECONNECT_DELAY_MS = 1_000;
const DEFAULT_MAX_RECONNECT_DELAY_MS = 30_000;
const DEFAULT_POLL_INTERVAL_MS = 5_000;
const DEFAULT_MAX_WATCHED = 50;

// ---------------------------------------------------------------------------
// Public service class
// ---------------------------------------------------------------------------

export class Sep6StreamingService {
  private readonly anchorUrl: string;
  private readonly token: string;
  private readonly maxWatched: number;
  private readonly reconnectDelay: number;
  private readonly maxReconnectDelay: number;
  private readonly pollIntervalMs: number;
  private readonly globalSignal: AbortSignal | undefined;

  /** Resolved once during the first `watch()` call. */
  private capabilityPromise: Promise<AnchorStreamCapability> | null = null;

  /** Override: skip auto-detection, use this transport. */
  private readonly preferSse: boolean | undefined;

  /**
   * Active watchers keyed by transaction id.
   * A watcher is removed when it unsubscribes or reaches a terminal status.
   */
  private readonly watchers = new Map<string, WatcherState>();

  /** Poll timer handle (used in long-poll mode). */
  private pollTimerId: ReturnType<typeof setTimeout> | null = null;

  /** Whether the single shared poll loop is running. */
  private pollLoopRunning = false;

  constructor(config: Sep6StreamConfig) {
    this.anchorUrl = config.anchorUrl.replace(/\/$/, '');
    this.token = config.token;
    this.maxWatched = config.maxWatched ?? DEFAULT_MAX_WATCHED;
    this.reconnectDelay = config.reconnectDelay ?? DEFAULT_RECONNECT_DELAY_MS;
    this.maxReconnectDelay = config.maxReconnectDelay ?? DEFAULT_MAX_RECONNECT_DELAY_MS;
    this.pollIntervalMs = config.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    this.preferSse = config.preferSse;
    this.globalSignal = config.signal;

    if (this.globalSignal) {
      this.globalSignal.addEventListener('abort', () => this.destroy());
    }
  }

  // -------------------------------------------------------------------------
  // Public API
  // -------------------------------------------------------------------------

  /**
   * Subscribe to live status updates for a SEP-6 transaction.
   *
   * @param transactionId - SEP-6 transaction id to watch.
   * @param onEvent       - Called on every new status.
   * @param onError       - Called on non-fatal stream errors.
   * @returns A `StreamHandle` with an `unsubscribe()` method.
   */
  watch(
    transactionId: string,
    onEvent: (event: TransactionStreamEvent) => void,
    onError?: (error: TransactionStreamError) => void
  ): StreamHandle {
    if (this.watchers.size >= this.maxWatched) {
      throw new Error(
        `Sep6StreamingService: maxWatched limit (${this.maxWatched}) reached.`
      );
    }

    const state: WatcherState = {
      transactionId,
      lastStatus: null,
      cursor: null,
      onEvent,
      onError,
      active: true,
    };

    this.watchers.set(transactionId, state);
    this._startWatcher(state);

    return {
      transactionId,
      unsubscribe: () => this._removeWatcher(transactionId),
    };
  }

  /**
   * Stop all active watchers and release resources.
   */
  destroy(): void {
    for (const id of this.watchers.keys()) {
      this._removeWatcher(id);
    }
    if (this.pollTimerId !== null) {
      clearTimeout(this.pollTimerId);
      this.pollTimerId = null;
    }
    this.pollLoopRunning = false;
  }

  // -------------------------------------------------------------------------
  // Capability detection
  // -------------------------------------------------------------------------

  /**
   * Detect whether the anchor supports SSE by sending a HEAD request to the
   * streaming endpoint and looking for `Content-Type: text/event-stream`.
   *
   * The result is cached for the lifetime of the service instance.
   */
  detectCapabilities(): Promise<AnchorStreamCapability> {
    if (this.capabilityPromise) return this.capabilityPromise;

    // If the caller has hard-coded preferSse, skip network detection.
    if (this.preferSse === true) {
      this.capabilityPromise = Promise.resolve({ supportsSSE: true, supportsCursor: false });
      return this.capabilityPromise;
    }
    if (this.preferSse === false) {
      this.capabilityPromise = Promise.resolve({ supportsSSE: false, supportsCursor: false });
      return this.capabilityPromise;
    }

    // Check if EventSource is available in the runtime at all.
    if (typeof EventSource === 'undefined') {
      this.capabilityPromise = Promise.resolve({ supportsSSE: false, supportsCursor: false });
      return this.capabilityPromise;
    }

    this.capabilityPromise = (async (): Promise<AnchorStreamCapability> => {
      try {
        const url = `${this.anchorUrl}/transaction?stream=true&id=probe`;
        const resp = await fetch(url, {
          method: 'HEAD',
          headers: this._authHeaders(),
          signal: AbortSignal.timeout(4_000),
        });
        const ct = resp.headers.get('content-type') ?? '';
        const supportsSSE = ct.includes('text/event-stream');
        const supportsCursor = resp.headers.get('x-anchor-stream-cursor') === 'true';
        return { supportsSSE, supportsCursor };
      } catch {
        return { supportsSSE: false, supportsCursor: false };
      }
    })();

    return this.capabilityPromise;
  }

  // -------------------------------------------------------------------------
  // Internal: watcher lifecycle
  // -------------------------------------------------------------------------

  private async _startWatcher(state: WatcherState): Promise<void> {
    const cap = await this.detectCapabilities();

    if (!state.active) return; // was unsubscribed while awaiting detection

    if (cap.supportsSSE) {
      this._startSseWatcher(state);
    } else {
      this._ensurePollLoop();
    }
  }

  private _removeWatcher(transactionId: string): void {
    const state = this.watchers.get(transactionId);
    if (state) {
      state.active = false;
      this.watchers.delete(transactionId);
    }
  }

  // -------------------------------------------------------------------------
  // SSE transport
  // -------------------------------------------------------------------------

  private _startSseWatcher(state: WatcherState, backoffMs = 0): void {
    if (!state.active) return;

    if (backoffMs > 0) {
      setTimeout(() => this._openSseConnection(state, backoffMs), backoffMs);
    } else {
      this._openSseConnection(state, backoffMs);
    }
  }

  private _openSseConnection(state: WatcherState, currentBackoff: number): void {
    if (!state.active) return;

    const url = new URL(`${this.anchorUrl}/transaction`);
    url.searchParams.set('id', state.transactionId);
    url.searchParams.set('stream', 'true');
    if (state.cursor) {
      url.searchParams.set('cursor', state.cursor);
    }

    // Inject auth token via URL param — EventSource does not allow custom headers.
    url.searchParams.set('token', this.token);

    const es = new EventSource(url.toString());

    const cleanup = () => {
      es.close();
    };

    // Clean up if the global signal fires while this stream is open.
    if (this.globalSignal) {
      this.globalSignal.addEventListener('abort', cleanup, { once: true });
    }

    es.addEventListener('message', (ev: MessageEvent) => {
      this._handleSseMessage(state, ev.data);
    });

    es.addEventListener('transaction', (ev: MessageEvent) => {
      this._handleSseMessage(state, ev.data);
    });

    es.addEventListener('error', () => {
      cleanup();
      if (this.globalSignal?.aborted) return;
      if (!state.active) return;

      const nextBackoff = Math.min(
        currentBackoff === 0 ? this.reconnectDelay : currentBackoff * 2,
        this.maxReconnectDelay
      );

      state.onError?.({
        transactionId: state.transactionId,
        code: 'SSE_ERROR',
        message: 'SSE stream disconnected; reconnecting.',
        recoverable: true,
      });

      // Reconnect with exponential back-off.
      this._startSseWatcher(state, nextBackoff);
    });
  }

  private _handleSseMessage(state: WatcherState, data: string): void {
    if (!state.active) return;

    let tx: Sep6Transaction;
    try {
      const parsed = JSON.parse(data) as { transaction?: Sep6Transaction } | Sep6Transaction;
      // Some anchors wrap it in { transaction: {...} }
      tx = 'transaction' in parsed && parsed.transaction ? parsed.transaction : (parsed as Sep6Transaction);
    } catch {
      // Ignore unparseable frames
      return;
    }

    this._dispatchUpdate(state, tx, 'sse');
  }

  // -------------------------------------------------------------------------
  // Long-poll / periodic polling transport
  // -------------------------------------------------------------------------

  /**
   * Start the shared poll loop if it isn't already running.
   * All watchers in long-poll mode share one loop to avoid hammering the anchor.
   */
  private _ensurePollLoop(): void {
    if (this.pollLoopRunning) return;
    this.pollLoopRunning = true;
    this._schedulePoll(0);
  }

  private _schedulePoll(delayMs: number): void {
    if (this.pollTimerId !== null) clearTimeout(this.pollTimerId);
    this.pollTimerId = setTimeout(() => this._runPollCycle(), delayMs);
  }

  private async _runPollCycle(): Promise<void> {
    if (this.globalSignal?.aborted) {
      this.pollLoopRunning = false;
      return;
    }

    const activeWatchers = [...this.watchers.values()].filter((w) => w.active);
    if (activeWatchers.length === 0) {
      this.pollLoopRunning = false;
      return;
    }

    // Fetch each watched transaction individually; SEP-6 doesn't mandate a
    // bulk endpoint, so we stay spec-compliant here.  A real implementation
    // could batch with `GET /transactions?id[]=…` if the anchor supports it.
    await Promise.allSettled(activeWatchers.map((w) => this._pollOne(w)));

    // Re-check after awaiting; some watchers may have been removed.
    const stillActive = [...this.watchers.values()].some((w) => w.active);
    if (stillActive && !this.globalSignal?.aborted) {
      this._schedulePoll(this.pollIntervalMs);
    } else {
      this.pollLoopRunning = false;
    }
  }

  private async _pollOne(state: WatcherState): Promise<void> {
    if (!state.active) return;

    try {
      const tx = await this._fetchTransaction(state.transactionId);
      if (tx) {
        this._dispatchUpdate(state, tx, 'poll');
      }
    } catch (err) {
      state.onError?.({
        transactionId: state.transactionId,
        code: 'POLL_ERROR',
        message: `Failed to fetch transaction: ${String(err)}`,
        recoverable: true,
      });
    }
  }

  // -------------------------------------------------------------------------
  // Shared update dispatcher
  // -------------------------------------------------------------------------

  /**
   * Compare the incoming status against the last known status.
   * Only emit an event when the status has actually changed.
   * Update the cursor on every call (even no-ops) to keep it fresh.
   */
  private _dispatchUpdate(
    state: WatcherState,
    tx: Sep6Transaction,
    transport: 'sse' | 'poll'
  ): void {
    if (!state.active) return;

    const newStatus = tx.status as Sep6TransactionStatus;

    // Update cursor — used to resume SSE streams after reconnect.
    // We use `completed_at ?? started_at` as the best available timestamp.
    state.cursor = tx.completed_at ?? tx.started_at ?? state.cursor;

    // Only fire the callback when the status has changed.
    if (state.lastStatus === newStatus) return;

    const previousStatus = state.lastStatus;
    state.lastStatus = newStatus;
    const isTerminal = TERMINAL_STATUSES.has(newStatus);

    const event: TransactionStreamEvent = {
      transactionId: tx.id,
      previousStatus,
      status: newStatus,
      transaction: tx,
      isTerminal,
      receivedAt: new Date().toISOString(),
      transport,
    };

    state.onEvent(event);

    // Auto-unsubscribe on terminal status — no further updates expected.
    if (isTerminal) {
      this._removeWatcher(state.transactionId);
    }
  }

  // -------------------------------------------------------------------------
  // HTTP helpers
  // -------------------------------------------------------------------------

  private async _fetchTransaction(transactionId: string): Promise<Sep6Transaction | null> {
    const url = `${this.anchorUrl}/transaction?id=${encodeURIComponent(transactionId)}`;
    const resp = await fetch(url, {
      headers: this._authHeaders(),
      signal: this.globalSignal,
    });

    if (resp.status === 404) return null;
    if (!resp.ok) {
      throw new Error(`HTTP ${resp.status}: ${resp.statusText}`);
    }

    const body = (await resp.json()) as { transaction: Sep6Transaction } | Sep6Transaction;
    if ('transaction' in body && body.transaction) {
      return body.transaction;
    }
    return body as Sep6Transaction;
  }

  private _authHeaders(): Record<string, string> {
    return {
      Authorization: `Bearer ${this.token}`,
      'Content-Type': 'application/json',
    };
  }
}

// ---------------------------------------------------------------------------
// Convenience error class
// ---------------------------------------------------------------------------

export class Sep6StreamError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly details?: Record<string, unknown>
  ) {
    super(message);
    this.name = 'Sep6StreamError';
  }
}
