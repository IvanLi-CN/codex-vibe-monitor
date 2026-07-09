/** @vitest-environment jsdom */
import { act, useSyncExternalStore } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import {
  ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX,
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
} from './dashboardActivityRange'
import {
  DashboardActivityOverview,
} from './DashboardActivityOverview'

const hookMocks = vi.hoisted(() => ({
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
  useParallelWorkStats: vi.fn(),
}))

const componentState = vi.hoisted(() => ({
  chartRenderCount: 0,
}))

const sseState = vi.hoisted(() => ({
  listeners: new Set<(payload: unknown) => void>(),
}))

vi.mock('../../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
}))

vi.mock('../../hooks/useTimeseries', () => ({
  useTimeseries: hookMocks.useTimeseries,
}))

vi.mock('../../hooks/useParallelWorkStats', () => ({
  useParallelWorkStats: hookMocks.useParallelWorkStats,
}))

vi.mock('../../lib/sse', () => ({
  subscribeToSse: (listener: (payload: unknown) => void) => {
    sseState.listeners.add(listener)
    return () => sseState.listeners.delete(listener)
  },
  subscribeToSseOpen: () => () => {},
}))

vi.mock('../../theme', () => ({
  useTheme: () => ({ themeMode: 'light' }),
}))

vi.mock('../../i18n', () => ({
  useTranslation: () => ({
    locale: 'en',
    t: (key: string) => {
      const map: Record<string, string> = {
        'dashboard.activityOverview.title': 'Activity Overview',
        'dashboard.activityOverview.rangeToday': 'Today',
        'dashboard.activityOverview.rangeYesterday': 'Yesterday',
        'dashboard.activityOverview.range24h': '24 Hours',
        'dashboard.activityOverview.range7d': '7 Days',
        'dashboard.activityOverview.rangeUsage': 'History',
        'dashboard.activityOverview.rangeToggleAria': 'Switch activity range',
        'heatmap.metricsToggleAria': 'Switch metric',
        'metric.totalCount': 'Calls',
        'metric.totalCost': 'Cost',
        'metric.totalTokens': 'Tokens',
        'chart.trend': 'Trend',
      }
      return map[key] ?? key
    },
  }),
}))

vi.mock('./TodayStatsOverview', () => ({
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
      totalCount?: number
      inProgressConversationCount?: number
      inProgressRetryConversationCount?: number
      inProgressAvgWaitMs?: number
      nonSuccessCost?: number
      nonSuccessTokens?: number
    } | null
    showSurface?: boolean
    showHeader?: boolean
    showDayBadge?: boolean
    rate?: { tokensPerMinute?: number; spendRate?: number } | null
    rateLoading?: boolean
    rateError?: string | null
    parallelWorkStats?: { current?: { avgCount?: number | null } } | null
    parallelWorkError?: string | null
    showInProgressConversations?: boolean
  }) => (
    <div data-testid="today-stats-overview-mock">
      {`total:${stats?.totalCount ?? 'null'};inProgress:${stats?.inProgressConversationCount ?? 'null'};retry:${stats?.inProgressRetryConversationCount ?? 'null'};wait:${stats?.inProgressAvgWaitMs ?? 'null'};nonSuccessCost:${stats?.nonSuccessCost ?? 'null'};nonSuccessTokens:${stats?.nonSuccessTokens ?? 'null'};surface:${String(showSurface)};header:${String(showHeader)};badge:${String(showDayBadge)};tpm:${rate?.tokensPerMinute ?? 'null'};spendRate:${rate?.spendRate ?? 'null'};rateLoading:${String(rateLoading)};rateError:${rateError ?? 'null'};parallelAvg:${parallelWorkStats?.current?.avgCount ?? 'null'};parallelError:${parallelWorkError ?? 'null'};showInProgress:${String(showInProgressConversations)}`}
    </div>
  ),
}))

