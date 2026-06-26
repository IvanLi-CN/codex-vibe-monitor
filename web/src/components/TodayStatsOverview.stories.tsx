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
  inProgressConversationCount: 11,
  inProgressRetryConversationCount: 4,
  inProgressAvgWaitMs: 1850,
  nonSuccessCost: 0.83,
  nonSuccessTokens: 12640,
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
  inProgressConversationCount: 7,
  inProgressRetryConversationCount: 2,
  inProgressAvgWaitMs: 1320,
  nonSuccessCost: 0.96,
  nonSuccessTokens: 14020,
}

const previous7dStats: StatsResponse = {
  totalCount: 12880,
  successCount: 12461,
  failureCount: 419,
  totalCost: 72.1,
  totalTokens: 4380000,
  inProgressConversationCount: 5,
  inProgressRetryConversationCount: 1,
  inProgressAvgWaitMs: 1100,
  nonSuccessCost: 5.6,
  nonSuccessTokens: 88310,
}

const sampleTimeseries: TimeseriesResponse = {
  rangeStart: '2026-04-10T00:00:00.000Z',
  rangeEnd: '2026-04-10T00:08:00.000Z',
  bucketSeconds: 60,
  points: Array.from({ length: 8 }, (_, index) => {
    const sampleCount = index % 3 === 0 ? 0 : 2 + index
    return {
      bucketStart: new Date(Date.parse('2026-04-10T00:00:00.000Z') + index * 60_000).toISOString(),
      bucketEnd: new Date(Date.parse('2026-04-10T00:01:00.000Z') + index * 60_000).toISOString(),
      totalCount: index % 3 === 0 ? 0 : 2 + index,
      successCount: index % 3 === 0 ? 0 : 2 + index,
      failureCount: index === 5 ? 1 : 0,
      totalTokens: 78000 + index * 6100,
      cacheInputTokens: 18000 + index * 1200,
      totalCost: Number((1.1 + index * 0.08).toFixed(2)),
      avgTotalMs: sampleCount > 0 ? Number((1260 + index * 132.5).toFixed(1)) : null,
      totalLatencySampleCount: sampleCount,
      firstResponseByteTotalSampleCount: sampleCount,
      firstResponseByteTotalAvgMs: index % 3 === 0 ? null : Number((780 + index * 96.5).toFixed(1)),
      firstResponseByteTotalP95Ms: index % 3 === 0 ? null : Number((960 + index * 118.5).toFixed(1)),
    }
  }),
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

const comparisonArgs = {
  timeseries: sampleTimeseries,
  comparisonStats,
  comparisonTimeseries,
  previous7dStats,
}

function buildParallelWorkWindow(
  counts: number[],
  {
    rangeStart,
    bucketSeconds = 60,
  }: {
    rangeStart: string
    bucketSeconds?: number
  },
): ParallelWorkStatsResponse['current'] {
  const startMs = Date.parse(rangeStart)
  const lastBucketStart = startMs + Math.max(counts.length - 1, 0) * bucketSeconds * 1000
  const rangeEnd = new Date(lastBucketStart + bucketSeconds * 1000).toISOString()

  return {
    rangeStart,
    rangeEnd,
    bucketSeconds,
    completeBucketCount: counts.length,
    activeBucketCount: counts.length,
    minCount: counts.length > 0 ? Math.min(...counts) : null,
    maxCount: counts.length > 0 ? Math.max(...counts) : null,
    avgCount:
      counts.length > 0
        ? Number((counts.reduce((sum, value) => sum + value, 0) / counts.length).toFixed(2))
        : null,
    points: counts.map((parallelCount, index) => ({
      bucketStart: new Date(startMs + index * bucketSeconds * 1000).toISOString(),
      bucketEnd: new Date(startMs + (index + 1) * bucketSeconds * 1000).toISOString(),
      parallelCount,
    })),
  }
}

const sampleParallelWorkStats: ParallelWorkStatsResponse = {
  current: buildParallelWorkWindow([8, 10, 9], {
    rangeStart: '2026-04-10T00:00:00.000Z',
  }),
  minute7d: buildParallelWorkWindow([6, 7, 8, 9], {
    rangeStart: '2026-04-03T00:00:00.000Z',
  }),
  hour30d: buildParallelWorkWindow([5, 6, 7], {
    rangeStart: '2026-03-11T00:00:00.000Z',
    bucketSeconds: 3600,
  }),
  dayAll: buildParallelWorkWindow([7], {
    rangeStart: '2026-04-09T00:00:00.000Z',
    bucketSeconds: 86400,
  }),
}

const comparisonParallelWorkStats: ParallelWorkStatsResponse = {
  current: buildParallelWorkWindow([7, 8, 9], {
    rangeStart: '2026-04-09T00:00:00.000Z',
  }),
  minute7d: buildParallelWorkWindow([5, 6, 7, 8], {
    rangeStart: '2026-04-02T00:00:00.000Z',
  }),
  hour30d: buildParallelWorkWindow([4, 5, 6], {
    rangeStart: '2026-03-10T00:00:00.000Z',
    bucketSeconds: 3600,
  }),
  dayAll: buildParallelWorkWindow([8], {
    rangeStart: '2026-04-08T00:00:00.000Z',
    bucketSeconds: 86400,
  }),
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
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
    loading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /tokens per minute|每分钟 tokens/i }))
    await expect(within(document.body).getByRole('tooltip')).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: /time to first byte|首字用时/i }))
    await expect(within(document.body).getByRole('tooltip')).toBeInTheDocument()
  },
}

