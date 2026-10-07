/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from "vitest";
import type { DemoRealtimePayload } from "./events";

const mocks = vi.hoisted(() => ({
  resolveDemoTopicPayload: vi.fn(),
  subscribeToDemoRealtime: vi.fn(),
}));

vi.mock("./topic-payloads", async () => {
  const actual = await vi.importActual<typeof import("./topic-payloads")>("./topic-payloads");
  return { ...actual, resolveDemoTopicPayload: mocks.resolveDemoTopicPayload };
});

vi.mock("./events", () => ({
  subscribeToDemoRealtime: mocks.subscribeToDemoRealtime,
}));

import { DemoTopicEventSource, isDemoTopicEventSourcePath } from "./event-source";

describe("DemoTopicEventSource", () => {
  afterEach(() => {
    vi.useRealTimers();
    window.history.replaceState({}, "", "/");
    mocks.resolveDemoTopicPayload.mockReset();
    mocks.subscribeToDemoRealtime.mockReset();
  });

  it("does not subscribe after close races the initial snapshot", async () => {
    vi.useFakeTimers();
    let resolvePayload: ((value: unknown) => void) | undefined;
    mocks.resolveDemoTopicPayload.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePayload = resolve;
        }),
    );
    const unsubscribe = vi.fn();
    mocks.subscribeToDemoRealtime.mockReturnValue(unsubscribe);
    const source = new DemoTopicEventSource("/events?topics=W3sidG9waWMiOiJhcHAudmVyc2lvbiJ9XQ");
    await vi.advanceTimersByTimeAsync(0);
    source.close();
    resolvePayload?.({ backend: "demo", frontend: "demo" });
    await Promise.resolve();
    await Promise.resolve();

    expect(mocks.subscribeToDemoRealtime).not.toHaveBeenCalled();
    expect(unsubscribe).not.toHaveBeenCalled();
  });

  it("reports an error when the initial topic snapshot cannot be resolved", async () => {
    vi.useFakeTimers();
    mocks.resolveDemoTopicPayload.mockRejectedValue(new Error("network unavailable"));
    const source = new DemoTopicEventSource("/events?topics=W3sidG9waWMiOiJhcHAudmVyc2lvbiJ9XQ");
    const events: string[] = [];
    source.addEventListener("open", () => events.push("open"));
    source.addEventListener("error", () => events.push("error"));

    await vi.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    expect(events).toEqual(["open", "error"]);
    expect(source.readyState).toBe(DemoTopicEventSource.CLOSED);
    expect(mocks.subscribeToDemoRealtime).not.toHaveBeenCalled();
  });

  it("supports deterministic connecting and post-snapshot disconnect states", async () => {
    vi.useFakeTimers();
    mocks.resolveDemoTopicPayload.mockResolvedValue({ backend: "demo" });
    const encodedTopics = btoa(JSON.stringify([{ topic: "app.version" }]));

    window.history.replaceState({}, "", "/#/system/tasks?demoSse=connecting");
    const connecting = new DemoTopicEventSource(`/events?attempt=1&topics=${encodedTopics}`);
    const connectingEvents: string[] = [];
    connecting.addEventListener("open", () => connectingEvents.push("open"));
    connecting.addEventListener("error", () => connectingEvents.push("error"));
    await vi.advanceTimersByTimeAsync(0);
    expect(connecting.readyState).toBe(DemoTopicEventSource.CONNECTING);
    expect(connectingEvents).toEqual([]);
    connecting.close();

    window.history.replaceState({}, "", "/#/system/tasks?demoSse=disconnect");
    const disconnected = new DemoTopicEventSource(`/events?attempt=8&topics=${encodedTopics}`);
    const disconnectedEvents: string[] = [];
    disconnected.addEventListener("open", () => disconnectedEvents.push("open"));
    disconnected.addEventListener("message", () => disconnectedEvents.push("snapshot"));
    disconnected.addEventListener("error", () => disconnectedEvents.push("error"));
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(900);
    expect(disconnectedEvents).toEqual(["open", "snapshot", "error"]);
    expect(disconnected.readyState).toBe(DemoTopicEventSource.CLOSED);

    const retry = new DemoTopicEventSource(`/events?attempt=9&topics=${encodedTopics}`);
    await vi.advanceTimersByTimeAsync(0);
    expect(retry.readyState).toBe(DemoTopicEventSource.CONNECTING);
    retry.close();
  });

  it("limits empty-record revisions to the dashboard activity topic", async () => {
    vi.useFakeTimers();
    mocks.resolveDemoTopicPayload.mockResolvedValue({ liveRevision: 1 });
    let publishRealtime: ((payload: DemoRealtimePayload) => void) | undefined;
    mocks.subscribeToDemoRealtime.mockImplementation((listener) => {
      publishRealtime = listener;
      return vi.fn();
    });
    const topics = [
      { topic: "dashboard.activity.current" },
      { topic: "stats.timeseries.open-window" },
    ];
    const encodedTopics = btoa(JSON.stringify(topics));
    const source = new DemoTopicEventSource(`/events?topics=${encodedTopics}`);
    const liveTopics: string[] = [];
    source.addEventListener("message", (event) => {
      const envelope = JSON.parse((event as MessageEvent<string>).data) as {
        type: string;
        topic: { topic: string };
      };
      if (envelope.type === "live") liveTopics.push(envelope.topic.topic);
    });

    await vi.advanceTimersByTimeAsync(0);
    publishRealtime?.({ type: "records", records: [] });
    await vi.advanceTimersByTimeAsync(0);

    expect(liveTopics).toEqual(["dashboard.activity.current"]);
    source.close();
  });

  it("publishes live task runtime and timeline events to both demo topics", async () => {
    vi.useFakeTimers();
    mocks.resolveDemoTopicPayload.mockImplementation(async (topic: { topic: string }) => ({
      observedAt: "2026-10-01T00:00:00.000Z",
      topic: topic.topic,
    }));
    let publishRealtime: ((payload: DemoRealtimePayload) => void) | undefined;
    mocks.subscribeToDemoRealtime.mockImplementation((listener) => {
      publishRealtime = listener;
      return vi.fn();
    });
    const topics = [
      { topic: "system.managed-tasks.runtime" },
      { topic: "system.managed-tasks.timeline", params: { schemaVersion: "2" } },
    ];
    const encodedTopics = btoa(JSON.stringify(topics));
    const source = new DemoTopicEventSource(`/events?topics=${encodedTopics}`);
    const messages: Array<{ type: string; topic: { topic: string } }> = [];
    source.addEventListener("message", (event) => {
      messages.push(JSON.parse((event as MessageEvent<string>).data));
    });

    await vi.advanceTimersByTimeAsync(0);
    expect(
      messages
        .filter((message) => message.type === "snapshot")
        .map((message) => message.topic.topic),
    ).toEqual(topics.map((topic) => topic.topic));

    publishRealtime?.({
      type: "records",
      records: [{ taskKey: "retention_archive" }],
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(
      messages.filter((message) => message.type === "live").map((message) => message.topic.topic),
    ).toEqual(topics.map((topic) => topic.topic));
    source.close();
  });

  it("matches the topic SSE endpoint beneath a deploy base", () => {
    expect(isDemoTopicEventSourcePath("/repo/demo/events?topics=abc", "/repo/demo/")).toBe(true);
    expect(isDemoTopicEventSourcePath("/events?topics=abc", "/repo/demo/")).toBe(false);
  });
});
