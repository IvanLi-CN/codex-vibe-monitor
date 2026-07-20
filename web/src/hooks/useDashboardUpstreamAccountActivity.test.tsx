/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DashboardActivityLiveSnapshot, DashboardActivityResponse } from "../lib/api";
import {
  mergeDashboardActivityLiveSnapshot,
  useDashboardActivitySnapshot,
} from "./useDashboardUpstreamAccountActivity";

const apiMocks = vi.hoisted(() => ({
  fetchDashboardActivity: vi.fn<() => Promise<DashboardActivityResponse>>(),
}));

const topicMocks = vi.hoisted(() => ({
  state: {
    data: null as DashboardActivityResponse | null,
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
    fetchDashboardActivity: apiMocks.fetchDashboardActivity,
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
  apiMocks.fetchDashboardActivity.mockReset();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
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

function createResponse(inProgressCount: number): DashboardActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-07-16T00:00:00Z",
    rangeEnd: "2026-07-16T10:05:00Z",
    snapshotId: 1,
    rateWindow: {
      start: "2026-07-16T10:04:00Z",
      end: "2026-07-16T10:05:00Z",
      windowMinutes: 1,
      mode: "rolling_60s_live_mean",
    },
    summary: {
      stats: {
        totalCount: 5,
        successCount: 4,
        failureCount: 1,
        totalCost: 0.5,
        totalTokens: 500,
        inProgressConversationCount: inProgressCount,
        inProgressRetryConversationCount: 1,
      },
    },
    networkLiveBucket: {
      bucketStart: "2026-07-16T10:00:00Z",
      bucketEnd: "2026-07-16T10:05:00Z",
      uploadBytesPerSecond: 64,
      downloadBytesPerSecond: 128,
      uploadBytes: 19_200,
      downloadBytes: 38_400,
      isLiveBucket: true,
    },
    networkRealtimeRate: {
      sampleStart: "2026-07-16T10:04:59Z",
      sampleEnd: "2026-07-16T10:05:00Z",
      sampleSeconds: 1,
      uploadBytesPerSecond: 64,
      downloadBytesPerSecond: 128,
      uploadBytes: 64,
      downloadBytes: 128,
    },
    accounts: [
      {
        accountKey: "upstream:7",
        upstreamAccountId: 7,
        displayName: "Pool A",
        requestCount: 5,
        successCount: 4,
        failureCount: 1,
        nonSuccessCount: 1,
        totalTokens: 500,
        successTokens: 400,
        nonSuccessTokens: 100,
        failureTokens: 100,
        failureCost: 0.1,
        totalCost: 0.5,
        usageBreakdown: {
          cacheWriteTokens: 0,
          cacheReadTokens: 0,
          outputTokens: 50,
          models: [],
        },
        inProgressInvocationCount: inProgressCount,
        inProgressPhaseCounts: {
          queued: 0,
          requesting: inProgressCount,
          responding: 0,
        },
        retryInvocationCount: 1,
        uploadBytesPerSecond: 0,
        downloadBytesPerSecond: 0,
        effectiveRoutingRule: { source: "none" } as never,
        recentInvocations: [],
      },
    ],
  } as DashboardActivityResponse;
}

function Probe({ range }: { range: string }) {
  const snapshot = useDashboardActivitySnapshot(range, true, true, 8, true);

  return (
    <div>
      <div data-testid="total">{String(snapshot.data?.summary.stats.totalCount ?? 0)}</div>
      <div data-testid="accounts">{String(snapshot.data?.accounts?.length ?? 0)}</div>
      <div data-testid="recent-limit">{String(snapshot.recentInvocationLimit)}</div>
      <div data-testid="loading">{snapshot.isLoading ? "true" : "false"}</div>
    </div>
  );
}

describe("useDashboardActivitySnapshot", () => {
  it("uses topic hydration for open dashboard ranges", () => {
    topicMocks.state.data = createResponse(6);

    render(<Probe range="today" />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "dashboard.activity.current",
      params: expect.objectContaining({
        range: "today",
        recentLimit: "8",
        includeAccounts: "true",
        includeRecent: "true",
      }),
    });
    expect(topicMocks.lastEnabled).toBe(true);
    expect(text("total")).toBe("5");
    expect(text("accounts")).toBe("1");
    expect(text("recent-limit")).toBe("6");
  });

  it("falls back to HTTP for the closed yesterday window", async () => {
    apiMocks.fetchDashboardActivity.mockResolvedValue(createResponse(4));

    render(<Probe range="yesterday" />);
    await flushAsync();

    expect(topicMocks.lastDescriptor).toBeNull();
    expect(topicMocks.lastEnabled).toBe(false);
    expect(apiMocks.fetchDashboardActivity).toHaveBeenCalledTimes(1);
    expect(text("total")).toBe("5");
    expect(text("recent-limit")).toBe("4");
  });
});

describe("mergeDashboardActivityLiveSnapshot", () => {
  it("overlays live in-flight counters on top of the authoritative snapshot", () => {
    const response = createResponse(1);
    const live: DashboardActivityLiveSnapshot = {
      revision: 2,
      generatedAt: "2026-07-16T10:05:30Z",
      inProgressInvocationCount: 3,
      inProgressPhaseCounts: {
        queued: 1,
        requesting: 2,
        responding: 0,
      },
      retryInvocationCount: 2,
      networkLiveBucket: {
        bucketStart: "2026-07-16T10:00:00Z",
        bucketEnd: "2026-07-16T10:05:00Z",
        uploadBytesPerSecond: 512,
        downloadBytesPerSecond: 1024,
        uploadBytes: 153_600,
        downloadBytes: 307_200,
        isLiveBucket: true,
      },
      networkRealtimeRate: {
        sampleStart: "2026-07-16T10:05:29Z",
        sampleEnd: "2026-07-16T10:05:30Z",
        sampleSeconds: 1,
        uploadBytesPerSecond: 77,
        downloadBytesPerSecond: 155,
        uploadBytes: 77,
        downloadBytes: 155,
      },
      accounts: [
        {
          accountKey: "upstream:7",
          upstreamAccountId: 7,
          inProgressInvocationCount: 3,
          inProgressPhaseCounts: {
            queued: 1,
            requesting: 2,
            responding: 0,
          },
          retryInvocationCount: 2,
          uploadBytesPerSecond: 12,
          downloadBytesPerSecond: 34,
        },
      ],
    };

    const merged = mergeDashboardActivityLiveSnapshot(response, live);
    expect(merged.liveRevision).toBe(2);
    expect(merged.summary.stats.inProgressConversationCount).toBe(3);
    expect(merged.networkLiveBucket?.uploadBytesPerSecond).toBe(512);
    expect(merged.networkLiveBucket?.downloadBytesPerSecond).toBe(1024);
    expect(merged.networkRealtimeRate?.uploadBytesPerSecond).toBe(77);
    expect(merged.networkRealtimeRate?.downloadBytesPerSecond).toBe(155);
    expect(merged.accounts?.[0]).toEqual(
      expect.objectContaining({
        inProgressInvocationCount: 3,
        retryInvocationCount: 2,
        uploadBytesPerSecond: 12,
        downloadBytesPerSecond: 34,
      }),
    );
  });
});
