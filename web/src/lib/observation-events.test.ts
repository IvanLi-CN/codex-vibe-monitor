/** @vitest-environment jsdom */
import { describe, expect, it, vi } from "vitest";
import {
  configureBrowserObservation,
  flushBrowserObservations,
  observeBrowser,
  observeSseLifecycle,
} from "./observation-events";

describe("bounded browser observations", () => {
  it("drops failed batches, bounds samples and does not retry within a minute", async () => {
    configureBrowserObservation("dashboard", false);
    configureBrowserObservation("dashboard", true);
    for (let n = 0; n < 1000; n++) observeBrowser("api_request", 0.01);
    observeBrowser("data_ready", 0.2);
    const send = vi.fn().mockRejectedValue(new Error("rejected"));
    await flushBrowserObservations(send, 100_000);
    expect(send).toHaveBeenCalledTimes(1);
    const events = send.mock.calls[0][0];
    expect(events.length).toBeLessThanOrEqual(8);
    expect(new TextEncoder().encode(JSON.stringify({ events })).byteLength).toBeLessThanOrEqual(
      2048,
    );
    expect(events.some((event: { kind: string }) => event.kind === "data_ready")).toBe(true);
    await flushBrowserObservations(send, 110_000);
    expect(send).toHaveBeenCalledTimes(1);
    await flushBrowserObservations(send, 160_000);
    expect(send.mock.calls[1][0].some((event: { kind: string }) => event.kind === "dropped")).toBe(
      true,
    );
  });
  it("records actual SSE open/error/close intervals once", async () => {
    configureBrowserObservation("system", false);
    configureBrowserObservation("system", true);
    const source = new EventTarget();
    const stop = observeSseLifecycle(source as EventSource);
    source.dispatchEvent(new Event("open"));
    source.dispatchEvent(new Event("error"));
    source.dispatchEvent(new Event("error"));
    source.dispatchEvent(new Event("open"));
    stop();
    stop();
    const send = vi.fn().mockResolvedValue(undefined);
    await flushBrowserObservations(send, 220_000);
    expect(
      send.mock.calls[0][0]
        .filter((event: { kind: string }) => event.kind === "sse")
        .map((event: { outcome: string }) => event.outcome),
    ).toEqual(["error", "normal"]);
  });
});
