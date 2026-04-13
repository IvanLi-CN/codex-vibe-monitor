/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DashboardWorkingConversationCardModel } from "../lib/dashboardWorkingConversations";
import {
  DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
  getDashboardPerformanceDiagnosticsSnapshot,
  publishWorkingConversationPatchMetrics,
  recordTodayChartRender,
  recordTodaySummaryRefresh,
  resetDashboardPerformanceDiagnostics,
} from "../lib/dashboardPerformanceDiagnostics";
import DashboardPage from "./Dashboard";

const hookMocks = vi.hoisted(() => ({
  useDashboardWorkingConversations: vi.fn(),
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
}));

vi.mock("../hooks/useDashboardWorkingConversations", () => ({
  useDashboardWorkingConversations: hookMocks.useDashboardWorkingConversations,
}));

vi.mock("../hooks/useStats", () => ({
  useSummary: hookMocks.useSummary,
}));

vi.mock("../hooks/useTimeseries", () => ({
  useTimeseries: hookMocks.useTimeseries,
}));

vi.mock("../components/TodayStatsOverview", () => ({
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

vi.mock("../components/DashboardTodayActivityChart", () => ({
  DashboardTodayActivityChart: ({ metric }: { metric?: string }) => (
    <div data-testid="dashboard-today-activity-chart-mock">{`metric:${metric ?? "unset"}`}</div>
  ),
}));

vi.mock("../components/UsageCalendar", () => ({
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

vi.mock("../components/StatsCards", () => ({
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
      {loading
        ? "loading"
        : error
          ? `error:${error}`
          : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}));

vi.mock("../components/Last24hTenMinuteHeatmap", () => ({
  Last24hTenMinuteHeatmap: ({
    metric,
    showHeader,
  }: {
    metric?: string;
    showHeader?: boolean;
  }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? "unset"};header:${String(showHeader)}`}
    </div>
  ),
}));

vi.mock("../components/WeeklyHourlyHeatmap", () => ({
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

vi.mock("../components/DashboardWorkingConversationsSection", () => ({
  DashboardWorkingConversationsSection: ({
    cards,
    setRefreshTargetCount,
    onOpenUpstreamAccount,
    onOpenInvocation,
  }: {
    cards: DashboardWorkingConversationCardModel[];
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  }) => (
    <div data-testid="dashboard-working-conversations-section">
      {cards.map((card) => card.conversationSequenceId).join(",")}
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
            onClick={() =>
              onOpenUpstreamAccount?.(77, "section-account@example.com")
            }
          >
            open account
          </button>
        </>
      ) : null}
    </div>
  ),
}));

vi.mock("../components/DashboardInvocationDetailDrawer", () => ({
  DashboardInvocationDetailDrawer: ({
    open,
    selection,
    onClose,
    onOpenUpstreamAccount,
  }: {
    open: boolean;
    selection: { invocation: { record: { invokeId: string } } } | null;
    onClose: () => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  }) =>
    open ? (
      <div data-testid="dashboard-invocation-detail-drawer-mock">
        <span data-testid="dashboard-invocation-drawer-selection">
          {selection?.invocation.record.invokeId ?? "none"}
        </span>
        <button
          type="button"
          data-testid="dashboard-invocation-drawer-close"
          onClick={onClose}
        >
          close invocation drawer
        </button>
        <button
          type="button"
          data-testid="dashboard-invocation-drawer-open-account"
          onClick={() =>
            onOpenUpstreamAccount?.(88, "drawer-account@example.com")
          }
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
    onClose,
  }: {
    open: boolean;
    accountId: number | null;
    onClose: () => void;
  }) =>
    open ? (
      <div data-testid="shared-upstream-account-detail-drawer-mock">
        <span data-testid="shared-upstream-account-drawer-account-id">
          {accountId}
        </span>
        <button
          type="button"
          data-testid="shared-upstream-account-drawer-close"
          onClick={onClose}
        >
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

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<MemoryRouter>{ui}</MemoryRouter>);
  });
}

function installSummaryMocks() {
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
}

function createWorkingConversationCard(): DashboardWorkingConversationCardModel {
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
        upstreamAccountName: "section-account@example.com",
        endpoint: "/v1/responses",
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
    expect(
      host?.querySelectorAll('[data-testid="dashboard-activity-overview"]'),
    ).toHaveLength(1);
    expect(
      host
        ?.querySelector('[data-testid="dashboard-activity-range-today"]')
        ?.getAttribute("data-active"),
    ).toBe("true");
    expect(
      host?.querySelector('[data-testid="today-stats-overview-mock"]')
        ?.textContent,
    ).toBe("surface:false;header:false;badge:false");
    expect(
      host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')
        ?.textContent,
    ).toBe("metric:totalCount");
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="dashboard-working-conversations-section"]',
      )?.textContent,
    ).toContain("WC-ABCD12");

    const historyButton = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((button) => button.textContent === "历史");
    if (!(historyButton instanceof HTMLButtonElement)) {
      throw new Error("missing history range button");
    }

    act(() => {
      historyButton.click();
    });

    expect(
      host?.querySelector('[data-testid="usage-calendar"]')?.textContent,
    ).toBe("metric:totalCount;surface:false;toggle:false;meta:false");

    const yesterdayButton = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((button) => button.textContent === "昨日");
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

    expect(
      host?.querySelector('[data-testid="dashboard-performance-diagnostics"]'),
    ).toBeNull();

    act(() => {
      window.localStorage.setItem(
        DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
        "1",
      );
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

    expect(
      host?.querySelector('[data-testid="dashboard-performance-diagnostics"]'),
    ).not.toBeNull();
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
    vi.useFakeTimers();
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

    expect(
      host?.querySelector('[data-testid="dashboard-performance-diagnostics"]'),
    ).toBeNull();

    act(() => {
      window.localStorage.setItem(
        DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
        "1",
      );
      vi.advanceTimersByTime(1_000);
    });

    expect(
      host?.querySelector('[data-testid="dashboard-performance-diagnostics"]'),
    ).not.toBeNull();

    act(() => {
      window.localStorage.removeItem(
        DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
      );
      vi.advanceTimersByTime(1_000);
    });

    expect(
      host?.querySelector('[data-testid="dashboard-performance-diagnostics"]'),
    ).toBeNull();
  });

  it("dedupes identical chart render signatures in diagnostics", () => {
    window.localStorage.setItem(
      DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
      "1",
    );
    resetDashboardPerformanceDiagnostics();

    act(() => {
      recordTodayChartRender("today:stable");
      recordTodayChartRender("today:stable");
      recordTodayChartRender("today:updated");
    });

    expect(
      getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount,
    ).toBe(2);
  });

  it("switches between the invocation drawer and the shared account drawer from dashboard interactions", () => {
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

    const openInvocationButton = host?.querySelector(
      '[data-testid="dashboard-open-invocation"]',
    );
    if (!(openInvocationButton instanceof HTMLButtonElement)) {
      throw new Error("missing invocation trigger");
    }

    act(() => {
      openInvocationButton.click();
    });

    expect(
      host?.querySelector(
        '[data-testid="dashboard-invocation-detail-drawer-mock"]',
      ),
    ).not.toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="dashboard-invocation-drawer-selection"]',
      )?.textContent,
    ).toBe("invoke-dashboard-current");
    expect(
      host?.querySelector(
        '[data-testid="shared-upstream-account-detail-drawer-mock"]',
      ),
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
      host?.querySelector(
        '[data-testid="dashboard-invocation-detail-drawer-mock"]',
      ),
    ).toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="shared-upstream-account-drawer-account-id"]',
      )?.textContent,
    ).toBe("77");

    act(() => {
      openInvocationButton.click();
    });

    expect(
      host?.querySelector(
        '[data-testid="shared-upstream-account-detail-drawer-mock"]',
      ),
    ).toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="dashboard-invocation-detail-drawer-mock"]',
      ),
    ).not.toBeNull();

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
      host?.querySelector(
        '[data-testid="dashboard-invocation-detail-drawer-mock"]',
      ),
    ).toBeNull();
    expect(
      host?.querySelector(
        '[data-testid="shared-upstream-account-drawer-account-id"]',
      )?.textContent,
    ).toBe("88");
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

    const button = host?.querySelector(
      '[data-testid="dashboard-set-refresh-target"]',
    );
    if (!(button instanceof HTMLButtonElement)) {
      throw new Error("missing refresh target trigger");
    }

    act(() => {
      button.click();
    });

    expect(setRefreshTargetCount).toHaveBeenCalledWith(24);
  });
});
