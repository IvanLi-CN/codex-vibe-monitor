/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { TimeseriesResponse } from "../lib/api";
import { clearTimeseriesRemountCache, useTimeseries } from "./useTimeseries";

const apiMocks = vi.hoisted(() => ({
  fetchTimeseries: vi.fn<() => Promise<TimeseriesResponse>>(),
}));

const topicMocks = vi.hoisted(() => ({
  state: {
    data: null as TimeseriesResponse | null,
    isLoading: false,
    error: null as string | null,
    refresh: vi.fn(),
  },
  lastDescriptor: null as Record<string, unknown> | null,
  lastEnabled: true,
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchTimeseries: apiMocks.fetchTimeseries,
  };
});

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: Record<string, unknown> | null, enabled = true) => {
    topicMocks.lastDescriptor = descriptor;
    topicMocks.lastEnabled = enabled;
    return topicMocks.state;
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
  topicMocks.state.refresh.mockReset();
  topicMocks.lastDescriptor = null;
  topicMocks.lastEnabled = true;
  apiMocks.fetchTimeseries.mockReset();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  clearTimeseriesRemountCache();
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function createTimeseries(points = 2): TimeseriesResponse {
  return {
    rangeStart: "2026-07-16T00:00:00Z",
    rangeEnd: "2026-07-16T10:00:00Z",
    bucketSeconds: 60,
    snapshotId: 1,
    points: Array.from({ length: points }, (_, index) => ({
      bucketStart: `2026-07-16T10:0${index}:00Z`,
      bucketEnd: `2026-07-16T10:0${index + 1}:00Z`,
      totalCount: index + 1,
      successCount: index + 1,
      failureCount: 0,
      totalTokens: (index + 1) * 100,
      totalCost: (index + 1) * 0.1,
    })),
  };
}

function Probe({ range }: { range: string }) {
  const { data, isLoading } = useTimeseries(range, { bucket: "1m" });
  return (
    <div>
      <div data-testid="points">{String(data?.points.length ?? 0)}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
    </div>
  );
}

describe("useTimeseries", () => {
  it("subscribes to stats.timeseries.open-window for open ranges", () => {
    topicMocks.state.data = createTimeseries(3);

    render(<Probe range="today" />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "stats.timeseries.open-window",
      params: expect.objectContaining({
        range: "today",
        bucket: "1m",
      }),
    });
    expect(topicMocks.lastEnabled).toBe(true);
    expect(text("points")).toBe("3");
  });

  it("uses HTTP hydration for yesterday", async () => {
    apiMocks.fetchTimeseries.mockResolvedValue(createTimeseries(1));

    render(<Probe range="yesterday" />);
    await flushAsync();

    expect(topicMocks.lastDescriptor).toBeNull();
    expect(topicMocks.lastEnabled).toBe(false);
    expect(apiMocks.fetchTimeseries).toHaveBeenCalledTimes(1);
    expect(text("points")).toBe("1");
  });
});
