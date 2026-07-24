/**
 * Integration tests for Sep6StreamingService
 *
 * Acceptance criteria covered:
 *   ✓ AC1 – Falls back gracefully to polling if the anchor doesn't support streaming
 *   ✓ AC2 – Handles reconnection after a dropped stream without duplicate/missed updates
 *   ✓ AC3 – Integration test simulates a status transition arriving over the stream
 *
 * We use vitest's fake-timer and fetch/EventSource mocking so there is no
 * real network I/O.  The tests are deterministic and run in-process.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Sep6StreamingService } from '../sep6Streaming';
import type {
  Sep6Transaction,
  Sep6TransactionStatus,
  TransactionStreamEvent,
  TransactionStreamError,
} from '../../types/sep6';

// ---------------------------------------------------------------------------
// Flush the microtask queue without relying on vi.runAllMicrotasksAsync
// (added in vitest 2.2; not available in 2.1.x).
// ---------------------------------------------------------------------------
function flushPromises(): Promise<void> {
  return new Promise<void>((resolve) => {
    // Two ticks: first resolves any immediately-queued promises,
    // second resolves callbacks that were enqueued by those.
    Promise.resolve().then(() => Promise.resolve().then(resolve));
  });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeTx(
  id: string,
  status: Sep6TransactionStatus,
  overrides: Partial<Sep6Transaction> = {}
): Sep6Transaction {
  return {
    id,
    kind: 'deposit',
    status,
    started_at: '2024-01-01T00:00:00Z',
    ...overrides,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function sseHeadResponse(): Response {
  return new Response(null, {
    status: 200,
    headers: { 'Content-Type': 'text/event-stream' },
  });
}

// ---------------------------------------------------------------------------
// Fake EventSource
// ---------------------------------------------------------------------------

interface FakeESInstance {
  url: string;
  listeners: Map<string, ((ev: MessageEvent) => void)[]>;
  emit: (type: string, data: string) => void;
  drop: () => void;
  close: () => void;
  closed: boolean;
}

const eventSourceInstances: FakeESInstance[] = [];

function createFakeEventSourceClass(): typeof EventSource {
  class FakeEventSource {
    url: string;
    readyState = 1;
    onmessage: ((ev: MessageEvent) => void) | null = null;
    onerror: ((ev: Event) => void) | null = null;
    listeners: Map<string, ((ev: MessageEvent) => void)[]> = new Map();
    closed = false;

    constructor(url: string) {
      this.url = url;
      const self = this as unknown as FakeESInstance;
      self.emit = (type: string, data: string) => {
        const msg = new MessageEvent(type, { data });
        if (type === 'message' && this.onmessage) this.onmessage(msg);
        const handlers = this.listeners.get(type) ?? [];
        handlers.forEach((h) => h(msg));
      };
      self.drop = () => {
        this.readyState = 2;
        const ev = new Event('error');
        if (this.onerror) this.onerror(ev);
        (this.listeners.get('error') ?? []).forEach(
          (h) => h(ev as unknown as MessageEvent)
        );
      };
      eventSourceInstances.push(self);
    }

    addEventListener(type: string, handler: (ev: MessageEvent) => void) {
      if (!this.listeners.has(type)) this.listeners.set(type, []);
      this.listeners.get(type)!.push(handler);
    }

    removeEventListener(type: string, handler: (ev: MessageEvent) => void) {
      const list = this.listeners.get(type);
      if (list) this.listeners.set(type, list.filter((h) => h !== handler));
    }

    close() {
      this.closed = true;
      this.readyState = 2;
    }
  }
  return FakeEventSource as unknown as typeof EventSource;
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('Sep6StreamingService', () => {
  beforeEach(() => {
    eventSourceInstances.length = 0;
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  // ─────────────────────────────────────────────────────────────────────────
  // AC3 – Status transition arriving over an SSE stream
  // ─────────────────────────────────────────────────────────────────────────

  describe('AC3 – SSE status transition', () => {
    it('delivers a status update when SSE emits a transaction event', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
      });

      const received: TransactionStreamEvent[] = [];
      const gotFirst = new Promise<void>((resolve) => {
        svc.watch('tx-001', (e) => {
          received.push(e);
          resolve();
        });
      });

      // Flush the capability detection promise and EventSource construction.
      await flushPromises();

      expect(eventSourceInstances).toHaveLength(1);
      const es = eventSourceInstances[0] as FakeESInstance;

      const pendingTx = makeTx('tx-001', 'pending_anchor');
      es.emit('transaction', JSON.stringify({ transaction: pendingTx }));

      await gotFirst;

      expect(received).toHaveLength(1);
      expect(received[0].status).toBe('pending_anchor');
      expect(received[0].previousStatus).toBeNull();
      expect(received[0].transport).toBe('sse');
      expect(received[0].isTerminal).toBe(false);

      svc.destroy();
    });

    it('fires separate events for each distinct status change', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
      });

      const received: TransactionStreamEvent[] = [];
      const gotTwo = new Promise<void>((resolve) => {
        svc.watch('tx-002', (e) => {
          received.push(e);
          if (received.length >= 2) resolve();
        });
      });

      await flushPromises();

      const es = eventSourceInstances[0] as FakeESInstance;
      es.emit('transaction', JSON.stringify(makeTx('tx-002', 'pending_anchor')));
      es.emit('transaction', JSON.stringify(makeTx('tx-002', 'pending_stellar')));

      await gotTwo;

      expect(received[0].status).toBe('pending_anchor');
      expect(received[1].status).toBe('pending_stellar');
      expect(received[1].previousStatus).toBe('pending_anchor');

      svc.destroy();
    });

    it('does NOT fire duplicate events when SSE repeats the same status', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
      });

      const received: TransactionStreamEvent[] = [];
      svc.watch('tx-003', (e) => received.push(e));

      await flushPromises();

      const es = eventSourceInstances[0] as FakeESInstance;
      es.emit('transaction', JSON.stringify(makeTx('tx-003', 'pending_anchor')));
      es.emit('transaction', JSON.stringify(makeTx('tx-003', 'pending_anchor'))); // duplicate

      await flushPromises();
      expect(received).toHaveLength(1);

      svc.destroy();
    });

    it('auto-unsubscribes and stops emitting after a terminal status', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
      });

      const received: TransactionStreamEvent[] = [];
      svc.watch('tx-004', (e) => received.push(e));

      await flushPromises();

      const es = eventSourceInstances[0] as FakeESInstance;
      es.emit(
        'transaction',
        JSON.stringify(makeTx('tx-004', 'completed', { completed_at: '2024-01-02T00:00:00Z' }))
      );
      // Send another event after terminal — should be ignored because the watcher was removed.
      es.emit('transaction', JSON.stringify(makeTx('tx-004', 'pending_anchor')));

      await flushPromises();
      expect(received).toHaveLength(1);
      expect(received[0].isTerminal).toBe(true);

      svc.destroy();
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // AC1 – Fallback to polling when anchor doesn't support SSE
  // ─────────────────────────────────────────────────────────────────────────

  describe('AC1 – Long-poll fallback', () => {
    it('uses polling when EventSource is not available', async () => {
      vi.stubGlobal('EventSource', undefined);

      let callCount = 0;
      const txStatuses: Sep6TransactionStatus[] = [
        'pending_external',
        'pending_stellar',
        'completed',
      ];

      vi.stubGlobal(
        'fetch',
        vi.fn(async () => {
          const status = txStatuses[Math.min(callCount++, txStatuses.length - 1)];
          return jsonResponse({
            transaction: makeTx('tx-100', status, {
              completed_at: status === 'completed' ? '2024-01-02T00:00:00Z' : undefined,
            }),
          });
        })
      );

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        pollIntervalMs: 100,
        preferSse: false,
      });

      const received: TransactionStreamEvent[] = [];
      const done = new Promise<void>((resolve) => {
        svc.watch('tx-100', (e) => {
          received.push(e);
          if (e.isTerminal) resolve();
        });
      });

      // Drive several poll cycles.
      for (let i = 0; i < 5; i++) {
        await flushPromises();
        await vi.advanceTimersByTimeAsync(150);
      }
      await flushPromises();

      await done;

      const statuses = received.map((e) => e.status);
      expect(statuses).toContain('pending_external');
      expect(statuses).toContain('pending_stellar');
      expect(statuses).toContain('completed');
      expect(received.every((e) => e.transport === 'poll')).toBe(true);
      expect(received[received.length - 1].isTerminal).toBe(true);

      svc.destroy();
    });

    it('reports a non-fatal error when a poll request fails, then keeps polling', async () => {
      vi.stubGlobal('EventSource', undefined);

      let attempt = 0;
      vi.stubGlobal(
        'fetch',
        vi.fn(async () => {
          attempt++;
          if (attempt === 1) throw new Error('network error');
          return jsonResponse({ transaction: makeTx('tx-101', 'pending_external') });
        })
      );

      const errors: TransactionStreamError[] = [];
      const events: TransactionStreamEvent[] = [];

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        pollIntervalMs: 50,
        preferSse: false,
      });

      const gotEvent = new Promise<void>((resolve) => {
        svc.watch(
          'tx-101',
          (e) => { events.push(e); resolve(); },
          (err) => errors.push(err)
        );
      });

      for (let i = 0; i < 5; i++) {
        await flushPromises();
        await vi.advanceTimersByTimeAsync(100);
      }
      await flushPromises();

      await gotEvent;

      expect(errors).toHaveLength(1);
      expect(errors[0].code).toBe('POLL_ERROR');
      expect(errors[0].recoverable).toBe(true);
      expect(events.length).toBeGreaterThanOrEqual(1);
      expect(events[0].status).toBe('pending_external');

      svc.destroy();
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // AC2 – Reconnection without duplicates or missed updates
  // ─────────────────────────────────────────────────────────────────────────

  describe('AC2 – Reconnection without duplicates or missed updates', () => {
    it('reconnects SSE after a drop and does NOT re-emit the already-seen status', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
        reconnectDelay: 10,
      });

      const received: TransactionStreamEvent[] = [];
      svc.watch('tx-200', (e) => received.push(e));

      await flushPromises();

      // First connection: anchor sends a status.
      const es1 = eventSourceInstances[0] as FakeESInstance;
      es1.emit('transaction', JSON.stringify(makeTx('tx-200', 'pending_anchor')));
      await flushPromises();
      expect(received).toHaveLength(1);

      // Network drops the stream.
      es1.drop();
      await vi.advanceTimersByTimeAsync(50);
      await flushPromises();

      // Second EventSource should have opened.
      expect(eventSourceInstances).toHaveLength(2);
      const es2 = eventSourceInstances[1] as FakeESInstance;

      // Anchor re-sends the SAME status (reconnect overlap) — must NOT produce a duplicate.
      es2.emit('transaction', JSON.stringify(makeTx('tx-200', 'pending_anchor')));
      await flushPromises();
      expect(received).toHaveLength(1); // still 1

      // NEW status arrives — must NOT be missed.
      es2.emit('transaction', JSON.stringify(makeTx('tx-200', 'pending_stellar')));
      await flushPromises();
      expect(received).toHaveLength(2);
      expect(received[1].status).toBe('pending_stellar');

      svc.destroy();
    });

    it('reconnects with an exponentially increasing back-off delay', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
        reconnectDelay: 100,
        maxReconnectDelay: 1_000,
      });

      svc.watch('tx-201', () => {});
      await flushPromises();

      const es1 = eventSourceInstances[0] as FakeESInstance;
      es1.drop();

      // 99 ms — reconnect not yet due.
      await vi.advanceTimersByTimeAsync(99);
      await flushPromises();
      expect(eventSourceInstances).toHaveLength(1);

      // 1 ms more — reconnect fires.
      await vi.advanceTimersByTimeAsync(1);
      await flushPromises();
      expect(eventSourceInstances).toHaveLength(2);

      // Drop again; back-off doubles to ~200 ms.
      (eventSourceInstances[1] as FakeESInstance).drop();
      await vi.advanceTimersByTimeAsync(199);
      await flushPromises();
      expect(eventSourceInstances).toHaveLength(2);

      await vi.advanceTimersByTimeAsync(1);
      await flushPromises();
      expect(eventSourceInstances).toHaveLength(3);

      svc.destroy();
    });

    it('includes the cursor URL param on reconnected SSE requests', async () => {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseHeadResponse()));
      vi.stubGlobal('EventSource', createFakeEventSourceClass());

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        preferSse: true,
        reconnectDelay: 10,
      });

      svc.watch('tx-202', () => {});
      await flushPromises();

      const es1 = eventSourceInstances[0] as FakeESInstance;
      // Emit an event to set the cursor.
      es1.emit(
        'transaction',
        JSON.stringify(makeTx('tx-202', 'pending_anchor', { started_at: '2024-06-01T10:00:00Z' }))
      );
      await flushPromises();

      // Drop and reconnect.
      es1.drop();
      await vi.advanceTimersByTimeAsync(50);
      await flushPromises();

      expect(eventSourceInstances).toHaveLength(2);
      expect((eventSourceInstances[1] as FakeESInstance).url).toContain('cursor=');

      svc.destroy();
    });

    it('stops polling after unsubscribe', async () => {
      vi.stubGlobal('EventSource', undefined);

      const fetchMock = vi.fn(async () =>
        jsonResponse({ transaction: makeTx('tx-300', 'pending_external') })
      );
      vi.stubGlobal('fetch', fetchMock);

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        pollIntervalMs: 50,
        preferSse: false,
      });

      const handle = svc.watch('tx-300', () => {});
      await flushPromises();
      await vi.advanceTimersByTimeAsync(60);
      await flushPromises();

      const countBefore = fetchMock.mock.calls.length;
      handle.unsubscribe();

      await vi.advanceTimersByTimeAsync(200);
      await flushPromises();

      // No additional calls after unsubscribe.
      expect(fetchMock.mock.calls.length).toBe(countBefore);

      svc.destroy();
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Edge cases
  // ─────────────────────────────────────────────────────────────────────────

  describe('edge cases', () => {
    it('throws when maxWatched limit is exceeded', () => {
      vi.stubGlobal('EventSource', undefined);
      vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 200 })));

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        maxWatched: 2,
        preferSse: false,
      });

      svc.watch('tx-a', () => {});
      svc.watch('tx-b', () => {});
      expect(() => svc.watch('tx-c', () => {})).toThrow('maxWatched limit');

      svc.destroy();
    });

    it('destroy() stops all watchers immediately', async () => {
      vi.stubGlobal('EventSource', undefined);

      const fetchMock = vi.fn(async () =>
        jsonResponse({ transaction: makeTx('tx-x', 'pending_external') })
      );
      vi.stubGlobal('fetch', fetchMock);

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        pollIntervalMs: 50,
        preferSse: false,
      });

      svc.watch('tx-x', () => {});
      svc.watch('tx-y', () => {});
      svc.destroy();

      const countAfterDestroy = fetchMock.mock.calls.length;
      await vi.advanceTimersByTimeAsync(500);
      await flushPromises();

      expect(fetchMock.mock.calls.length).toBe(countAfterDestroy);
    });

    it('handles anchors that return the transaction at the top level (no wrapper)', async () => {
      vi.stubGlobal('EventSource', undefined);

      vi.stubGlobal(
        'fetch',
        vi.fn(async () =>
          // Anchor returns { id, status, ... } directly — no { transaction: ... } wrapper.
          jsonResponse(makeTx('tx-flat', 'completed', { completed_at: '2024-01-02T00:00:00Z' }))
        )
      );

      const svc = new Sep6StreamingService({
        anchorUrl: 'https://anchor.example.com',
        token: 'test-jwt',
        pollIntervalMs: 50,
        preferSse: false,
      });

      const received: TransactionStreamEvent[] = [];
      const done = new Promise<void>((resolve) => {
        svc.watch('tx-flat', (e) => {
          received.push(e);
          if (e.isTerminal) resolve();
        });
      });

      for (let i = 0; i < 3; i++) {
        await flushPromises();
        await vi.advanceTimersByTimeAsync(60);
      }
      await flushPromises();

      await done;
      expect(received[0].status).toBe('completed');

      svc.destroy();
    });
  });
});
