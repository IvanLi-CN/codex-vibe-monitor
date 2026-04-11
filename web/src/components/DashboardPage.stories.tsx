import { useLayoutEffect, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { MemoryRouter } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  StatsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
} from '../lib/api'
import DashboardPage from '../pages/Dashboard'
import { DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY } from './DashboardActivityOverview'
import { FullPageStorySurface, StorybookPageEnvironment } from './storybookPageHelpers'
import { jsonResponse } from './storybookResponse'

type DashboardScenario = 'default' | 'degraded'

type DashboardStoryParameters = {
  scenario?: DashboardScenario
}

function DashboardRangeStorageReset({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const previousValue = window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
    window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)

    return () => {
      if (previousValue === null) {
        window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
      } else {
        window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, previousValue)
      }
    }
  }, [])

  return <>{children}</>
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
  valueOffset = 0,
}: {
  count: number
  bucketSeconds: number
  startMs: number
  valueOffset?: number
}) {
  return Array.from({ length: count }, (_, index) => {
    const bucketStartMs = startMs + index * bucketSeconds * 1000
    const bucketEndMs = bucketStartMs + bucketSeconds * 1000
    const pulse = (index + valueOffset) % 24
    const totalCount = pulse >= 7 && pulse <= 11 ? 24 + (index % 6) : pulse >= 18 && pulse <= 22 ? 16 + (index % 5) : index % 4
    const failureCount = totalCount === 0 ? 0 : index % 11 === 0 ? 2 : index % 7 === 0 ? 1 : 0
    const successCount = Math.max(totalCount - failureCount, 0)
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 3200,
      totalCost: Number((totalCount * 0.018).toFixed(4)),
    } satisfies TimeseriesPoint
  })
}

function buildTodayTimeseriesPoints({
  startMs,
  endMs,
  summary,
}: {
  startMs: number
  endMs: number
  summary: StatsResponse
}) {
  const count = Math.floor((endMs - startMs) / 60_000) + 1
  const minuteIndexes = Array.from({ length: count }, (_, index) => index)
  const successCounts = distributeInteger(
    summary.successCount,
    minuteIndexes.map((index) => buildActivityWeight(index, 'success')),
  )
  const failureCounts = distributeInteger(
    summary.failureCount,
    minuteIndexes.map((index) => buildActivityWeight(index, 'failure')),
  )
  const tokenTotals = distributeInteger(
    summary.totalTokens,
    minuteIndexes.map((index) => buildUsageWeight(successCounts[index] + failureCounts[index], index, 'tokens')),
  )
  const costCents = distributeInteger(
    Math.round(summary.totalCost * 100),
    minuteIndexes.map((index) => buildUsageWeight(successCounts[index] + failureCounts[index], index, 'cost')),
  )

  return minuteIndexes.map((index) => {
    const bucketStartMs = startMs + index * 60_000
    const bucketEndMs = bucketStartMs + 60_000
    const successCount = successCounts[index] ?? 0
    const failureCount = failureCounts[index] ?? 0
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount: successCount + failureCount,
      successCount,
      failureCount,
      totalTokens: tokenTotals[index] ?? 0,
      totalCost: Number(((costCents[index] ?? 0) / 100).toFixed(2)),
    } satisfies TimeseriesPoint
  })
}

function buildActivityWeight(index: number, mode: 'success' | 'failure') {
  const hour = Math.floor(index / 60)
  const minute = index % 60
  const rush = hour < 6 ? 2 : hour < 9 ? 5 : hour < 12 ? 9 : 4
  const pulse = (index % 11) + 1
  const boundaryBoost = minute % 15 === 0 ? 4 : minute % 5 === 0 ? 2 : 0
  const failureBias = mode === 'failure' ? (hour >= 9 && hour <= 11 ? 6 : 3) : 0
  return rush + pulse + boundaryBoost + failureBias
}

function buildUsageWeight(totalCount: number, index: number, mode: 'tokens' | 'cost') {
  const base = Math.max(totalCount, 1)
  if (mode === 'tokens') {
    return base * (14 + (index % 17)) + ((index % 7) + 1) * 19
  }
  return base * (6 + (index % 9)) + ((index % 5) + 1) * 7
}

