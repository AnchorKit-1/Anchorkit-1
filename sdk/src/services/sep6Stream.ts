/**
 * TransactionStream — live SEP-6 transaction status updates.
 *
 * Streaming priority:
 *   1. SSE  (`EventSource` on `GET /transaction/stream?id=<id>`)
 *   2. Long-poll  (`GET /transactions?id=<id>&long_poll_timeout=<n>`)
 *   3. Polling  (`GET /transaction?id=<id>` on a fixed interval)
 *
 * SSE and long-poll are tried in order; if the anchor returns HTTP 404 or
 * 405 the mode is marked unsupported and the next mode is used for this run
 * and all future reconnects. Reconnection on transient network failures uses
 * truncated exponential back-off. SSE reconnection passes `Last-Event-ID` so
 * the anchor can replay any updates that arrived during the gap.
 *
 * The stream auto-closes when a terminal status is received. Callers can
 * also close it explicitly with `stop()`.
 */

import type {
  Sep6StreamConfig,
  Sep6Transaction,
  Sep6TransactionResponse,
  Sep6TransactionsResponse,
  Sep6StatusUpdate,
  StreamCloseEvent,
  StreamCloseReason,
  StreamMode,
} from '../types/sep6';
import { isTerminalStatus } from '../types/sep6';

type StatusUpdateHandler = (update: Sep6StatusUpdate) => void;
type CloseHandler = (event: StreamCloseEvent) => void;
type ErrorHandler = (error: Error) => void;

/** `EventSource`-like interface so tests can inject a stub. */
interface EventSourceLike extends EventTarget {
  readonly readyState: number;
  readonly url: string;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onopen: ((event: Event) => void) | null;
  close(): void;
}

type EventSourceFactory = (url: string, init?: { withCredentials?: boolean }) => EventSourceLike;

/**
 * Live SEP-6 transaction status stream.
 *
 * ```ts
 * const stream = new TransactionStream({
 *   transferServerUrl: 'https://anchor.example.com',
 *   transactionId: 'abc123',
 *   authToken: jwtToken,
 * });
 *
 * stream.onUpdate((update) => console.log(update.transaction.status));
 * stream.onClose((ev) => console.log('done:', ev.reason));
 * stream.start();
 * ```
 */
export class TransactionStream {
  private readonly config: Required<
    Pick<
      Sep6StreamConfig,
      | 'transferServerUrl'
      | 'transactionId'
      | 'longPollTimeoutSecs'
      | 'pollingIntervalMs'
      | 'maxReconnectAttempts'
      | 'initialReconnectDelayMs'
      | 'maxReconnectDelayMs'
    >
  > & {
    authToken: string | undefined;
    preferredMode: StreamMode;
    fetch: typeof fetch;
  };

  /** Injected EventSource factory (swappable for tests). */
  private readonly eventSourceFactory: EventSourceFactory;

  private updateHandlers: StatusUpdateHandler[] = [];
  private closeHandlers: CloseHandler[] = [];
  private errorHandlers: ErrorHandler[] = [];

  /** Current active mode. Downgrades permanently when a mode proves unsupported. */
  private currentMode: StreamMode;

  /** `false` after SSE proves unsupported; never flips back to `true`. */
  private sseSupported = true;
  /** `false` after long-poll proves unsupported; never flips back to `true`. */
  private longPollSupported = true;

  private running = false;
  private stopped = false;
  private reconnectAttempts = 0;
  private lastEventId: string | undefined;
  private lastKnownTransaction: Sep6Transaction | undefined;

