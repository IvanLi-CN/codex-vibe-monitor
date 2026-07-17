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
  it("rebuilds the connection with resume cursors from cached topic state", async () => {
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

    unsubscribeQuota();
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
