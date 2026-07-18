/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
  getDashboardPerformanceDiagnosticsSnapshot,
  publishWorkingConversationPatchMetrics,
  recordTodayChartRender,
  recordTodaySummaryRefresh,
  resetDashboardPerformanceDiagnostics,
} from "../lib/dashboardPerformanceDiagnostics";
import type { DashboardWorkingConversationCardModel } from "../lib/dashboardWorkingConversations";
import DashboardPage from "./Dashboard";

const hookMocks = vi.hoisted(() => ({
  useDashboardWorkingConversations: vi.fn(),
  useDashboardOverviewSnapshotRuntime: vi.fn(),
  useDashboardActivitySnapshot: vi.fn(),
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
  useParallelWorkStats: vi.fn(),
}));

vi.mock("../hooks/useDashboardWorkingConversations", () => ({
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX: 16,
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN: 4,
  useDashboardWorkingConversations: hookMocks.useDashboardWorkingConversations,
}));

vi.mock("../hooks/useDashboardOverviewSnapshotRuntime", () => ({
  useDashboardOverviewSnapshotRuntime: hookMocks.useDashboardOverviewSnapshotRuntime,
  default: hookMocks.useDashboardOverviewSnapshotRuntime,
}));

vi.mock("../hooks/useDashboardUpstreamAccountActivity", () => ({
  useDashboardActivitySnapshot: hookMocks.useDashboardActivitySnapshot,
}));

vi.mock("../hooks/useStats", () => ({
  useSummary: hookMocks.useSummary,
}));

vi.mock("../hooks/useTimeseries", () => ({
  useTimeseries: hookMocks.useTimeseries,
}));

vi.mock("../hooks/useParallelWorkStats", () => ({
  useParallelWorkStats: hookMocks.useParallelWorkStats,
}));

vi.mock("../features/dashboard/TodayStatsOverview", () => ({
  TodayStatsOverview: ({
    showSurface,
    showHeader,
    showDayBadge,
  }: {
    showSurface?: boolean;
    showHeader?: boolean;
    showDayBadge?: boolean;
  }) => (
    <div data-testid="today-stats-overview-mock">
      {`surface:${String(showSurface)};header:${String(showHeader)};badge:${String(showDayBadge)}`}
    </div>
  ),
}));

vi.mock("../features/dashboard/DashboardTodayActivityChart", () => ({
  DashboardTodayActivityChart: ({ metric }: { metric?: string }) => (
    <div data-testid="dashboard-today-activity-chart-mock">{`metric:${metric ?? "unset"}`}</div>
  ),
}));

vi.mock("../features/dashboard/UsageCalendar", () => ({
  UsageCalendar: ({
    metric,
    showSurface,
    showMetricToggle,
    showMeta,
  }: {
    metric?: string;
    showSurface?: boolean;
    showMetricToggle?: boolean;
    showMeta?: boolean;
  }) => (
    <div data-testid="usage-calendar">
      {`metric:${metric ?? "unset"};surface:${String(showSurface)};toggle:${String(showMetricToggle)};meta:${String(showMeta)}`}
    </div>
  ),
}));

