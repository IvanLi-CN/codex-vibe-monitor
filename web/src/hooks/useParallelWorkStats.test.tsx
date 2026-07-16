/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError, type ParallelWorkStatsResponse } from "../lib/api";
import {
  getParallelWorkRecordsResyncDelay,
  PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS,
  PARALLEL_WORK_REFRESH_THROTTLE_MS,
  shouldRetryParallelWorkError,
  shouldTriggerParallelWorkOpenResync,
  useParallelWorkStats,
} from "./useParallelWorkStats";

const topicMocks = vi.hoisted(() => ({
  calls: [] as Array<{ descriptor: unknown; enabled: boolean }>,
  refresh: vi.fn(),
  state: {
    data: null as ParallelWorkStatsResponse | null,
    isLoading: false,
    error: null as string | null,
  },
}));

vi.mock("../lib/timeZone", () => ({
  getBrowserTimeZone: () => "Asia/Shanghai",
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: unknown, enabled = true) => {
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

function createStats(): ParallelWorkStatsResponse {
  const current = {
    rangeStart: "2026-03-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 60,
    completeBucketCount: 10080,
    activeBucketCount: 3,
    minCount: 0,
    maxCount: 3,
    avgCount: 1.2,
    points: [
      { bucketStart: "2026-03-07T10:00:00Z", bucketEnd: "2026-03-07T10:01:00Z", parallelCount: 1 },
    ],
  };

  return {
    current,
    minute7d: current,
    hour30d: {
      rangeStart: "2026-02-06T00:00:00Z",
      rangeEnd: "2026-03-08T00:00:00Z",
      bucketSeconds: 3600,
      completeBucketCount: 720,
      activeBucketCount: 2,
      minCount: 0,
      maxCount: 2,
      avgCount: 0.4,
      points: [
        {
          bucketStart: "2026-03-07T00:00:00Z",
          bucketEnd: "2026-03-07T01:00:00Z",
          parallelCount: 2,
        },
      ],
    },
    dayAll: {
      rangeStart: "2026-01-01T00:00:00Z",
      rangeEnd: "2026-03-08T00:00:00Z",
      bucketSeconds: 86400,
      completeBucketCount: 5,
      activeBucketCount: 5,
      minCount: 1,
      maxCount: 2,
      avgCount: 1.4,
      points: [
        {
          bucketStart: "2026-03-07T00:00:00Z",
          bucketEnd: "2026-03-08T00:00:00Z",
          parallelCount: 2,
        },
      ],
    },
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
  range = "7d",
  bucket = "1m",
  upstreamAccountId,
  enabled = true,
}: {
  range?: string;
  bucket?: string;
  upstreamAccountId?: number;
  enabled?: boolean;
}) {
  const { data, isLoading, error, refresh } = useParallelWorkStats({
    range,
    bucket,
    upstreamAccountId,
    enabled,
  });

  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="current-count">{String(data?.current.points[0]?.parallelCount ?? 0)}</div>
      <button type="button" data-testid="refresh" onClick={() => void refresh()} />
    </div>
  );
}

describe("useParallelWorkStats helpers", () => {
  it("computes refresh delay from the last records refresh", () => {
    expect(getParallelWorkRecordsResyncDelay(10_000, 20_000)).toBe(0);
    expect(getParallelWorkRecordsResyncDelay(20_000, 22_000)).toBe(
      PARALLEL_WORK_REFRESH_THROTTLE_MS - 2_000,
    );
  });

  it("enforces the open-resync cooldown unless forced", () => {
    expect(shouldTriggerParallelWorkOpenResync(0, 4_999)).toBe(false);
    expect(shouldTriggerParallelWorkOpenResync(0, PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS + 1)).toBe(
      true,
    );
    expect(shouldTriggerParallelWorkOpenResync(0, 1, true)).toBe(true);
  });

  it("only retries transient request failures", () => {
    expect(shouldRetryParallelWorkError(new ApiRequestError(400, "Request failed: 400"))).toBe(
      false,
    );
    expect(shouldRetryParallelWorkError(new ApiRequestError(429, "Request failed: 429"))).toBe(
      true,
    );
    expect(shouldRetryParallelWorkError(new ApiRequestError(503, "Request failed: 503"))).toBe(
      true,
    );
    expect(shouldRetryParallelWorkError(new Error("network down"))).toBe(true);
  });
});

describe("useParallelWorkStats", () => {
  it("subscribes to the parallel-work topic and exposes topic state", () => {
    topicMocks.state.data = createStats();
    topicMocks.state.error = "topic warning";

    render(<Probe upstreamAccountId={42} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "stats.parallel-work.current",
        params: {
          range: "7d",
          bucket: "1m",
          timeZone: "Asia/Shanghai",
          upstreamAccountId: "42",
        },
      },
      enabled: true,
    });
    expect(text("current-count")).toBe("1");
    expect(text("loading")).toBe("false");
    expect(text("error")).toBe("topic warning");
  });

  it("disables the topic when the hook is not enabled and forwards manual refresh", () => {
    render(<Probe enabled={false} range="today" bucket="5m" />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: null,
      enabled: false,
    });

    topicMocks.state.data = createStats();
    topicMocks.state.isLoading = true;
    render(<Probe range="today" bucket="5m" />);

    act(() => {
      const button = host?.querySelector('[data-testid="refresh"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("Missing refresh button");
      }
      button.click();
    });

    expect(text("loading")).toBe("true");
    expect(topicMocks.refresh).toHaveBeenCalledTimes(1);
  });
});