export const DesktopSingleRow: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
    expect(labels[3]).toMatch(/in-progress conversations|进行中对话/i)
    expect(labels[4]).toMatch(/time to first byte|首字用时/i)
  },
}

export const EmbeddedTodayTab: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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

export const ScopedAccountEmbedded: Story = {
  args: {
    stats: sampleStats,
    rate: sampleRate,
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
    const grid = canvas.getByTestId('today-stats-metrics-grid')
    const tiles = canvas.getAllByTestId('today-stats-metric-tile')
    await expect(grid).toHaveClass(/lg:grid-cols-4/)
    await expect(grid).toHaveClass(/xl:grid-cols-7/)
    await expect(tiles).toHaveLength(7)
    await expect(canvas.getByText(/in-progress conversations|进行中对话/i)).toBeInTheDocument()
    await expect(canvas.getByTestId('today-stats-secondary-in-progress-delta')).toHaveTextContent('+37.5%')
    await expect(canvas.getByTestId('today-stats-secondary-in-progress-day-average')).toHaveTextContent('9')
    await expect(canvas.getByTestId('today-stats-secondary-in-progress-retry')).toBeVisible()
    await expect(canvas.getByTestId('today-stats-secondary-tpm-per-conversation')).not.toHaveTextContent('—')
    await expect(canvas.getByTestId('today-stats-secondary-success-ratio')).not.toHaveTextContent('—')
    await expect(canvas.getByTestId('today-stats-secondary-response-time-avg-total')).not.toHaveTextContent('—')
    await expect(canvas.getByTestId('today-stats-secondary-cost-failed')).not.toHaveTextContent('—')
    await expect(canvas.getByTestId('today-stats-secondary-tokens-failed')).not.toHaveTextContent('—')
    await expect(canvas.getByText(/time to first byte|首字用时/i)).toBeInTheDocument()
  },
}

export const RateLoading: Story = {
  args: {
    stats: sampleStats,
    rate: null,
    loading: false,
    rateLoading: true,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
      inProgressConversationCount: 11,
      inProgressRetryConversationCount: 4,
      inProgressAvgWaitMs: 1850,
      nonSuccessCost: 14.72,
      nonSuccessTokens: 56210,
    },
    rate: sampleRate,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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

export const Desktop1280PrecisionGuard: Story = {
  args: {
    stats: {
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 488.96,
      totalTokens: 1_049_600_000,
      inProgressConversationCount: 11,
      inProgressRetryConversationCount: 4,
      inProgressAvgWaitMs: 1850,
      nonSuccessCost: 60.93,
      nonSuccessTokens: 88_834_346,
    },
    rate: {
      tokensPerMinute: 1_049_600,
      spendRate: 8.31,
      windowMinutes: 5,
      available: true,
    },
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1280',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      const totalTokensValue = canvas.getByTestId('today-stats-value-total-tokens')
      expect(totalTokensValue.textContent ?? '').toContain('1.05B')
      expect(totalTokensValue.textContent ?? '').not.toBe('1B')
      expect(canvas.getByTestId('today-stats-secondary-tokens-failed').textContent ?? '').toMatch(/88(\.8|\.83)?M/i)
      expect(canvas.getByTestId('today-stats-secondary-tokens-delta').textContent ?? '').not.toContain('…')
    })
  },
}

