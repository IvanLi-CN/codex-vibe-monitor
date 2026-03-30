/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n'
import type { ApiInvocation, UpstreamStickyConversationsResponse } from '../lib/api'
import { StickyKeyConversationTable } from './StickyKeyConversationTable'

const apiMocks = vi.hoisted(() => ({
  fetchInvocationRecords: vi.fn<() => Promise<{
    snapshotId: number
    total: number
    page: number
    pageSize: number
    records: ApiInvocation[]
  }>>(),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
  }
})

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

beforeEach(() => {
  apiMocks.fetchInvocationRecords.mockReset()
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
})

function createStats(
  overrides: Partial<UpstreamStickyConversationsResponse> = {},
): UpstreamStickyConversationsResponse {
  return {
    rangeStart: '2026-03-02T00:00:00Z',
    rangeEnd: '2026-03-03T00:00:00Z',
    selectionMode: 'count',
    selectedLimit: 50,
    selectedActivityHours: null,
    implicitFilter: {
      kind: null,
      filteredCount: 0,
    },
    conversations: [
      {
        stickyKey: 'sticky-chat-001',
        requestCount: 8,
        totalTokens: 2_048,
        totalCost: 0.3456,
        createdAt: '2026-03-02T01:00:00Z',
        lastActivityAt: '2026-03-02T12:00:00Z',
        recentInvocations: [
          {
            id: 11,
            invokeId: 'sticky-preview-001',
            occurredAt: '2026-03-02T12:00:00Z',
            status: 'completed',
            failureClass: 'none',
            routeMode: 'sticky',
            model: 'gpt-5.4',
            totalTokens: 256,
            cost: 0.0345,
            proxyDisplayName: 'Tokyo Edge',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            endpoint: '/v1/responses',
            source: 'proxy',
            inputTokens: 200,
            outputTokens: 56,
            cacheInputTokens: 32,
            reasoningTokens: 10,
            reasoningEffort: 'high',
            responseContentEncoding: 'br',
            requestedServiceTier: 'flex',
            serviceTier: 'scale',
            tReqReadMs: 10,
            tReqParseMs: 11,
            tUpstreamConnectMs: 12,
            tUpstreamTtfbMs: 13,
            tUpstreamStreamMs: 14,
            tRespParseMs: 15,
            tPersistMs: 16,
            tTotalMs: 91,
          },
        ],
        last24hRequests: [
          {
            occurredAt: '2026-03-02T01:00:00Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 256,
            cumulativeTokens: 256,
          },
          {
            occurredAt: '2026-03-02T10:00:00Z',
            status: 'failed',
            isSuccess: false,
            requestTokens: 128,
            cumulativeTokens: 384,
          },
        ],
      },
    ],
    ...overrides,
  }
}

function renderStatic(stats: UpstreamStickyConversationsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <StickyKeyConversationTable
        accountId={101}
        accountDisplayName="Codex Pro - Tokyo"
        stats={stats}
        isLoading={false}
        error={null}
      />
    </I18nProvider>,
  )
}

function renderInteractive(stats: UpstreamStickyConversationsResponse) {
  if (!host) {
    host = document.createElement('div')
    document.body.appendChild(host)
    root = createRoot(host)
  }

  act(() => {
    root?.render(
      <I18nProvider>
        <StickyKeyConversationTable
          accountId={101}
          accountDisplayName="Codex Pro - Tokyo"
          stats={stats}
          isLoading={false}
          error={null}
        />
      </I18nProvider>,
    )
  })
}

