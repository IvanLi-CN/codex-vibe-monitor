/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import DashboardPage from './Dashboard'

const hookMocks = vi.hoisted(() => ({
  useInvocationStream: vi.fn(),
  useSummary: vi.fn(),
}))

vi.mock('../hooks/useInvocations', () => ({
  useInvocationStream: hookMocks.useInvocationStream,
}))

vi.mock('../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
}))

vi.mock('../components/TodayStatsOverview', () => ({
  TodayStatsOverview: () => <div data-testid="today-stats-overview" />,
}))

vi.mock('../components/UsageCalendar', () => ({
  UsageCalendar: () => <div data-testid="usage-calendar" />,
}))

vi.mock('../components/StatsCards', () => ({
  StatsCards: ({
    stats,
    loading,
    error,
  }: {
    stats: { totalCount?: number } | null
    loading: boolean
    error?: string | null
  }) => (
    <div data-testid="stats-cards">
      {loading ? 'loading' : error ? `error:${error}` : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}))

vi.mock('../components/Last24hTenMinuteHeatmap', () => ({
  Last24hTenMinuteHeatmap: ({
    metric,
    showHeader,
  }: {
    metric?: string
    showHeader?: boolean
  }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? 'unset'};header:${String(showHeader)}`}
    </div>
  ),
}))

vi.mock('../components/WeeklyHourlyHeatmap', () => ({
  WeeklyHourlyHeatmap: ({
    metric,
    showHeader,
    showSurface,
  }: {
    metric?: string
    showHeader?: boolean
    showSurface?: boolean
  }) => (
    <div data-testid="heatmap-7d">
      {`metric:${metric ?? 'unset'};header:${String(showHeader)};surface:${String(showSurface)}`}
    </div>
  ),
}))

vi.mock('../components/InvocationTable', () => ({
  InvocationTable: () => <div data-testid="invocation-table" />,
}))

vi.mock('../theme', () => ({
  useTheme: () => ({ themeMode: 'light' }),
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string, values?: Record<string, string | number>) => {
      const map: Record<string, string> = {
        'dashboard.activityOverview.title': '活动总览',
        'dashboard.activityOverview.range24h': '24 小时',
        'dashboard.activityOverview.range7d': '7 日',
        'dashboard.activityOverview.rangeToggleAria': '时间范围切换',
        'dashboard.today.title': '今日统计信息',
        'dashboard.section.recentLiveTitle': `最近 ${values?.count ?? 0} 条实况`,
        'heatmap.metricsToggleAria': '指标切换',
        'metric.totalCount': '次数',
        'metric.totalCost': '金额',
        'metric.totalTokens': 'Tokens',
      }
      return map[key] ?? key
    },
  }),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
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
  vi.clearAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<MemoryRouter>{ui}</MemoryRouter>)
  })
}

describe('DashboardPage', () => {
  it('merges the 24h and 7d activity sections into a single overview card with per-range metric memory', () => {
    hookMocks.useSummary.mockImplementation((window: string) => {
      if (window === 'today') {
        return { summary: { totalCount: 12 }, isLoading: false, error: null }
      }
      if (window === '1d') {
        return { summary: { totalCount: 100 }, isLoading: false, error: null }
      }
      if (window === '7d') {
        return { summary: { totalCount: 700 }, isLoading: false, error: null }
      }
      return { summary: null, isLoading: false, error: null }
    })
    hookMocks.useInvocationStream.mockReturnValue({
      records: [],
      isLoading: false,
      error: null,
    })

    render(<DashboardPage />)

    expect(host?.textContent).toContain('活动总览')
    expect(host?.querySelectorAll('[data-testid="dashboard-activity-overview"]')).toHaveLength(1)
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')?.getAttribute('aria-hidden')).toBe('false')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.getAttribute('aria-hidden')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.className).toContain('invisible')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toContain('metric:totalCount')

    const rangeButtons = host?.querySelectorAll('button[role="tab"]')
    const range7dButton = Array.from(rangeButtons ?? []).find(
      (button) => button.textContent === '7 日',
    )
    if (!(range7dButton instanceof HTMLButtonElement)) {
      throw new Error('missing 7d range button')
    }

    act(() => {
      range7dButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:700')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')?.getAttribute('aria-hidden')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')?.className).toContain('invisible')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.getAttribute('aria-hidden')).toBe('false')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      'metric:totalCount;header:false;surface:false',
    )

    const costButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === '金额',
    )
    if (!(costButton instanceof HTMLButtonElement)) {
      throw new Error('missing cost metric button')
    }

    act(() => {
      costButton.click()
    })

    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      'metric:totalCost;header:false;surface:false',
    )

    const range24hButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === '24 小时',
    )
    if (!(range24hButton instanceof HTMLButtonElement)) {
      throw new Error('missing 24h range button')
    }

    act(() => {
      range24hButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toContain('metric:totalCount')

    const tokensButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === 'Tokens',
    )
    if (!(tokensButton instanceof HTMLButtonElement)) {
      throw new Error('missing tokens metric button')
    }

    act(() => {
      tokensButton.click()
    })

    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toContain('metric:totalTokens')

    act(() => {
      range7dButton.click()
    })

    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      'metric:totalCost;header:false;surface:false',
    )
  })
})
