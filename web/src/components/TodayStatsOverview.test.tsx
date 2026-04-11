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
        'dashboard.today.tokensPerMinute5m': 'TPM (5m avg)',
        'dashboard.today.costPerMinute5m': 'Cost/min (5m avg)',
        'stats.cards.loadError': 'Load error',
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
  it('uses a six-tile desktop grid with TPM and cost-per-minute leading metrics', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
        }}
        rate={{
          tokensPerMinute: 1000,
          costPerMinute: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    )

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]')
    expect(grid?.className).toContain('lg:grid-cols-6')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(6)
    expect(host?.textContent).toContain('Today summary')
    expect(host?.textContent).toContain('TPM (5m avg)')
    expect(host?.textContent).toContain('Cost/min (5m avg)')
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
        rate={{
          tokensPerMinute: 320,
          costPerMinute: 0.13,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
        showSurface={false}
      />,
    )

    expect(host?.querySelector('.surface-panel')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-overview-card"]')).not.toBeNull()
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(6)
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
        rate={{
          tokensPerMinute: 416,
          costPerMinute: 0.1,
          windowMinutes: 5,
          available: true,
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
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(6)
  })

  it('renders partial loading only for rate tiles while summary metrics stay visible', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={null}
        loading={false}
        rateLoading
        error={null}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-value-cost-per-minute"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-value-success"]')?.textContent).toContain('80')
    expect(host?.querySelector('[data-testid="today-stats-value-failures"]')?.textContent).toContain('8')
  })

  it('renders TPM as a whole number even when the averaged rate is fractional', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
        }}
        rate={{
          tokensPerMinute: 1000.6,
          costPerMinute: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    )

    const tpmText = host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent ?? ''
    const costPerMinuteText = host?.querySelector('[data-testid="today-stats-value-cost-per-minute"]')?.textContent ?? ''

    expect(tpmText).toContain('1,001')
    expect(tpmText).not.toContain('.')
    expect(costPerMinuteText).toContain('$0.10')
  })

  it('shows unavailable placeholders for rate tiles when timeseries loading fails', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={null}
        loading={false}
        rateLoading={false}
        rateError="timeseries failed"
        error={null}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-value-cost-per-minute"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-value-success"]')?.textContent).toContain('80')
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
        rate={{
          tokensPerMinute: 1000,
          costPerMinute: 0.1,
          windowMinutes: 5,
          available: true,
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
