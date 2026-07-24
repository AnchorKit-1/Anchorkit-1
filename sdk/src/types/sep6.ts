/**
 * SEP-6 deposit/withdraw types and streaming types.
 *
 * SEP-6 spec: https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0006.md
 */

// ---------------------------------------------------------------------------
// Transaction status values defined in the SEP-6 spec
// ---------------------------------------------------------------------------

export type Sep6TransactionStatus =
  | 'incomplete'
  | 'pending_external'
  | 'pending_anchor'
  | 'pending_stellar'
  | 'pending_trust'
  | 'pending_user'
  | 'pending_user_transfer_start'
  | 'pending_customer_info_update'
  | 'pending_transaction_info_update'
  | 'completed'
  | 'refunded'
  | 'expired'
  | 'error'
  | 'no_market'
  | 'too_small'
  | 'too_large';

/** Statuses that are considered terminal (no further updates expected). */
export const TERMINAL_STATUSES: ReadonlySet<Sep6TransactionStatus> = new Set([
  'completed',
  'refunded',
  'expired',
  'error',
  'no_market',
  'too_small',
  'too_large',
]);

// ---------------------------------------------------------------------------
// Core transaction shape (subset of SEP-6 /transaction response)
// ---------------------------------------------------------------------------

export interface Sep6Transaction {
  id: string;
  kind: 'deposit' | 'withdrawal';
  status: Sep6TransactionStatus;
  status_eta?: number;
  /** ISO-8601 timestamp */
  started_at: string;
  /** ISO-8601 timestamp, present when terminal */
  completed_at?: string;
  message?: string;
  amount_in?: { amount: string; asset: string };
  amount_out?: { amount: string; asset: string };
  amount_fee?: { amount: string; asset: string };
  more_info_url?: string;
  required_info_message?: string;
  required_info_updates?: string[];
  stellar_transaction_id?: string;
  external_transaction_id?: string;
}

// ---------------------------------------------------------------------------
// Streaming configuration
// ---------------------------------------------------------------------------

/**
 * Configuration for a Sep6StreamingService instance.
 */
export interface Sep6StreamConfig {
  /** Anchor server base URL (no trailing slash). */
  anchorUrl: string;
  /** SEP-10 JWT token for authenticated requests. */
  token: string;
  /**
   * Maximum number of transactions to watch simultaneously.
   * Defaults to 50.
   */
  maxWatched?: number;
  /**
   * How long (ms) to wait before reconnecting after a dropped stream.
   * Defaults to 1 000 ms.  Doubles each failed attempt up to `maxReconnectDelay`.
   */
  reconnectDelay?: number;
  /**
   * Upper bound (ms) on reconnect back-off.
   * Defaults to 30 000 ms (30 s).
   */
  maxReconnectDelay?: number;
  /**
   * How often (ms) to poll when SSE is unavailable.
   * Defaults to 5 000 ms.
   */
  pollIntervalMs?: number;
  /**
   * Override whether the client should use SSE.
   * Auto-detected (presence of `EventSource` + anchor support) when omitted.
   */
  preferSse?: boolean;
  /** Abort signal that cancels all streams. */
  signal?: AbortSignal;
}

// ---------------------------------------------------------------------------
// Stream events emitted to callers
// ---------------------------------------------------------------------------

/**
 * A status update event delivered to subscribers.
 */
export interface TransactionStreamEvent {
  /** SEP-6 transaction id. */
  transactionId: string;
  /** Previous status (null on first delivery). */
  previousStatus: Sep6TransactionStatus | null;
  /** Current status. */
  status: Sep6TransactionStatus;
  /** Full transaction record at the time of this event. */
  transaction: Sep6Transaction;
  /** Whether this status is terminal (no further updates expected). */
  isTerminal: boolean;
  /** ISO-8601 timestamp of when the SDK produced this event. */
  receivedAt: string;
  /** Transport used for this event ('sse' | 'poll'). */
  transport: 'sse' | 'poll';
}

/**
 * An error event delivered to subscribers when the stream encounters
 * a non-fatal error (e.g. a failed reconnect attempt).
 */
export interface TransactionStreamError {
  transactionId: string;
  code: string;
  message: string;
  /** Whether the service will attempt to recover automatically. */
  recoverable: boolean;
}

// ---------------------------------------------------------------------------
// Subscription / handle returned to the caller
// ---------------------------------------------------------------------------

/**
 * A live subscription handle returned by `Sep6StreamingService.watch()`.
 * Call `unsubscribe()` to stop receiving updates for this transaction.
 */
export interface StreamHandle {
  /** The transaction id being watched. */
  transactionId: string;
  /** Stop watching this transaction and release resources. */
  unsubscribe: () => void;
}

// ---------------------------------------------------------------------------
// Internal per-transaction state (not exported as part of the public API)
// ---------------------------------------------------------------------------

export type StreamTransport = 'sse' | 'poll';

export interface WatcherState {
  transactionId: string;
  lastStatus: Sep6TransactionStatus | null;
  /**
   * `cursor` is the `updated_at` ISO string from the last seen transaction.
   * Kept so reconnected streams can request events newer than this timestamp,
   * preventing missed updates.
   */
  cursor: string | null;
  onEvent: (event: TransactionStreamEvent) => void;
  onError?: (error: TransactionStreamError) => void;
  active: boolean;
}

// ---------------------------------------------------------------------------
// Anchor capability detection
// ---------------------------------------------------------------------------

/**
 * Represents the anchor's declared streaming capabilities as discovered from
 * the `/info` endpoint or a HEAD request to the `/transaction` SSE endpoint.
 */
export interface AnchorStreamCapability {
  /** True if the anchor supports SSE on `/transaction?id=<id>&stream=true` */
  supportsSSE: boolean;
  /** True if the anchor supports `cursor` query param on `/transactions` for long-poll */
  supportsCursor: boolean;
}
