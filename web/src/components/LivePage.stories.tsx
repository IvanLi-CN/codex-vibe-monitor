import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { MemoryRouter } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import type {
  ApiInvocation,
  ForwardProxyLiveStatsResponse,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  StatsResponse,
} from '../lib/api'
import LivePage from '../pages/Live'
import { FullPageStorySurface, StorybookPageEnvironment, jsonResponse } from './storybookPageHelpers'

type LiveScenario = 'default' | 'proxy-error'

type LiveStoryParameters = {
  scenario?: LiveScenario
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

function createForwardProxyLiveStats(): ForwardProxyLiveStatsResponse {
  return {
    rangeStart: '2026-04-05T12:00:00.000Z',
    rangeEnd: '2026-04-06T12:00:00.000Z',
    bucketSeconds: 3600,
    nodes: [
      {
        key: 'tokyo-edge-01',
        source: 'manual',
        displayName: 'tokyo-edge-01',
        endpointUrl: 'socks5://127.0.0.1:1080',
        weight: 1.12,
        penalized: false,
        stats: {
          oneMinute: { attempts: 26, successRate: 0.96, avgLatencyMs: 182 },
          fifteenMinutes: { attempts: 280, successRate: 0.95, avgLatencyMs: 204 },
          oneHour: { attempts: 1112, successRate: 0.94, avgLatencyMs: 219 },
          oneDay: { attempts: 21421, successRate: 0.93, avgLatencyMs: 241 },
          sevenDays: { attempts: 145221, successRate: 0.92, avgLatencyMs: 259 },
        },
        last24h: Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-04-05T12:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-04-05T13:00:00.000Z') + index * 3600_000).toISOString(),
          successCount: 12 + (index % 5),
          failureCount: index % 7 === 0 ? 2 : 0,
        })),
        weight24h: Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-04-05T12:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-04-05T13:00:00.000Z') + index * 3600_000).toISOString(),
          sampleCount: 1,
          minWeight: 1.04,
          maxWeight: 1.16,
          avgWeight: 1.1 + (index % 3) * 0.01,
          lastWeight: 1.1 + (index % 3) * 0.01,
        })),
      },
      {
        key: 'singapore-edge-02',
        source: 'subscription',
        displayName: 'singapore-edge-02',
        endpointUrl: 'vless://example@sg.example.com:443',
        weight: 0.74,
        penalized: true,
        stats: {
          oneMinute: { attempts: 9, successRate: 0.78, avgLatencyMs: 318 },
          fifteenMinutes: { attempts: 95, successRate: 0.81, avgLatencyMs: 332 },
          oneHour: { attempts: 404, successRate: 0.82, avgLatencyMs: 345 },
          oneDay: { attempts: 8211, successRate: 0.84, avgLatencyMs: 367 },
          sevenDays: { attempts: 54112, successRate: 0.85, avgLatencyMs: 378 },
        },
        last24h: Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-04-05T12:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-04-05T13:00:00.000Z') + index * 3600_000).toISOString(),
          successCount: 4 + (index % 4),
          failureCount: index % 4 === 0 ? 2 : 1,
        })),
        weight24h: Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-04-05T12:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-04-05T13:00:00.000Z') + index * 3600_000).toISOString(),
          sampleCount: 1,
          minWeight: 0.68,
          maxWeight: 0.82,
          avgWeight: 0.74 + (index % 2) * 0.01,
          lastWeight: 0.74 + (index % 2) * 0.01,
        })),
      },
    ],
  }
}

