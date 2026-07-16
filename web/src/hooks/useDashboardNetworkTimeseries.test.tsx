/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DashboardNetworkTimeseriesResponse } from "../lib/api";
import { useDashboardNetworkTimeseries } from "./useDashboardNetworkTimeseries";

const topicMocks = vi.hoisted(() => ({
  calls: [] as Array<{ descriptor: unknown; enabled: boolean }>,
  refresh: vi.fn(),
  state: {
    data: null as DashboardNetworkTimeseriesResponse | null,
    isLoading: false,
    error: null as string | null,
  },
}));

vi.mock("../lib/timeZone", () => ({
  getBrowserTimeZone: () => "Asia/Shanghai",
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: unknown, enabled: boolean) => {
    topicMocks.calls.push({ descriptor, enabled });
    return {
      data: topicMocks.state.data,
      isLoading: topicMocks.state.isLoading,
      error: topicMocks.state.error,
      refresh: topicMocks.refresh,
    };
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  topicMocks.calls = [];
  topicMocks.refresh.mockReset();
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function createResponse(): DashboardNetworkTimeseriesResponse {
  return {
    range: "today",
    rangeStart: "2026-07-16T10:00:00.000Z",
    rangeEnd: "2026-07-16T10:05:00.000Z",
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
  const { data, isLoading, isRefreshing, error, reload } = useDashboardNetworkTimeseries(
    range,
    enabled,
    upstreamAccountId,
  );
  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="refreshing">{isRefreshing ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="point-count">{String(data?.points.length ?? 0)}</div>
      <button type="button" data-testid="reload" onClick={() => void reload()} />
    </div>
  );
}

describe("useDashboardNetworkTimeseries", () => {
  it("subscribes to the dashboard network topic and exposes topic state", () => {
    topicMocks.state.data = createResponse();

    render(<Probe upstreamAccountId={7} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "dashboard.network-timeseries.window",
        params: {
          range: "today",
          timeZone: "Asia/Shanghai",
          upstreamAccountId: "7",
        },
      },
      enabled: true,
    });
    expect(text("loading")).toBe("false");
    expect(text("refreshing")).toBe("false");
    expect(text("point-count")).toBe("1");
  });

  it("disables the topic when the hook is not enabled and forwards manual refresh", () => {
    render(<Probe enabled={false} range="1d" />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: null,
      enabled: false,
    });

    topicMocks.state.data = createResponse();
    topicMocks.state.isLoading = true;
    render(<Probe />);

    act(() => {
      const button = host?.querySelector('[data-testid="reload"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("Missing reload button");
      }
      button.click();
    });

    expect(text("loading")).toBe("true");
    expect(topicMocks.refresh).toHaveBeenCalledTimes(1);
  });
});