function distributeInteger(total: number, weights: number[]) {
  if (weights.length === 0) return []
  const sanitizedWeights = weights.map((weight) => (Number.isFinite(weight) && weight > 0 ? weight : 1))
  const weightSum = sanitizedWeights.reduce((sum, weight) => sum + weight, 0)
  if (weightSum <= 0) {
    const evenShare = Math.floor(total / weights.length)
    const remainder = total - evenShare * weights.length
    return weights.map((_, index) => evenShare + (index < remainder ? 1 : 0))
  }

  const rawAllocations = sanitizedWeights.map((weight) => (total * weight) / weightSum)
  const allocations = rawAllocations.map((value) => Math.floor(value))
  let remainder = total - allocations.reduce((sum, value) => sum + value, 0)

  if (remainder > 0) {
    const remainders = rawAllocations
      .map((value, index) => ({ index, fraction: value - Math.floor(value), weight: sanitizedWeights[index] }))
      .sort((left, right) => {
        if (right.fraction !== left.fraction) return right.fraction - left.fraction
        if (right.weight !== left.weight) return right.weight - left.weight
        return left.index - right.index
      })

    for (let cursor = 0; cursor < remainders.length && remainder > 0; cursor += 1, remainder -= 1) {
      allocations[remainders[cursor].index] += 1
    }
  }

  return allocations
}

function buildTimeseriesResponse(options: {
  rangeStart: string
  rangeEnd: string
  bucketSeconds: number
  effectiveBucket?: string
  availableBuckets?: string[]
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

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number
    invokeId: string
    occurredAt: string
    status: string
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? 'none',
    routeMode: overrides.routeMode ?? 'pool',
    model: overrides.model ?? 'gpt-5.4',
    totalTokens: overrides.totalTokens ?? 280,
    cost: overrides.cost ?? 0.0178,
    proxyDisplayName: overrides.proxyDisplayName ?? 'tokyo-edge-01',
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? 'pool-alpha@example.com',
    endpoint: overrides.endpoint ?? '/v1/responses',
    source: overrides.source ?? 'pool',
    inputTokens: overrides.inputTokens ?? 164,
    outputTokens: overrides.outputTokens ?? 116,
    cacheInputTokens: overrides.cacheInputTokens ?? 42,
    reasoningTokens: overrides.reasoningTokens ?? 18,
    reasoningEffort: overrides.reasoningEffort ?? 'high',
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? 'gzip',
    requestedServiceTier: overrides.requestedServiceTier ?? 'priority',
    serviceTier: overrides.serviceTier ?? 'priority',
    tReqReadMs: overrides.tReqReadMs ?? 11,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 84,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 91,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 220,
    tRespParseMs: overrides.tRespParseMs ?? 10,
    tPersistMs: overrides.tPersistMs ?? 8,
    tTotalMs: overrides.tTotalMs ?? 431,
  }
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: recentInvocations.reduce((sum, invocation) => sum + invocation.totalTokens, 0),
    totalCost: Number(
      recentInvocations.reduce((sum, invocation) => sum + (invocation.cost ?? 0), 0).toFixed(4),
    ),
    createdAt: recentInvocations.at(-1)?.occurredAt ?? '2026-04-06T12:00:00.000Z',
    lastActivityAt: recentInvocations[0]?.occurredAt ?? '2026-04-06T12:00:00.000Z',
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: recentInvocations.map((invocation, index) => ({
      occurredAt: invocation.occurredAt,
      status: invocation.status,
      isSuccess: invocation.status === 'completed' || invocation.status === 'success',
      requestTokens: 180 + index * 24,
      cumulativeTokens: 180 + index * 24,
    })),
  }
}