vi.mock("../features/stats/StatsCards", () => ({
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

vi.mock("../features/dashboard/Last24hTenMinuteHeatmap", () => ({
  Last24hTenMinuteHeatmap: ({ metric, showHeader }: { metric?: string; showHeader?: boolean }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? "unset"};header:${String(showHeader)}`}
    </div>
  ),
}));

vi.mock("../features/dashboard/WeeklyHourlyHeatmap", () => ({
  WeeklyHourlyHeatmap: ({
    metric,
    showHeader,
    showSurface,
  }: {
    metric?: string;
    showHeader?: boolean;
    showSurface?: boolean;
  }) => (
    <div data-testid="heatmap-7d">
      {`metric:${metric ?? "unset"};header:${String(showHeader)};surface:${String(showSurface)}`}
    </div>
  ),
}));

vi.mock("../features/dashboard/DashboardWorkingConversationsSection", () => ({
  DashboardWorkingConversationsSection: ({
    cards,
    setRefreshTargetCount,
    onOpenUpstreamAccount,
    onOpenConversation,
    onOpenInvocation,
    upstreamAccountActivity,
  }: {
    cards: DashboardWorkingConversationCardModel[];
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: { tab?: "overview" | "routing" },
    ) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
      tab?: "overview" | "calls" | "settings" | "operations";
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
    upstreamAccountActivity?: {
      networkLiveBucket?: {
        uploadBytesPerSecond?: number;
        downloadBytesPerSecond?: number;
      } | null;
    } | null;
  }) => (
    <div data-testid="dashboard-working-conversations-section">
      {cards.map((card) => card.conversationSequenceId).join(",")}
      <span data-testid="dashboard-working-conversations-endpoints">
        {cards.map((card) => card.currentInvocation.preview.endpoint ?? "").join(",")}
      </span>
      <span data-testid="dashboard-working-conversations-network-live-bucket">
        {upstreamAccountActivity?.networkLiveBucket
          ? `${upstreamAccountActivity.networkLiveBucket.uploadBytesPerSecond ?? 0}/${upstreamAccountActivity.networkLiveBucket.downloadBytesPerSecond ?? 0}`
          : "missing"}
      </span>
      {cards[0] ? (
        <>
          <button
            type="button"
            data-testid="dashboard-set-refresh-target"
            onClick={() => setRefreshTargetCount?.(24)}
          >
            set refresh target
          </button>
          <button
            type="button"
            data-testid="dashboard-open-conversation"
            onClick={() =>
              onOpenConversation?.({
                conversationSequenceId: cards[0].conversationSequenceId,
                promptCacheKey: cards[0].promptCacheKey,
              })
            }
          >
            open conversation
          </button>
          <button
            type="button"
            data-testid="dashboard-open-conversation-settings"
            onClick={() =>
              onOpenConversation?.({
                conversationSequenceId: cards[0].conversationSequenceId,
                promptCacheKey: cards[0].promptCacheKey,
                tab: "settings",
              })
            }
          >
            open conversation settings
          </button>
          <button
            type="button"
            data-testid="dashboard-open-invocation"
            onClick={() =>
              onOpenInvocation?.({
                slotKind: "current",
                conversationSequenceId: cards[0].conversationSequenceId,
                promptCacheKey: cards[0].promptCacheKey,
                invocation: cards[0].currentInvocation,
              })
            }
          >
            open invocation
          </button>
          <button
            type="button"
            data-testid="dashboard-open-account"
            onClick={() => onOpenUpstreamAccount?.(77, "section-account@example.com")}
          >
            open account
          </button>
          <button
            type="button"
            data-testid="dashboard-open-account-routing"
            onClick={() =>
              onOpenUpstreamAccount?.(77, "section-account@example.com", {
                tab: "routing",
              })
            }
          >
            open account routing
          </button>
        </>
      ) : null}
    </div>
  ),
}));

vi.mock("../features/prompt-cache/PromptCacheConversationTable", () => ({
  PromptCacheConversationHistoryDrawer: ({
    open,
    conversationKey,
    conversationLabel,
    initialTab,
    onClose,
    onOpenUpstreamAccount,
  }: {
    open: boolean;
    conversationKey: string | null;
    conversationLabel?: string | null;
    initialTab?: "overview" | "calls" | "settings" | "operations";
    onClose: () => void;
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: { tab?: "overview" | "routing" },
    ) => void;
  }) =>
    open ? (
      <div data-testid="dashboard-conversation-history-drawer-mock">
        <span data-testid="dashboard-conversation-drawer-key">{conversationKey}</span>
        <span data-testid="dashboard-conversation-drawer-label">{conversationLabel}</span>
        <span data-testid="dashboard-conversation-drawer-tab">{initialTab ?? "overview"}</span>
        <button type="button" data-testid="dashboard-conversation-drawer-close" onClick={onClose}>
          close conversation drawer
        </button>
        <button
          type="button"
          data-testid="dashboard-conversation-drawer-open-account"
          onClick={() => onOpenUpstreamAccount?.(99, "conversation-account@example.com")}
        >
          open account from conversation drawer
        </button>
      </div>
    ) : null,
}));

vi.mock("../features/dashboard/DashboardInvocationDetailDrawer", () => ({
  DashboardInvocationDetailDrawer: ({
    open,
    invocationId,
    selection,
    onClose,
    onOpenUpstreamAccount,
  }: {
    open: boolean;
    invocationId?: string | null;
    selection: { invocation: { record: { invokeId: string } } } | null;
    onClose: () => void;
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: { tab?: "overview" | "routing" },
    ) => void;
  }) =>
    open ? (
      <div data-testid="dashboard-invocation-detail-drawer-mock">
        <span data-testid="dashboard-invocation-drawer-selection">
          {selection?.invocation.record.invokeId ?? "none"}
        </span>
        <span data-testid="dashboard-invocation-drawer-route-id">{invocationId ?? "none"}</span>
        <button type="button" data-testid="dashboard-invocation-drawer-close" onClick={onClose}>
          close invocation drawer
        </button>
        <button
          type="button"
          data-testid="dashboard-invocation-drawer-open-account"
          onClick={() => onOpenUpstreamAccount?.(88, "drawer-account@example.com")}
        >
          open account from invocation drawer
        </button>
      </div>
    ) : null,
}));

vi.mock("./account-pool/UpstreamAccounts", () => ({
  SharedUpstreamAccountDetailDrawer: ({
    open,
    accountId,
    initialTab,
    onClose,
  }: {
    open: boolean;
    accountId: number | null;
    initialTab?: "overview" | "routing";
    onClose: () => void;
  }) =>
    open ? (
      <div data-testid="shared-upstream-account-detail-drawer-mock">
        <span data-testid="shared-upstream-account-drawer-account-id">{accountId}</span>
        <span data-testid="shared-upstream-account-drawer-tab">{initialTab ?? "overview"}</span>
        <button type="button" data-testid="shared-upstream-account-drawer-close" onClick={onClose}>
          close account drawer
        </button>
      </div>
    ) : null,
}));

vi.mock("../theme", () => ({
  useTheme: () => ({ themeMode: "light" }),
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string) => {
      const map: Record<string, string> = {
        "dashboard.activityOverview.title": "活动总览",
        "dashboard.activityOverview.rangeToday": "今日",
        "dashboard.activityOverview.rangeYesterday": "昨日",
        "dashboard.activityOverview.range24h": "24 小时",
        "dashboard.activityOverview.range7d": "7 日",
        "dashboard.activityOverview.rangeUsage": "历史",
        "dashboard.activityOverview.rangeToggleAria": "时间范围切换",
        "dashboard.section.workingConversationsTitle": "当前工作中的对话",
        "heatmap.metricsToggleAria": "指标切换",
        "metric.totalCount": "次数",
        "metric.totalCost": "金额",
        "metric.totalTokens": "Tokens",
      };
      return map[key] ?? key;
    },
  }),
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

function LocationProbe() {
  const location = useLocation();
  return (
    <>
      <div data-testid="dashboard-location-search">{location.search}</div>
      <div data-testid="dashboard-location-path">{location.pathname}</div>
    </>
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
  resetDashboardPerformanceDiagnostics();
  vi.useRealTimers();
  vi.clearAllMocks();
});

function render(ui: React.ReactNode, initialEntry = "/dashboard") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[initialEntry]}>
        <LocationProbe />
        {ui}
      </MemoryRouter>,
    );
  });
}

function installSnapshotRuntimeMock() {
  hookMocks.useDashboardOverviewSnapshotRuntime.mockReturnValue({
    status: {
      mode: "live",
      cachedAt: null,
      readyRanges: [],
    },
    bundle: null,
  });
}

function installSummaryMocks() {
  installSnapshotRuntimeMock();
  hookMocks.useDashboardActivitySnapshot.mockReturnValue({
    data: {
      range: "today",
      rangeStart: "2026-04-08T00:00:00.000Z",
      rangeEnd: "2026-04-08T00:03:00.000Z",
      snapshotId: 1,
      liveRevision: 7,
      rateWindow: {
        start: "2026-04-08T00:02:00.000Z",
        end: "2026-04-08T00:03:00.000Z",
        windowMinutes: 1,
        mode: "rolling_60s_live_mean",
      },
      summary: {
        stats: { totalCount: 12 },
        tokensPerMinute: 24,
        spendRate: 0.5,
        currentFirstResponseByteTotalAvgMs: 1200,
        currentAvgTotalMs: 2400,
        modelPerformance: {
          available: false,
          total: {
            tokensPerMinute: 0,
            streamingResponseRate: null,
            avgResponseMs: null,
            avgFirstResponseByteTotalMs: null,
            wallClockUsageDurationMs: null,
            cumulativeUsageDurationMs: null,
            parallelism: null,
          },
          models: [],
        },
      },
      networkLiveBucket: {
        bucketStart: "2026-04-08T00:00:00.000Z",
        bucketEnd: "2026-04-08T00:05:00.000Z",
        uploadBytesPerSecond: 2048,
        downloadBytesPerSecond: 4096,
        uploadBytes: 2048 * 300,
        downloadBytes: 4096 * 300,
        isLiveBucket: true,
      },
      accounts: [
        {
          accountKey: "upstream:77",
          upstreamAccountId: 77,
          displayName: "section-account@example.com",
          isUnassigned: false,
          requestCount: 1,
          successCount: 1,
          failureCount: 0,
          nonSuccessCount: 0,
          totalTokens: 120,
          successTokens: 120,
          nonSuccessTokens: 0,
          failureTokens: 0,
          failureCost: 0,
          totalCost: 0.01,
          usageBreakdown: {
            cacheWriteTokens: 0,
            cacheReadTokens: 0,
            outputTokens: 0,
            costs: null,
            models: [],
          },
          modelPerformance: null,
          cacheHitRate: null,
          tokensPerMinute: 24,
          spendRate: 0.5,
          firstByteAvgMs: null,
          firstResponseByteTotalAvgMs: null,
          avgTotalMs: null,
          currentFirstResponseByteTotalAvgMs: null,
          currentAvgTotalMs: null,
          inProgressInvocationCount: 0,
          inProgressPhaseCounts: {
            queued: 0,
            requesting: 0,
            responding: 0,
          },
          retryInvocationCount: 0,
          uploadBytesPerSecond: 128,
          downloadBytesPerSecond: 256,
          effectiveRoutingRule: {
            mode: "inherit_default",
            reason: null,
            targetGroupName: null,
            targetAccountId: null,
            targetAccountLabel: null,
            sourceLabel: null,
          },
          recentInvocations: [],
        },
      ],
    },
    isLoading: false,
    isRefreshing: false,
    recentLoading: false,
    recentError: null,
    error: null,
    recentInvocationLimit: 4,
    reload: vi.fn(),
    retryRecent: vi.fn(),
  });
  hookMocks.useSummary.mockImplementation((window: string) => {
    if (window === "today") {
      return { summary: { totalCount: 12 }, isLoading: false, error: null };
    }
    if (window === "yesterday") {
      return { summary: { totalCount: 8 }, isLoading: false, error: null };
    }
    if (window === "1d") {
      return { summary: { totalCount: 100 }, isLoading: false, error: null };
    }
    if (window === "7d") {
      return { summary: { totalCount: 700 }, isLoading: false, error: null };
    }
    if (window === "previous7d") {
      return { summary: { totalCount: 70 }, isLoading: false, error: null };
    }
    return { summary: null, isLoading: false, error: null };
  });

  hookMocks.useTimeseries.mockReturnValue({
    data: {
      rangeStart: "2026-04-08T00:00:00.000Z",
      rangeEnd: "2026-04-08T00:03:00.000Z",
      bucketSeconds: 60,
      points: [],
    },
    isLoading: false,
    error: null,
  });
  hookMocks.useParallelWorkStats.mockReturnValue({
    data: {
      current: {
        rangeStart: "2026-04-08T00:00:00.000Z",
        rangeEnd: "2026-04-08T00:03:00.000Z",
        bucketSeconds: 60,
        completeBucketCount: 0,
        activeBucketCount: 0,
        minCount: null,
        maxCount: null,
        avgCount: 0,
        points: [],
      },
      minute7d: {},
      hour30d: {},
      dayAll: {},
    },
    isLoading: false,
    error: null,
  });
}

function createWorkingConversationCard(options?: {
  endpoint?: string;
  upstreamAccountName?: string;
}): DashboardWorkingConversationCardModel {
  return {
    promptCacheKey: "pck-drawer-switch",
    normalizedPromptCacheKey: "pck-drawer-switch",
    conversationSequenceId: "WC-ABCD12",
    currentInvocation: {
      preview: {
        id: 101,
        invokeId: "invoke-dashboard-current",
        occurredAt: "2026-04-06T10:20:00Z",
        status: "completed",
        failureClass: null,
        routeMode: "forward_proxy",
        model: "gpt-5.4",
        totalTokens: 120,
        cost: 0.01,
        proxyDisplayName: "tokyo-edge-01",
        upstreamAccountId: 77,
        upstreamAccountName: options?.upstreamAccountName ?? "section-account@example.com",
        endpoint: options?.endpoint ?? "/v1/responses",
      },
      record: {
        id: 101,
        invokeId: "invoke-dashboard-current",
        occurredAt: "2026-04-06T10:20:00Z",
        createdAt: "2026-04-06T10:20:00Z",
        status: "completed",
        source: "proxy",
        routeMode: "forward_proxy",
        model: "gpt-5.4",
        totalTokens: 120,
      },
      displayStatus: "success",
      occurredAtEpoch: Date.parse("2026-04-06T10:20:00Z"),
      isInFlight: false,
      isTerminal: true,
      tone: "success",
    },
    previousInvocation: null,
    hasPreviousPlaceholder: true,
    createdAtEpoch: Date.parse("2026-04-06T10:20:00Z"),
    sortAnchorEpoch: Date.parse("2026-04-06T10:20:00Z"),
    lastTerminalAtEpoch: Date.parse("2026-04-06T10:20:00Z"),
    lastInFlightAtEpoch: null,
    tone: "success",
    requestCount: 1,
    totalTokens: 120,
    totalCost: 0.01,
  };
}

describe("DashboardPage", () => {
  it("opens the invocation drawer from a shareable route without card context", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [],
      totalMatched: 0,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />, "/dashboard/invocations/shared-invoke-42");

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-drawer-route-id"]')?.textContent,
    ).toBe("shared-invoke-42");
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-drawer-selection"]')?.textContent,
    ).toBe("none");

    const closeButton = host?.querySelector('[data-testid="dashboard-invocation-drawer-close"]');
    if (!(closeButton instanceof HTMLButtonElement)) throw new Error("missing close button");
    act(() => closeButton.click());
    expect(host?.querySelector('[data-testid="dashboard-location-path"]')?.textContent).toBe(
      "/dashboard",
    );
  });

  it("keeps today inside the shared overview card instead of as a standalone top card", () => {
    installSummaryMocks();
    const setRefreshTargetCount = vi.fn();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount,
    });

    render(<DashboardPage />);

    expect(host?.textContent).toContain("活动总览");
    expect(host?.querySelectorAll('[data-testid="dashboard-activity-overview"]')).toHaveLength(1);
    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-today"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      "surface:false;header:false;badge:false",
    );
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent,
    ).toBe("metric:totalCount");
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-working-conversations-section"]')?.textContent,
    ).toContain("WC-ABCD12");
    expect(
      host?.querySelector('[data-testid="dashboard-working-conversations-endpoints"]')?.textContent,
    ).toContain("/v1/responses");
    expect(
      host?.querySelector('[data-testid="dashboard-working-conversations-network-live-bucket"]')
        ?.textContent,
    ).toBe("2048/4096");

    const historyButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === "历史",
    );
    if (!(historyButton instanceof HTMLButtonElement)) {
      throw new Error("missing history range button");
    }

    act(() => {
      historyButton.click();
    });

    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      "metric:totalCount;surface:false;toggle:false;meta:false",
    );

    const yesterdayButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === "昨日",
    );
    if (!(yesterdayButton instanceof HTMLButtonElement)) {
      throw new Error("missing yesterday range button");
    }

    act(() => {
      yesterdayButton.click();
    });

    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-yesterday"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
  });

  it("keeps diagnostics hidden by default and exposes them when the debug flag is enabled", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    expect(host?.querySelector('[data-testid="dashboard-performance-diagnostics"]')).toBeNull();

    act(() => {
      window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
      resetDashboardPerformanceDiagnostics();
      publishWorkingConversationPatchMetrics(
        new Map([
          [
            "pck-drawer-switch",
            new Map([["invoke-dashboard-current", { totalTokens: 120, cost: 0.01 }]]),
          ],
        ]),
      );
      recordTodaySummaryRefresh("today");
      recordTodayChartRender();
    });

    expect(host?.querySelector('[data-testid="dashboard-performance-diagnostics"]')).not.toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-working-conversations-patch-bucket-count"]',
      )?.textContent,
    ).toBe("1");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-working-conversations-patch-entry-count"]',
      )?.textContent,
    ).toBe("1");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-today-summary-refresh-count"]',
      )?.textContent,
    ).toBe("1");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-today-chart-render-count"]',
      )?.textContent,
    ).toBe("1");
  });

  it("reacts to the debug toggle on an already-open dashboard", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    expect(host?.querySelector('[data-testid="dashboard-performance-diagnostics"]')).toBeNull();

    act(() => {
      window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
    });

    expect(host?.querySelector('[data-testid="dashboard-performance-diagnostics"]')).not.toBeNull();

    act(() => {
      window.localStorage.removeItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY);
    });

    expect(host?.querySelector('[data-testid="dashboard-performance-diagnostics"]')).toBeNull();
  });

  it("starts diagnostics counters from zero after enabling on a long-lived dashboard", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    act(() => {
      publishWorkingConversationPatchMetrics(
        new Map([["stale-pck", new Map([["stale-invoke", { totalTokens: 64, cost: 0.02 }]])]]),
      );
      recordTodaySummaryRefresh("today");
      recordTodayChartRender("stale-chart");
    });

    act(() => {
      window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
    });

    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-working-conversations-patch-bucket-count"]',
      )?.textContent,
    ).toBe("0");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-working-conversations-patch-entry-count"]',
      )?.textContent,
    ).toBe("0");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-today-summary-refresh-count"]',
      )?.textContent,
    ).toBe("0");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-performance-diagnostics-today-chart-render-count"]',
      )?.textContent,
    ).toBe("0");
  });

  it("dedupes identical chart render signatures in diagnostics", () => {
    window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
    resetDashboardPerformanceDiagnostics();

    act(() => {
      recordTodayChartRender("today:stable");
      recordTodayChartRender("today:stable");
      recordTodayChartRender("today:updated");
    });

    expect(getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount).toBe(2);
  });

  it("switches between conversation, invocation, and account drawers from dashboard interactions", () => {
    installSummaryMocks();
    const setRefreshTargetCount = vi.fn();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount,
    });

    render(<DashboardPage />);

    const openConversationButton = host?.querySelector(
      '[data-testid="dashboard-open-conversation"]',
    );
    if (!(openConversationButton instanceof HTMLButtonElement)) {
      throw new Error("missing conversation trigger");
    }

    act(() => {
      openConversationButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-conversation-drawer-key"]')?.textContent,
    ).toBe("pck-drawer-switch");
    expect(
      host?.querySelector('[data-testid="dashboard-conversation-drawer-label"]')?.textContent,
    ).toBe("ABCD12");
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull();

    const openInvocationButton = host?.querySelector('[data-testid="dashboard-open-invocation"]');
    if (!(openInvocationButton instanceof HTMLButtonElement)) {
      throw new Error("missing invocation trigger");
    }

    act(() => {
      openInvocationButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-drawer-selection"]')?.textContent,
    ).toBe("invoke-dashboard-current");
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-detail-drawer-mock"]'),
    ).toBeNull();

    const openAccountFromSectionButton = host?.querySelector(
      '[data-testid="dashboard-open-account"]',
    );
    if (!(openAccountFromSectionButton instanceof HTMLButtonElement)) {
      throw new Error("missing section account trigger");
    }

    act(() => {
      openAccountFromSectionButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe("77");
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-tab"]')?.textContent,
    ).toBe("overview");

    act(() => {
      openInvocationButton.click();
    });

    expect(
      host?.querySelector('[data-testid="shared-upstream-account-detail-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).not.toBeNull();

    act(() => {
      openConversationButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).not.toBeNull();

    const openAccountFromConversationDrawerButton = host?.querySelector(
      '[data-testid="dashboard-conversation-drawer-open-account"]',
    );
    if (!(openAccountFromConversationDrawerButton instanceof HTMLButtonElement)) {
      throw new Error("missing conversation drawer account trigger");
    }

    act(() => {
      openAccountFromConversationDrawerButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe("99");

    act(() => {
      openInvocationButton.click();
    });

    const openAccountFromInvocationDrawerButton = host?.querySelector(
      '[data-testid="dashboard-invocation-drawer-open-account"]',
    );
    if (!(openAccountFromInvocationDrawerButton instanceof HTMLButtonElement)) {
      throw new Error("missing invocation drawer account trigger");
    }

    act(() => {
      openAccountFromInvocationDrawerButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe("88");

    act(() => {
      openConversationButton.click();
    });

    const openAccountFromHistoryDrawerButton = host?.querySelector(
      '[data-testid="dashboard-conversation-drawer-open-account"]',
    );
    if (!(openAccountFromHistoryDrawerButton instanceof HTMLButtonElement)) {
      throw new Error("missing conversation history drawer account trigger");
    }

    act(() => {
      openAccountFromHistoryDrawerButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).toBeNull();
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe("99");
  });

  it("opens the shared drawer on the routing tab and keeps the tab in the URL", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    const openAccountRoutingButton = host?.querySelector(
      '[data-testid="dashboard-open-account-routing"]',
    );
    if (!(openAccountRoutingButton instanceof HTMLButtonElement)) {
      throw new Error("missing routing account trigger");
    }

    act(() => {
      openAccountRoutingButton.click();
    });

    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe("77");
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-tab"]')?.textContent,
    ).toBe("routing");
    expect(host?.querySelector('[data-testid="dashboard-location-search"]')?.textContent).toBe(
      "?upstreamAccountId=77&upstreamAccountTab=routing",
    );

    const closeAccountDrawerButton = host?.querySelector(
      '[data-testid="shared-upstream-account-drawer-close"]',
    );
    if (!(closeAccountDrawerButton instanceof HTMLButtonElement)) {
      throw new Error("missing account drawer close button");
    }

    act(() => {
      closeAccountDrawerButton.click();
    });

    expect(
      host?.querySelector('[data-testid="shared-upstream-account-detail-drawer-mock"]'),
    ).toBeNull();
    expect(host?.querySelector('[data-testid="dashboard-location-search"]')?.textContent).toBe("");
  });

  it("opens the conversation drawer directly on settings when requested by the badge route", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    const openConversationSettingsButton = host?.querySelector(
      '[data-testid="dashboard-open-conversation-settings"]',
    );
    if (!(openConversationSettingsButton instanceof HTMLButtonElement)) {
      throw new Error("missing conversation settings trigger");
    }

    act(() => {
      openConversationSettingsButton.click();
    });

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-conversation-drawer-tab"]')?.textContent,
    ).toBe("settings");
    expect(host?.querySelector('[data-testid="dashboard-location-search"]')?.textContent).toBe(
      "?promptCacheConversationKey=pck-drawer-switch&promptCacheConversationTab=settings",
    );
  });

  it("keeps the settings-tab conversation route when the dashboard switches into compact page mode", () => {
    const previousMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: true,
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });

    try {
      installSummaryMocks();
      hookMocks.useDashboardWorkingConversations.mockReturnValue({
        cards: [createWorkingConversationCard()],
        totalMatched: 1,
        hasMore: false,
        isLoading: false,
        isLoadingMore: false,
        error: null,
        loadMore: vi.fn(),
        setRefreshTargetCount: vi.fn(),
      });

      render(<DashboardPage />);

      const openConversationSettingsButton = host?.querySelector(
        '[data-testid="dashboard-open-conversation-settings"]',
      );
      if (!(openConversationSettingsButton instanceof HTMLButtonElement)) {
        throw new Error("missing compact conversation settings trigger");
      }

      act(() => {
        openConversationSettingsButton.click();
      });

      expect(
        host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
      ).not.toBeNull();
      expect(
        host?.querySelector('[data-testid="dashboard-conversation-drawer-tab"]')?.textContent,
      ).toBe("settings");
      expect(host?.querySelector('[data-testid="dashboard-location-search"]')?.textContent).toBe(
        "?promptCacheConversationKey=pck-drawer-switch&promptCacheConversationTab=settings",
      );
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        writable: true,
        value: previousMatchMedia,
      });
    }
  });

  it("opens the operations-tab conversation route from the location search params", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(
      <DashboardPage />,
      "/dashboard?promptCacheConversationKey=pck-drawer-switch&promptCacheConversationTab=operations",
    );

    expect(
      host?.querySelector('[data-testid="dashboard-conversation-history-drawer-mock"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-conversation-drawer-tab"]')?.textContent,
    ).toBe("operations");
    expect(host?.querySelector('[data-testid="dashboard-location-search"]')?.textContent).toBe(
      "?promptCacheConversationKey=pck-drawer-switch&promptCacheConversationTab=operations",
    );
  });

  it("passes refresh target updates from the working conversations section back into the hook", () => {
    installSummaryMocks();
    const setRefreshTargetCount = vi.fn();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount,
    });

    render(<DashboardPage />);

    const button = host?.querySelector('[data-testid="dashboard-set-refresh-target"]');
    if (!(button instanceof HTMLButtonElement)) {
      throw new Error("missing refresh target trigger");
    }

    act(() => {
      button.click();
    });

    expect(setRefreshTargetCount).toHaveBeenCalledWith(24);
  });

  it("passes compact endpoint previews into the working conversations section", () => {
    installSummaryMocks();
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [
        createWorkingConversationCard({
          endpoint: "/v1/responses/compact",
          upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        }),
      ],
      totalMatched: 1,
      hasMore: false,
      isLoading: false,
      isLoadingMore: false,
      error: null,
      loadMore: vi.fn(),
      setRefreshTargetCount: vi.fn(),
    });

    render(<DashboardPage />);

    expect(
      host?.querySelector('[data-testid="dashboard-working-conversations-endpoints"]')?.textContent,
    ).toContain("/v1/responses/compact");
  });
});
