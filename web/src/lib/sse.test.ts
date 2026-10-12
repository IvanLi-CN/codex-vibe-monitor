/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const createEventSourceMock = vi.fn();

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  readonly listeners = new Map<string, Set<EventListener>>();
  readyState = FakeGlobalEventSource.CONNECTING;
  closed = false;

  constructor(readonly path: string) {
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const bucket = this.listeners.get(type) ?? new Set<EventListener>();
    bucket.add(listener);
    this.listeners.set(type, bucket);
  }

  removeEventListener(type: string, listener: EventListener) {
    this.listeners.get(type)?.delete(listener);
  }

  close() {
    this.closed = true;
    this.readyState = FakeGlobalEventSource.CLOSED;
  }

  emit(type: "open" | "error" | "message", event: Event | MessageEvent<string>) {
    if (type === "open") {
      this.readyState = FakeGlobalEventSource.OPEN;
    }
    if (type === "error") {
      this.readyState = FakeGlobalEventSource.CLOSED;
    }
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}

class FakeGlobalEventSource {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;
}

function decodeBase64UrlJson<T>(value: string): T {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  return JSON.parse(atob(padded)) as T;
}

function decodePath(path: string) {
  const url = new URL(path, "http://localhost");
  const rawTopics = url.searchParams.get("topics");
  const rawResume = url.searchParams.get("resume");
  return {
    topics: rawTopics
      ? decodeBase64UrlJson<Array<{ topic: string; params?: Record<string, string> }>>(rawTopics)
      : [],
    resume: rawResume
      ? decodeBase64UrlJson<Array<{ topicIndex: number; cursor: number; schemaEpoch: string }>>(
          rawResume,
        )
      : [],
    attempt: url.searchParams.get("attempt"),
    reason: url.searchParams.get("reason"),
    delta: url.searchParams.get("delta"),
  };
}

async function loadSseModule() {
  vi.resetModules();
  vi.doMock("./api", () => ({
    createEventSource: createEventSourceMock,
  }));
  return import("./sse");
}

beforeEach(() => {
  vi.useFakeTimers();
  createEventSourceMock.mockReset();
  FakeEventSource.instances = [];
  createEventSourceMock.mockImplementation((path: string) => new FakeEventSource(path));
  vi.stubGlobal("EventSource", FakeGlobalEventSource);
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.resetModules();
  document.body.innerHTML = "";
});

