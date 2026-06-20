/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { TimeseriesResponse } from '../lib/api'
import { TodayStatsOverview } from './TodayStatsOverview'

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'en',
    t: (key: string, values?: { timezone?: string }) => {
      const map: Record<string, string> = {
        'dashboard.today.title': 'Today summary',
        'dashboard.today.subtitle': `Accumulated in natural day (${values?.timezone ?? 'UTC'})`,
        'dashboard.today.dayBadge': 'Today',
        'dashboard.today.tokensPerMinute': 'TPM',
        'dashboard.today.spendRate': 'Spend rate',
        'dashboard.today.responseTime': 'Response time',
        'dashboard.today.responseTimeDescription': 'Response time uses the latest 5-minute active tail.',
        'dashboard.today.parallelConversations': 'Parallel conversations',
        'dashboard.today.todayCost': 'Today cost',
        'dashboard.today.yesterdayCost': 'Yesterday cost',
        'dashboard.today.todayTokens': 'Today tokens',
        'dashboard.today.yesterdayTokens': 'Yesterday tokens',
        'dashboard.today.tokensPerMinuteDescription': 'TPM uses the active tail inside the latest 5-minute window.',
        'dashboard.today.spendRateDescription': 'Spend rate uses the active tail inside the latest 5-minute window.',
        'dashboard.today.parallelConversationsDescription': 'Current parallel conversations.',
        'dashboard.today.successDescription': 'Successful calls in the selected day.',
        'dashboard.today.failuresDescription': 'Failed calls in the selected day.',
        'dashboard.today.totalCostDescription': 'Total cost in the selected day.',
        'dashboard.today.totalTokensDescription': 'Total tokens in the selected day.',
        'dashboard.today.secondary.dayAverage': 'Day avg',
        'dashboard.today.secondary.previous7dAverage': '7d daily avg',
        'dashboard.today.secondary.vsYesterday': 'vs yesterday',
        'dashboard.today.secondary.comparison': 'Comparison',
        'dashboard.today.secondary.failureRate': 'Failure rate',
        'dashboard.today.secondary.cacheHitRate': 'Cache hit',
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

function buildTimeseriesWithLatency(): TimeseriesResponse {
  const points = Array.from({ length: 8 }, (_, index) => {
    const bucketStart = new Date(Date.parse('2026-04-10T00:00:00.000Z') + index * 60_000).toISOString()
    const bucketEnd = new Date(Date.parse('2026-04-10T00:01:00.000Z') + index * 60_000).toISOString()
    const totalCount = index % 3 === 0 ? 0 : 2 + index
    const sampleCount = totalCount > 0 ? 2 + index : 0

    return {
      bucketStart,
      bucketEnd,
      totalCount,
      successCount: totalCount,
      failureCount: 0,
      totalTokens: 78000 + index * 6100,
      cacheInputTokens: 18000 + index * 1200,
      totalCost: Number((1.1 + index * 0.08).toFixed(2)),
      firstResponseByteTotalSampleCount: sampleCount,
      firstResponseByteTotalAvgMs: sampleCount > 0 ? Number((820 + index * 41.5).toFixed(1)) : null,
    }
  })

  return {
    rangeStart: '2026-04-10T00:00:00.000Z',
    rangeEnd: '2026-04-10T00:08:00.000Z',
    bucketSeconds: 60,
    points,
  }
}

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
  it('uses a seven-tile desktop grid with response time after parallel conversations', () => {
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
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
      />,
    )

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]')
    expect(grid?.className).toContain('lg:grid-cols-7')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7)
    expect(host?.textContent).toContain('Today summary')
    expect(host?.textContent).toContain('TPM')
    expect(host?.textContent).toContain('Spend rate')
    expect(host?.textContent).toContain('Response time')
    expect(host?.textContent).toContain('Parallel conversations')
    expect(host?.textContent).toContain('Today cost')
    expect(host?.textContent).toContain('Today tokens')
  })

  it('uses a six-tile desktop grid when parallel conversations are hidden', () => {
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
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
        showParallelWork={false}
      />,
    )

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]')
    expect(grid?.className).toContain('lg:grid-cols-6')
    expect(grid?.className).not.toContain('lg:grid-cols-7')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(6)
    expect(host?.textContent).not.toContain('Parallel conversations')
    expect(host?.textContent).toContain('Response time')
    expect(host?.textContent).toContain('Today cost')
    expect(host?.textContent).toContain('Today tokens')
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
          spendRate: 0.13,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
        showSurface={false}
      />,
    )

    expect(host?.querySelector('.surface-panel')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-overview-card"]')).not.toBeNull()
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7)
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
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
        showSurface={false}
        showHeader={false}
        showDayBadge={false}
      />,
    )

    expect(host?.textContent).not.toContain('Today summary')
    expect(host?.textContent).not.toContain('Accumulated in natural day')
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7)
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
        comparisonStats={{
          totalCount: 176,
          successCount: 160,
          failureCount: 16,
          totalCost: 4.2,
          totalTokens: 16000,
        }}
        rate={null}
        loading={false}
        rateLoading
        error={null}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate"]')).toBeNull()
    expect(host?.querySelector('[data-testid="today-stats-value-success"]')?.textContent).toContain('80')
    expect(host?.querySelector('[data-testid="today-stats-secondary-failures"]')?.textContent).toContain('8')
    expect(host?.textContent).toContain('vs yesterday')
    expect(host?.querySelector('[data-testid="today-stats-secondary-cost-delta"]')?.textContent).toBe('-50%')
    expect(host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent).toBe('-50%')
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
          spendRate: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
      />,
    )

    const tpmText = host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent ?? ''
    const spendRateText = host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent ?? ''

    expect(tpmText).toContain('1,001')
    expect(tpmText).not.toContain('.')
    expect(spendRateText).toContain('$0.10')
  })

  it('compares cost and token totals against yesterday at the same day progress', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12,
          successCount: 12,
          failureCount: 0,
          totalCost: 12,
          totalTokens: 1200,
        }}
        comparisonStats={{
          totalCount: 100,
          successCount: 100,
          failureCount: 0,
          totalCost: 100,
          totalTokens: 10000,
        }}
        timeseries={{
          rangeStart: '2026-04-10T00:00:00.000Z',
          rangeEnd: '2026-04-10T00:03:00.000Z',
          bucketSeconds: 60,
          points: [],
        }}
        comparisonTimeseries={{
          rangeStart: '2026-04-09T00:00:00.000Z',
          rangeEnd: '2026-04-10T00:00:00.000Z',
          bucketSeconds: 60,
          points: [1, 2, 3, 99].map((value, index) => ({
            bucketStart: new Date(Date.parse('2026-04-09T00:00:00.000Z') + index * 60_000).toISOString(),
            bucketEnd: new Date(Date.parse('2026-04-09T00:01:00.000Z') + index * 60_000).toISOString(),
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: value * 100,
            cacheInputTokens: 0,
            totalCost: value,
          })),
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    )

    expect(host?.textContent).toContain('vs yesterday')
    expect(host?.textContent).not.toContain('vs yesterday same time')
    expect(host?.querySelector('[data-testid="today-stats-secondary-cost-delta"]')?.textContent).toBe('+100%')
    expect(host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent).toBe('+100%')
  })

  it('opens field descriptions from metric titles', () => {
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
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
      />,
    )

    const tpmTitle = [...(host?.querySelectorAll('[role="button"]') ?? [])]
      .find((element) => element.textContent === 'TPM')
    expect(tpmTitle).toBeInstanceOf(HTMLElement)
    expect(tpmTitle?.getAttribute('aria-label')).toBeNull()

    act(() => {
      tpmTitle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const tooltip = document.body.querySelector('[role="tooltip"]')
    expect(tooltip?.textContent).toContain('active tail inside the latest 5-minute window')
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
        timeseries={buildTimeseriesWithLatency()}
        error={null}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.textContent).toBe('—')
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
          spendRate: 0.1,
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

  it('drops compact decimals before truncating the magnitude suffix in narrow tiles', () => {
    metricContainerWidth = 76

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 281110000,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    )

    const totalTokensValue = host?.querySelector('[data-testid="today-stats-value-total-tokens"]')
    const totalTokensVisible = totalTokensValue?.querySelector('[data-adaptive-metric-visible="true"]')
    expect(totalTokensValue?.getAttribute('data-compact')).toBe('true')
    expect(totalTokensValue?.getAttribute('data-compact-precision')).toBe('0')
    expect(totalTokensVisible?.textContent).toContain('281M')
    expect(totalTokensVisible?.textContent).not.toContain('281.11M')
  })

  it('uses a width-capped loading placeholder instead of a fixed narrow-tile width', () => {
    render(
      <TodayStatsOverview
        stats={null}
        rate={null}
        loading
        error={null}
      />,
    )

    const tpmLoading = host?.querySelector('[data-testid="today-stats-value-tpm-loading"]')
    expect(tpmLoading).toBeInstanceOf(HTMLElement)
    expect((tpmLoading as HTMLElement | null)?.className).toContain('w-full')
    expect((tpmLoading as HTMLElement | null)?.className).toContain('max-w-[7.5rem]')
    expect((tpmLoading as HTMLElement | null)?.className).not.toContain('w-28')
  })

  it('renders the response-time card with recent-window and day-average latency values', () => {
    const timeseries = buildTimeseriesWithLatency()
    const comparisonTimeseries = {
      ...timeseries,
      rangeStart: '2026-04-09T00:00:00.000Z',
      rangeEnd: '2026-04-09T00:08:00.000Z',
      points: timeseries.points.map((point, index) => ({
        ...point,
        bucketStart: new Date(Date.parse('2026-04-09T00:00:00.000Z') + index * 60_000).toISOString(),
        bucketEnd: new Date(Date.parse('2026-04-09T00:01:00.000Z') + index * 60_000).toISOString(),
        firstResponseByteTotalAvgMs:
          point.firstResponseByteTotalAvgMs == null
            ? null
            : Number((point.firstResponseByteTotalAvgMs * 0.75).toFixed(1)),
      })),
    }

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={{
          tokensPerMinute: 416,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={timeseries}
        comparisonTimeseries={comparisonTimeseries}
        loading={false}
        error={null}
      />,
    )

    const responseTimeValue = host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent ?? ''
    const dayAverage = host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')?.textContent ?? ''
    const delta = host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.textContent ?? ''

    expect(responseTimeValue).toMatch(/ms|s/)
    expect(dayAverage).toMatch(/ms|s/)
    expect(delta).toContain('%')
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.className).toContain(
      'text-error',
    )
    expect(host?.textContent).toContain('Response time')
  })

  it('keeps response-time day average visible when the recent window is idle', () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 10,
          successCount: 10,
          failureCount: 0,
          totalCost: 0.5,
          totalTokens: 1000,
        }}
        rate={{
          tokensPerMinute: 0,
          spendRate: 0,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={{
          rangeStart: '2026-04-10T00:00:00.000Z',
          rangeEnd: '2026-04-10T00:08:00.000Z',
          bucketSeconds: 60,
          points: [
            {
              bucketStart: '2026-04-10T00:00:00.000Z',
              bucketEnd: '2026-04-10T00:01:00.000Z',
              totalCount: 2,
              successCount: 2,
              failureCount: 0,
              totalTokens: 1000,
              totalCost: 0.5,
              firstResponseByteTotalSampleCount: 2,
              firstResponseByteTotalAvgMs: 500,
            },
          ],
        }}
        loading={false}
        error={null}
      />,
    )

    expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')?.textContent).toBe(
      '500 ms',
    )
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.textContent).toBe('—')
  })

  it('recomputes the response-time recent window when now changes without falling back to older samples', () => {
    const timeseries: TimeseriesResponse = {
      rangeStart: '2026-04-10T00:00:00.000Z',
      rangeEnd: '2026-04-10T00:03:00.000Z',
      bucketSeconds: 60,
      points: [
        {
          bucketStart: '2026-04-10T00:02:00.000Z',
          bucketEnd: '2026-04-10T00:03:00.000Z',
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalTokens: 1000,
          totalCost: 0.5,
          firstResponseByteTotalSampleCount: 2,
          firstResponseByteTotalAvgMs: 500,
        },
      ],
    }

    const renderOverview = (now: Date) => (
      <TodayStatsOverview
        stats={{
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalCost: 0.5,
          totalTokens: 1000,
        }}
        rate={{
          tokensPerMinute: 0,
          spendRate: 0,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={timeseries}
        loading={false}
        error={null}
        now={now}
      />
    )

    render(renderOverview(new Date('2026-04-10T00:04:00.000Z')))

    expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe('500 ms')

    act(() => {
      root?.render(renderOverview(new Date('2026-04-10T00:10:00.000Z')))
    })

    expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe('—')
    expect(host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')?.textContent).toBe(
      '500 ms',
    )
  })
})
