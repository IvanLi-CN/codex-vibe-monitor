/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { DashboardActivityOverview } from './DashboardActivityOverview'

const hookMocks = vi.hoisted(() => ({
  useSummary: vi.fn(),
}))

vi.mock('../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
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

vi.mock('./StatsCards', () => ({
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
    <div data-testid="usage-calendar">{`metric:${metric ?? 'unset'};surface:${String(showSurface)};toggle:${String(showMetricToggle)};meta:${String(showMeta)}`}</div>
  ),
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
    root?.render(ui)
  })
}

function installSummaryMocks() {
  hookMocks.useSummary.mockImplementation((window: string) => {
    if (window === '1d') {
      return { summary: { totalCount: 100 }, isLoading: false, error: null }
    }
    if (window === '7d') {
      return { summary: { totalCount: 700 }, isLoading: false, error: null }
    }
    return { summary: null, isLoading: false, error: null }
  })
}

describe('DashboardActivityOverview', () => {
  it('switches among 24h, 7d, and usage views while keeping per-view metric memory', () => {
    installSummaryMocks()

    render(<DashboardActivityOverview />)

    expect(host?.textContent).toContain('Activity Overview')
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalCount')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')).toBeNull()

    const buttons = Array.from(host?.querySelectorAll('button[role="tab"]') ?? [])
    const usageButton = buttons.find((button) => button.textContent === 'History')
    if (!(usageButton instanceof HTMLButtonElement)) {
      throw new Error('missing usage activity button')
    }

    act(() => {
      usageButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull()
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCount;surface:false;toggle:false;meta:false',
    )

    const costButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === 'Cost',
    )
    if (!(costButton instanceof HTMLButtonElement)) {
      throw new Error('missing cost button')
    }

    act(() => {
      costButton.click()
    })

    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCost;surface:false;toggle:false;meta:false',
    )

    const range7dButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === '7 Days',
    )
    if (!(range7dButton instanceof HTMLButtonElement)) {
      throw new Error('missing 7d button')
    }

    act(() => {
      range7dButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:700')
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalCount')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')?.getAttribute('data-active')).toBe(
      'false',
    )
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCost;surface:false;toggle:false;meta:false',
    )

    const tokensButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === 'Tokens',
    )
    if (!(tokensButton instanceof HTMLButtonElement)) {
      throw new Error('missing tokens button')
    }

    act(() => {
      tokensButton.click()
    })

    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe('metric:totalTokens')

    act(() => {
      usageButton.click()
    })

    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCost;surface:false;toggle:false;meta:false',
    )

    const range24hButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === '24 Hours',
    )
    if (!(range24hButton instanceof HTMLButtonElement)) {
      throw new Error('missing 24h button')
    }

    act(() => {
      range24hButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toBe('metric:totalCount')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.getAttribute('data-active')).toBe(
      'false',
    )
  })
})
