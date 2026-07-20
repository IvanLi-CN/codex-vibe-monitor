/** @vitest-environment jsdom */
import { act, useSyncExternalStore } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { DashboardActivityOverview } from "./DashboardActivityOverview";
import {
  ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX,
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
} from "./dashboardActivityRange";

const hookMocks = vi.hoisted(() => ({
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
  useParallelWorkStats: vi.fn(),
  useDashboardNetworkTimeseries: vi.fn(),
}));

const componentState = vi.hoisted(() => ({
  chartRenderCount: 0,
}));

const sseState = vi.hoisted(() => ({
  listeners: new Set<(payload: unknown) => void>(),
}));

vi.mock("../../hooks/useStats", () => ({
  useSummary: hookMocks.useSummary,
}));

vi.mock("../../hooks/useTimeseries", () => ({
  useTimeseries: hookMocks.useTimeseries,
}));

vi.mock("../../hooks/useParallelWorkStats", () => ({
  useParallelWorkStats: hookMocks.useParallelWorkStats,
}));

vi.mock("../../hooks/useDashboardNetworkTimeseries", () => ({
  useDashboardNetworkTimeseries: hookMocks.useDashboardNetworkTimeseries,
}));

vi.mock("../../lib/sse", () => ({
  subscribeToSse: (listener: (payload: unknown) => void) => {
    sseState.listeners.add(listener);
    return () => sseState.listeners.delete(listener);
  },
  subscribeToSseOpen: () => () => {},
}));

