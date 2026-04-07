import { useLayoutEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import {
  HISTORY_CALENDAR_BUCKET,
  HISTORY_CALENDAR_RANGE,
  UsageCalendar,
  type UsageCalendarProps,
} from './UsageCalendar'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1280px]">{children}</div>
    </div>
  )
}

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
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
    const amplitude = (index * 7 + bucketStart.getDay() * 2) % 12
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: amplitude,
      successCount: amplitude,
      failureCount: 0,
      totalTokens: amplitude * 900,
      totalCost: Number((amplitude * 0.28).toFixed(2)),
    })
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: endExclusive.toISOString(),
    bucketSeconds: 86400,
    points,
  }
}

function UsageCalendarMockApi({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)
  const dailyFixtureRef = useRef(buildDailyPoints())

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource

    window.fetch = async (input, init) => {
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(inputUrl, window.location.origin)
      if (
        url.pathname === '/api/stats/timeseries' &&
        url.searchParams.get('range') === HISTORY_CALENDAR_RANGE &&
        url.searchParams.get('bucket') === HISTORY_CALENDAR_BUCKET
      ) {
        return jsonResponse(dailyFixtureRef.current)
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
  title: 'Dashboard/UsageCalendar',
  component: UsageCalendar,
  tags: ['autodocs'],
  args: {},
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <UsageCalendarMockApi>
          <StorySurface>
            <Story />
          </StorySurface>
        </UsageCalendarMockApi>
      </I18nProvider>
    ),
  ],
} satisfies Meta<UsageCalendarProps>

export default meta

type Story = StoryObj<typeof meta>

export const Standalone: Story = {}

export const Embedded: Story = {
  args: {
    metric: 'totalCost',
    showSurface: false,
    showMetricToggle: false,
    showMeta: false,
  },
  render: (args) => (
    <section className="surface-panel">
      <div className="surface-panel-body gap-4">
        <UsageCalendar {...args} />
      </div>
    </section>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => {
      expect(canvas.queryByRole('tab', { name: /金额|cost/i })).toBeNull()
      expect(canvas.queryByText(/历史|history/i)).toBeNull()
      expect(canvas.queryByText(/时区|timezone/i)).toBeNull()
    })
  },
}

export const MetricSwitchFlow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: /金额|cost/i }))
    await waitFor(() => {
      expect(canvas.getByRole('tab', { name: /金额|cost/i }).getAttribute('aria-selected')).toBe('true')
    })
  },
}
