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
import { FullPageStorySurface, StorybookPageEnvironment } from './storybookPageHelpers'
import { jsonResponse } from './storybookResponse'

type DashboardScenario = 'default' | 'degraded'

type DashboardStoryParameters = {
  scenario?: DashboardScenario
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
          createConversation('wc-current-3', [
            createPreview({
              id: 5,
              invokeId: 'wc-3-a',
              occurredAt: '2026-04-06T11:58:40.000Z',
              status: 'completed',
              upstreamAccountName: 'pool-gamma@example.com',
              totalTokens: 364,
              cost: 0.0204,
              inputTokens: 218,
              outputTokens: 146,
              cacheInputTokens: 68,
            }),
          ]),
          createConversation('wc-current-4', [
            createPreview({
              id: 6,
              invokeId: 'wc-4-a',
              occurredAt: '2026-04-06T11:58:02.000Z',
              status: 'pending',
              upstreamAccountName: 'pool-delta@example.com',
              tTotalMs: null,
            }),
            createPreview({
              id: 7,
              invokeId: 'wc-4-b',
              occurredAt: '2026-04-06T11:54:32.000Z',
              status: 'success',
              upstreamAccountName: 'pool-delta@example.com',
            }),
          ]),
          createConversation('wc-current-5', [
            createPreview({
              id: 8,
              invokeId: 'wc-5-a',
              occurredAt: '2026-04-06T11:57:24.000Z',
              status: 'completed',
              upstreamAccountName: 'pool-epsilon@example.com',
              totalTokens: 412,
              cost: 0.0236,
              inputTokens: 244,
              outputTokens: 168,
              cacheInputTokens: 72,
            }),
            createPreview({
              id: 9,
              invokeId: 'wc-5-b',
              occurredAt: '2026-04-06T11:53:50.000Z',
              status: 'success',
              upstreamAccountName: 'pool-epsilon@example.com',
              model: 'gpt-5.4-mini',
            }),
          ]),
          createConversation('wc-current-6', [
            createPreview({
              id: 10,
              invokeId: 'wc-6-a',
              occurredAt: '2026-04-06T11:56:48.000Z',
              status: 'running',
              upstreamAccountName: 'pool-zeta@example.com',
              tTotalMs: null,
            }),
            createPreview({
              id: 11,
              invokeId: 'wc-6-b',
              occurredAt: '2026-04-06T11:52:18.000Z',
              status: 'success',
              upstreamAccountName: 'pool-zeta@example.com',
            }),
          ]),
          createConversation('wc-current-7', [
            createPreview({
              id: 12,
              invokeId: 'wc-7-a',
              occurredAt: '2026-04-06T11:56:06.000Z',
              status: 'http_429',
              failureClass: 'service_failure',
              failureKind: 'upstream_rate_limit',
              errorMessage: 'upstream rate limit reached for the current account',
              upstreamAccountName: 'pool-eta@example.com',
              requestedServiceTier: 'priority',
              serviceTier: 'priority',
              tUpstreamTtfbMs: null,
              tUpstreamStreamMs: null,
              tTotalMs: 1820,
            }),
            createPreview({
              id: 13,
              invokeId: 'wc-7-b',
              occurredAt: '2026-04-06T11:51:12.000Z',
              status: 'success',
              upstreamAccountName: 'pool-eta@example.com',
            }),
          ]),
          createConversation('wc-current-8', [
            createPreview({
              id: 14,
              invokeId: 'wc-8-a',
              occurredAt: '2026-04-06T11:55:20.000Z',
              status: 'completed',
              upstreamAccountName: 'pool-theta@example.com',
              totalTokens: 396,
              cost: 0.0228,
              inputTokens: 228,
              outputTokens: 168,
              cacheInputTokens: 66,
            }),
            createPreview({
              id: 15,
              invokeId: 'wc-8-b',
              occurredAt: '2026-04-06T11:50:20.000Z',
              status: 'success',
              upstreamAccountName: 'pool-theta@example.com',
              model: 'gpt-5.4-mini',
            }),
          ]),
        ],
  }
}

function createDashboardRequestHandler(scenario: DashboardScenario = 'default') {
  const now = Date.parse('2026-04-06T12:00:00.000Z')
  const range1dStart = now - 24 * 60 * 60 * 1000
  const range7dStart = now - 7 * 24 * 60 * 60 * 1000
  const range6moStart = now - 180 * 24 * 60 * 60 * 1000

  const responses = {
    today: buildSummary({
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 539.42,
      totalTokens: 1314275579,
    }),
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
      return jsonResponse(responses[window as keyof Pick<typeof responses, 'today' | '1d' | '7d'>] ?? responses.today)
    }

    if (url.pathname === '/api/stats/timeseries') {
      const range = url.searchParams.get('range')
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
                <Story />
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
    await expect(canvas.getByTestId('today-stats-overview-card')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-activity-overview')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-working-conversations')).toBeVisible()
    await expect(canvas.queryByTestId('usage-calendar-card')).toBeNull()

    const historyTab = canvas.getByRole('tab', { name: '历史' })
    await userEvent.click(historyTab)
    await expect(historyTab).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByTestId('usage-calendar-card')).toBeVisible()

    const range7d = canvas.getByRole('tab', { name: '7 日' })
    await userEvent.click(range7d)
    await expect(range7d).toHaveAttribute('aria-selected', 'true')
  },
}

export const Degraded: Story = {
  parameters: {
    scenario: 'degraded',
  },
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByTestId('today-stats-overview-card')).toBeVisible()
    await expect(canvas.getByTestId('dashboard-working-conversations')).toBeVisible()
    await expect(canvas.getAllByRole('alert').at(0)).toBeVisible()
    await expect(canvas.queryAllByTestId('dashboard-working-conversation-card')).toHaveLength(0)
  },
}