function findButton(labels: string[]) {
  return Array.from(document.querySelectorAll('button')).find(
    (button): button is HTMLButtonElement =>
      labels.some((label) =>
        button.textContent?.includes(label) === true
        || button.getAttribute('aria-label')?.includes(label) === true,
      ),
  ) ?? null
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

describe('StickyKeyConversationTable', () => {
  it('renders sticky key metrics, actions, and the shared 24h sparkline', () => {
    const html = renderStatic(createStats())

    expect(html).toContain('sticky-chat-001')
    expect(html.includes('Upstream Accounts') || html.includes('上游账号')).toBe(true)
    expect(html).toContain('Codex Pro - Tokyo')
    expect(html).toContain('data-chart-kind="keyed-conversation-sparkline"')
  })

  it('renders implicit filter notes for capped activity windows', () => {
    const html = renderStatic(
      createStats({
        selectionMode: 'activityWindow',
        selectedLimit: null,
        selectedActivityHours: 3,
        implicitFilter: {
          kind: 'cappedTo50',
          filteredCount: 7,
        },
      }),
    )

    expect(
      html.includes('7 conversation(s) matched the activity window')
      || html.includes('有 7 个对话命中了活动时间筛选'),
    ).toBe(true)
  })

  it('renders activity-window inactivity notes with the selected hour range', () => {
    const html = renderStatic(
      createStats({
        selectionMode: 'activityWindow',
        selectedLimit: null,
        selectedActivityHours: 3,
        implicitFilter: {
          kind: 'inactiveOutside24h',
          filteredCount: 2,
        },
      }),
    )

    expect(
      html.includes('2 conversation(s) were hidden because they had no activity in the last 3 hour(s).')
      || html.includes('有 2 个对话因近 3 小时内没有活动而未显示。'),
    ).toBe(true)
  })

  it('loads sticky history with the shared live drawer and sticky filters', async () => {
    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 1,
        total: 3,
        page: 1,
        pageSize: 200,
        records: [
          {
            id: 99,
            invokeId: 'sticky-history-001',
            occurredAt: '2026-03-02T12:00:00Z',
            status: 'completed',
            model: 'gpt-5.4',
            totalTokens: 512,
            cost: 0.12,
            promptCacheKey: 'sticky-chat-001',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            createdAt: '2026-03-02T12:00:00Z',
          },
        ],
      })
      .mockResolvedValueOnce({
        snapshotId: 1,
        total: 3,
        page: 2,
        pageSize: 200,
        records: [
          {
            id: 100,
            invokeId: 'sticky-history-002',
            occurredAt: '2026-03-02T11:00:00Z',
            status: 'completed',
            model: 'gpt-5.4',
            totalTokens: 256,
            cost: 0.08,
            promptCacheKey: 'sticky-chat-001',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            createdAt: '2026-03-02T11:00:00Z',
          },
        ],
      })
      .mockResolvedValueOnce({
        snapshotId: 1,
        total: 3,
        page: 3,
        pageSize: 200,
        records: [
          {
            id: 101,
            invokeId: 'sticky-history-003',
            occurredAt: '2026-03-02T10:00:00Z',
            status: 'completed',
            model: 'gpt-5.4',
            totalTokens: 128,
            cost: 0.04,
            promptCacheKey: 'sticky-chat-001',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            createdAt: '2026-03-02T10:00:00Z',
          },
        ],
      })

    renderInteractive(createStats())

    const openHistoryButton = findButton(['Open full call history', '打开全部调用记录'])
    expect(openHistoryButton).not.toBeNull()

    await act(async () => {
      openHistoryButton?.click()
    })
    await flushAsync()

    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(1, {
      stickyKey: 'sticky-chat-001',
      upstreamAccountId: 101,
      page: 1,
      pageSize: 200,
      sortBy: 'occurredAt',
      sortOrder: 'desc',
    })

    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(2, {
      stickyKey: 'sticky-chat-001',
      upstreamAccountId: 101,
      page: 2,
      pageSize: 200,
      sortBy: 'occurredAt',
      sortOrder: 'desc',
      snapshotId: 1,
    })

    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(3, {
      stickyKey: 'sticky-chat-001',
      upstreamAccountId: 101,
      page: 3,
      pageSize: 200,
      sortBy: 'occurredAt',
      sortOrder: 'desc',
      snapshotId: 1,
    })
    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(3)

    expect(document.body.textContent).toContain('sticky-chat-001')
    expect(document.body.textContent).toContain('gpt-5.4')
  })
})
