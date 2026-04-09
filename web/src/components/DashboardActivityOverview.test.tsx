/** @vitest-environment jsdom */
import { act } from 'react'
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
  TodayStatsOverview: ({ showSurface, showHeader, showDayBadge }: { showSurface?: boolean; showHeader?: boolean; showDayBadge?: boolean }) => (
    <div data-testid="today-stats-overview-mock">
      {`surface:${String(showSurface)};header:${String(showHeader)};badge:${String(showDayBadge)}`}
    </div>
  ),
}))

vi.mock('./DashboardTodayActivityChart', () => ({
  DashboardTodayActivityChart: ({ metric }: { metric?: string }) => (
    <div data-testid="dashboard-today-activity-chart-mock">{`metric:${metric ?? 'unset'}`}</div>
  ),
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
  hookMocks.useSummary.mockImplementation((window: string) => {
    if (window === 'today') {
      return {
        summary: { totalCount: 12, successCount: 10, failureCount: 2, totalCost: 0.52, totalTokens: 2048 },
        isLoading: false,
        error: null,
      }
    }
    if (window === '1d') {
      return { summary: { totalCount: 100 }, isLoading: false, error: null }
    }
    if (window === '7d') {
      return { summary: { totalCount: 700 }, isLoading: false, error: null }
    }
    return { summary: null, isLoading: false, error: null }
  })

  hookMocks.useTimeseries.mockReturnValue({
    data: { rangeStart: '2026-04-08T00:00:00.000Z', rangeEnd: '2026-04-08T00:03:00.000Z', bucketSeconds: 60, points: [] },
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

function getPanel(testId: string) {
  return host?.querySelector(`[data-testid="${testId}"]`)
}

function getPanelText(panelTestId: string, childTestId: string) {
  return getPanel(panelTestId)?.querySelector(`[data-testid="${childTestId}"]`)?.textContent
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
  it('loads each range lazily and keeps visited panels mounted with per-range metric memory', () => {
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
      'surface:false;header:false;badge:false',
    )
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCount',
    )
    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull()

    clickTab('Cost')
    expect(host?.querySelector('[data-testid="dashboard-today-activity-chart-mock"]')?.textContent).toBe(
      'metric:totalCost',
    )

    clickTab('History')
    expect(hookMocks.useSummary.mock.calls.every(([window]) => window === 'today')).toBe(true)
    expect(hookMocks.useTimeseries.mock.calls.every(([window]) => window === 'today')).toBe(true)
    expect(getPanel('dashboard-activity-range-today')?.getAttribute('data-active')).toBe('false')
    expect(getPanel('dashboard-activity-range-usage')?.getAttribute('data-active')).toBe('true')
    expect(getPanelText('dashboard-activity-range-usage', 'usage-calendar')).toBe(
      'metric:totalCount;surface:false;toggle:false;meta:false',
    )
    clickTab('Tokens')
    expect(getPanelText('dashboard-activity-range-usage', 'usage-calendar')).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false',
    )

    clickTab('7 Days')
    const sevenDayPanel = getPanel('dashboard-activity-range-7d')
    expect(sevenDayPanel?.getAttribute('data-active')).toBe('true')
    expect(getPanel('dashboard-activity-range-usage')?.getAttribute('data-active')).toBe('false')
    expect(getPanelText('dashboard-activity-range-7d', 'stats-cards')).toBe('total:700')
    expect(getPanelText('dashboard-activity-range-7d', 'heatmap-7d')).toBe('metric:totalCount')
    clickTab('Cost')
    expect(getPanelText('dashboard-activity-range-7d', 'heatmap-7d')).toBe('metric:totalCost')

    clickTab('24 Hours')
    expect(getPanel('dashboard-activity-range-1d')?.getAttribute('data-active')).toBe('true')
    expect(getPanelText('dashboard-activity-range-1d', 'stats-cards')).toBe('total:100')
    expect(getPanelText('dashboard-activity-range-1d', 'heatmap-24h')).toBe('metric:totalCount')
    clickTab('Tokens')
    expect(getPanelText('dashboard-activity-range-1d', 'heatmap-24h')).toBe('metric:totalTokens')

    clickTab('Today')
    expect(getPanel('dashboard-activity-range-today')?.getAttribute('data-active')).toBe('true')
    expect(getPanel('dashboard-activity-range-7d')).toBe(sevenDayPanel)
    expect(getPanel('dashboard-activity-range-7d')?.getAttribute('data-active')).toBe('false')
    expect(getPanelText('dashboard-activity-range-today', 'dashboard-today-activity-chart-mock')).toBe(
      'metric:totalCost',
    )
    clickTab('History')
    expect(getPanelText('dashboard-activity-range-usage', 'usage-calendar')).toBe(
      'metric:totalTokens;surface:false;toggle:false;meta:false',
    )
    clickTab('7 Days')
    expect(getPanel('dashboard-activity-range-7d')).toBe(sevenDayPanel)
    expect(getPanelText('dashboard-activity-range-7d', 'heatmap-7d')).toBe('metric:totalCost')
    clickTab('24 Hours')
    expect(getPanelText('dashboard-activity-range-1d', 'heatmap-24h')).toBe('metric:totalTokens')
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

    clickTab('7 Days')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', '7d'])

    clickTab('24 Hours')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', '7d', '1d'])

    clickTab('History')
    expect(getFirstSeenSummaryWindows()).toEqual(['today', '7d', '1d'])
  })
})