  /** Handle for the current SSE connection so we can close it. */
  private activeEventSource: EventSourceLike | undefined;
  /** Handle for the current poll/long-poll timer/promise abort. */
  private activeAbortController: AbortController | undefined;
  /** Handle for the reconnect delay timer. */
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(config: Sep6StreamConfig, eventSourceFactory?: EventSourceFactory) {
    this.config = {
      transferServerUrl: config.transferServerUrl.replace(/\/$/, ''),
      transactionId: config.transactionId,
      authToken: config.authToken,
      preferredMode: config.preferredMode ?? 'sse',
      longPollTimeoutSecs: config.longPollTimeoutSecs ?? 30,
      pollingIntervalMs: config.pollingIntervalMs ?? 5000,
      maxReconnectAttempts: config.maxReconnectAttempts ?? 10,
      initialReconnectDelayMs: config.initialReconnectDelayMs ?? 1000,
      maxReconnectDelayMs: config.maxReconnectDelayMs ?? 30000,
      fetch: config.fetch ?? globalThis.fetch.bind(globalThis),
    };

    // If the caller starts from a lower-priority mode, mark higher-priority
    // ones as unsupported so we never attempt them.
    if (this.config.preferredMode === 'long-poll') {
      this.sseSupported = false;
    } else if (this.config.preferredMode === 'polling') {
      this.sseSupported = false;
      this.longPollSupported = false;
    }

    this.currentMode = this.config.preferredMode;

    // Default EventSource factory: uses the global EventSource
    this.eventSourceFactory =
      eventSourceFactory ??
      ((url) => {
        if (typeof EventSource === 'undefined') {
          throw new Error('EventSource is not available in this environment');
        }
        return new EventSource(url) as unknown as EventSourceLike;
      });
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  /** Register a callback for status update events. */
  onUpdate(handler: StatusUpdateHandler): this {
    this.updateHandlers.push(handler);
    return this;
  }

  /** Register a callback for stream-close events. */
  onClose(handler: CloseHandler): this {
    this.closeHandlers.push(handler);
    return this;
  }

  /** Register a callback for non-fatal errors (connection drops, parse failures). */
  onError(handler: ErrorHandler): this {
    this.errorHandlers.push(handler);
    return this;
  }

  /**
   * Start streaming. Idempotent: calling `start()` on an already-running
   * stream is a no-op.
   */
  start(): void {
    if (this.running || this.stopped) return;
    this.running = true;
    this.reconnectAttempts = 0;
    this.runLoop();
  }

  /**
   * Stop the stream immediately. After `stop()`, no further callbacks fire
   * and the stream cannot be restarted.
   */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.running = false;
    this.cancelActive();
    this.emitClose({ reason: 'closed_by_caller', lastTransaction: this.lastKnownTransaction });
  }

  /** The last transaction state received, or `undefined` before the first update. */
  get lastTransaction(): Sep6Transaction | undefined {
    return this.lastKnownTransaction;
  }

  /** The transport currently in use. */
  get mode(): StreamMode {
    return this.currentMode;
  }

  // ---------------------------------------------------------------------------
  // Main loop
  // ---------------------------------------------------------------------------

  private runLoop(): void {
    if (this.stopped) return;

    if (this.sseSupported && this.currentMode === 'sse') {
      this.runSse();
    } else if (this.longPollSupported && this.currentMode !== 'polling') {
      this.currentMode = 'long-poll';
      this.runLongPoll();
    } else {
      this.currentMode = 'polling';
      this.runPolling();
    }
  }

  // ---------------------------------------------------------------------------
  // SSE transport
  // ---------------------------------------------------------------------------

  private runSse(): void {
    const url = this.buildSseUrl();

    let es: EventSourceLike;
    try {
      es = this.eventSourceFactory(url);
    } catch (err) {
      // EventSource unavailable (e.g., Node.js without polyfill)
      this.sseSupported = false;
      this.currentMode = this.longPollSupported ? 'long-poll' : 'polling';
      this.runLoop();
      return;
    }

    this.activeEventSource = es;

    es.onmessage = (event: MessageEvent) => {
      this.lastEventId = (event as MessageEvent & { lastEventId?: string }).lastEventId ?? this.lastEventId;
      this.onRawData(event.data as string, 'sse');
    };

    es.onerror = (_ev: Event) => {
      this.activeEventSource = undefined;
      es.close();
      this.emitError(new Error('SSE connection dropped'));
      this.scheduleReconnect();
    };
  }

