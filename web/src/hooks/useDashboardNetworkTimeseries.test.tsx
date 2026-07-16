/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BroadcastPayload, DashboardNetworkTimeseriesResponse } from "../lib/api";
import { useDashboardNetworkTimeseries } from "./useDashboardNetworkTimeseries";

const apiMocks = vi.hoisted(() => ({
  fetchDashboardNetworkTimeseries:
    vi.fn<
      (
        range: string,
        options?: {
          timeZone?: string;
          upstreamAccountId?: number;
          signal?: AbortSignal;
        },
      ) => Promise<DashboardNetworkTimeseriesResponse>
    >(),
}));

const sseMocks = vi.hoisted(() => ({
  listener: null as null | ((payload: BroadcastPayload) => void),
  openListener: null as null | (() => void),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchDashboardNetworkTimeseries: apiMocks.fetchDashboardNetworkTimeseries,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: (listener: (payload: BroadcastPayload) => void) => {
    sseMocks.listener = listener;
    return () => {
      sseMocks.listener = null;
    };
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListener = listener;
    return () => {
      sseMocks.openListener = null;
    };
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
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  sseMocks.listener = null;
  sseMocks.openListener = null;
  vi.clearAllMocks();
  vi.useRealTimers();
});

function createResponse(
  range: "today" | "yesterday" | "1d",
  pointOverrides: Partial<DashboardNetworkTimeseriesResponse["points"][number]> = {},
): DashboardNetworkTimeseriesResponse {
  return {
    range,
    rangeStart: "2026-07-16T10:00:00.000Z",
    rangeEnd: "2026-07-16T10:05:30.000Z",
    snapshotId: 1,
    bucketSeconds: 300,
    points: [
      {
        bucketStart: "2026-07-16T10:00:00.000Z",
        bucketEnd: "2026-07-16T10:05:00.000Z",
        uploadBytesPerSecond: 1024,
        downloadBytesPerSecond: 2048,
        uploadBytes: 307200,
        downloadBytes: 614400,
        isLiveBucket: true,
        ...pointOverrides,
      },
    ],
  };
}

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

function Probe({
  range = "today",
  enabled = true,
  upstreamAccountId,
}: {
  range?: "today" | "yesterday" | "1d";
  enabled?: boolean;
  upstreamAccountId?: number;
}) {
  const { data, isLoading, isRefreshing, error } = useDashboardNetworkTimeseries(
    range,
    enabled,
    upstreamAccountId,
  );
  const lastPoint = data?.points[data.points.length - 1];
  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="refreshing">{isRefreshing ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="point-count">{String(data?.points.length ?? 0)}</div>
      <div data-testid="download-rate">{String(lastPoint?.downloadBytesPerSecond ?? 0)}</div>
      <div data-testid="bucket-start">{lastPoint?.bucketStart ?? ""}</div>
    </div>
  );
}

describe("useDashboardNetworkTimeseries", () => {
  it("hydrates once and does not poll on an interval", async () => {
    apiMocks.fetchDashboardNetworkTimeseries.mockResolvedValue(createResponse("today"));

    render(<Probe />);
    await flushAsync();

    expect(apiMocks.fetchDashboardNetworkTimeseries).toHaveBeenCalledTimes(1);
    expect(text("download-rate")).toBe("2048");

    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    await flushAsync();

    expect(apiMocks.fetchDashboardNetworkTimeseries).toHaveBeenCalledTimes(1);
  });

  it("merges pushed live bucket updates without issuing another fetch", async () => {
    apiMocks.fetchDashboardNetworkTimeseries.mockResolvedValue(createResponse("today"));

    render(<Probe />);
    await flushAsync();

    act(() => {
      sseMocks.listener?.({
        type: "dashboardActivityLive",
        snapshot: {
          revision: 2,
          generatedAt: "2026-07-16T10:05:20.000Z",
          inProgressInvocationCount: 1,
          inProgressPhaseCounts: { queued: 0, requesting: 0, responding: 1 },
          retryInvocationCount: 0,
          networkLiveBucket: {
            bucketStart: "2026-07-16T10:00:00.000Z",
            bucketEnd: "2026-07-16T10:05:00.000Z",
            uploadBytesPerSecond: 1024,
            downloadBytesPerSecond: 4096,
            uploadBytes: 307200,
            downloadBytes: 1228800,
            isLiveBucket: true,
          },
          accounts: [],
        },
      });
    });

    expect(text("download-rate")).toBe("4096");
    expect(apiMocks.fetchDashboardNetworkTimeseries).toHaveBeenCalledTimes(1);
  });

  it("ignores live bucket pushes for the yesterday range", async () => {
    apiMocks.fetchDashboardNetworkTimeseries.mockResolvedValue(createResponse("yesterday"));

    render(<Probe range="yesterday" />);
    await flushAsync();

    act(() => {
      sseMocks.listener?.({
        type: "dashboardActivityLive",
        snapshot: {
          revision: 3,
          generatedAt: "2026-07-16T10:05:20.000Z",
          inProgressInvocationCount: 0,
          inProgressPhaseCounts: { queued: 0, requesting: 0, responding: 0 },
          retryInvocationCount: 0,
          networkLiveBucket: {
            bucketStart: "2026-07-16T10:00:00.000Z",
            bucketEnd: "2026-07-16T10:05:00.000Z",
            uploadBytesPerSecond: 1024,
            downloadBytesPerSecond: 8192,
            uploadBytes: 307200,
            downloadBytes: 2457600,
            isLiveBucket: true,
          },
          accounts: [],
        },
      });
    });

    expect(text("download-rate")).toBe("2048");
    expect(apiMocks.fetchDashboardNetworkTimeseries).toHaveBeenCalledTimes(1);
  });

  it("resyncs once when the pushed live bucket rolls into a new bucket", async () => {
    apiMocks.fetchDashboardNetworkTimeseries
      .mockResolvedValueOnce(createResponse("today"))
      .mockResolvedValueOnce(
        createResponse("today", {
          bucketStart: "2026-07-16T10:05:00.000Z",
          bucketEnd: "2026-07-16T10:10:00.000Z",
          downloadBytesPerSecond: 5120,
          downloadBytes: 1536000,
        }),
      );

    render(<Probe />);
    await flushAsync();

    act(() => {
      sseMocks.listener?.({
        type: "dashboardActivityLive",
        snapshot: {
          revision: 4,
          generatedAt: "2026-07-16T10:05:20.000Z",
          inProgressInvocationCount: 1,
          inProgressPhaseCounts: { queued: 0, requesting: 1, responding: 0 },
          retryInvocationCount: 0,
          networkLiveBucket: {
            bucketStart: "2026-07-16T10:05:00.000Z",
            bucketEnd: "2026-07-16T10:10:00.000Z",
            uploadBytesPerSecond: 1024,
            downloadBytesPerSecond: 5120,
            uploadBytes: 307200,
            downloadBytes: 1536000,
            isLiveBucket: true,
          },
          accounts: [],
        },
      });
    });
    await flushAsync();

    expect(apiMocks.fetchDashboardNetworkTimeseries).toHaveBeenCalledTimes(2);
    expect(text("bucket-start")).toBe("2026-07-16T10:05:00.000Z");
    expect(text("download-rate")).toBe("5120");
  });
});
