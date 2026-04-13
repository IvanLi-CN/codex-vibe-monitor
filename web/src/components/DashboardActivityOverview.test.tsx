/** @vitest-environment jsdom */
import { act, useSyncExternalStore } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  DashboardActivityOverview,
} from './DashboardActivityOverview'

const hookMocks = vi.hoisted(() => ({
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
}))

const componentState = vi.hoisted(() => ({
  chartRenderCount: 0,
}))

vi.mock('../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
}))

vi.mock('../hooks/useTimeseries', () => ({
  useTimeseries: hookMocks.useTimeseries,
}))

vi.mock('../theme', () => ({
  useTheme: () => ({ themeMode: 'light' }),
}))

vi.mock('../i18n', () => ({
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
  }: {
    stats?: { totalCount?: number } | null
    showSurface?: boolean
    showHeader?: boolean
    showDayBadge?: boolean
    rate?: { tokensPerMinute?: number; costPerMinute?: number } | null
    rateLoading?: boolean
    rateError?: string | null
  }) => (
    <div data-testid="today-stats-overview-mock">
      {`total:${stats?.totalCount ?? 'null'};surface:${String(showSurface)};header:${String(showHeader)};badge:${String(showDayBadge)};tpm:${rate?.tokensPerMinute ?? 'null'};cpm:${rate?.costPerMinute ?? 'null'};rateLoading:${String(rateLoading)};rateError:${rateError ?? 'null'}`}
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

vi.mock('./StatsCards', () => ({
  StatsCards: ({ stats, loading, error }: { stats: { totalCount?: number } | null; loading: boolean; error?: string | null }) => (
    <div data-testid="stats-cards">
      {loading ? 'loading' : error ? `error:${error}` : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}))

vi.mock('./Last24hTenMinuteHeatmap', () => ({
  Last24hTenMinuteHeatmap: ({ metric }: { metric?: string }) => (
    <div data-testid="heatmap-24h">{`metric:${metric ?? 'unset'}`}</div>
  ),
}))

vi.mock('./WeeklyHourlyHeatmap', () => ({
  WeeklyHourlyHeatmap: ({ metric }: { metric?: string }) => (
    <div data-testid="heatmap-7d">{`metric:${metric ?? 'unset'}`}</div>
  ),
}))

vi.mock('./UsageCalendar', () => ({
  UsageCalendar: ({
    metric,
    showSurface,
    showMetricToggle,
    showMeta,
  }: {
    metric?: string
    showSurface?: boolean
    showMetricToggle?: boolean
    showMeta?: boolean
  }) => (
    <div data-testid="usage-calendar">
      {`metric:${metric ?? 'unset'};surface:${String(showSurface)};toggle:${String(showMetricToggle)};meta:${String(showMeta)}`}
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

function installSummaryMocks() {
  summaryStore.set('today', {
    summary: { totalCount: 12, successCount: 10, failureCount: 2, totalCost: 0.52, totalTokens: 2048 },
    isLoading: false,
    error: null,
  })
  summaryStore.set('yesterday', {
    summary: { totalCount: 8, successCount: 7, failureCount: 1, totalCost: 0.21, totalTokens: 1024 },
    isLoading: false,
    error: null,
  })
  summaryStore.set('1d', { summary: { totalCount: 100 }, isLoading: false, error: null })
  summaryStore.set('7d', { summary: { totalCount: 700 }, isLoading: false, error: null })

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

describe('DashboardActivityOverview', () => {
  it('loads only the active range and keeps per-range metric memory across all five tabs', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)

    expect(host?.textContent).toContain('Activity Overview')
    expect(hookMocks.useSummary.mock.calls.every(([window]) => window === 'today')).toBe(true)
    expect(hookMocks.useTimeseries.mock.calls.every(([window]) => window === 'today')).toBe(true)
    expect(host?.querySelector('[data-testid="dashboard-activity-range-today"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')).toBeNull()
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')).toBeNull()
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toBe(
      'total:12;surface:false;header:false;badge:false;tpm:1000;cpm:0.1;rateLoading:false;rateError:null',
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

    clickTab('History')
    expect(hookMocks.useSummary.mock.calls.every(([window]) => window === 'today' || window === 'yesterday')).toBe(true)
    expect(hookMocks.useTimeseries.mock.calls.every(([window]) => window === 'today' || window === 'yesterday')).toBe(true)
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCount;surface:false;toggle:false;meta:false',
    )
    clickTab('Tokens')
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false',
    )

    clickTab('7 Days')
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:700')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCount')
    clickTab('Cost')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCost')

    clickTab('24 Hours')
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalCount')
    clickTab('Tokens')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalTokens')

    clickTab('Today')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCost',
    )
    clickTab('History')
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false',
    )
    clickTab('Yesterday')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalTokens',
    )
    clickTab('7 Days')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCost')
    clickTab('24 Hours')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalTokens')
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
    expect(hookMocks.useSummary).toHaveBeenCalledWith('today')
  })

  it('loads each summary only after its range is selected', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)
    expect(getFirstSeenSummaryWindows()).toEqual(['today'])

    clickTab('Yesterday')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday'])

    clickTab('7 Days')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', '7d'])

    clickTab('24 Hours')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', '7d', '1d'])

    clickTab('History')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', 'yesterday', '7d', '1d'])
  })

  it('does not rerender the today chart when only the summary hook updates', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)

    expect(componentState.chartRenderCount).toBe(1)
    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('total:12')

    act(() => {
      summaryStore.set('today', {
        summary: { totalCount: 18, successCount: 15, failureCount: 3, totalCost: 0.66, totalTokens: 2600 },
        isLoading: false,
        error: null,
      })
    })

    expect(host?.querySelector('[data-testid="today-stats-overview-mock"]')?.textContent).toContain('total:18')
    expect(componentState.chartRenderCount).toBe(1)
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.getAttribute('data-render-count')).toBe('1')
  })
})