function createInvocation(overrides: Partial<ApiInvocation> & { id: number; invokeId: string; occurredAt: string; createdAt: string; status: string }): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    createdAt: overrides.createdAt,
    status: overrides.status,
    source: overrides.source ?? 'proxy',
    routeMode: overrides.routeMode ?? 'pool',
    proxyDisplayName: overrides.proxyDisplayName ?? 'tokyo-edge-01',
    endpoint: overrides.endpoint ?? '/v1/responses',
    model: overrides.model ?? 'gpt-5.4',
    inputTokens: overrides.inputTokens ?? 240,
    outputTokens: overrides.outputTokens ?? 112,
    cacheInputTokens: overrides.cacheInputTokens ?? 40,
    totalTokens: overrides.totalTokens ?? 352,
    cost: overrides.cost ?? 0.0214,
    requestedServiceTier: overrides.requestedServiceTier ?? 'priority',
    serviceTier: overrides.serviceTier ?? 'priority',
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 118,
    tTotalMs: overrides.tTotalMs ?? 924,
    failureClass: overrides.failureClass,
    errorMessage: overrides.errorMessage,
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? 'pool-alpha@example.com',
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
    totalTokens: overrides.totalTokens ?? 260,
    cost: overrides.cost ?? 0.0164,
    proxyDisplayName: overrides.proxyDisplayName ?? 'tokyo-edge-01',
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? 'pool-alpha@example.com',
    endpoint: overrides.endpoint ?? '/v1/responses',
    source: overrides.source ?? 'pool',
    inputTokens: overrides.inputTokens ?? 150,
    outputTokens: overrides.outputTokens ?? 110,
    cacheInputTokens: overrides.cacheInputTokens ?? 32,
    reasoningTokens: overrides.reasoningTokens ?? 20,
    reasoningEffort: overrides.reasoningEffort ?? 'medium',
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? 'gzip',
    requestedServiceTier: overrides.requestedServiceTier ?? 'priority',
    serviceTier: overrides.serviceTier ?? 'priority',
    tReqReadMs: overrides.tReqReadMs ?? 12,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 78,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 95,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 214,
    tRespParseMs: overrides.tRespParseMs ?? 9,
    tPersistMs: overrides.tPersistMs ?? 7,
    tTotalMs: overrides.tTotalMs ?? 422,
  }
}

function createConversation(promptCacheKey: string, recentInvocations: PromptCacheConversationInvocationPreview[]): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: recentInvocations.reduce((sum, invocation) => sum + invocation.totalTokens, 0),
    totalCost: Number(recentInvocations.reduce((sum, invocation) => sum + (invocation.cost ?? 0), 0).toFixed(4)),
    createdAt: recentInvocations.at(-1)?.occurredAt ?? '2026-04-06T12:00:00.000Z',
    lastActivityAt: recentInvocations[0]?.occurredAt ?? '2026-04-06T12:00:00.000Z',
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: recentInvocations.map((invocation, index) => ({
      occurredAt: invocation.occurredAt,
      status: invocation.status,
      isSuccess: invocation.status === 'success',
      requestTokens: 160 + index * 18,
      cumulativeTokens: 160 + index * 18,
    })),
  }
}

function createPromptCacheConversationsResponse(): PromptCacheConversationsResponse {
  return {
    rangeStart: '2026-04-06T11:00:00.000Z',
    rangeEnd: '2026-04-06T12:00:00.000Z',
    selectionMode: 'count',
    selectedLimit: 50,
    selectedActivityHours: null,
    selectedActivityMinutes: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations: [
      createConversation('pc-live-1', [
        createPreview({ id: 101, invokeId: 'pc-live-1-a', occurredAt: '2026-04-06T12:00:00.000Z', status: 'running', tTotalMs: null }),
        createPreview({ id: 102, invokeId: 'pc-live-1-b', occurredAt: '2026-04-06T11:56:00.000Z', status: 'success', model: 'gpt-5.4-mini' }),
      ]),
      createConversation('pc-live-2', [
        createPreview({ id: 103, invokeId: 'pc-live-2-a', occurredAt: '2026-04-06T11:58:00.000Z', status: 'success' }),
        createPreview({ id: 104, invokeId: 'pc-live-2-b', occurredAt: '2026-04-06T11:52:00.000Z', status: 'http_502', failureClass: 'service_failure', errorMessage: 'upstream timeout', failureKind: 'upstream_timeout' }),
      ]),
    ],
  }
}

