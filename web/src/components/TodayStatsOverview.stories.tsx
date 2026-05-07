import { useEffect, useMemo, useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import type { ParallelWorkStatsResponse, StatsResponse, TimeseriesResponse } from '../lib/api'
import { TodayStatsOverview } from './TodayStatsOverview'
import type { DashboardTodayRateSnapshot } from './dashboardTodayRateSnapshot'

const sampleStats: StatsResponse = {
  totalCount: 2184,
  successCount: 2149,
  failureCount: 35,
  totalCost: 12.47,
  totalTokens: 842190,
}

const sampleRate: DashboardTodayRateSnapshot = {
  tokensPerMinute: 1000.6,
  spendRate: 0.1,
  windowMinutes: 5,
  available: true,
}

const comparisonStats: StatsResponse = {
  totalCount: 1760,
  successCount: 1704,
  failureCount: 56,
  totalCost: 10.12,
  totalTokens: 640000,
}

const previous7dStats: StatsResponse = {
  totalCount: 12880,
  successCount: 12461,
  failureCount: 419,
  totalCost: 72.1,
  totalTokens: 4380000,
}

const sampleTimeseries: TimeseriesResponse = {
  rangeStart: '2026-04-10T00:00:00.000Z',
  rangeEnd: '2026-04-10T00:08:00.000Z',
  bucketSeconds: 60,
  points: Array.from({ length: 8 }, (_, index) => ({
    bucketStart: new Date(Date.parse('2026-04-10T00:00:00.000Z') + index * 60_000).toISOString(),
    bucketEnd: new Date(Date.parse('2026-04-10T00:01:00.000Z') + index * 60_000).toISOString(),
    totalCount: index % 3 === 0 ? 0 : 2 + index,
    successCount: index % 3 === 0 ? 0 : 2 + index,
    failureCount: index === 5 ? 1 : 0,
    totalTokens: 78000 + index * 6100,
    cacheInputTokens: 18000 + index * 1200,
    totalCost: Number((1.1 + index * 0.08).toFixed(2)),
    firstResponseByteTotalSampleCount: index % 3 === 0 ? 0 : 2 + index,
    firstResponseByteTotalAvgMs: index % 3 === 0 ? null : Number((780 + index * 96.5).toFixed(1)),
  })),
}

const comparisonTimeseries: TimeseriesResponse = {
  ...sampleTimeseries,
  rangeStart: '2026-04-09T00:00:00.000Z',
  rangeEnd: '2026-04-09T00:07:00.000Z',
  points: sampleTimeseries.points.slice(0, 7).map((point, index) => ({
    ...point,
    bucketStart: new Date(Date.parse('2026-04-09T00:00:00.000Z') + index * 60_000).toISOString(),
    bucketEnd: new Date(Date.parse('2026-04-09T00:01:00.000Z') + index * 60_000).toISOString(),
    firstResponseByteTotalAvgMs:
      point.firstResponseByteTotalAvgMs == null
        ? null
        : Number((point.firstResponseByteTotalAvgMs * 0.82).toFixed(1)),
  })),
}

const sampleParallelWorkStats: ParallelWorkStatsResponse = {
  current: {
    rangeStart: '2026-04-10T00:00:00.000Z',
    rangeEnd: '2026-04-10T00:08:00.000Z',
    bucketSeconds: 60,
    completeBucketCount: 8,
    activeBucketCount: 7,
    minCount: 0,
    maxCount: 6,
    avgCount: 3.38,
    points: [1, 2, 4, 3, 5, 6, 4, 5].map((parallelCount, index) => ({
      bucketStart: new Date(Date.parse('2026-04-10T00:00:00.000Z') + index * 60_000).toISOString(),
      bucketEnd: new Date(Date.parse('2026-04-10T00:01:00.000Z') + index * 60_000).toISOString(),
      parallelCount,
    })),
  },
  minute7d: {} as never,
  hour30d: {} as never,
  dayAll: {} as never,
}

const comparisonParallelWorkStats: ParallelWorkStatsResponse = {
  ...sampleParallelWorkStats,
  current: {
    ...sampleParallelWorkStats.current,
    avgCount: 2.62,
    points: [1, 2, 2, 3, 3, 4, 3].map((parallelCount, index) => ({
      bucketStart: new Date(Date.parse('2026-04-09T00:00:00.000Z') + index * 60_000).toISOString(),
      bucketEnd: new Date(Date.parse('2026-04-09T00:01:00.000Z') + index * 60_000).toISOString(),
      parallelCount,
    })),
  },
}

const comparisonArgs = {
  timeseries: sampleTimeseries,
  comparisonStats,
  comparisonTimeseries,
  previous7dStats,
  parallelWorkStats: sampleParallelWorkStats,
  comparisonParallelWorkStats,
}

const meta = {
  title: 'Dashboard/TodayStatsOverview',
  component: TodayStatsOverview,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-[1560px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof TodayStatsOverview>

export default meta

type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    loading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /tokens per minute|每分钟 tokens/i }))
    await expect(within(document.body).getByRole('tooltip')).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: /response time|响应时间/i }))
    await expect(within(document.body).getByRole('tooltip')).toBeInTheDocument()
  },
}

export const DesktopSingleRow: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    loading: false,
    error: null,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const tiles = canvas.getAllByTestId('today-stats-metric-tile')
    await expect(tiles).toHaveLength(7)
    const labels = tiles.map((tile) => tile.textContent ?? '')
    expect(labels[3]).toMatch(/parallel conversations|并行对话/i)
    expect(labels[4]).toMatch(/response time|响应时间/i)
  },
}

