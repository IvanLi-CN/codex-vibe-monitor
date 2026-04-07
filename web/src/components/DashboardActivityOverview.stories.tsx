import { useLayoutEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import { DashboardActivityOverview } from './DashboardActivityOverview'

type SummaryKey = '1d' | '7d'
type TimeseriesKey = '1d:1m' | '7d:1h' | '6mo:1d'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1560px]">{children}</div>
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

function isoAt(date: Date) {
  return new Date(date).toISOString()
}

function buildMinutePoints() {
  const now = new Date()
  const points: Array<Record<string, number | string>> = []
  for (let hourOffset = 0; hourOffset < 12; hourOffset += 1) {
    for (let minuteSlot = 0; minuteSlot < 6; minuteSlot += 1) {
      const bucketStart = new Date(now.getTime() - (hourOffset * 60 + minuteSlot * 10) * 60_000)
      bucketStart.setSeconds(0, 0)
      const bucketEnd = new Date(bucketStart.getTime() + 60_000)
      const scale = hourOffset + minuteSlot + 1
      points.push({
        bucketStart: isoAt(bucketStart),
        bucketEnd: isoAt(bucketEnd),
        totalCount: scale % 5 === 0 ? 0 : scale * 2,
        successCount: scale * 2,
        failureCount: scale % 4 === 0 ? 1 : 0,
        totalTokens: scale * 380,
        totalCost: Number((scale * 0.17).toFixed(2)),
      })
    }
  }
  const rangeEnd = new Date(now)
  const rangeStart = new Date(rangeEnd.getTime() - 24 * 60 * 60_000)
  return {
    rangeStart: isoAt(rangeStart),
    rangeEnd: isoAt(rangeEnd),
    bucketSeconds: 60,
    points,
  }
}

function buildHourlyPoints() {
  const now = new Date()
  const end = new Date(now)
  end.setMinutes(0, 0, 0)
  const start = new Date(end.getTime() - 7 * 24 * 60 * 60_000)
  const points: Array<Record<string, number | string>> = []
  for (let index = 0; index < 7 * 24; index += 1) {
    const bucketStart = new Date(start.getTime() + index * 60 * 60_000)
    const bucketEnd = new Date(bucketStart.getTime() + 60 * 60_000)
    const hour = bucketStart.getHours()
    const day = bucketStart.getDay()
    const density = ((hour + 3) * (day + 2)) % 9
    points.push({
      bucketStart: isoAt(bucketStart),
      bucketEnd: isoAt(bucketEnd),
      totalCount: density,
      successCount: Math.max(density - (density > 6 ? 1 : 0), 0),
      failureCount: density > 6 ? 1 : 0,
      totalTokens: density * 620,
      totalCost: Number((density * 0.23).toFixed(2)),
    })
  }
  return {
    rangeStart: isoAt(start),
    rangeEnd: isoAt(end),
    bucketSeconds: 3600,
    points,
  }
}

function buildDailyPoints() {
  const endExclusive = new Date()
  endExclusive.setHours(0, 0, 0, 0)
  endExclusive.setDate(endExclusive.getDate() + 1)
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
      bucketStart: isoAt(bucketStart),
      bucketEnd: isoAt(bucketEnd),
      totalCount: amplitude,
      successCount: amplitude,
      failureCount: 0,
      totalTokens: amplitude * 840,
      totalCost: Number((amplitude * 0.31).toFixed(2)),
    })
  }
  return {
    rangeStart: isoAt(start),
    rangeEnd: isoAt(endExclusive),
    bucketSeconds: 86400,
    points,
  }
}

const SUMMARY_FIXTURES: Record<SummaryKey, ReturnType<typeof createSummary>> = {
  '1d': createSummary(76421, 70115, 6306, 3128.74, 8764311220),
  '7d': createSummary(182904, 171240, 11664, 8422.18, 21640351742),
}

const TIMESERIES_FIXTURES: Record<TimeseriesKey, ReturnType<typeof buildMinutePoints> | ReturnType<typeof buildHourlyPoints> | ReturnType<typeof buildDailyPoints>> = {
  '1d:1m': buildMinutePoints(),
  '7d:1h': buildHourlyPoints(),
  '6mo:1d': buildDailyPoints(),
}

function DashboardOverviewMockApi({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource

    window.fetch = async (input, init) => {
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(inputUrl, window.location.origin)

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
    }
  }, [])

  return <>{children}</>
}

const meta = {
  title: 'Dashboard/DashboardActivityOverview',
  component: DashboardActivityOverview,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <DashboardOverviewMockApi>
          <StorySurface>
            <Story />
          </StorySurface>
        </DashboardOverviewMockApi>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardActivityOverview>

export default meta

type Story = StoryObj<typeof meta>

export const Default24Hours: Story = {}

export const SevenDaysView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /7 日|7 days/i }))
    await waitFor(() => {
      expect(canvas.queryByRole('tab', { name: /24 小时|24 hours/i })?.getAttribute('aria-selected')).toBe('false')
    })
  },
}

export const HistoryView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /历史|history/i }))
    await waitFor(() => {
      expect(canvas.queryByRole('tab', { name: /历史|history/i })?.getAttribute('aria-selected')).toBe('true')
    })
    await waitFor(() => {
      expect(canvas.queryByText(/总 TOKENS|total tokens/i)).toBeNull()
    })
    await waitFor(() => {
      expect(canvas.queryByText(/时区|timezone/i)).toBeNull()
      expect(canvas.queryAllByText(/^历史$/i)).toHaveLength(1)
    })
  },
}

export const MetricMemoryFlow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /历史|history/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /金额|cost/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /7 日|7 days/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /tokens/i }))
    await userEvent.click(canvas.getByRole('tab', { name: /历史|history/i }))
    await waitFor(() => {
      expect(canvas.getByRole('tab', { name: /金额|cost/i }).getAttribute('aria-selected')).toBe('true')
    })
  },
}