  private buildSseUrl(): string {
    const params = new URLSearchParams({ id: this.config.transactionId });
    if (this.lastEventId) {
      params.set('last_event_id', this.lastEventId);
    }
    if (this.config.authToken) {
      // Some anchors accept the token as a query parameter for SSE because
      // EventSource doesn't support custom headers.
      params.set('auth_token', this.config.authToken);
    }
    return `${this.config.transferServerUrl}/transaction/stream?${params}`;
  }

  // ---------------------------------------------------------------------------
  // Long-poll transport
  // ---------------------------------------------------------------------------

  private async runLongPoll(): Promise<void> {
    if (this.stopped) return;

    const controller = new AbortController();
    this.activeAbortController = controller;

    const url = this.buildLongPollUrl();
    const headers = this.buildAuthHeaders();

    let response: Response;
    try {
      response = await this.config.fetch(url, {
        headers,
        signal: controller.signal,
      });
    } catch (err) {
      if (this.stopped) return;
      if ((err as Error).name === 'AbortError') return;
      this.emitError(err instanceof Error ? err : new Error(String(err)));
      this.scheduleReconnect();
      return;
    }

    if (this.stopped) return;

    // 404 / 405 means the anchor doesn't support long-poll at this endpoint.
    if (response.status === 404 || response.status === 405) {
      this.longPollSupported = false;
      this.currentMode = 'polling';
      this.runLoop();
      return;
    }

    if (!response.ok) {
      this.emitError(new Error(`Long-poll request failed: HTTP ${response.status}`));
      this.scheduleReconnect();
      return;
    }

    let body: Sep6TransactionsResponse;
    try {
      body = (await response.json()) as Sep6TransactionsResponse;
    } catch (err) {
      this.emitError(new Error(`Failed to parse long-poll response: ${err}`));
      this.scheduleReconnect();
      return;
    }

    const tx = body.transactions?.[0];
    if (tx) {
      const terminal = this.processTransaction(tx, 'long-poll');
      if (terminal) return;
    }

    // Continue long-polling immediately (the server already imposed its
    // own hold; our next call is the next "heartbeat").
    if (!this.stopped) {
      this.reconnectAttempts = 0;
      void this.runLongPoll();
    }
  }

  private buildLongPollUrl(): string {
    const params = new URLSearchParams({
      id: this.config.transactionId,
      long_poll_timeout: String(this.config.longPollTimeoutSecs),
    });
    return `${this.config.transferServerUrl}/transactions?${params}`;
  }

  // ---------------------------------------------------------------------------
  // Polling transport (interval-based fallback)
  // ---------------------------------------------------------------------------

  private runPolling(): void {
    void this.pollOnce();
  }

  private async pollOnce(): Promise<void> {
    if (this.stopped) return;

    const controller = new AbortController();
    this.activeAbortController = controller;

    const url = `${this.config.transferServerUrl}/transaction?id=${encodeURIComponent(this.config.transactionId)}`;
    const headers = this.buildAuthHeaders();

    let response: Response;
    try {
      response = await this.config.fetch(url, {
        headers,
        signal: controller.signal,
      });
    } catch (err) {
      if (this.stopped) return;
      if ((err as Error).name === 'AbortError') return;
      this.emitError(err instanceof Error ? err : new Error(String(err)));
      this.scheduleReconnect();
      return;
    }

    if (this.stopped) return;

    if (!response.ok) {
      this.emitError(new Error(`Poll request failed: HTTP ${response.status}`));
      this.scheduleReconnect();
      return;
    }

    let body: Sep6TransactionResponse;
    try {
      body = (await response.json()) as Sep6TransactionResponse;
    } catch (err) {
      this.emitError(new Error(`Failed to parse poll response: ${err}`));
      this.scheduleReconnect();
      return;
    }

    const tx = body.transaction;
    if (tx) {
      const terminal = this.processTransaction(tx, 'polling');
      if (terminal) return;
    }

    // Schedule next poll
    if (!this.stopped) {
      this.reconnectAttempts = 0;
      this.reconnectTimer = setTimeout(() => {
        void this.pollOnce();
      }, this.config.pollingIntervalMs);
    }
  }

