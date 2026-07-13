/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ApiRequestError, type BroadcastPayload, type ParallelWorkStatsResponse } from "../lib/api";
import {
  getParallelWorkRecordsResyncDelay,
  PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS,
  PARALLEL_WORK_REFRESH_THROTTLE_MS,
  shouldRetryParallelWorkError,
  shouldTriggerParallelWorkOpenResync,
  useParallelWorkStats,
} from "./useParallelWorkStats";

const apiMocks = vi.hoisted(() => ({
  fetchParallelWorkStatsConditional:
    vi.fn<
      (options?: {
        range?: string;
        bucket?: string;
        timeZone?: string;
        upstreamAccountId?: number;
        signal?: AbortSignal;
        etag?: string | null;
      }) => Promise<{
        data: ParallelWorkStatsResponse | null;
        etag: string | null;
        notModified: boolean;
      }>
    >(),
}));

type ConditionalParallelWorkStatsResponse = Awaited<
  ReturnType<typeof apiMocks.fetchParallelWorkStatsConditional>
>;

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchParallelWorkStatsConditional: apiMocks.fetchParallelWorkStatsConditional,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: (listener: (payload: BroadcastPayload) => void) => {
    sseMocks.listeners.add(listener);
    return () => sseMocks.listeners.delete(listener);
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListeners.add(listener);
    return () => sseMocks.openListeners.delete(listener);
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

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  sseMocks.listeners.clear();
  sseMocks.openListeners.clear();
  vi.useRealTimers();
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

async function waitFor(check: () => void, attempts = 20) {
  let lastError: unknown = null;
  for (let index = 0; index < attempts; index += 1) {
    try {
      check();
      return;
    } catch (error) {
      lastError = error;
      await flushAsync();
    }
  }
  throw lastError;
}

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

function fullStatsResponse(stats = createStats(), etag = '"parallel-work-test"') {
  return {
    data: stats,
    etag,
    notModified: false,
  };
}

function notModifiedResponse(etag = '"parallel-work-test"') {
  return {
    data: null,
    etag,
    notModified: true,
  };
}

function Probe() {
  const { data, isLoading, error } = useParallelWorkStats({ range: "7d", bucket: "1m" });
  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="current-count">{String(data?.current.points[0]?.parallelCount ?? 0)}</div>
    </div>
  );
}

