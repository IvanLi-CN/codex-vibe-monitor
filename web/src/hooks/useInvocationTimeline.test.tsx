/** @vitest-environment jsdom */

import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../lib/api";
import { useInvocationTimeline } from "./useInvocationTimeline";

const timelineMocks = vi.hoisted(() => ({
  fetch: vi.fn(),
}));

vi.mock("../lib/api", () => ({
  fetchInvocationTimeline: timelineMocks.fetch,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createTimeseries(rangeStart: string, rangeEnd: string): TimeseriesResponse {
  return {
    rangeStart,
    rangeEnd,
    bucketSeconds: 300,
    points: [],
  };
}

function createTimeline(invokeId: string): InvocationTimelineResponse {
  return {
    rangeStart: "2026-07-16T10:00:00.000Z",
    rangeEnd: "2026-07-16T10:30:00.000Z",
    asOf: "2026-07-16T10:30:00.000Z",
    total: 1,
    overLimit: false,
    records: [
      {
        id: 1,
        invokeId,
        occurredAt: "2026-07-16T10:15:00.000Z",
        endAt: "2026-07-16T10:16:00.000Z",
        isInFlight: false,
        status: "success",
        tTotalMs: 60_000,
      },
    ],
  };
}

function render(ui: React.ReactNode) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function Probe({ response }: { response: TimeseriesResponse }) {
  const { data } = useInvocationTimeline({
    response,
    closedNaturalDay: false,
  });
  return <output data-testid="invoke-id">{data?.records[0]?.invokeId ?? ""}</output>;
}

describe("useInvocationTimeline", () => {
  beforeEach(() => {
    timelineMocks.fetch.mockReset();
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
  });

  it("invalidates and aborts a pending request when the timeline context changes", async () => {
    const pending: Array<{
      resolve: (value: InvocationTimelineResponse) => void;
      signal?: AbortSignal;
    }> = [];
    timelineMocks.fetch.mockImplementation((_options: { signal?: AbortSignal }) => {
      return new Promise<InvocationTimelineResponse>((resolve) => {
        pending.push({ resolve, signal: _options.signal });
      });
    });

    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const secondResponse = createTimeseries("2026-07-17T10:00:00.000Z", "2026-07-17T10:30:00.000Z");

    render(<Probe response={firstResponse} />);
    const initialRequestCount = pending.length;
    expect(initialRequestCount).toBeGreaterThan(0);
    const firstSignal = pending[0]?.signal;

    act(() => {
      root?.render(<Probe response={secondResponse} />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(firstSignal?.aborted).toBe(true);
    expect(pending.length).toBeGreaterThan(initialRequestCount);
    for (const request of pending.slice(0, initialRequestCount)) {
      request.resolve(createTimeline("stale-invoke"));
    }
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("");

    pending.at(-1)?.resolve(createTimeline("current-invoke"));
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("current-invoke");
  });
});
