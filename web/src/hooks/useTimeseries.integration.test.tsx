/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  ApiInvocation,
  BroadcastPayload,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  TimeseriesResponse,
} from "../lib/api";
import { clearTimeseriesRemountCache, useTimeseries } from "./useTimeseries";

const apiMocks = vi.hoisted(() => ({
  fetchTimeseries: vi.fn<
    (
      range: string,
      params?: {
        bucket?: string;
        settlementHour?: number;
        timeZone?: string;
        signal?: AbortSignal;
      },
    ) => Promise<TimeseriesResponse>
  >(),
  fetchInvocationRecords:
    vi.fn<
      (query: InvocationRecordsQuery) => Promise<InvocationRecordsResponse>
    >(),
}));

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchTimeseries: apiMocks.fetchTimeseries,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
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
  clearTimeseriesRemountCache();
  sseMocks.listeners.clear();
  sseMocks.openListeners.clear();
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

function unmountCurrent() {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
}

async function flushAsync(turns = 3) {
  for (let index = 0; index < turns; index += 1) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function emitRecords(records: ApiInvocation[]) {
  act(() => {
    sseMocks.listeners.forEach((listener) =>
      listener({ type: "records", records }),
    );
  });
}

function createBaseTimeseries(
  overrides?: Partial<TimeseriesResponse["points"][number]>,
  responseOverrides?: Partial<Omit<TimeseriesResponse, "points">>,
): TimeseriesResponse {
  return {
    rangeStart: "2026-03-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 3600,
    ...responseOverrides,
    points: [
      {
        bucketStart: "2026-03-07T23:00:00Z",
        bucketEnd: "2026-03-08T00:00:00Z",
        totalCount: 1,
        successCount: 0,
        failureCount: 0,
        totalTokens: 0,
        totalCost: 0,
        ...overrides,
      },
    ],
  };
}

function createRunningRecord(): ApiInvocation {
  return {
    id: 91,
    invokeId: "cached-running",
    occurredAt: "2026-03-07T23:30:00Z",
    status: "running",
    totalTokens: 0,
    cost: 0,
    createdAt: "2026-03-07T23:30:00Z",
  };
}

function createSettledRecord(): ApiInvocation {
  return {
    ...createRunningRecord(),
    status: "failed",
    totalTokens: 22,
    cost: 0.18,
    errorMessage: "upstream timed out",
    failureKind: "upstream_timeout",
  };
}

function createRecordsPage(
  records: ApiInvocation[],
): InvocationRecordsResponse {
  return {
    snapshotId: 1,
    total: records.length,
    page: 1,
    pageSize: Math.max(1, records.length),
    records,
  };
}

function Probe() {
  const { data, error, isLoading } = useTimeseries("7d", { bucket: "1h" });
  const point = data?.points[0];

  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="total">{String(point?.totalCount ?? 0)}</div>
      <div data-testid="success">{String(point?.successCount ?? 0)}</div>
      <div data-testid="failure">{String(point?.failureCount ?? 0)}</div>
      <div data-testid="tokens">{String(point?.totalTokens ?? 0)}</div>
      <div data-testid="cost">{String(point?.totalCost ?? 0)}</div>
    </div>
  );
}

describe("useTimeseries remount cache hydration", () => {
  it("restores cached live deltas before applying a remount SSE settle", async () => {
    const response = createBaseTimeseries();
    const runningRecord = createRunningRecord();
    const settledRecord = createSettledRecord();
    const silentRefresh: { resolve: (value: TimeseriesResponse) => void } = {
      resolve: (_value: TimeseriesResponse) => {
        throw new Error("expected silent refresh resolver");
      },
    };

    apiMocks.fetchTimeseries
      .mockResolvedValueOnce(response)
      .mockImplementationOnce(
        () =>
          new Promise<TimeseriesResponse>((resolve) => {
            silentRefresh.resolve = resolve as (value: TimeseriesResponse) => void;
          }),
      );
    apiMocks.fetchInvocationRecords.mockImplementation(async ({ status }) =>
      createRecordsPage(status === "running" ? [runningRecord] : []),
    );

    render(<Probe />);
    await flushAsync();

    expect(text("total")).toBe("1");
    expect(text("failure")).toBe("0");
    unmountCurrent();

    render(<Probe />);
    await flushAsync(1);

    expect(text("loading")).toBe("false");
    expect(text("total")).toBe("1");

    emitRecords([settledRecord]);

    expect(text("total")).toBe("1");
    expect(text("success")).toBe("0");
    expect(text("failure")).toBe("1");
    expect(text("error")).toBe("");

    silentRefresh.resolve(response);
    await flushAsync();
  });

  it("does not double-count a record that settles between the base fetch and the live seed fetch", async () => {
    const response = createBaseTimeseries();
    const settledRecord = createSettledRecord();

    apiMocks.fetchTimeseries.mockResolvedValue(response);
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsPage([]));

    render(<Probe />);
    await flushAsync();

    expect(text("loading")).toBe("false");
    expect(text("total")).toBe("1");
    expect(text("failure")).toBe("0");

    emitRecords([settledRecord]);

    expect(text("total")).toBe("1");
    expect(text("success")).toBe("0");
    expect(text("failure")).toBe("1");
    expect(text("tokens")).toBe("22");
    expect(text("cost")).toBe("0.18");
    expect(text("error")).toBe("");
  });

  it("does not let a new post-load record consume an older anonymous in-flight placeholder from the same bucket", async () => {
    const response = createBaseTimeseries(
      undefined,
      {
        rangeEnd: "2026-03-07T23:45:00Z",
      },
    );
    const newSettledRecord: ApiInvocation = {
      ...createSettledRecord(),
      invokeId: "new-after-load",
      occurredAt: "2026-03-07T23:46:00Z",
      createdAt: "2026-03-07T23:46:00Z",
    };

    apiMocks.fetchTimeseries.mockResolvedValue(response);
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsPage([]));

    render(<Probe />);
    await flushAsync();

    emitRecords([newSettledRecord]);

    expect(text("total")).toBe("2");
    expect(text("success")).toBe("0");
    expect(text("failure")).toBe("1");
    expect(text("tokens")).toBe("22");
    expect(text("cost")).toBe("0.18");
  });

  it("keeps the fetched chart data visible when in-flight seeding fails", async () => {
    const response = createBaseTimeseries({
      totalTokens: 10,
      totalCost: 0.1,
    });
    const settledRecord = createSettledRecord();

    apiMocks.fetchTimeseries.mockResolvedValue(response);
    apiMocks.fetchInvocationRecords.mockRejectedValue(
      new Error("seed sync unavailable"),
    );

    render(<Probe />);
    await flushAsync();

    expect(text("loading")).toBe("false");
    expect(text("total")).toBe("1");
    expect(text("failure")).toBe("0");
    expect(text("tokens")).toBe("10");
    expect(text("cost")).toBe("0.1");
    expect(text("error")).toBe("");

    emitRecords([settledRecord]);

    expect(text("total")).toBe("1");
    expect(text("failure")).toBe("1");
    expect(text("tokens")).toBe("10");
    expect(text("cost")).toBe("0.1");
  });
});
