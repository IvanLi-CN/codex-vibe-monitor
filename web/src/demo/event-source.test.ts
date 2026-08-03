/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from "vitest";

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

import { DemoTopicEventSource } from "./event-source";

describe("DemoTopicEventSource", () => {
  afterEach(() => {
    vi.useRealTimers();
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
});