function createLiveRequestHandler(scenario: LiveScenario = 'default') {
  const summaryByWindow: Record<string, StatsResponse> = {
    current: buildSummary({ totalCount: 50, successCount: 44, failureCount: 6, totalCost: 2.71, totalTokens: 58340 }),
    '30m': buildSummary({ totalCount: 318, successCount: 290, failureCount: 28, totalCost: 16.94, totalTokens: 382140 }),
    '1h': buildSummary({ totalCount: 604, successCount: 551, failureCount: 53, totalCost: 31.22, totalTokens: 713205 }),
    '1d': buildSummary({ totalCount: 8242, successCount: 7825, failureCount: 417, totalCost: 424.11, totalTokens: 10812114 }),
  }

  const invocations = {
    records: [
      createInvocation({ id: 1, invokeId: 'live-1', occurredAt: '2026-04-06T12:00:00.000Z', createdAt: '2026-04-06T12:00:00.000Z', status: 'running', tTotalMs: null }),
      createInvocation({ id: 2, invokeId: 'live-2', occurredAt: '2026-04-06T11:59:00.000Z', createdAt: '2026-04-06T11:59:00.000Z', status: 'success', proxyDisplayName: 'singapore-edge-02', requestedServiceTier: 'auto', serviceTier: 'auto' }),
      createInvocation({ id: 3, invokeId: 'live-3', occurredAt: '2026-04-06T11:58:00.000Z', createdAt: '2026-04-06T11:58:00.000Z', status: 'http_502', failureClass: 'service_failure', errorMessage: 'upstream timeout', endpoint: '/v1/chat/completions' }),
      createInvocation({ id: 4, invokeId: 'live-4', occurredAt: '2026-04-06T11:57:00.000Z', createdAt: '2026-04-06T11:57:00.000Z', status: 'success' }),
      createInvocation({ id: 5, invokeId: 'live-5', occurredAt: '2026-04-06T11:56:00.000Z', createdAt: '2026-04-06T11:56:00.000Z', status: 'success', model: 'gpt-5.4-mini' }),
      createInvocation({ id: 6, invokeId: 'live-6', occurredAt: '2026-04-06T11:55:00.000Z', createdAt: '2026-04-06T11:55:00.000Z', status: 'success', proxyDisplayName: 'singapore-edge-02' }),
    ],
  }

  return ({ url }: { url: URL }) => {
    if (url.pathname === '/api/stats/forward-proxy') {
      if (scenario === 'proxy-error') {
        return new Response('forward proxy live stats unavailable', { status: 500 })
      }
      return jsonResponse(createForwardProxyLiveStats())
    }

    if (url.pathname === '/api/stats/summary') {
      const window = url.searchParams.get('window') ?? 'current'
      return jsonResponse(summaryByWindow[window] ?? summaryByWindow.current)
    }

    if (url.pathname === '/api/invocations') {
      return jsonResponse(invocations)
    }

    if (url.pathname === '/api/stats/prompt-cache-conversations') {
      return jsonResponse(createPromptCacheConversationsResponse())
    }

    return undefined
  }
}

const meta = {
  title: 'Pages/LivePage',
  component: LivePage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
    scenario: 'default',
  },
  decorators: [
    (Story, context) => {
      const scenario = ((context.parameters as LiveStoryParameters).scenario ?? 'default') as LiveScenario
      return (
        <I18nProvider>
          <StorybookPageEnvironment onRequest={createLiveRequestHandler(scenario)}>
            <MemoryRouter initialEntries={['/live']}>
              <FullPageStorySurface>
                <Story />
              </FullPageStorySurface>
            </MemoryRouter>
          </StorybookPageEnvironment>
        </I18nProvider>
      )
    },
  ],
} satisfies Meta<typeof LivePage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <LivePage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('实时概览')).toBeVisible()
    await expect(canvas.getByText('代理运行态')).toBeVisible()
    await expect(canvas.getByText('对话')).toBeVisible()

    const oneHourTab = canvas.getByRole('tab', { name: '1 小时' })
    await userEvent.click(oneHourTab)
    await expect(oneHourTab).toHaveAttribute('aria-selected', 'true')
  },
}

export const ProxyError: Story = {
  parameters: {
    scenario: 'proxy-error',
  },
  render: () => <LivePage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('实时概览')).toBeVisible()
    await expect(canvas.getAllByRole('alert').at(0)).toBeVisible()
  },
}
