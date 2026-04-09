/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { TodayStatsOverview } from './TodayStatsOverview'

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'en',
    t: (key: string, values?: { timezone?: string }) => {
      const map: Record<string, string> = {
        'dashboard.today.title': 'Today summary',
        'dashboard.today.subtitle': `Accumulated in natural day (${values?.timezone ?? 'UTC'})`,
        'dashboard.today.dayBadge': 'Today',
        'stats.cards.loadError': 'Load error',
        'stats.cards.totalCalls': 'Calls',
        'stats.cards.success': 'Success',
        'stats.cards.failures': 'Failures',
        'stats.cards.totalCost': 'Cost',
        'stats.cards.totalTokens': 'Tokens',
      }
      return map[key] ?? key
    },
  }),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null
let metricContainerWidth = 640

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === 'true') {
        return metricContainerWidth
      }
      return 0
    },
  })

  Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === 'true') {
        return (this.textContent?.length ?? 0) * 16
      }
      return 0
    },
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  metricContainerWidth = 640
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

describe('TodayStatsOverview', () => {
  it('uses a five-tile desktop grid without a prominent total card', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
        }}
        loading={false}
        error={null}
      />,
    )

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]')
    expect(grid?.className).toContain('lg:grid-cols-5')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(5)
    expect(host?.textContent).toContain('Today summary')
    expect(host?.innerHTML).not.toContain('sm:col-span-2')
  })

  it('supports embedded mode without rendering the outer surface panel', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 32,
          successCount: 30,
          failureCount: 2,
          totalCost: 1.28,
          totalTokens: 4096,
        }}
        loading={false}
        error={null}
        showSurface={false}
      />,
    )

    expect(host?.querySelector('.surface-panel')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-overview-card"]')).not.toBeNull()
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(5)
  })

  it('hides the heading block when used inside the overview today tab', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12,
          successCount: 10,
          failureCount: 2,
          totalCost: 0.52,
          totalTokens: 2080,
        }}
        loading={false}
        error={null}
        showSurface={false}
        showHeader={false}
        showDayBadge={false}
      />,
    )

    expect(host?.textContent).not.toContain('Today summary')
    expect(host?.textContent).not.toContain('Accumulated in natural day')
    expect(host?.textContent).not.toContain('Today')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(5)
  })

  it('switches to compact notation when the full metric value would overflow', () => {
    metricContainerWidth = 180

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
        }}
        loading={false}
        error={null}
      />,
    )

    const totalTokensValue = host?.querySelector('[data-testid="today-stats-value-total-tokens"]')
    expect(totalTokensValue?.getAttribute('data-compact')).toBe('true')
    expect(totalTokensValue?.textContent).toContain('1.31B')
    expect(totalTokensValue?.getAttribute('title')).toBe('1,314,275,579')
  })
})
