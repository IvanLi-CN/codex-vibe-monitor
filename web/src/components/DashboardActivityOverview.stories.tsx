import { useLayoutEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  DashboardActivityOverview,
} from './DashboardActivityOverview'

type SummaryKey = 'today' | '1d' | '7d'
type TimeseriesKey = 'today:1m' | '1d:1m' | '7d:1h' | '6mo:1d'
type PersistedRange = 'today' | '1d' | '7d' | 'usage' | null
type WindowWithDashboardFetchLog = Window & {
  __dashboardOverviewFetchLog__?: string[]
}

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1660px]">{children}</div>
    </div>
  )
}

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
}

function createSummary(totalCount: number, successCount: number, failureCount: number, totalCost: number, totalTokens: number) {
  return { totalCount, successCount, failureCount, totalCost, totalTokens }
}

function buildTodayMinutePoints() {
  const rangeStart = new Date('2026-04-08T00:00:00+08:00')
  const rangeEnd = new Date('2026-04-08T12:24:00+08:00')
  const points: Array<Record<string, number | string>> = []

  for (let minute = 0; minute <= 12 * 60 + 24; minute += 1) {
    const bucketStart = new Date(rangeStart.getTime() + minute * 60_000)
    const bucketEnd = new Date(bucketStart.getTime() + 60_000)
    const totalCount = minute % 11 === 0 ? 0 : (minute % 4) + (minute % 9 === 0 ? 2 : 0)
    const failureCount = totalCount > 0 && minute % 13 === 0 ? 1 : 0
    const successCount = Math.max(totalCount - failureCount, 0)
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 420,
      totalCost: Number((totalCount * 0.0195).toFixed(4)),
    })
  }

  return {
    rangeStart: rangeStart.toISOString(),
    rangeEnd: rangeEnd.toISOString(),
    bucketSeconds: 60,
    points,
  }
}

function build24HourPoints() {
  const end = new Date('2026-04-08T12:20:00+08:00')
  const start = new Date(end.getTime() - 24 * 60 * 60_000)
  const points: Array<Record<string, number | string>> = []
  for (let index = 0; index < 24 * 60; index += 1) {
    const bucketStart = new Date(start.getTime() + index * 60_000)
    const bucketEnd = new Date(bucketStart.getTime() + 60_000)
    const totalCount = index % 17 === 0 ? 0 : (index % 6)
    const failureCount = totalCount > 0 && index % 19 === 0 ? 1 : 0
    const successCount = Math.max(totalCount - failureCount, 0)
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 390,
      totalCost: Number((totalCount * 0.017).toFixed(4)),
    })
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: end.toISOString(),
    bucketSeconds: 60,
    points,
  }
}

function buildHourlyPoints() {
  const end = new Date('2026-04-08T00:00:00+08:00')
  const start = new Date(end.getTime() - 7 * 24 * 60 * 60_000)
  const points: Array<Record<string, number | string>> = []
  for (let index = 0; index < 7 * 24; index += 1) {
    const bucketStart = new Date(start.getTime() + index * 60 * 60_000)
    const bucketEnd = new Date(bucketStart.getTime() + 60 * 60_000)
    const hour = bucketStart.getHours()
    const day = bucketStart.getDay()
    const density = ((hour + 3) * (day + 2)) % 9
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: density,
      successCount: Math.max(density - (density > 6 ? 1 : 0), 0),
      failureCount: density > 6 ? 1 : 0,
      totalTokens: density * 620,
      totalCost: Number((density * 0.23).toFixed(2)),
    })
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: end.toISOString(),
    bucketSeconds: 3600,
    points,
  }
}

function buildDailyPoints() {
  const endExclusive = new Date('2026-04-09T00:00:00+08:00')
  const start = new Date(endExclusive)
  start.setDate(start.getDate() - 180)
  const points: Array<Record<string, number | string>> = []
  for (let index = 0; index < 180; index += 1) {
    const bucketStart = new Date(start)
    bucketStart.setDate(start.getDate() + index)
    const bucketEnd = new Date(bucketStart)
    bucketEnd.setDate(bucketEnd.getDate() + 1)
    const weekday = bucketStart.getDay()
    const amplitude = (index * 5 + weekday * 3) % 11
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: amplitude,
      successCount: amplitude,
      failureCount: 0,
      totalTokens: amplitude * 840,
      totalCost: Number((amplitude * 0.31).toFixed(2)),
    })
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: endExclusive.toISOString(),
    bucketSeconds: 86400,
    points,
  }
}

const SUMMARY_FIXTURES: Record<SummaryKey, ReturnType<typeof createSummary>> = {
  today: createSummary(12474, 9949, 2525, 539.42, 1314275579),
  '1d': createSummary(76421, 70115, 6306, 3128.74, 8764311220),
  '7d': createSummary(182904, 171240, 11664, 8422.18, 21640351742),
}