function buildWorkingConversationsResponse(empty = false): PromptCacheConversationsResponse {
  return {
    rangeStart: '2026-04-06T11:55:00.000Z',
    rangeEnd: '2026-04-06T12:00:00.000Z',
    selectionMode: 'activityWindow',
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations: empty
      ? []
      : [
          createConversation('wc-current-1', [
            createPreview({
              id: 1,
              invokeId: 'wc-1-a',
              occurredAt: '2026-04-06T12:00:00.000Z',
              status: 'running',
              upstreamAccountName: 'pool-alpha@example.com',
              tTotalMs: null,
            }),
            createPreview({
              id: 2,
              invokeId: 'wc-1-b',
              occurredAt: '2026-04-06T11:57:20.000Z',
              status: 'success',
              model: 'gpt-5.4-mini',
            }),
          ]),
          createConversation('wc-current-2', [
            createPreview({
              id: 3,
              invokeId: 'wc-2-a',
              occurredAt: '2026-04-06T11:59:10.000Z',
              status: 'http_502',
              failureClass: 'service_failure',
              failureKind: 'upstream_timeout',
              errorMessage: 'upstream gateway closed before first byte',
              upstreamAccountName: 'pool-beta@example.com',
              requestedServiceTier: 'auto',
              serviceTier: 'auto',
            }),
            createPreview({
              id: 4,
              invokeId: 'wc-2-b',
              occurredAt: '2026-04-06T11:56:10.000Z',
              status: 'success',
              upstreamAccountName: 'pool-beta@example.com',
            }),
          ]),
        ],
  }
}