export const Desktop1280LabelGuard: Story = {
  args: {
    stats: {
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 488.96,
      totalTokens: 1_049_600_000,
      inProgressConversationCount: 11,
      inProgressRetryConversationCount: 4,
      inProgressAvgWaitMs: 1850,
      nonSuccessCost: 60.93,
      nonSuccessTokens: 88_834_346,
    },
    rate: {
      tokensPerMinute: 1_049_600,
      spendRate: 8.31,
      windowMinutes: 5,
      available: true,
    },
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1280',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      const labels = canvas.getAllByRole('button')
      for (const label of labels) {
        expect(label.textContent ?? '').not.toContain('\n')
      }
      expect(canvas.getByTestId('today-stats-label-total-tokens')).toHaveTextContent('Today Token')
      expect(canvas.getByTestId('today-stats-secondary-cost-failed').textContent ?? '').not.toContain('…')
      expect(canvas.getByTestId('today-stats-secondary-tokens-failed').textContent ?? '').not.toContain('…')
    })
  },
}

export const NarrowTileMetaStackGuard: Story = {
  args: {
    stats: {
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 488.96,
      totalTokens: 1_049_600_000,
      inProgressConversationCount: 11,
      inProgressRetryConversationCount: 4,
      inProgressAvgWaitMs: 1850,
      nonSuccessCost: 60.93,
      nonSuccessTokens: 88_834_346,
    },
    rate: {
      tokensPerMinute: 1_049_600,
      spendRate: 8.31,
      windowMinutes: 5,
      available: true,
    },
    ...comparisonArgs,
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
    loading: false,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1280',
    },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto w-full max-w-[1280px]">
        <div className="grid grid-cols-7 gap-3">
          <div className="col-start-7 min-w-0">
            <Story />
          </div>
        </div>
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      const tile = canvas.getByTestId('today-stats-metric-tile')
      expect(tile).toHaveAttribute('data-stack-meta', 'true')
      expect(canvas.getByTestId('today-stats-value-total-tokens-stacked-meta')).toBeVisible()
      expect(canvas.getByTestId('today-stats-secondary-tokens-delta').textContent ?? '').not.toContain('…')
      expect(canvas.getByTestId('today-stats-secondary-tokens-failed').textContent ?? '').toMatch(/88(\.8|\.83)?M/i)
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

export const NarrowDesktopLoading: Story = {
  args: {
    stats: null,
    rate: null,
    loading: true,
    error: null,
    showSurface: false,
    showHeader: false,
    showDayBadge: false,
  },
  parameters: {
    viewport: {
      defaultViewport: 'desktop1280',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      const loadingTile = canvas.getByTestId('today-stats-value-tpm-loading')
      expect(loadingTile.className).toContain('w-full')
      expect(loadingTile.className).toContain('max-w-[7.5rem]')
    })
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
    parallelWorkStats: sampleParallelWorkStats,
    comparisonParallelWorkStats,
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
    inProgressConversationCount: (sampleStats.inProgressConversationCount ?? 0) + (step % 4),
    inProgressRetryConversationCount: 2 + (step % 3),
    inProgressAvgWaitMs: 1400 + step * 60,
    nonSuccessCost: Number((0.45 + step * 0.04).toFixed(2)),
    nonSuccessTokens: 8400 + step * 220,
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
        parallelWorkStats={sampleParallelWorkStats}
        comparisonParallelWorkStats={comparisonParallelWorkStats}
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
            parallelWorkStats={sampleParallelWorkStats}
            comparisonParallelWorkStats={comparisonParallelWorkStats}
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
            parallelWorkStats={sampleParallelWorkStats}
            comparisonParallelWorkStats={comparisonParallelWorkStats}
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
            parallelWorkStats={sampleParallelWorkStats}
            comparisonParallelWorkStats={comparisonParallelWorkStats}
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