function AccountProbe() {
  const { data } = useParallelWorkStats({ range: "today", bucket: "1m", upstreamAccountId: 42 });
  return (
    <div data-testid="account-current-count">
      {String(data?.current.points[0]?.parallelCount ?? 0)}
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
  it("throttles records-triggered silent refreshes to the configured interval", async () => {
    vi.useFakeTimers();
    apiMocks.fetchParallelWorkStatsConditional.mockResolvedValue(fullStatsResponse());

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenLastCalledWith(
      expect.objectContaining({ range: "7d", bucket: "1m", etag: null }),
    );
    expect(host?.querySelector('[data-testid="current-count"]')?.textContent).toBe("1");

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({ type: "records", records: [] });
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({ type: "records", records: [] });
      });
    });
    await vi.advanceTimersByTimeAsync(PARALLEL_WORK_REFRESH_THROTTLE_MS - 1);
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(1);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(3);
  });

  it("passes upstreamAccountId through to the conditional fetch", async () => {
    apiMocks.fetchParallelWorkStatsConditional.mockResolvedValue(fullStatsResponse());

    render(<AccountProbe />);
    await flushAsync();

    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledWith(
      expect.objectContaining({
        range: "today",
        bucket: "1m",
        upstreamAccountId: 42,
        etag: null,
      }),
    );
    expect(host?.querySelector('[data-testid="account-current-count"]')?.textContent).toBe("1");
  });

  it("respects the SSE-open cooldown before queueing another refresh", async () => {
    vi.useFakeTimers();
    apiMocks.fetchParallelWorkStatsConditional.mockResolvedValue(fullStatsResponse());

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);

    act(() => {
      sseMocks.openListeners.forEach((listener) => {
        listener();
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);

    act(() => {
      sseMocks.openListeners.forEach((listener) => {
        listener();
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS);
    act(() => {
      sseMocks.openListeners.forEach((listener) => {
        listener();
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(3);
  });

  it("reuses the prior payload when the server returns 304", async () => {
    vi.useFakeTimers();
    apiMocks.fetchParallelWorkStatsConditional
      .mockResolvedValueOnce(fullStatsResponse(createStats(), '"parallel-work-a"'))
      .mockResolvedValueOnce(notModifiedResponse('"parallel-work-a"'));

    render(<Probe />);
    await flushAsync();
    expect(host?.querySelector('[data-testid="current-count"]')?.textContent).toBe("1");

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({ type: "records", records: [] });
      });
    });
    await flushAsync();

    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenLastCalledWith(
      expect.objectContaining({ etag: '"parallel-work-a"' }),
    );
    expect(host?.querySelector('[data-testid="current-count"]')?.textContent).toBe("1");
    expect(host?.querySelector('[data-testid="error"]')?.textContent).toBe("");
  });

  it("queues SSE-open refreshes instead of aborting an in-flight request", async () => {
    let resolveRefresh: ((value: ConditionalParallelWorkStatsResponse) => void) | null = null;
    let refreshSignal: AbortSignal | undefined;
    apiMocks.fetchParallelWorkStatsConditional
      .mockResolvedValueOnce(fullStatsResponse())
      .mockImplementationOnce(
        ({ signal } = {}) =>
          new Promise<ConditionalParallelWorkStatsResponse>((resolve) => {
            refreshSignal = signal;
            resolveRefresh = resolve;
          }),
      )
      .mockResolvedValueOnce(fullStatsResponse());

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({ type: "records", records: [] });
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);
    expect(refreshSignal?.aborted).toBe(false);

    act(() => {
      sseMocks.openListeners.forEach((listener) => {
        listener();
      });
    });
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);
    expect(refreshSignal?.aborted).toBe(false);

    await act(async () => {
      resolveRefresh?.(fullStatsResponse());
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(3);
  });

  it("queues a follow-up silent refresh when SSE-open arrives during hydration", async () => {
    let resolveInitialLoad: ((value: ConditionalParallelWorkStatsResponse) => void) | null = null;
    apiMocks.fetchParallelWorkStatsConditional
      .mockImplementationOnce(
        () =>
          new Promise<ConditionalParallelWorkStatsResponse>((resolve) => {
            resolveInitialLoad = resolve;
          }),
      )
      .mockResolvedValueOnce(fullStatsResponse());

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);

    act(() => {
      sseMocks.openListeners.forEach((listener) => {
        listener();
      });
    });

    await act(async () => {
      resolveInitialLoad?.(fullStatsResponse());
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);
  });

  it("does not auto-retry permanent client errors", async () => {
    vi.useFakeTimers();
    apiMocks.fetchParallelWorkStatsConditional.mockRejectedValue(
      new ApiRequestError(
        400,
        "Request failed: 400 unsupported timeZone for historical parallel-work rollups",
      ),
    );

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);
    expect(host?.querySelector('[data-testid="error"]')?.textContent).toContain(
      "Request failed: 400",
    );

    await vi.advanceTimersByTimeAsync(2_000);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);
  });

  it("keeps auto-retrying transient server failures", async () => {
    vi.useFakeTimers();
    apiMocks.fetchParallelWorkStatsConditional
      .mockRejectedValueOnce(new ApiRequestError(503, "Request failed: 503 gateway timeout"))
      .mockResolvedValueOnce(fullStatsResponse());

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(1);
    expect(host?.querySelector('[data-testid="error"]')?.textContent).toContain(
      "Request failed: 503",
    );

    await vi.advanceTimersByTimeAsync(2_000);
    await waitFor(() => {
      expect(apiMocks.fetchParallelWorkStatsConditional).toHaveBeenCalledTimes(2);
      expect(host?.querySelector('[data-testid="error"]')?.textContent).toBe("");
    });
    expect(host?.querySelector('[data-testid="current-count"]')?.textContent).toBe("1");
  });
});