export const EmbeddedTodayTab: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
}

export const RateLoading: Story = {
  args: {
    stats: sampleStats,
    rate: null,
    loading: false,
    rateLoading: true,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
}

export const RateUnavailable: Story = {
  args: {
    stats: sampleStats,
    rate: null,
    loading: false,
    rateLoading: false,
    rateError: 'timeseries failed',
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
}

export const ZeroRate: Story = {
  args: {
    stats: sampleStats,
    rate: {
      tokensPerMinute: 0,
      spendRate: 0,
      windowMinutes: 0,
      available: true,
    },
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
}

export const OverflowFallback: Story = {
  args: {
    stats: {
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 539.42,
      totalTokens: 1314275579,
    },
    rate: sampleRate,
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      const totalTokensValue = canvas.getByTestId('today-stats-value-total-tokens')
      expect(totalTokensValue).toHaveAttribute('data-compact', 'true')
      expect(totalTokensValue.textContent ?? '').toContain('1.31B')
    })
  },
}

export const Loading: Story = {
  args: {
    stats: null,
    rate: null,
    loading: true,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    stats: null,
    rate: {
      tokensPerMinute: 0,
      spendRate: 0,
      windowMinutes: 0,
      available: true,
    },
    loading: false,
    error: null,
  },
}

export const LoadError: Story = {
  args: {
    stats: null,
    rate: null,
    loading: false,
    error: 'Request failed: 500 unable to open database file',
  },
}

function buildAnimatedStats(step: number): StatsResponse {
  const totalCount = sampleStats.totalCount + step * 17
  const failureCount = 18 + (step % 5) * 3
  const successCount = Math.max(totalCount - failureCount, 0)
  const totalTokens = sampleStats.totalTokens + step * 5630 + (step % 3) * 830
  const totalCost = Number((sampleStats.totalCost + step * 0.11 + (step % 4) * 0.03).toFixed(2))

  return {
    totalCount,
    successCount,
    failureCount,
    totalCost,
    totalTokens,
  }
}

function buildAnimatedRate(step: number): DashboardTodayRateSnapshot {
  return {
    tokensPerMinute: 1000 + step * 27,
    spendRate: Number((0.1 + step * 0.006).toFixed(3)),
    windowMinutes: 5,
    available: true,
  }
}

function LiveTickerPreview() {
  const [ready, setReady] = useState(false)
  const [step, setStep] = useState(0)

  useEffect(() => {
    const warmup = window.setTimeout(() => {
      setReady(true)
    }, 900)

    return () => {
      window.clearTimeout(warmup)
    }
  }, [])

  useEffect(() => {
    if (!ready) return undefined
    const timer = window.setInterval(() => {
      setStep((value) => value + 1)
    }, 1400)

    return () => {
      window.clearInterval(timer)
    }
  }, [ready])

  const stats = useMemo(() => buildAnimatedStats(step), [step])
  const rate = useMemo(() => buildAnimatedRate(step), [step])

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between rounded-xl border border-primary/25 bg-primary/10 px-4 py-2 text-sm text-base-content/75">
        <span>Live demo auto-updates every 1.4s</span>
        <span className="font-semibold text-primary">Tick #{step}</span>
      </div>
      <TodayStatsOverview
        stats={ready ? stats : null}
        rate={ready ? rate : null}
        timeseries={ready ? sampleTimeseries : null}
        comparisonTimeseries={ready ? comparisonTimeseries : null}
        loading={!ready}
        error={null}
      />
    </div>
  )
}

export const LiveTicker: Story = {
  args: {
    stats: null,
    rate: null,
    loading: true,
    error: null,
  },
  render: () => <LiveTickerPreview />,
}

function StateGalleryPreview() {
  return (
    <div className="space-y-6">
      <div className="section-heading">
        <h2 className="section-title">Today stats states</h2>
        <p className="section-description">
          Desktop preview keeps all seven KPI tiles on one row while preserving loading, partial fallback, and failure states.
        </p>
      </div>
      <div className="grid gap-10">
        <div className="space-y-4">
          <div className="text-sm font-semibold text-base-content/70">Populated</div>
          <TodayStatsOverview
            stats={sampleStats}
            rate={sampleRate}
            {...comparisonArgs}
            loading={false}
            error={null}
          />
        </div>
        <div className="space-y-4">
          <div className="text-sm font-semibold text-base-content/70">Rate loading</div>
          <TodayStatsOverview
            stats={sampleStats}
            rate={null}
            {...comparisonArgs}
            loading={false}
            rateLoading
            error={null}
          />
        </div>
        <div className="space-y-4">
          <div className="text-sm font-semibold text-base-content/70">Rate unavailable</div>
          <TodayStatsOverview
            stats={sampleStats}
            rate={null}
            {...comparisonArgs}
            loading={false}
            rateError="timeseries failed"
            error={null}
          />
        </div>
        <div className="space-y-4">
          <div className="text-sm font-semibold text-base-content/70">Loading</div>
          <TodayStatsOverview stats={null} rate={null} loading error={null} />
        </div>
        <div className="space-y-4">
          <div className="text-sm font-semibold text-base-content/70">Load error</div>
          <TodayStatsOverview
            stats={null}
            rate={null}
            loading={false}
            error="Request failed: 500 unable to open database file"
          />
        </div>
      </div>
    </div>
  )
}

export const StateGallery: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    loading: false,
    error: null,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1440',
    },
  },
  render: () => <StateGalleryPreview />,
}