const TIMESERIES_FIXTURES: Record<TimeseriesKey, ReturnType<typeof buildTodayMinutePoints> | ReturnType<typeof build24HourPoints> | ReturnType<typeof buildHourlyPoints> | ReturnType<typeof buildDailyPoints>> = {
  'today:1m': buildTodayMinutePoints(),
  '1d:1m': build24HourPoints(),
  '7d:1h': buildHourlyPoints(),
  '6mo:1d': buildDailyPoints(),
}

function DashboardOverviewMockApi({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)

  useLayoutEffect(() => {
    const windowWithFetchLog = window as WindowWithDashboardFetchLog
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource
    windowWithFetchLog.__dashboardOverviewFetchLog__ = []

    window.fetch = async (input, init) => {
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(inputUrl, window.location.origin)
      windowWithFetchLog.__dashboardOverviewFetchLog__?.push(`${url.pathname}${url.search}`)

      if (url.pathname === '/api/stats/summary') {
        const windowKey = url.searchParams.get('window') as SummaryKey | null
        if (windowKey && windowKey in SUMMARY_FIXTURES) {
          return jsonResponse(SUMMARY_FIXTURES[windowKey])
        }
      }

      if (url.pathname === '/api/stats/timeseries') {
        const range = url.searchParams.get('range')
        const bucket = url.searchParams.get('bucket')
        const key = `${range}:${bucket}` as TimeseriesKey
        if (key in TIMESERIES_FIXTURES) {
          return jsonResponse(TIMESERIES_FIXTURES[key])
        }
      }

      return (originalFetchRef.current ?? fetch)(input as RequestInfo | URL, init)
    }

    Object.defineProperty(window, 'EventSource', {
      configurable: true,
      writable: true,
      value: undefined,
    })

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
      Object.defineProperty(window, 'EventSource', {
        configurable: true,
        writable: true,
        value: originalEventSourceRef.current,
      })
      delete windowWithFetchLog.__dashboardOverviewFetchLog__
    }
  }, [])

  return <>{children}</>
}

function RangeStorageHarness({
  persistedRange,
  children,
}: {
  persistedRange: PersistedRange
  children: ReactNode
}) {
  useLayoutEffect(() => {
    const previousValue = window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
    if (persistedRange) {
      window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, persistedRange)
    } else {
      window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
    }

    return () => {
      if (previousValue === null) {
        window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
      } else {
        window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, previousValue)
      }
    }
  }, [persistedRange])

  return <>{children}</>
}

const meta = {
  title: 'Dashboard/DashboardActivityOverview',
  component: DashboardActivityOverview,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
    persistedRange: null,
  },
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <DashboardOverviewMockApi>
          <StorySurface>
            <RangeStorageHarness persistedRange={(context.parameters.persistedRange ?? null) as PersistedRange}>
              <Story />
            </RangeStorageHarness>
          </StorySurface>
        </DashboardOverviewMockApi>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardActivityOverview>

export default meta

type Story = StoryObj<typeof meta>

export const TodayView: Story = {}

export const TodayCostCumulative: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /金额|cost/i }))
    await waitFor(() => {
      expect(canvas.getByTestId('dashboard-today-activity-chart')).toHaveAttribute('data-chart-mode', 'cumulative-area')
    })
  },
}

export const HistoryView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /历史|history/i }))
    await waitFor(() => {
      expect(canvas.getByRole('tab', { name: /历史|history/i })).toHaveAttribute('aria-selected', 'true')
    })
    await expect(canvas.getByTestId('usage-calendar-card')).toBeVisible()
    await expect(canvas.queryByText(/总 TOKENS|total tokens/i)).toBeNull()
  },
}

export const RestoresPersistedHistory: Story = {
  parameters: {
    persistedRange: 'usage',
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      expect(canvas.getByRole('tab', { name: /历史|history/i })).toHaveAttribute('aria-selected', 'true')
    })
  },
}

export const MetricMemoryFlow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /金额|cost/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /7 日|7 days/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /tokens/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /今日|today/i }))
    await waitFor(() => {
      expect(canvas.getByRole('tab', { name: /金额|cost/i })).toHaveAttribute('aria-selected', 'true')
    })
  },
}

export const LoadsRangesOnDemand: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const windowWithFetchLog = window as WindowWithDashboardFetchLog

    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? []
      expect(fetchLog).toContain('/api/stats/summary?window=today')
      expect(fetchLog).toContain('/api/stats/timeseries?range=today&bucket=1m')
      expect(fetchLog.some((entry) => entry.includes('window=1d'))).toBe(false)
      expect(fetchLog.some((entry) => entry.includes('window=7d'))).toBe(false)
    })

    await userEvent.click(canvas.getByRole('tab', { name: /7 日|7 days/i }))
    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? []
      expect(fetchLog).toContain('/api/stats/summary?window=7d')
      expect(fetchLog.some((entry) => entry.includes('window=1d'))).toBe(false)
    })

    await userEvent.click(canvas.getByRole('tab', { name: /24 小时|24 hours/i }))
    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? []
      expect(fetchLog).toContain('/api/stats/summary?window=1d')
    })
  },
}