function createDashboardRequestHandler(scenario: DashboardScenario = 'default') {
  const now = Date.parse('2026-04-09T12:24:00+08:00')
  const rangeYesterdayStart = Date.parse('2026-04-08T00:00:00+08:00')
  const rangeYesterdayEnd = Date.parse('2026-04-09T00:00:00+08:00')
  const rangeTodayStart = Date.parse('2026-04-09T00:00:00+08:00')
  const range1dStart = now - 24 * 60 * 60 * 1000
  const range7dStart = now - 7 * 24 * 60 * 60 * 1000
  const range6moStart = now - 180 * 24 * 60 * 60 * 1000

  const todaySummary = buildSummary({
    totalCount: 12474,
    successCount: 9949,
    failureCount: 2525,
    totalCost: 539.42,
    totalTokens: 1314275579,
  })
  const yesterdaySummary = buildSummary({
    totalCount: 10864,
    successCount: 9532,
    failureCount: 1332,
    totalCost: 418.76,
    totalTokens: 1092456123,
  })

  const responses = {
    today: todaySummary,
    yesterday: yesterdaySummary,
    '1d': buildSummary({
      totalCount: 13564,
      successCount: 10948,
      failureCount: 2616,
      totalCost: 605.33,
      totalTokens: 1456067763,
    }),
    '7d': buildSummary({
      totalCount: 76421,
      successCount: 70115,
      failureCount: 6306,
      totalCost: 3128.74,
      totalTokens: 8764311220,
    }),
    timeseriesToday: buildTimeseriesResponse({
      rangeStart: new Date(rangeTodayStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: '1m',
      availableBuckets: ['1m'],
      points: buildTodayTimeseriesPoints({
        startMs: rangeTodayStart,
        endMs: now,
        summary: todaySummary,
      }),
    }),
    timeseriesYesterday: buildTimeseriesResponse({
      rangeStart: new Date(rangeYesterdayStart).toISOString(),
      rangeEnd: new Date(rangeYesterdayEnd).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: '1m',
      availableBuckets: ['1m'],
      points: buildTodayTimeseriesPoints({
        startMs: rangeYesterdayStart,
        endMs: rangeYesterdayEnd - 60 * 1000,
        summary: yesterdaySummary,
      }),
    }),
    timeseries1d: buildTimeseriesResponse({
      rangeStart: new Date(range1dStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: '1m',
      availableBuckets: ['1m'],
      points: buildTimeseriesPoints({ count: 24 * 60, bucketSeconds: 60, startMs: range1dStart }),
    }),
    timeseries7d: buildTimeseriesResponse({
      rangeStart: new Date(range7dStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 3600,
      effectiveBucket: '1h',
      availableBuckets: ['1h'],
      points: buildTimeseriesPoints({ count: 7 * 24, bucketSeconds: 3600, startMs: range7dStart, valueOffset: 7 }),
    }),
    timeseries6mo: buildTimeseriesResponse({
      rangeStart: new Date(range6moStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 86400,
      effectiveBucket: '1d',
      availableBuckets: ['1d'],
      points: buildTimeseriesPoints({ count: 180, bucketSeconds: 86400, startMs: range6moStart, valueOffset: 11 }),
    }),
  }

  return ({ url }: { url: URL }) => {
    if (url.pathname === '/api/stats/summary') {
      const window = url.searchParams.get('window') ?? 'today'
      if (scenario === 'degraded' && window === 'today') {
        return new Response('dashboard today summary unavailable', { status: 500 })
      }
      return jsonResponse(responses[window as keyof Pick<typeof responses, 'today' | 'yesterday' | '1d' | '7d'>] ?? responses.today)
    }

    if (url.pathname === '/api/stats/timeseries') {
      const range = url.searchParams.get('range')
      if (range === 'today') return jsonResponse(responses.timeseriesToday)
      if (range === 'yesterday') return jsonResponse(responses.timeseriesYesterday)
      if (range === '1d') return jsonResponse(responses.timeseries1d)
      if (range === '7d') return jsonResponse(responses.timeseries7d)
      if (range === '6mo') return jsonResponse(responses.timeseries6mo)
    }

    if (url.pathname === '/api/stats/prompt-cache-conversations') {
      return jsonResponse(buildWorkingConversationsResponse(scenario === 'degraded'))
    }

    return undefined
  }
}

const meta = {
  title: 'Pages/DashboardPage',
  component: DashboardPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
    scenario: 'default',
  },
  decorators: [
    (Story, context) => {
      const scenario = ((context.parameters as DashboardStoryParameters).scenario ?? 'default') as DashboardScenario
      return (
        <I18nProvider>
          <StorybookPageEnvironment onRequest={createDashboardRequestHandler(scenario)}>
            <MemoryRouter initialEntries={['/dashboard']}>
              <FullPageStorySurface>
                <DashboardRangeStorageReset>
                  <Story />
                </DashboardRangeStorageReset>
              </FullPageStorySurface>
            </MemoryRouter>
          </StorybookPageEnvironment>
        </I18nProvider>
      )
    },
  ],
} satisfies Meta<typeof DashboardPage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByTestId('dashboard-activity-overview')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-working-conversations')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-activity-range-today')).toHaveAttribute('data-active', 'true')
    await expect(canvas.getByTestId('dashboard-today-activity-chart')).toBeVisible()
    await expect(canvas.queryByTestId('usage-calendar-card')).toBeNull()

    const historyTab = canvas.getByRole('tab', { name: '历史' })
    await userEvent.click(historyTab)
    await expect(historyTab).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByTestId('usage-calendar-card')).toBeVisible()

    const yesterdayTab = canvas.getByRole('tab', { name: '昨日' })
    await userEvent.click(yesterdayTab)
    await expect(yesterdayTab).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByTestId('dashboard-activity-range-yesterday')).toHaveAttribute('data-active', 'true')

    const range7d = canvas.getByRole('tab', { name: '7 日' })
    await userEvent.click(range7d)
    await expect(range7d).toHaveAttribute('aria-selected', 'true')

    const todayTab = canvas.getByRole('tab', { name: '今日' })
    await userEvent.click(todayTab)
    await expect(todayTab).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByTestId('dashboard-activity-range-today')).toHaveAttribute('data-active', 'true')
  },
}

export const Degraded: Story = {
  parameters: {
    scenario: 'degraded',
  },
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByTestId('dashboard-activity-overview')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-working-conversations')).toBeVisible()
    await expect(canvas.getAllByRole('alert').at(0)).toBeVisible()
    await expect(canvas.queryAllByTestId('dashboard-working-conversation-card')).toHaveLength(0)
  },
}