  // ---------------------------------------------------------------------------
  // Shared helpers
  // ---------------------------------------------------------------------------

  /**
   * Parse raw SSE data string into a transaction and process it.
   * Returns `true` if a terminal status was reached.
   */
  private onRawData(data: string, mode: StreamMode): boolean {
    let tx: Sep6Transaction;
    try {
      const parsed = JSON.parse(data) as unknown;
      // Anchors may wrap it as `{ transaction: { ... } }` or unwrap.
      if (
        parsed !== null &&
        typeof parsed === 'object' &&
        'transaction' in (parsed as Record<string, unknown>)
      ) {
        tx = (parsed as Sep6TransactionResponse).transaction;
      } else {
        tx = parsed as Sep6Transaction;
      }
    } catch {
      this.emitError(new Error(`Failed to parse SSE data: ${data}`));
      return false;
    }
    return this.processTransaction(tx, mode);
  }

  /**
   * Record the transaction, emit an update event, and close the stream if
   * the status is terminal.
   *
   * Returns `true` when a terminal status has been reached.
   */
  private processTransaction(tx: Sep6Transaction, mode: StreamMode): boolean {
    this.lastKnownTransaction = tx;
    const terminal = isTerminalStatus(tx.status);

    this.emitUpdate({ transaction: tx, mode, terminal });

    if (terminal) {
      this.stopped = true;
      this.running = false;
      this.cancelActive();
      this.emitClose({ reason: 'terminal_status', lastTransaction: tx });
      return true;
    }

    return false;
  }

  /**
   * Schedule a reconnect with truncated exponential back-off.
   * If `maxReconnectAttempts` is exhausted, emits a close event instead.
   */
  private scheduleReconnect(): void {
    if (this.stopped) return;

    if (this.reconnectAttempts >= this.config.maxReconnectAttempts) {
      this.stopped = true;
      this.running = false;
      this.emitClose({
        reason: 'max_reconnect_attempts',
        lastTransaction: this.lastKnownTransaction,
        error: new Error(
          `Gave up after ${this.reconnectAttempts} reconnect attempt(s)`
        ),
      });
      return;
    }

    const delay = Math.min(
      this.config.initialReconnectDelayMs * 2 ** this.reconnectAttempts,
      this.config.maxReconnectDelayMs
    );
    this.reconnectAttempts++;

    this.reconnectTimer = setTimeout(() => {
      if (!this.stopped) this.runLoop();
    }, delay);
  }

  /** Cancel any in-flight SSE connection, fetch request, or poll timer. */
  private cancelActive(): void {
    if (this.activeEventSource) {
      this.activeEventSource.close();
      this.activeEventSource = undefined;
    }
    if (this.activeAbortController) {
      this.activeAbortController.abort();
      this.activeAbortController = undefined;
    }
    if (this.reconnectTimer !== undefined) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
  }

  private buildAuthHeaders(): Record<string, string> {
    if (this.config.authToken) {
      return { Authorization: `Bearer ${this.config.authToken}` };
    }
    return {};
  }

  private emitUpdate(update: Sep6StatusUpdate): void {
    for (const handler of this.updateHandlers) {
      try {
        handler(update);
      } catch {
        // Never let a handler crash the stream loop.
      }
    }
  }

  private emitClose(event: StreamCloseEvent): void {
    for (const handler of this.closeHandlers) {
      try {
        handler(event);
      } catch {
        // Never let a handler crash the stream loop.
      }
    }
  }

  private emitError(error: Error): void {
    for (const handler of this.errorHandlers) {
      try {
        handler(error);
      } catch {
        // Never let a handler crash.
      }
    }
  }
}