vi.mock('./DashboardTodayActivityChart', () => ({
  DashboardTodayActivityChart: ({ metric }: { metric?: string }) => {
    componentState.chartRenderCount += 1
    return (
      <div
        data-testid="dashboard-today-activity-chart-mock"
        data-render-count={String(componentState.chartRenderCount)}
      >
        {`metric:${metric ?? 'unset'}`}
      </div>
    )
  },
}))

vi.mock('../stats/StatsCards', () => ({
  StatsCards: ({ stats, loading, error }: { stats: { totalCount?: number } | null; loading: boolean; error?: string | null }) => (
    <div data-testid="stats-cards">
      {loading ? 'loading' : error ? `error:${error}` : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}))

vi.mock('./Last24hTenMinuteHeatmap', () => ({
  Last24hTenMinuteHeatmap: ({
    metric,
    upstreamAccountId,
  }: {
    metric?: string
    upstreamAccountId?: number
  }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? 'unset'};account:${upstreamAccountId ?? 'global'}`}
    </div>
  ),
}))

vi.mock('./WeeklyHourlyHeatmap', () => ({
  WeeklyHourlyHeatmap: ({
    metric,
    upstreamAccountId,
  }: {
    metric?: string
    upstreamAccountId?: number
  }) => (
    <div data-testid="heatmap-7d">
      {`metric:${metric ?? 'unset'};account:${upstreamAccountId ?? 'global'}`}
    </div>
  ),
}))

vi.mock('./UsageCalendar', () => ({
  UsageCalendar: ({
    metric,
    showSurface,
    showMetricToggle,
    showMeta,
    upstreamAccountId,
  }: {
    metric?: string
    showSurface?: boolean
    showMetricToggle?: boolean
    showMeta?: boolean
    upstreamAccountId?: number
  }) => (
    <div data-testid="usage-calendar">
      {`metric:${metric ?? 'unset'};surface:${String(showSurface)};toggle:${String(showMetricToggle)};meta:${String(showMeta)};account:${upstreamAccountId ?? 'global'}`}
    </div>
  ),
}))

const storage = new Map<string, string>()
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => {
    storage.set(key, value)
  },
  removeItem: (key: string) => {
    storage.delete(key)
  },
  clear: () => {
    storage.clear()
  },
}

let host: HTMLDivElement | null = null
let root: Root | null = null

function createSummaryStore() {
  const values = new Map<
    string,
    {
      summary: Record<string, unknown> | null
      isLoading: boolean
      error: string | null
    }
  >()
  const listeners = new Set<() => void>()

  const getFallback = () => ({
    summary: null,
    isLoading: false,
    error: null,
  })

  return {
    use(window: string) {
      return useSyncExternalStore(
        (listener) => {
          listeners.add(listener)
          return () => listeners.delete(listener)
        },
        () => values.get(window) ?? getFallback(),
        () => values.get(window) ?? getFallback(),
      )
    },
    set(
      window: string,
      value: {
        summary: Record<string, unknown> | null
        isLoading: boolean
        error: string | null
      },
    ) {
      values.set(window, value)
      listeners.forEach((listener) => listener())
    },
    reset() {
      values.clear()
      listeners.clear()
    },
  }
}

const summaryStore = createSummaryStore()

beforeAll(() => {
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: localStorageMock,
  })
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  window.localStorage.clear()
  componentState.chartRenderCount = 0
  summaryStore.reset()
  sseState.listeners.clear()
  vi.clearAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function buildParallelWorkStatsFixture(avgCount = 2) {
  return {
    current: {
      rangeStart: '2026-04-08 00:00:00',
      rangeEnd: '2026-04-08 00:06:00',
      bucketSeconds: 60,
      completeBucketCount: 2,
      activeBucketCount: 2,
      minCount: 1,
      maxCount: 3,
      avgCount,
      points: [
        { bucketStart: '2026-04-08 00:04:00', bucketEnd: '2026-04-08 00:05:00', parallelCount: 1 },
        { bucketStart: '2026-04-08 00:05:00', bucketEnd: '2026-04-08 00:06:00', parallelCount: 3 },
      ],
    },
    minute7d: {},
    hour30d: {},
    dayAll: {},
  }
}

function installSummaryMocks() {
  summaryStore.set('today', {
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
  })
  summaryStore.set('yesterday', {
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
  })
  summaryStore.set('1d', { summary: { totalCount: 100, inProgressConversationCount: 6 }, isLoading: false, error: null })
  summaryStore.set('7d', { summary: { totalCount: 700, inProgressConversationCount: 7 }, isLoading: false, error: null })
  summaryStore.set('previous7d', {
    summary: { totalCount: 70, successCount: 66, failureCount: 4, totalCost: 1.4, totalTokens: 7000, inProgressConversationCount: 5 },
    isLoading: false,
    error: null,
  })

  hookMocks.useSummary.mockImplementation((window: string) => summaryStore.use(window))

  hookMocks.useTimeseries.mockReturnValue({
    data: {
      rangeStart: '2026-04-08 00:00:00',
      rangeEnd: '2026-04-08 00:06:00',
      bucketSeconds: 60,
      points: [
        {
          bucketStart: '2026-04-08 00:01:00',
          bucketEnd: '2026-04-08 00:02:00',
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 600,
          totalCost: 0.06,
        },
        {
          bucketStart: '2026-04-08 00:02:00',
          bucketEnd: '2026-04-08 00:03:00',
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 800,
          totalCost: 0.08,
        },
        {
          bucketStart: '2026-04-08 00:03:00',
          bucketEnd: '2026-04-08 00:04:00',
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 1000,
          totalCost: 0.1,
        },
        {
          bucketStart: '2026-04-08 00:04:00',
          bucketEnd: '2026-04-08 00:05:00',
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 1200,
          totalCost: 0.12,
        },
        {
          bucketStart: '2026-04-08 00:05:00',
          bucketEnd: '2026-04-08 00:06:00',
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
  })
  hookMocks.useParallelWorkStats.mockReturnValue({
    data: buildParallelWorkStatsFixture(),
    isLoading: false,
    error: null,
  })
}

function clickTab(label: string) {
  const button = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (candidate) => candidate.textContent === label,
  )
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing tab button: ${label}`)
  }
  act(() => {
    button.click()
  })
}

function getFirstSeenSummaryWindows() {
  const seen = new Set<string>()
  const ordered: string[] = []
  for (const [window] of hookMocks.useSummary.mock.calls) {
    if (typeof window !== 'string' || seen.has(window)) continue
    seen.add(window)
    ordered.push(window)
  }
  return ordered
}

function emitSse(payload: unknown) {
  for (const listener of [...sseState.listeners]) {
    listener(payload)
  }
}

describe('DashboardActivityOverview', () => {
  it('uses a dashboard activity snapshot for the visible top KPI summary overlay without remounting duplicate today summary fetches', () => {
    installSummaryMocks()

    render(
      <DashboardActivityOverview
        dashboardActivity={{
          range: 'today',
          rangeStart: '2026-07-05T00:00:00Z',
          rangeEnd: '2026-07-05T12:00:00Z',
          snapshotId: 1783233600000,
          rateWindow: {
            start: '2026-07-05T11:55:00Z',
            end: '2026-07-05T12:00:00Z',
            windowMinutes: 5,
            mode: 'account_active_tail_sum',
          },
          summary: {
            stats: {
              totalCount: 21,
              successCount: 18,
              failureCount: 2,
              totalCost: 0.42,
              totalTokens: 4200,
              inProgressConversationCount: 7,
              inProgressRetryConversationCount: 1,
              inProgressAvgWaitMs: 2500,
              nonSuccessCost: 0.04,
              nonSuccessTokens: 300,
            },
            tokensPerMinute: 1234,
            spendRate: 0.45,
          },
        }}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      'total:21;inProgress:7;retry:1;wait:2500;nonSuccessCost:0.04;nonSuccessTokens:300;surface:false;header:false;badge:false;tpm:1000;spendRate:0.1;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true',
    )
    expect(hookMocks.useSummary.mock.calls.map(([window]) => window)).not.toContain(
      'today',
    )

    act(() => {
      emitSse({
        type: 'summary',
        window: 'today',
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
      })
    })

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      'total:34;inProgress:9;retry:2;wait:1100;nonSuccessCost:0.08;nonSuccessTokens:420;surface:false;header:false;badge:false;tpm:1000;spendRate:0.1;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true',
    )
  })

  it('skips the duplicate yesterday summary hook when a snapshot-backed yesterday panel is visible', () => {
    installSummaryMocks()

    render(
      <DashboardActivityOverview
        dashboardActivity={{
          range: 'yesterday',
          rangeStart: '2026-07-04T00:00:00Z',
          rangeEnd: '2026-07-05T00:00:00Z',
          snapshotId: 1783147200000,
          rateWindow: {
            start: '2026-07-04T23:55:00Z',
            end: '2026-07-05T00:00:00Z',
            windowMinutes: 5,
            mode: 'account_active_tail_sum',
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
    )

    expect(hookMocks.useSummary.mock.calls.map(([window]) => window)).not.toContain(
      'yesterday',
    )
  })

  it('loads only the active range and keeps per-range metric memory across all five tabs', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)

    expect(host?.textContent).toContain('Activity Overview')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d'])
    expect(hookMocks.useTimeseries.mock.calls.every(([window]) => window === 'today' || window === 'yesterday')).toBe(true)
    expect(host?.querySelector('[data-testid="dashboard-activity-range-today"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')).toBeNull()
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')).toBeNull()
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      'total:12;inProgress:11;retry:3;wait:1700;nonSuccessCost:0.12;nonSuccessTokens:256;surface:false;header:false;badge:false;tpm:1000;spendRate:0.1;rateLoading:false;rateError:null;parallelAvg:2;parallelError:null;showInProgress:true',
    )
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCount',
    )
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull()

    clickTab('Cost')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCost',
    )

    clickTab('Yesterday')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-yesterday"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCount',
    )
    clickTab('Tokens')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalTokens',
    )
    clickTab('Trend')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:trend',
    )

    clickTab('History')
    expect(hookMocks.useSummary.mock.calls.every(([window]) => window === 'today' || window === 'yesterday' || window === 'previous7d')).toBe(true)
    expect(hookMocks.useTimeseries.mock.calls.every(([window]) => window === 'today' || window === 'yesterday')).toBe(true)
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCount;surface:false;toggle:false;meta:false;account:global',
    )
    clickTab('Tokens')
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false;account:global',
    )

    clickTab('7 Days')
    expect(Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).some((button) => button.textContent === 'Trend')).toBe(false)
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:700')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCount;account:global')
    clickTab('Cost')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCost;account:global')

    clickTab('24 Hours')
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalCount;account:global')
    clickTab('Tokens')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalTokens;account:global')

    clickTab('Today')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCost',
    )
    clickTab('History')
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false;account:global',
    )
    clickTab('Yesterday')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:trend',
    )
    clickTab('7 Days')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCost;account:global')
    clickTab('24 Hours')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalTokens;account:global')
  })

  it('restores the last active range from localStorage and falls back to today on invalid values', () => {
    installSummaryMocks()
    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, 'usage')

    render(<DashboardActivityOverview />)

    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="usage-calendar"]')).not.toBeNull()
    expect(hookMocks.useSummary).not.toHaveBeenCalled()
    expect(hookMocks.useTimeseries).not.toHaveBeenCalled()

    act(() => {
      root?.unmount()
    })
    host?.remove()
    host = null
    root = null

    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, 'bogus')
    render(<DashboardActivityOverview />)
    expect(document.body.querySelector('[data-testid="dashboard-activity-range-today"]')?.getAttribute('data-active')).toBe('true')
    expect(hookMocks.useSummary).toHaveBeenCalledWith('today', undefined)
  })

  it('uses account-scoped fetch options and storage without changing dashboard storage', () => {
    installSummaryMocks()
    const storageKey = `${ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX}.42`
    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, 'usage')

    render(
      <DashboardActivityOverview
        title="Account activity"
        storageKey={storageKey}
        testId="account-activity-overview"
        upstreamAccountId={42}
      />,
    )

    expect(host?.querySelector('[data-testid="account-activity-overview"]')?.textContent).toContain(
      'Account activity',
    )
    expect(host?.querySelector('[data-testid="dashboard-activity-range-today"]')?.getAttribute('data-active')).toBe('true')
    expect(window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)).toBe('usage')
    expect(window.localStorage.getItem(storageKey)).toBe('today')
    expect(hookMocks.useSummary).toHaveBeenCalledWith('today', {
      upstreamAccountId: 42,
    })
    expect(hookMocks.useTimeseries).toHaveBeenCalledWith('today', {
      bucket: '1m',
      upstreamAccountId: 42,
    })
    expect(hookMocks.useParallelWorkStats).toHaveBeenCalledWith({
      range: 'today',
      bucket: '1m',
      upstreamAccountId: 42,
    })
    expect(hookMocks.useParallelWorkStats).toHaveBeenCalledWith({
      range: 'yesterday',
      bucket: '1m',
      upstreamAccountId: 42,
    })
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      'retry:3;wait:1700;nonSuccessCost:0.12;nonSuccessTokens:256;',
    )
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      'parallelAvg:2;parallelError:null;showInProgress:true',
    )

    clickTab('7 Days')

    expect(window.localStorage.getItem(storageKey)).toBe('7d')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      'metric:totalCount;account:42',
    )
    expect(hookMocks.useSummary).toHaveBeenCalledWith('7d', {
      upstreamAccountId: 42,
    })
  })

  it('does not request duplicate yesterday comparison data for account-scoped yesterday view', () => {
    installSummaryMocks()
    const storageKey = `${ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX}.42`
    window.localStorage.setItem(storageKey, 'yesterday')

    render(
      <DashboardActivityOverview
        title="Account activity"
        storageKey={storageKey}
        testId="account-activity-overview"
        upstreamAccountId={42}
      />,
    )

    const yesterdayCalls = hookMocks.useSummary.mock.calls.filter(
      ([window, options]) =>
        window === 'yesterday' &&
        options != null &&
        typeof options === 'object' &&
        'upstreamAccountId' in options &&
        options.upstreamAccountId === 42,
    )
    expect(yesterdayCalls).toHaveLength(1)

    const yesterdayTimeseriesCalls = hookMocks.useTimeseries.mock.calls.filter(
      ([window, options]) =>
        window === 'yesterday' &&
        options != null &&
        typeof options === 'object' &&
        'upstreamAccountId' in options &&
        options.upstreamAccountId === 42,
    )
    expect(yesterdayTimeseriesCalls).toHaveLength(1)
  })

  it('loads each summary only after its range is selected', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d'])

    clickTab('Yesterday')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d'])

    clickTab('7 Days')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d', '7d'])

    clickTab('24 Hours')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d', '7d', '1d'])

    clickTab('History')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', 'previous7d', '7d', '1d'])
  })

  it('does not rerender the today chart when only the summary hook updates', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)

    expect(componentState.chartRenderCount).toBe(1)
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('total:12')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('inProgress:11')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('retry:3')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('wait:1700')

    act(() => {
      summaryStore.set('today', {
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
      })
    })

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('total:18')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('inProgress:9')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('retry:4')
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('wait:2200')
    expect(componentState.chartRenderCount).toBe(1)
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.getAttribute('data-render-count')).toBe('1')
  })

  it('keeps trend comparison data visible when the comparison parallel request fails', () => {
    installSummaryMocks()
    hookMocks.useParallelWorkStats.mockImplementation(({ range }: { range: string }) => {
      if (range === 'yesterday') {
        return {
          data: null,
          isLoading: false,
          error: 'comparison unavailable',
        }
      }
      return {
        data: buildParallelWorkStatsFixture(2),
        isLoading: false,
        error: null,
      }
    })

    render(<DashboardActivityOverview />)

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain(
      'parallelAvg:2;parallelError:null',
    )
  })
})