describe("sse topic registry", () => {
  it("merges a timeseries point delta into the cached snapshot", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const listener = vi.fn();
    const unsubscribe = sse.subscribeToTopic(topic, listener);
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 7,
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
            pointIndex: 0,
            point: { total: 2 },
          },
        }),
      }),
    );

    expect(listener).toHaveBeenLastCalledWith(
      expect.objectContaining({
        type: "live",
        delivery: "delta",
        baseCursor: 7,
        cursor: 8,
        payload: {
          rangeEnd: "2026-10-12T00:02:00Z",
          snapshotId: 11,
          points: [{ total: 2 }],
        },
      }),
    );
    expect(sse.getCachedTopicState(topic)).toMatchObject({
      cursor: 8,
      payload: {
        rangeEnd: "2026-10-12T00:02:00Z",
        snapshotId: 11,
        points: [{ total: 2 }],
      },
    });
    unsubscribe();
  });

  it("merges a metadata-only timeseries delta without changing points", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const listener = vi.fn();
    const unsubscribe = sse.subscribeToTopic(topic, listener);
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 7,
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
            pointIndex: null,
            point: null,
          },
        }),
      }),
    );

    expect(sse.getCachedTopicState(topic)?.payload).toEqual({
      rangeEnd: "2026-10-12T00:02:00Z",
      snapshotId: 11,
      points: [{ total: 1 }],
    });
    unsubscribe();
  });

  it("merges a working-conversations item delta into the cached collection", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("dashboard.working-conversations.current");
    const listener = vi.fn();
    const unsubscribe = sse.subscribeToTopic(topic, listener);
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "dashboard-working-conversations-current",
          schemaEpoch: "dashboard.working-conversations.current/v1",
          cursor: 3,
          payload: {
            hasMore: false,
            conversations: [{ promptCacheKey: "alpha", requestCount: 1 }],
          },
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 3,
          topic,
          topicKey: "dashboard-working-conversations-current",
          schemaEpoch: "dashboard.working-conversations.current/v1",
          cursor: 4,
          payload: {
            kind: "collection-item/v1",
            collection: "conversations",
            itemIndex: 0,
            item: { promptCacheKey: "alpha", requestCount: 2 },
            fields: { hasMore: true },
          },
        }),
      }),
    );

    expect(sse.getCachedTopicState(topic)?.payload).toEqual({
      hasMore: true,
      conversations: [{ promptCacheKey: "alpha", requestCount: 2 }],
    });
    unsubscribe();
  });

  it("forces a full snapshot for malformed or unsupported delta envelopes", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const unsubscribe = sse.subscribeToTopic(topic, vi.fn());
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );

    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
            pointIndex: null,
            point: null,
          },
        }),
      }),
    );
    vi.advanceTimersByTime(0);

    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    expect(sse.getCachedTopicState(topic)?.cursor).toBe(7);
    expect(sse.getCurrentSseDiagnostics().forcedSnapshotTopics).toEqual([
      "stats.timeseries.open-window?range=today",
    ]);

    unsubscribe();
  });

  it("rejects fractional indexes and unsupported collection deltas", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const unsubscribe = sse.subscribeToTopic(topic, vi.fn());
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 7,
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
            pointIndex: 0.5,
            point: { total: 2 },
          },
        }),
      }),
    );
    vi.advanceTimersByTime(0);

    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    expect(sse.getCachedTopicState(topic)?.cursor).toBe(7);
    unsubscribe();
  });

  it("rejects omitted delta fields and unsafe cursors", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const unsubscribe = sse.subscribeToTopic(topic, vi.fn());
    const connection = FakeEventSource.instances[0]!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 7,
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
          },
        }),
      }),
    );
    vi.advanceTimersByTime(0);
    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    expect(sse.getCachedTopicState(topic)?.cursor).toBe(7);

    const reconnect = FakeEventSource.instances[1]!;
    reconnect.emit("open", new Event("open"));
    reconnect.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: -1,
          payload: { rangeEnd: "bad", snapshotId: 12, points: [] },
        }),
      }),
    );
    expect(sse.getCachedTopicState(topic)?.cursor).toBe(7);
    unsubscribe();
  });

  it("rejects invalid payload and delta value shapes before caching", async () => {
    const sse = await loadSseModule();
    const timeseries = sse.buildTopicDescriptor("stats.timeseries.open-window", { range: "today" });
    const unsubscribeTimeseries = sse.subscribeToTopic(timeseries, vi.fn());
    const firstConnection = FakeEventSource.instances[0]!;
    firstConnection.emit("open", new Event("open"));
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: timeseries,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 7,
          payload: {
            rangeEnd: "2026-10-12T00:01:00Z",
            snapshotId: 10,
            points: [{ total: 1 }],
          },
        }),
      }),
    );
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "live",
          delivery: "delta",
          baseCursor: 7,
          topic: timeseries,
          topicKey: "stats-timeseries-open-window",
          schemaEpoch: "stats.timeseries.open-window/v1",
          cursor: 8,
          payload: {
            kind: "timeseries-point/v1",
            rangeEnd: "2026-10-12T00:02:00Z",
            snapshotId: 11,
            pointIndex: 0,
            point: "invalid",
          },
        }),
      }),
    );
    vi.advanceTimersByTime(0);
    expect(sse.getCachedTopicState(timeseries)?.cursor).toBe(7);

    const ordinary = sse.buildTopicDescriptor("forward-proxy.live");
    const unsubscribeOrdinary = sse.subscribeToTopic(ordinary, vi.fn());
    const secondConnection = FakeEventSource.instances.at(-1)!;
    secondConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: ordinary,
          topicKey: "forward-proxy-live",
          schemaEpoch: "forward-proxy.live/v1",
          cursor: 1,
        }),
      }),
    );
    vi.advanceTimersByTime(0);
    expect(sse.getCachedTopicState(ordinary)).toBeNull();

    unsubscribeOrdinary();
    unsubscribeTimeseries();
  });

  it("keeps an unavailable topic isolated from healthy topic snapshots", async () => {
    const sse = await loadSseModule();
    const summaryTopic = sse.buildTopicDescriptor("stats.summary.current", {
      window: "current",
    });
    const activityTopic = sse.buildTopicDescriptor("dashboard.activity.current", {
      range: "today",
    });
    const summaryListener = vi.fn();
    const activityListener = vi.fn();

    const unsubscribeSummary = sse.subscribeToTopic(summaryTopic, summaryListener);
    const unsubscribeActivity = sse.subscribeToTopic(activityTopic, activityListener);
    const connection = FakeEventSource.instances.at(-1)!;
    connection.emit("open", new Event("open"));
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "unavailable",
          topic: summaryTopic,
          topicKey: "summary-current",
          schemaEpoch: "stats.summary.current/v1",
          errorCode: "unavailable",
        }),
      }),
    );
    connection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: activityTopic,
          topicKey: "dashboard-activity-current",
          schemaEpoch: "dashboard.activity.current/v3",
          cursor: 3,
          payload: { range: "today" },
        }),
      }),
    );

    expect(summaryListener).toHaveBeenCalledWith(
      expect.objectContaining({ type: "unavailable", errorCode: "unavailable" }),
    );
    expect(activityListener).toHaveBeenCalledWith(
      expect.objectContaining({ type: "snapshot", payload: { range: "today" } }),
    );
    expect(sse.getCachedTopicState(summaryTopic)).toMatchObject({
      payload: null,
      cursor: null,
      error: "unavailable",
    });
    expect(sse.getCachedTopicState(activityTopic)).toMatchObject({
      payload: { range: "today" },
      cursor: 3,
      error: null,
    });

    unsubscribeActivity();
    unsubscribeSummary();
  });

  it("rebuilds the connection when a topic is added or removed and preserves remaining cursors", async () => {
    const sse = await loadSseModule();
    const summaryTopic = sse.buildTopicDescriptor("stats.summary.current", {
      limit: 20,
      window: "current",
    });

    const received: Array<{ payload: { total: number } }> = [];
    const unsubscribeSummary = sse.subscribeToTopic(summaryTopic, (event) => {
      received.push(event as { payload: { total: number } });
    });

    expect(createEventSourceMock).toHaveBeenCalledTimes(1);
    const firstConnection = FakeEventSource.instances[0];
    firstConnection.emit("open", new Event("open"));
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: summaryTopic,
          topic_key: "summary-current",
          schema_epoch: "stats.summary.current/v1",
          cursor: 4,
          payload: { total: 7 },
        }),
      }),
    );

    expect(received).toHaveLength(1);
    expect(received[0]?.payload.total).toBe(7);

    const quotaTopic = sse.buildTopicDescriptor("quota.current");
    const unsubscribeQuota = sse.subscribeToTopic(quotaTopic, vi.fn());

    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    const secondConnection = FakeEventSource.instances[1];
    const decoded = decodePath(createEventSourceMock.mock.calls[1][0] as string);
    expect(decoded.topics).toEqual([
      { topic: "quota.current", params: {} },
      { topic: "stats.summary.current", params: { limit: "20", window: "current" } },
    ]);
    expect(decoded.resume).toEqual([
      {
        topicIndex: 1,
        cursor: 4,
        schemaEpoch: "stats.summary.current/v1",
      },
    ]);
    expect(decoded.delta).toBe("1");

    unsubscribeQuota();

    expect(secondConnection.closed).toBe(true);
    expect(createEventSourceMock).toHaveBeenCalledTimes(3);
    const afterRemoval = decodePath(createEventSourceMock.mock.calls[2][0] as string);
    expect(afterRemoval.topics).toEqual([
      { topic: "stats.summary.current", params: { limit: "20", window: "current" } },
    ]);
    expect(afterRemoval.resume).toEqual([
      {
        topicIndex: 0,
        cursor: 4,
        schemaEpoch: "stats.summary.current/v1",
      },
    ]);

    unsubscribeSummary();
  });

  it("replays cached payloads to late subscribers and forces a fresh snapshot on manual refresh", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("forward-proxy.live");
    const firstListener = vi.fn();

    const unsubscribeFirst = sse.subscribeToTopic(topic, firstListener);
    expect(createEventSourceMock).toHaveBeenCalledTimes(1);

    const firstConnection = FakeEventSource.instances[0];
    firstConnection.emit("open", new Event("open"));
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "forward-proxy-live",
          schemaEpoch: "forward-proxy.live/v1",
          cursor: 9,
          payload: { activeRequests: 3 },
        }),
      }),
    );

    const secondListener = vi.fn();
    const unsubscribeSecond = sse.subscribeToTopic(topic, secondListener);

    expect(createEventSourceMock).toHaveBeenCalledTimes(1);
    expect(secondListener).toHaveBeenCalledTimes(1);
    expect(secondListener.mock.calls[0]?.[0]).toMatchObject({
      type: "snapshot",
      cursor: 9,
      payload: { activeRequests: 3 },
      deliverySource: "cache",
    });

    sse.requestTopicRefresh(topic);
    vi.advanceTimersByTime(0);

    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    const rebuilt = decodePath(createEventSourceMock.mock.calls[1][0] as string);
    expect(rebuilt.topics).toEqual([{ topic: "forward-proxy.live", params: {} }]);
    expect(rebuilt.resume).toEqual([]);

    unsubscribeSecond();
    unsubscribeFirst();
  });

  it("forces fresh snapshots for every active topic on manual reconnect and records diagnostics", async () => {
    const sse = await loadSseModule();
    const summaryTopic = sse.buildTopicDescriptor("stats.summary.current", {
      limit: 20,
      window: "current",
    });
    const quotaTopic = sse.buildTopicDescriptor("quota.current");

    const unsubscribeSummary = sse.subscribeToTopic(summaryTopic, vi.fn());
    const firstConnection = FakeEventSource.instances[0];
    firstConnection.emit("open", new Event("open"));
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: summaryTopic,
          topicKey: "summary-current",
          schemaEpoch: "stats.summary.current/v1",
          cursor: 4,
          payload: { total: 7 },
        }),
      }),
    );

    const unsubscribeQuota = sse.subscribeToTopic(quotaTopic, vi.fn());
    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    const secondConnection = FakeEventSource.instances[1];
    secondConnection.emit("open", new Event("open"));
    secondConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic: quotaTopic,
          topicKey: "quota-current",
          schemaEpoch: "quota.current/v1",
          cursor: 8,
          payload: { usd: 17 },
        }),
      }),
    );

    sse.requestImmediateReconnect();
    vi.advanceTimersByTime(0);

    expect(createEventSourceMock).toHaveBeenCalledTimes(3);
    const rebuilt = decodePath(createEventSourceMock.mock.calls[2][0] as string);
    expect(rebuilt.resume).toEqual([]);
    expect(rebuilt.reason).toBe("manual");
    expect(rebuilt.attempt).toBe("3");
    expect(sse.getCurrentSseDiagnostics()).toMatchObject({
      attempt: 3,
      reason: "manual",
      activeTopics: ["quota.current", "stats.summary.current?limit=20&window=current"],
      resumeTopics: [],
      forcedSnapshotTopics: ["quota.current", "stats.summary.current?limit=20&window=current"],
    });

    unsubscribeQuota();
    unsubscribeSummary();
  });

  it("preserves resume cursors on automatic reconnect after an eventsource error with backoff", async () => {
    const sse = await loadSseModule();
    const topic = sse.buildTopicDescriptor("forward-proxy.live");
    const unsubscribe = sse.subscribeToTopic(topic, vi.fn());

    const firstConnection = FakeEventSource.instances[0];
    firstConnection.emit("open", new Event("open"));
    firstConnection.emit(
      "message",
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "snapshot",
          topic,
          topicKey: "forward-proxy-live",
          schemaEpoch: "forward-proxy.live/v1",
          cursor: 9,
          payload: { activeRequests: 3 },
        }),
      }),
    );

    firstConnection.emit("error", new Event("error"));
    expect(createEventSourceMock).toHaveBeenCalledTimes(1);

    const retryAt = sse.getCurrentSseStatus().nextRetryAt;
    expect(retryAt).not.toBeNull();
    expect(sse.getCurrentSseStatus().phase).toBe("reconnecting");

    vi.advanceTimersByTime(1_999);
    expect(createEventSourceMock).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(1);

    expect(createEventSourceMock).toHaveBeenCalledTimes(2);
    const rebuilt = decodePath(createEventSourceMock.mock.calls[1][0] as string);
    expect(rebuilt.reason).toBe("eventsource-error");
    expect(rebuilt.resume).toEqual([
      {
        topicIndex: 0,
        cursor: 9,
        schemaEpoch: "forward-proxy.live/v1",
      },
    ]);
    expect(sse.getCurrentSseDiagnostics()).toMatchObject({
      attempt: 2,
      reason: "eventsource-error",
      resumeTopics: ["forward-proxy.live"],
      lastTerminalOutcome: "eventsource-error",
    });

    unsubscribe();
  });
});