vi.mock("../../theme", () => ({
  useTheme: () => ({ themeMode: "light" }),
}));

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string) => {
      const map: Record<string, string> = {
        "dashboard.activityOverview.title": "Activity Overview",
        "dashboard.activityOverview.rangeToday": "Today",
        "dashboard.activityOverview.rangeYesterday": "Yesterday",
        "dashboard.activityOverview.range24h": "24 Hours",
        "dashboard.activityOverview.range7d": "7 Days",
        "dashboard.activityOverview.rangeUsage": "History",
        "dashboard.activityOverview.rangeToggleAria": "Switch activity range",
        "dashboard.activityOverview.network": "Network",
        "dashboard.activityOverview.networkUpload": "Upload",
        "dashboard.activityOverview.networkDownload": "Download",
        "dashboard.activityOverview.networkRefreshing": "Refreshing",
        "dashboard.activityOverview.snapshotBannerTitle": "Offline snapshot",
        "dashboard.activityOverview.snapshotBannerDescription":
          "Showing the latest cached overview from {{cachedAt}}.",
        "dashboard.activityOverview.snapshotReadyRanges": "{{count}} / {{total}} ranges cached",
        "dashboard.activityOverview.snapshotCachedAtUnknown": "an earlier sync",
        "dashboard.activityOverview.snapshotNotReadyTitle": "This range is not cached yet",
        "dashboard.activityOverview.snapshotNotReadyDescription":
          "Reconnect once while this range is visible so the overview can be saved for offline reading.",
        "heatmap.metricsToggleAria": "Switch metric",
        "metric.totalCount": "Calls",
        "metric.totalCost": "Cost",
        "metric.totalTokens": "Tokens",
        "chart.trend": "Trend",
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock("./TodayStatsOverview", () => ({
  TodayStatsOverview: ({
    stats,
    showSurface,
    showHeader,
    showDayBadge,
    rate,
    rateLoading,
    rateError,
    parallelWorkStats,
    parallelWorkError,
    showInProgressConversations,
  }: {
    stats?: {
      totalCount?: number;
      inProgressConversationCount?: number;
      inProgressRetryConversationCount?: number;
      inProgressAvgWaitMs?: number;
      nonSuccessCost?: number;
      nonSuccessTokens?: number;
      usageBreakdown?: { total?: { cacheWriteTokens?: number } };
    } | null;
    showSurface?: boolean;
    showHeader?: boolean;
    showDayBadge?: boolean;
    rate?: { tokensPerMinute?: number; spendRate?: number } | null;
    rateLoading?: boolean;
    rateError?: string | null;
    parallelWorkStats?: { current?: { avgCount?: number | null } } | null;
    parallelWorkError?: string | null;
    showInProgressConversations?: boolean;
  }) => (
    <div data-testid="today-stats-overview-mock">
      {`total:${stats?.totalCount ?? "null"};cacheWrite:${stats?.usageBreakdown?.total?.cacheWriteTokens ?? "null"};inProgress:${stats?.inProgressConversationCount ?? "null"};retry:${stats?.inProgressRetryConversationCount ?? "null"};wait:${stats?.inProgressAvgWaitMs ?? "null"};nonSuccessCost:${stats?.nonSuccessCost ?? "null"};nonSuccessTokens:${stats?.nonSuccessTokens ?? "null"};surface:${String(showSurface)};header:${String(showHeader)};badge:${String(showDayBadge)};tpm:${rate?.tokensPerMinute ?? "null"};spendRate:${rate?.spendRate ?? "null"};rateLoading:${String(rateLoading)};rateError:${rateError ?? "null"};parallelAvg:${parallelWorkStats?.current?.avgCount ?? "null"};parallelError:${parallelWorkError ?? "null"};showInProgress:${String(showInProgressConversations)}`}
    </div>
  ),
}));

vi.mock("./DashboardTodayActivityChart", () => ({
  DashboardTodayActivityChart: ({ metric }: { metric?: string }) => {
    componentState.chartRenderCount += 1;
    return (
      <div
        data-testid="dashboard-today-activity-chart-mock"
        data-render-count={String(componentState.chartRenderCount)}
      >
        {`metric:${metric ?? "unset"}`}
      </div>
    );
  },
}));

vi.mock("./DashboardNetworkActivityChart", () => ({
  DashboardNetworkActivityChart: ({
    response,
    loading,
    error,
  }: {
    response?: { points?: unknown[] } | null;
    loading?: boolean;
    error?: string | null;
  }) => (
    <div data-testid="dashboard-network-activity-chart-mock">
      {`points:${response?.points?.length ?? 0};loading:${String(Boolean(loading))};error:${error ?? "null"}`}
    </div>
  ),
}));

vi.mock("../stats/StatsCards", () => ({
  StatsCards: ({
    stats,
    loading,
    error,
  }: {
    stats: { totalCount?: number } | null;
    loading: boolean;
    error?: string | null;
  }) => (
    <div data-testid="stats-cards">
      {loading ? "loading" : error ? `error:${error}` : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}));

vi.mock("./Last24hTenMinuteHeatmap", () => ({
  Last24hTenMinuteHeatmap: ({
    metric,
    upstreamAccountId,
    timeseriesResponse,
  }: {
    metric?: string;
    upstreamAccountId?: number;
    timeseriesResponse?: { points?: unknown[] } | null;
  }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? "unset"};account:${upstreamAccountId ?? "global"};points:${timeseriesResponse?.points?.length ?? 0}`}
    </div>
  ),
}));

vi.mock("./WeeklyHourlyHeatmap", () => ({
  WeeklyHourlyHeatmap: ({
    metric,
    upstreamAccountId,
    timeseriesResponse,
  }: {
    metric?: string;
    upstreamAccountId?: number;
    timeseriesResponse?: { points?: unknown[] } | null;
  }) => (
    <div data-testid="heatmap-7d">
      {`metric:${metric ?? "unset"};account:${upstreamAccountId ?? "global"};points:${timeseriesResponse?.points?.length ?? 0}`}
    </div>
  ),
}));

vi.mock("./UsageCalendar", () => ({
  UsageCalendar: ({
    metric,
    showSurface,
    showMetricToggle,
    showMeta,
    upstreamAccountId,
    timeseriesResponse,
  }: {
    metric?: string;
    showSurface?: boolean;
    showMetricToggle?: boolean;
    showMeta?: boolean;
    upstreamAccountId?: number;
    timeseriesResponse?: { points?: unknown[] } | null;
  }) => (
    <div data-testid="usage-calendar">
      {`metric:${metric ?? "unset"};surface:${String(showSurface)};toggle:${String(showMetricToggle)};meta:${String(showMeta)};account:${upstreamAccountId ?? "global"};points:${timeseriesResponse?.points?.length ?? 0}`}
    </div>
  ),
}));

const storage = new Map<string, string>();
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => {
    storage.set(key, value);
  },
  removeItem: (key: string) => {
    storage.delete(key);
  },
  clear: () => {
    storage.clear();
  },
};

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createSummaryStore() {
  const values = new Map<
    string,
    {
      summary: Record<string, unknown> | null;
      isLoading: boolean;
      error: string | null;
    }
  >();
  const listeners = new Set<() => void>();

  const getFallback = () => ({
    summary: null,
    isLoading: false,
    error: null,
  });

  return {
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getSnapshot(window: string) {
      return values.get(window) ?? getFallback();
    },
    set(
      window: string,
      value: {
        summary: Record<string, unknown> | null;
        isLoading: boolean;
        error: string | null;
      },
    ) {
      values.set(window, value);
      listeners.forEach((listener) => listener());
    },
    reset() {
      values.clear();
      listeners.clear();
    },
  };
}

const summaryStore = createSummaryStore();

function useSummaryStoreValue(window: string) {
  return useSyncExternalStore(
    (listener) => summaryStore.subscribe(listener),
    () => summaryStore.getSnapshot(window),
    () => summaryStore.getSnapshot(window),
  );
}

beforeAll(() => {
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
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
  window.localStorage.clear();
  componentState.chartRenderCount = 0;
  summaryStore.reset();
  sseState.listeners.clear();
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

function buildParallelWorkStatsFixture(avgCount = 2) {
  return {
    current: {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:06:00",
      bucketSeconds: 60,
      completeBucketCount: 2,
      activeBucketCount: 2,
      minCount: 1,
      maxCount: 3,
      avgCount,
      points: [
        { bucketStart: "2026-04-08 00:04:00", bucketEnd: "2026-04-08 00:05:00", parallelCount: 1 },
        { bucketStart: "2026-04-08 00:05:00", bucketEnd: "2026-04-08 00:06:00", parallelCount: 3 },
      ],
    },
    minute7d: {},
    hour30d: {},
    dayAll: {},
  };
}

function installSummaryMocks() {
  summaryStore.set("today", {
    summary: {
      totalCount: 12,
      successCount: 10,
      failureCount: 2,
      totalCost: 0.52,
      totalTokens: 2048,
      inProgressConversationCount: 11,
      inProgressRetryConversationCount: 3,
      inProgressAvgWaitMs: 1700,
      nonSuccessCost: 0.12,
      nonSuccessTokens: 256,
    },
    isLoading: false,
    error: null,
  });
  summaryStore.set("yesterday", {
    summary: {
      totalCount: 8,
      successCount: 7,
      failureCount: 1,
      totalCost: 0.21,
      totalTokens: 1024,
      inProgressConversationCount: 4,
      inProgressRetryConversationCount: 1,
      inProgressAvgWaitMs: 900,
      nonSuccessCost: 0.05,
      nonSuccessTokens: 96,
    },
    isLoading: false,
    error: null,
  });
  summaryStore.set("1d", {
    summary: { totalCount: 100, inProgressConversationCount: 6 },
    isLoading: false,
    error: null,
  });
  summaryStore.set("7d", {
    summary: { totalCount: 700, inProgressConversationCount: 7 },
    isLoading: false,
    error: null,
  });
  summaryStore.set("previous7d", {
    summary: {
      totalCount: 70,
      successCount: 66,
      failureCount: 4,
      totalCost: 1.4,
      totalTokens: 7000,
      inProgressConversationCount: 5,
    },
    isLoading: false,
    error: null,
  });

  hookMocks.useSummary.mockImplementation(useSummaryStoreValue);

  hookMocks.useTimeseries.mockReturnValue({
    data: {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:06:00",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:01:00",
          bucketEnd: "2026-04-08 00:02:00",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 600,
          totalCost: 0.06,
        },
        {
          bucketStart: "2026-04-08 00:02:00",
          bucketEnd: "2026-04-08 00:03:00",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 800,
          totalCost: 0.08,
        },
        {
          bucketStart: "2026-04-08 00:03:00",
          bucketEnd: "2026-04-08 00:04:00",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 1000,
          totalCost: 0.1,
        },
        {
          bucketStart: "2026-04-08 00:04:00",
          bucketEnd: "2026-04-08 00:05:00",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 1200,
          totalCost: 0.12,
        },
        {
          bucketStart: "2026-04-08 00:05:00",
          bucketEnd: "2026-04-08 00:06:00",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 1400,
          totalCost: 0.14,
        },
      ],
    },
    isLoading: false,
    error: null,
  });
  hookMocks.useParallelWorkStats.mockReturnValue({
    data: buildParallelWorkStatsFixture(),
    isLoading: false,
    error: null,
  });
  hookMocks.useDashboardNetworkTimeseries.mockReturnValue({
    data: {
      range: "today",
      rangeStart: "2026-04-08T00:00:00Z",
      rangeEnd: "2026-04-08T00:10:00Z",
      snapshotId: 99,
      bucketSeconds: 300,
      points: [
        {
          bucketStart: "2026-04-08T00:00:00Z",
          bucketEnd: "2026-04-08T00:05:00Z",
          uploadBytesPerSecond: 1200,
          downloadBytesPerSecond: 5400,
          uploadBytes: 360000,
          downloadBytes: 1620000,
          isLiveBucket: false,
        },
        {
          bucketStart: "2026-04-08T00:05:00Z",
          bucketEnd: "2026-04-08T00:10:00Z",
          uploadBytesPerSecond: 1600,
          downloadBytesPerSecond: 6800,
          uploadBytes: 480000,
          downloadBytes: 2040000,
          isLiveBucket: true,
        },
      ],
    },
    isLoading: false,
    isRefreshing: false,
    error: null,
    reload: vi.fn(),
  });
}

function buildCachedSnapshotBundle(
  range: "today" | "1d" | "7d" | "usage",
  overrides: Partial<{
    dashboardActivity: Record<string, unknown>;
    summary: Record<string, unknown>;
    timeseries: { points?: unknown[] };
  }> = {},
) {
  return {
    range,
    dashboardActivity:
      range === "usage"
        ? null
        : ({
            range,
            rangeStart: "2026-07-17T00:00:00.000Z",
            rangeEnd: "2026-07-17T12:00:00.000Z",
            snapshotId: 1,
            rateWindow: {
              start: "2026-07-17T11:55:00.000Z",
              end: "2026-07-17T12:00:00.000Z",
              windowMinutes: 5,
              mode: "last_complete_5m_sma",
            },
            summary: {
              stats: {
                totalCount: 42,
                inProgressConversationCount: 6,
                inProgressRetryConversationCount: 2,
                inProgressAvgWaitMs: 1800,
                nonSuccessCost: 0.13,
                nonSuccessTokens: 320,
              },
              tokensPerMinute: 512,
              spendRate: 0.31,
            },
            ...overrides.dashboardActivity,
          } as const),
    summary: overrides.summary ?? { totalCount: 420 },
    comparisonSummary: { totalCount: 21 },
    previous7dSummary: { totalCount: 70 },
    comparisonTimeseries: {
      rangeStart: "2026-07-16T00:00:00.000Z",
      rangeEnd: "2026-07-16T12:00:00.000Z",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "a",
          bucketEnd: "b",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 12,
          totalCost: 0.01,
        },
      ],
    },
    parallelWorkStats: {
      current: { avgCount: 4 },
    },
    comparisonParallelWorkStats: {
      current: { avgCount: 2 },
    },
    networkTimeseries: {
      range,
      rangeStart: "2026-07-17T00:00:00.000Z",
      rangeEnd: "2026-07-17T12:00:00.000Z",
      snapshotId: 7,
      bucketSeconds: 300,
      points: [
        {
          bucketStart: "a",
          bucketEnd: "b",
          uploadBytesPerSecond: 12,
          downloadBytesPerSecond: 24,
          uploadBytes: 120,
          downloadBytes: 240,
          isLiveBucket: false,
        },
      ],
    },
    timeseries: {
      rangeStart: "2026-07-17T00:00:00.000Z",
      rangeEnd: "2026-07-17T12:00:00.000Z",
      bucketSeconds: range === "7d" ? 3600 : range === "usage" ? 86400 : 60,
      points: overrides.timeseries?.points ?? [
        {
          bucketStart: "2026-07-17T11:55:00.000Z",
          bucketEnd: "2026-07-17T12:00:00.000Z",
          totalCount: 3,
          successCount: 3,
          failureCount: 0,
          totalTokens: 120,
          totalCost: 0.12,
        },
      ],
    },
  };
}

function clickTab(label: string) {
  const button = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (candidate) => candidate.textContent === label,
  );
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing tab button: ${label}`);
  }
  act(() => {
    button.click();
  });
}

function getFirstSeenSummaryWindows() {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const [window] of hookMocks.useSummary.mock.calls) {
    if (typeof window !== "string" || seen.has(window)) continue;
    seen.add(window);
    ordered.push(window);
  }
  return ordered;
}

function emitSse(payload: unknown) {
  for (const listener of [...sseState.listeners]) {
    listener(payload);
  }
}

describe("DashboardActivityOverview", () => {
  it("uses a dashboard activity snapshot for the visible top KPI summary overlay without remounting duplicate today summary fetches", () => {
    installSummaryMocks();

    render(
      <DashboardActivityOverview
        dashboardActivity={{
          range: "today",
          rangeStart: "2026-07-05T00:00:00Z",
          rangeEnd: "2026-07-05T12:00:00Z",
          snapshotId: 1783233600000,
          rateWindow: {
            start: "2026-07-05T11:59:00Z",
            end: "2026-07-05T12:00:00Z",
            windowMinutes: 1,
            mode: "rolling_60s_live_mean",
          },
          summary: {
            stats: {
              totalCount: 21,
              successCount: 18,
              failureCount: 2,
              totalCost: 0.42,
              totalTokens: 4200,
              usageBreakdown: {
                total: { cacheWriteTokens: 3200 },
                models: [],
              },
              inProgressConversationCount: 7,
              inProgressRetryConversationCount: 1,
              inProgressAvgWaitMs: 2500,
              nonSuccessCost: 0.04,
              nonSuccessTokens: 300,
            },
            tokensPerMinute: 1234,
            spendRate: 0.45,
            currentFirstResponseByteTotalAvgMs: 1500,
            currentAvgTotalMs: 2400,
          },
        }}
      />,
    );

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      "total:21;cacheWrite:3200;inProgress:7;retry:1;wait:2500;nonSuccessCost:0.04;nonSuccessTokens:300;surface:false;header:false;badge:false;tpm:1234;spendRate:0.45;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true",
    );
    expect(hookMocks.useSummary.mock.calls.map(([window]) => window)).not.toContain("today");

    act(() => {
      emitSse({
        type: "summary",
        window: "today",
        summary: {
          totalCount: 34,
          successCount: 29,
          failureCount: 3,
          totalCost: 0.66,
          totalTokens: 6600,
          inProgressConversationCount: 9,
          inProgressRetryConversationCount: 2,
          inProgressAvgWaitMs: 1100,
          nonSuccessCost: 0.08,
          nonSuccessTokens: 420,
        },
      });
    });

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      "total:21;cacheWrite:3200;inProgress:7;retry:1;wait:2500;nonSuccessCost:0.04;nonSuccessTokens:300;surface:false;header:false;badge:false;tpm:1234;spendRate:0.45;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true",
    );
  });

  it("skips the duplicate yesterday summary hook when a snapshot-backed yesterday panel is visible", () => {
    installSummaryMocks();

    render(
      <DashboardActivityOverview
        dashboardActivity={{
          range: "yesterday",
          rangeStart: "2026-07-04T00:00:00Z",
          rangeEnd: "2026-07-05T00:00:00Z",
          snapshotId: 1783147200000,
          rateWindow: {
            start: "2026-07-04T23:59:00Z",
            end: "2026-07-05T00:00:00Z",
            windowMinutes: 1,
            mode: "last_complete_1m_sma",
          },
          summary: {
            stats: {
              totalCount: 9,
              successCount: 8,
              failureCount: 1,
              totalCost: 0.18,
              totalTokens: 1800,
              inProgressConversationCount: 0,
              inProgressRetryConversationCount: 0,
              inProgressAvgWaitMs: 0,
              nonSuccessCost: 0.02,
              nonSuccessTokens: 120,
            },
            tokensPerMinute: 88,
            spendRate: 0.09,
          },
        }}
        activeRange="yesterday"
      />,
    );

    expect(hookMocks.useSummary.mock.calls.map(([window]) => window)).not.toContain("yesterday");
  });

  it("loads only the active range and keeps per-range metric memory across all five tabs", () => {
    installSummaryMocks();

    render(<DashboardActivityOverview />);

    expect(host?.textContent).toContain("Activity Overview");
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d"]);
    expect(
      hookMocks.useTimeseries.mock.calls.every(
        ([window]) => window === "today" || window === "yesterday",
      ),
    ).toBe(true);
    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-today"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')).toBeNull();
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')).toBeNull();
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')).toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      "total:12;cacheWrite:null;inProgress:11;retry:3;wait:1700;nonSuccessCost:0.12;nonSuccessTokens:256;surface:false;header:false;badge:false;tpm:null;spendRate:null;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true",
    );
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalCount");
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull();

    const mobileSelects = host?.querySelector('[data-testid="dashboard-activity-mobile-selects"]');
    const rangeSelect = host?.querySelector('[data-testid="dashboard-activity-range-select"]');
    const metricSelect = host?.querySelector('[data-testid="dashboard-activity-metric-select"]');
    expect(mobileSelects?.className).toContain("grid-cols-2");
    expect(mobileSelects?.className).toContain("min-[769px]:hidden");
    expect(rangeSelect?.getAttribute("aria-label")).toBe("Switch activity range");
    expect(metricSelect?.getAttribute("aria-label")).toBe("Switch metric");

    clickTab("Cost");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalCost");

    clickTab("Yesterday");
    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-yesterday"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalCount");
    clickTab("Tokens");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalTokens");
    clickTab("Trend");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:trend");

    clickTab("History");
    expect(
      hookMocks.useSummary.mock.calls.every(
        ([window]) => window === "today" || window === "yesterday" || window === "previous7d",
      ),
    ).toBe(true);
    expect(
      hookMocks.useTimeseries.mock.calls.every(
        ([window]) => window === "today" || window === "yesterday",
      ),
    ).toBe(true);
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      "metric:totalCount;surface:false;toggle:false;meta:false;account:global;points:0",
    );
    clickTab("Tokens");
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      "metric:totalTokens;surface:false;toggle:false;meta:false;account:global;points:0",
    );

    clickTab("7 Days");
    expect(
      Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).some(
        (button) => button.textContent === "Trend",
      ),
    ).toBe(false);
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe("total:700");
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      "metric:totalCount;account:global;points:0",
    );
    clickTab("Cost");
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      "metric:totalCost;account:global;points:0",
    );

    clickTab("24 Hours");
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe("total:100");
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe(
      "metric:totalCount;account:global;points:0",
    );
    clickTab("Tokens");
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe(
      "metric:totalTokens;account:global;points:0",
    );

    clickTab("Today");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalCost");
    clickTab("History");
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      "metric:totalTokens;surface:false;toggle:false;meta:false;account:global;points:0",
    );
    clickTab("Yesterday");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:trend");
    clickTab("7 Days");
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      "metric:totalCost;account:global;points:0",
    );
    clickTab("24 Hours");
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe(
      "metric:totalTokens;account:global;points:0",
    );
  });

  it("shows the network metric only on today, yesterday, and 24 hours, and switches those ranges to the network chart", () => {
    installSummaryMocks();

    render(<DashboardActivityOverview />);

    clickTab("Network");
    expect(
      host?.querySelector('[data-testid="dashboard-network-activity-chart-mock"]')?.textContent,
    ).toBe("points:2;loading:false;error:null");
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')).toBeNull();
    expect(hookMocks.useDashboardNetworkTimeseries).toHaveBeenCalledWith("today", true, undefined);

    clickTab("24 Hours");
    expect(host?.textContent).toContain("Network");
    clickTab("Network");
    expect(
      host?.querySelector('[data-testid="dashboard-network-activity-chart-mock"]')?.textContent,
    ).toBe("points:2;loading:false;error:null");
    expect(host?.querySelector('[data-testid="heatmap-24h"]')).toBeNull();
    expect(hookMocks.useDashboardNetworkTimeseries).toHaveBeenCalledWith("1d", true, undefined);

    clickTab("7 Days");
    expect(host?.textContent).not.toContain("Trend");
    expect(host?.textContent).not.toContain("Network");
    expect(hookMocks.useDashboardNetworkTimeseries).toHaveBeenCalledWith("1d", false, undefined);
  });

  it("does not surface the chart refreshing chrome when network data is already hydrated", () => {
    installSummaryMocks();
    hookMocks.useDashboardNetworkTimeseries.mockReturnValue({
      data: {
        range: "today",
        rangeStart: "2026-04-08T00:00:00Z",
        rangeEnd: "2026-04-08T00:10:00Z",
        snapshotId: 100,
        bucketSeconds: 300,
        points: [
          {
            bucketStart: "2026-04-08T00:05:00Z",
            bucketEnd: "2026-04-08T00:10:00Z",
            uploadBytesPerSecond: 1600,
            downloadBytesPerSecond: 6800,
            uploadBytes: 480000,
            downloadBytes: 2040000,
            isLiveBucket: true,
          },
        ],
      },
      isLoading: false,
      isRefreshing: true,
      error: null,
      reload: vi.fn(),
    });

    render(<DashboardActivityOverview />);
    clickTab("Network");

    expect(
      host?.querySelector('[data-testid="dashboard-network-activity-chart-mock"]')?.textContent,
    ).toBe("points:1;loading:false;error:null");
  });

  it("restores the last active range from localStorage and falls back to today on invalid values", () => {
    installSummaryMocks();
    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, "usage");

    render(<DashboardActivityOverview />);

    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-usage"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(host?.querySelector('[data-testid="usage-calendar"]')).not.toBeNull();
    expect(hookMocks.useSummary).not.toHaveBeenCalled();
    expect(hookMocks.useTimeseries).not.toHaveBeenCalled();

    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;

    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, "bogus");
    render(<DashboardActivityOverview />);
    expect(
      document.body
        .querySelector('[data-testid="dashboard-activity-range-today"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(hookMocks.useSummary).toHaveBeenCalledWith("today", undefined);
  });

  it("uses account-scoped fetch options and storage without changing dashboard storage", () => {
    installSummaryMocks();
    const storageKey = `${ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX}.42`;
    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, "usage");

    render(
      <DashboardActivityOverview
        title="Account activity"
        storageKey={storageKey}
        testId="account-activity-overview"
        upstreamAccountId={42}
      />,
    );

    expect(host?.querySelector('[data-testid="account-activity-overview"]')?.textContent).toContain(
      "Account activity",
    );
    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-today"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)).toBe("usage");
    expect(window.localStorage.getItem(storageKey)).toBe("today");
    expect(hookMocks.useSummary).toHaveBeenCalledWith("today", {
      upstreamAccountId: 42,
    });
    expect(hookMocks.useTimeseries).toHaveBeenCalledWith("today", {
      bucket: "1m",
      upstreamAccountId: 42,
    });
    expect(hookMocks.useParallelWorkStats).toHaveBeenCalledWith({
      range: "today",
      bucket: "1m",
      upstreamAccountId: 42,
    });
    expect(hookMocks.useParallelWorkStats).toHaveBeenCalledWith({
      range: "yesterday",
      bucket: "1m",
      upstreamAccountId: 42,
    });
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "retry:3;wait:1700;nonSuccessCost:0.12;nonSuccessTokens:256;",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "parallelAvg:2;parallelError:null;showInProgress:true",
    );

    clickTab("7 Days");

    expect(window.localStorage.getItem(storageKey)).toBe("7d");
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      "metric:totalCount;account:42;points:0",
    );
    expect(hookMocks.useSummary).toHaveBeenCalledWith("7d", {
      upstreamAccountId: 42,
    });
  });

  it("does not request duplicate yesterday comparison data for account-scoped yesterday view", () => {
    installSummaryMocks();
    const storageKey = `${ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX}.42`;
    window.localStorage.setItem(storageKey, "yesterday");

    render(
      <DashboardActivityOverview
        title="Account activity"
        storageKey={storageKey}
        testId="account-activity-overview"
        upstreamAccountId={42}
      />,
    );

    const yesterdayCalls = hookMocks.useSummary.mock.calls.filter(
      ([window, options]) =>
        window === "yesterday" &&
        options != null &&
        typeof options === "object" &&
        "upstreamAccountId" in options &&
        options.upstreamAccountId === 42,
    );
    expect(yesterdayCalls).toHaveLength(1);

    const yesterdayTimeseriesCalls = hookMocks.useTimeseries.mock.calls.filter(
      ([window, options]) =>
        window === "yesterday" &&
        options != null &&
        typeof options === "object" &&
        "upstreamAccountId" in options &&
        options.upstreamAccountId === 42,
    );
    expect(yesterdayTimeseriesCalls).toHaveLength(1);
  });

  it("loads each summary only after its range is selected", () => {
    installSummaryMocks();

    render(<DashboardActivityOverview />);
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d"]);

    clickTab("Yesterday");
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d"]);

    clickTab("7 Days");
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d", "7d"]);

    clickTab("24 Hours");
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d", "7d", "1d"]);

    clickTab("History");
    expect(getFirstSeenSummaryWindows()).toEqual(["today", "yesterday", "previous7d", "7d", "1d"]);
  });

  it("does not rerender the today chart when only the summary hook updates", () => {
    installSummaryMocks();

    render(<DashboardActivityOverview />);

    expect(componentState.chartRenderCount).toBe(1);
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "total:12",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "inProgress:11",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "retry:3",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "wait:1700",
    );

    act(() => {
      summaryStore.set("today", {
        summary: {
          totalCount: 18,
          successCount: 15,
          failureCount: 3,
          totalCost: 0.66,
          totalTokens: 2600,
          inProgressConversationCount: 9,
          inProgressRetryConversationCount: 4,
          inProgressAvgWaitMs: 2200,
          nonSuccessCost: 0.2,
          nonSuccessTokens: 512,
        },
        isLoading: false,
        error: null,
      });
    });

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "total:18",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "inProgress:9",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "retry:4",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "wait:2200",
    );
    expect(componentState.chartRenderCount).toBe(1);
    expect(
      host
        ?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')
        ?.getAttribute("data-render-count"),
    ).toBe("1");
  });

  it("keeps trend comparison data visible when the comparison parallel request fails", () => {
    installSummaryMocks();
    hookMocks.useParallelWorkStats.mockImplementation(({ range }: { range: string }) => {
      if (range === "yesterday") {
        return {
          data: null,
          isLoading: false,
          error: "comparison unavailable",
        };
      }
      return {
        data: buildParallelWorkStatsFixture(2),
        isLoading: false,
        error: null,
      };
    });

    render(<DashboardActivityOverview />);

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "parallelAvg:2;parallelError:null",
    );
  });

  it("renders the cached-offline banner and snapshot-backed today overview without live hooks", () => {
    installSummaryMocks();

    render(
      <DashboardActivityOverview
        snapshotStatus={{
          mode: "cached-offline",
          cachedAt: "2026-07-17T05:20:00.000Z",
          readyRanges: ["today", "1d", "7d", "usage"],
        }}
        snapshotBundle={buildCachedSnapshotBundle("today")}
      />,
    );

    expect(
      host?.querySelector('[data-testid="dashboard-overview-snapshot-banner"]'),
    ).not.toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "total:42",
    );
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      "tpm:512;spendRate:0.31",
    );
    expect(hookMocks.useSummary).not.toHaveBeenCalled();
    expect(hookMocks.useTimeseries).not.toHaveBeenCalled();
    expect(hookMocks.useParallelWorkStats).not.toHaveBeenCalled();
  });

  it("injects cached timeseries into 24 hour, 7 day, and usage snapshot panels", () => {
    installSummaryMocks();

    render(
      <DashboardActivityOverview
        activeRange="1d"
        snapshotStatus={{
          mode: "cached-offline",
          cachedAt: "2026-07-17T05:20:00.000Z",
          readyRanges: ["today", "1d", "7d", "usage"],
        }}
        snapshotBundle={buildCachedSnapshotBundle("1d", {
          summary: { totalCount: 144 },
          timeseries: { points: [{ bucketStart: "a", bucketEnd: "b", totalCount: 1 }] },
        })}
      />,
    );

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe("total:42");
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toContain("points:1");

    act(() => {
      root?.render(
        <DashboardActivityOverview
          activeRange="7d"
          snapshotStatus={{
            mode: "cached-offline",
            cachedAt: "2026-07-17T05:20:00.000Z",
            readyRanges: ["today", "1d", "7d", "usage"],
          }}
          snapshotBundle={buildCachedSnapshotBundle("7d", {
            timeseries: { points: [{ bucketStart: "a", bucketEnd: "b", totalCount: 1 }] },
          })}
        />,
      );
    });

    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toContain("points:1");

    act(() => {
      root?.render(
        <DashboardActivityOverview
          activeRange="usage"
          snapshotStatus={{
            mode: "cached-offline",
            cachedAt: "2026-07-17T05:20:00.000Z",
            readyRanges: ["today", "1d", "7d", "usage"],
          }}
          snapshotBundle={buildCachedSnapshotBundle("usage", {
            timeseries: { points: [{ bucketStart: "a", bucketEnd: "b", totalCount: 1 }] },
          })}
        />,
      );
    });

    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toContain(
      "points:1",
    );
  });

  it("surfaces a not-cached-yet state instead of live loading when the active range has no offline snapshot", () => {
    installSummaryMocks();

    render(
      <DashboardActivityOverview
        activeRange="usage"
        snapshotStatus={{
          mode: "not-cached-yet",
          cachedAt: null,
          readyRanges: ["today"],
        }}
        snapshotBundle={null}
      />,
    );

    expect(host?.querySelector('[data-testid="dashboard-overview-snapshot-empty"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="usage-calendar"]')).toBeNull();
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull();
  });
});
