import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import type {
  ErrorDistributionResponse,
  FailureSummaryResponse,
  StatsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
} from '../lib/api'
import StatsPage from '../pages/Stats'
import { FullPageStorySurface, StorybookPageEnvironment, jsonResponse } from './storybookPageHelpers'

type StatsScenario = 'default' | 'timeseries-error'

type StatsStoryParameters = {
  scenario?: StatsScenario
}

function buildSummary(overrides: Partial<StatsResponse>): StatsResponse {
  return {
    totalCount: 0,
    successCount: 0,
    failureCount: 0,
    totalCost: 0,
    totalTokens: 0,
    ...overrides,
  }
}

function buildTimeseriesPoints({
  count,
  bucketSeconds,
  startMs,
  offset = 0,
}: {
  count: number
  bucketSeconds: number
  startMs: number
  offset?: number
}) {
  return Array.from({ length: count }, (_, index) => {
    const bucketStartMs = startMs + index * bucketSeconds * 1000
    const bucketEndMs = bucketStartMs + bucketSeconds * 1000
    const totalCount = 24 + ((index + offset) % 6) * 8
    const failureCount = index % 5 === 0 ? 3 : index % 3 === 0 ? 1 : 0
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount,
      successCount: totalCount - failureCount,
      failureCount,
      totalTokens: totalCount * 4100,
      totalCost: Number((totalCount * 0.021).toFixed(4)),
      firstResponseByteTotalAvgMs: 520 + (index % 7) * 32,
      firstResponseByteTotalP95Ms: 710 + (index % 5) * 44,
      firstResponseByteTotalSampleCount: totalCount,
    } satisfies TimeseriesPoint
  })
}

function buildTimeseriesResponse(options: {
  rangeStart: string
  rangeEnd: string
  bucketSeconds: number
  effectiveBucket: string
  availableBuckets: string[]
  points: TimeseriesPoint[]
}): TimeseriesResponse {
  return {
    rangeStart: options.rangeStart,
    rangeEnd: options.rangeEnd,
    bucketSeconds: options.bucketSeconds,
    effectiveBucket: options.effectiveBucket,
    availableBuckets: options.availableBuckets,
    bucketLimitedToDaily: false,
    points: options.points,
  }
}

function buildStatsRequestHandler(scenario: StatsScenario = 'default') {
  const now = Date.parse('2026-04-06T12:00:00.000Z')
  const todayStart = now - 24 * 60 * 60 * 1000
  const weekStart = now - 7 * 24 * 60 * 60 * 1000

  const summaryByWindow: Record<string, StatsResponse> = {
    today: buildSummary({ totalCount: 18324, successCount: 16510, failureCount: 1814, totalCost: 812.41, totalTokens: 1732145566 }),
    '1d': buildSummary({ totalCount: 18324, successCount: 16510, failureCount: 1814, totalCost: 812.41, totalTokens: 1732145566 }),
    '7d': buildSummary({ totalCount: 102116, successCount: 95514, failureCount: 6602, totalCost: 4411.18, totalTokens: 10021567342 }),
  }

  const errorDistribution: ErrorDistributionResponse = {
    rangeStart: new Date(todayStart).toISOString(),
    rangeEnd: new Date(now).toISOString(),
    items: [
      { reason: 'upstream_timeout', count: 482 },
      { reason: 'rate_limited', count: 316 },
      { reason: 'connection_reset', count: 211 },
      { reason: 'invalid_request', count: 97 },
    ],
  }

  const failureSummary: FailureSummaryResponse = {
    rangeStart: new Date(todayStart).toISOString(),
    rangeEnd: new Date(now).toISOString(),
    totalFailures: 1106,
    serviceFailureCount: 742,
    clientFailureCount: 214,
    clientAbortCount: 150,
    actionableFailureCount: 956,
    actionableFailureRate: 0.864,
  }

  return ({ url }: { url: URL }) => {
    if (url.pathname === '/api/stats/summary') {
      const window = url.searchParams.get('window') ?? 'today'
      return jsonResponse(summaryByWindow[window] ?? summaryByWindow.today)
    }

    if (url.pathname === '/api/stats/timeseries') {
      if (scenario === 'timeseries-error') {
        return new Response('timeseries temporarily unavailable', { status: 500 })
      }
      const range = url.searchParams.get('range') ?? 'today'
      const bucket = url.searchParams.get('bucket') ?? (range === '7d' ? '1h' : '15m')
      if (range === '7d') {
        return jsonResponse(
          buildTimeseriesResponse({
            rangeStart: new Date(weekStart).toISOString(),
            rangeEnd: new Date(now).toISOString(),
            bucketSeconds: bucket === '1d' ? 86400 : bucket === '12h' ? 43200 : bucket === '6h' ? 21600 : 3600,
            effectiveBucket: bucket,
            availableBuckets: ['1h', '6h', '12h', '1d'],
            points: buildTimeseriesPoints({
              count: bucket === '1d' ? 7 : bucket === '12h' ? 14 : bucket === '6h' ? 28 : 7 * 24,
              bucketSeconds: bucket === '1d' ? 86400 : bucket === '12h' ? 43200 : bucket === '6h' ? 21600 : 3600,
              startMs: weekStart,
              offset: 3,
            }),
          }),
        )
      }
      return jsonResponse(
        buildTimeseriesResponse({
          rangeStart: new Date(todayStart).toISOString(),
          rangeEnd: new Date(now).toISOString(),
          bucketSeconds: bucket === '5m' ? 300 : bucket === '30m' ? 1800 : bucket === '1h' ? 3600 : 900,
          effectiveBucket: bucket,
          availableBuckets: ['15m', '30m', '1h', '6h'],
          points: buildTimeseriesPoints({
            count: bucket === '1h' ? 24 : bucket === '30m' ? 48 : bucket === '5m' ? 288 : 96,
            bucketSeconds: bucket === '5m' ? 300 : bucket === '30m' ? 1800 : bucket === '1h' ? 3600 : 900,
            startMs: todayStart,
          }),
        }),
      )
    }

    if (url.pathname === '/api/stats/errors') {
      return jsonResponse(errorDistribution)
    }

    if (url.pathname === '/api/stats/failures/summary') {
      return jsonResponse(failureSummary)
    }

    return undefined
  }
}

const meta = {
  title: 'Pages/StatsPage',
  component: StatsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
    scenario: 'default',
  },
  decorators: [
    (Story, context) => {
      const scenario = ((context.parameters as StatsStoryParameters).scenario ?? 'default') as StatsScenario
      return (
        <I18nProvider>
          <StorybookPageEnvironment onRequest={buildStatsRequestHandler(scenario)}>
            <FullPageStorySurface>
              <Story />
            </FullPageStorySurface>
          </StorybookPageEnvironment>
        </I18nProvider>
      )
    },
  ],
} satisfies Meta<typeof StatsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('统计')).toBeVisible()
    await expect(canvas.getByTestId('stats-range-select-trigger')).toBeVisible()
    await expect(canvas.getByTestId('stats-bucket-select-trigger')).toBeVisible()

    await userEvent.click(canvas.getByTestId('stats-range-select-trigger'))
    await userEvent.click(within(document.body).getByText('最近 7 天'))
    await expect(canvas.getByTestId('stats-range-select-trigger')).toHaveTextContent('最近 7 天')
  },
}

export const TimeseriesError: Story = {
  parameters: {
    scenario: 'timeseries-error',
  },
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('统计')).toBeVisible()
    await expect(canvas.getAllByRole('alert').at(0)).toBeVisible()
  },
}
