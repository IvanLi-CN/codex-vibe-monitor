import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
  ApiPoolUpstreamRequestAttempt,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from '../lib/api'
import {
  mapPromptCacheConversationsToDashboardCards,
  type DashboardWorkingConversationInvocationSelection,
} from '../lib/dashboardWorkingConversations'
import { DashboardInvocationDetailDrawer } from './DashboardInvocationDetailDrawer'
import { DashboardWorkingConversationsSection } from './DashboardWorkingConversationsSection'
import { AccountDetailDrawerShell } from './AccountDetailDrawerShell'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <div className="mx-auto w-full max-w-[1560px]">{children}</div>
    </div>
  )
}

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      'Content-Type': 'application/json',
    },
  })
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
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
    proxyDisplayName: overrides.proxyDisplayName ?? 'tokyo-edge-01',
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? 'pool-alpha@example.com',
    endpoint: overrides.endpoint ?? '/v1/responses',
    source: overrides.source ?? 'pool',
    inputTokens: overrides.inputTokens ?? 148,
    outputTokens: overrides.outputTokens ?? 92,
    cacheInputTokens: overrides.cacheInputTokens ?? 36,
    reasoningTokens: overrides.reasoningTokens ?? 24,
    reasoningEffort: overrides.reasoningEffort ?? 'high',
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? 'gzip',
    requestedServiceTier: overrides.requestedServiceTier ?? 'priority',
    serviceTier: overrides.serviceTier ?? 'priority',
    tReqReadMs: overrides.tReqReadMs ?? 14,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 136,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 98,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 324,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 601,
  }
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: overrides.requestCount ?? recentInvocations.length,
    totalTokens: overrides.totalTokens ?? recentInvocations.reduce(
      (sum, preview) => sum + Math.max(0, preview.totalTokens),
      0,
    ),
    totalCost: overrides.totalCost ?? Number(
      recentInvocations
        .reduce((sum, preview) => sum + (preview.cost ?? 0), 0)
        .toFixed(4),
    ),
    createdAt: overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      '2026-04-04T10:00:00Z',
    lastActivityAt:
      overrides.lastActivityAt ??
      recentInvocations[0]?.occurredAt ??
      '2026-04-04T10:00:00Z',
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations,
    last24hRequests: overrides.last24hRequests ?? [],
  }
}

function createResponse(
  conversations: PromptCacheConversation[],
): PromptCacheConversationsResponse {
  return {
    rangeStart: '2026-04-04T10:00:00Z',
    rangeEnd: '2026-04-04T10:05:00Z',
    selectionMode: 'activityWindow',
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
  }
}

function buildRecordFromPreview(
  preview: PromptCacheConversationInvocationPreview,
): ApiInvocation {
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    occurredAt: preview.occurredAt,
    createdAt: preview.occurredAt,
    source: preview.source ?? 'pool',
    routeMode: preview.routeMode ?? 'pool',
    proxyDisplayName: preview.proxyDisplayName ?? undefined,
    upstreamAccountId: preview.upstreamAccountId ?? null,
    upstreamAccountName: preview.upstreamAccountName ?? undefined,
    endpoint: preview.endpoint ?? undefined,
    model: preview.model ?? undefined,
    status: preview.status,
    inputTokens: preview.inputTokens,
    outputTokens: preview.outputTokens,
    cacheInputTokens: preview.cacheInputTokens,
    reasoningTokens: preview.reasoningTokens,
    reasoningEffort: preview.reasoningEffort,
    totalTokens: preview.totalTokens,
    cost: preview.cost ?? undefined,
    errorMessage: preview.errorMessage,
    failureKind: preview.failureKind,
    failureClass: preview.failureClass ?? undefined,
    isActionable: preview.isActionable,
    responseContentEncoding: preview.responseContentEncoding ?? undefined,
    requestedServiceTier: preview.requestedServiceTier ?? undefined,
    serviceTier: preview.serviceTier ?? undefined,
    tReqReadMs: preview.tReqReadMs,
    tReqParseMs: preview.tReqParseMs,
    tUpstreamConnectMs: preview.tUpstreamConnectMs,
    tUpstreamTtfbMs: preview.tUpstreamTtfbMs,
    tUpstreamStreamMs: preview.tUpstreamStreamMs,
    tRespParseMs: preview.tRespParseMs,
    tPersistMs: preview.tPersistMs,
    tTotalMs: preview.tTotalMs,
  }
}

const currentAndPreviousResponse = createResponse([
  createConversation('pck-current-previous', [
    createPreview({
      id: 12,
      invokeId: 'invoke-12',
      occurredAt: '2026-04-04T10:04:20Z',
      status: 'completed',
      upstreamAccountName: 'growth-alpha@example.com',
    }),
    createPreview({
      id: 11,
      invokeId: 'invoke-11',
      occurredAt: '2026-04-04T10:01:12Z',
      status: 'completed',
      model: 'gpt-5.4-mini',
      upstreamAccountName: 'backup-alpha@example.com',
      requestedServiceTier: 'auto',
      serviceTier: 'auto',
    }),
  ]),
])

const currentOnlyResponse = createResponse([
  createConversation('pck-placeholder-only', [
    createPreview({
      id: 21,
      invokeId: 'invoke-21',
      occurredAt: '2026-04-04T10:04:42Z',
      status: 'completed',
      upstreamAccountName: 'warmup-alpha@example.com',
    }),
  ]),
])

const runningOnlyResponse = createResponse([
  createConversation('pck-running-only', [
    createPreview({
      id: 31,
      invokeId: 'invoke-31',
      occurredAt: '2026-04-04T10:04:58Z',
      status: 'running',
      upstreamAccountName: 'watch-alpha@example.com',
      tTotalMs: null,
    }),
    createPreview({
      id: 30,
      invokeId: 'invoke-30',
      occurredAt: '2026-04-04T09:54:20Z',
      status: 'completed',
      upstreamAccountName: 'watch-alpha@example.com',
      model: 'gpt-5.4-mini',
    }),
  ]),
])

const failedClickableResponse = createResponse([
  createConversation('pck-failed-clickable', [
    createPreview({
      id: 41,
      invokeId: 'invoke-41',
      occurredAt: '2026-04-04T10:03:40Z',
      status: 'http_502',
      failureClass: 'service_failure',
      errorMessage: 'upstream gateway closed before first byte',
      failureKind: 'upstream_timeout',
      upstreamAccountId: 77,
      upstreamAccountName: 'pool-account-77@example.com',
      endpoint: '/v1/chat/completions',
      requestedServiceTier: 'auto',
      serviceTier: 'auto',
      responseContentEncoding: 'identity',
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 30018,
    }),
    createPreview({
      id: 40,
      invokeId: 'invoke-40',
      occurredAt: '2026-04-04T10:02:10Z',
      status: 'completed',
      upstreamAccountId: 77,
      upstreamAccountName: 'pool-account-77@example.com',
      model: 'gpt-5.4-mini',
    }),
  ]),
])

const createdAtDescendingOrderResponse = createResponse([
  createConversation(
    "pck-created-middle",
    [
      createPreview({
        id: 52,
        invokeId: "invoke-created-middle-running",
        occurredAt: "2026-04-04T10:04:58Z",
        status: "running",
        upstreamAccountName: "ordering-middle@example.com",
        tTotalMs: null,
      }),
      createPreview({
        id: 51,
        invokeId: "invoke-created-middle-previous",
        occurredAt: "2026-04-04T10:03:40Z",
        status: "completed",
        upstreamAccountName: "ordering-middle@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:02:00Z",
    },
  ),
  createConversation(
    "pck-created-oldest",
    [
      createPreview({
        id: 61,
        invokeId: "invoke-created-oldest",
        occurredAt: "2026-04-04T10:03:20Z",
        status: "completed",
        upstreamAccountName: "ordering-oldest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T09:58:00Z",
    },
  ),
  createConversation(
    "pck-created-newest",
    [
      createPreview({
        id: 71,
        invokeId: "invoke-created-newest",
        occurredAt: "2026-04-04T10:01:00Z",
        status: "completed",
        upstreamAccountName: "ordering-newest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:03:00Z",
    },
  ),
]);

function buildCards(response: PromptCacheConversationsResponse) {
  return mapPromptCacheConversationsToDashboardCards(response)
}

function buildStoryMockData(response: PromptCacheConversationsResponse) {
  const recordsByInvokeId = new Map<string, ApiInvocation>()
  const detailByRecordId = new Map<number, ApiInvocationRecordDetailResponse>()
  const responseBodyByRecordId = new Map<number, ApiInvocationResponseBodyResponse>()
  const poolAttemptsByInvokeId = new Map<string, ApiPoolUpstreamRequestAttempt[]>()

  for (const conversation of response.conversations) {
    for (const preview of conversation.recentInvocations) {
      const record = buildRecordFromPreview(preview)
      recordsByInvokeId.set(record.invokeId, record)

      const normalizedStatus = (record.status ?? '').trim().toLowerCase()
      const isAbnormal =
        record.failureClass === 'service_failure' ||
        normalizedStatus === 'failed' ||
        normalizedStatus.startsWith('http_')

      if (isAbnormal) {
        detailByRecordId.set(record.id, {
          id: record.id,
          abnormalResponseBody: {
            available: true,
            previewText: JSON.stringify({
              error: {
                message: record.errorMessage ?? 'upstream failure',
              },
            }),
            hasMore: false,
          },
        })
        responseBodyByRecordId.set(record.id, {
          available: true,
          bodyText: JSON.stringify({
            error: {
              message: record.errorMessage ?? 'upstream failure',
            },
            invokeId: record.invokeId,
          }),
        })
      }

      if (
        (record.routeMode ?? '').trim().toLowerCase() === 'pool' &&
        typeof record.upstreamAccountId === 'number'
      ) {
        poolAttemptsByInvokeId.set(record.invokeId, [
          {
            id: record.id * 10 + 1,
            invokeId: record.invokeId,
            occurredAt: record.occurredAt,
            endpoint: record.endpoint ?? '/v1/responses',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            status: isAbnormal ? 'failed' : 'success',
            httpStatus:
              normalizedStatus.startsWith('http_')
                ? Number(normalizedStatus.slice('http_'.length))
                : 200,
            createdAt: record.createdAt,
            upstreamAccountId: record.upstreamAccountId ?? null,
            upstreamAccountName: record.upstreamAccountName ?? null,
            firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
          },
        ])
      }
    }
  }

  return {
    recordsByInvokeId,
    detailByRecordId,
    responseBodyByRecordId,
    poolAttemptsByInvokeId,
  }
}

function resolveInitialSelection(
  cards: ReturnType<typeof buildCards>,
  target?: {
    promptCacheKey: string
    slotKind: 'current' | 'previous'
  },
): DashboardWorkingConversationInvocationSelection | null {
  if (!target) return null
  const card = cards.find((candidate) => candidate.promptCacheKey === target.promptCacheKey)
  if (!card) return null
  const invocation =
    target.slotKind === 'previous' ? card.previousInvocation : card.currentInvocation
  if (!invocation) return null
  return {
    slotKind: target.slotKind,
    conversationSequenceId: card.conversationSequenceId,
    promptCacheKey: card.promptCacheKey,
    invocation,
  }
}

function StoryAccountDrawer({
  account,
  onClose,
}: {
  account: { id: number; label: string } | null
  onClose: () => void
}) {
  const titleId = useId()

  return (
    <AccountDetailDrawerShell
      open={account != null}
      labelledBy={titleId}
      closeLabel="Close account drawer"
      onClose={onClose}
      header={null}
    >
      {account ? (
        <div
          data-testid="story-account-drawer"
          className="space-y-4 rounded-[1.6rem] border border-base-300/80 bg-base-100/85 p-5"
        >
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/70">
              Shared Account Drawer
            </p>
            <h2 id={titleId} className="text-xl font-semibold text-base-content">
              {account.label}
            </h2>
            <p className="font-mono text-sm text-base-content/60">Account ID {account.id}</p>
          </div>
          <p className="text-sm leading-6 text-base-content/70">
            Mock shared account detail drawer used to verify that Dashboard account clicks
            switch away from the invocation drawer without opening both drawers at once.
          </p>
        </div>
      ) : null}
    </AccountDetailDrawerShell>
  )
}

function DrawerPreviewStory({
  response,
  initialSelection,
}: {
  response: PromptCacheConversationsResponse
  initialSelection?: {
    promptCacheKey: string
    slotKind: 'current' | 'previous'
  }
}) {
  const cards = useMemo(() => buildCards(response), [response])
  const storyMocks = useMemo(() => buildStoryMockData(response), [response])
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(() =>
      resolveInitialSelection(cards, initialSelection),
    )
  const [selectedAccount, setSelectedAccount] =
    useState<{ id: number; label: string } | null>(null)

  useEffect(() => {
    setSelectedInvocation(resolveInitialSelection(cards, initialSelection))
    setSelectedAccount(null)
  }, [cards, initialSelection])

  useEffect(() => {
    if (!originalFetchRef.current) {
      originalFetchRef.current = window.fetch.bind(window)
    }

    window.fetch = async (input, init) => {
      const request =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      const url = new URL(request, window.location.origin)

      if (url.pathname === '/api/invocations') {
        const requestId = url.searchParams.get('requestId')
        if (requestId) {
          const record = storyMocks.recordsByInvokeId.get(requestId)
          return jsonResponse({
            snapshotId: 1,
            total: record ? 1 : 0,
            page: 1,
            pageSize: 1,
            records: record ? [record] : [],
          })
        }
      }

      const detailMatch = url.pathname.match(/^\/api\/invocations\/(\d+)\/detail$/)
      if (detailMatch) {
        const recordId = Number(detailMatch[1])
        return jsonResponse(
          storyMocks.detailByRecordId.get(recordId) ?? {
            id: recordId,
            abnormalResponseBody: null,
          },
        )
      }

      const responseBodyMatch = url.pathname.match(/^\/api\/invocations\/(\d+)\/response-body$/)
      if (responseBodyMatch) {
        const recordId = Number(responseBodyMatch[1])
        return jsonResponse(
          storyMocks.responseBodyByRecordId.get(recordId) ?? {
            available: false,
            unavailableReason: 'No storybook response body for this record.',
          },
        )
      }

      const attemptsMatch = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/)
      if (attemptsMatch) {
        const invokeId = decodeURIComponent(attemptsMatch[1] ?? '')
        return jsonResponse(storyMocks.poolAttemptsByInvokeId.get(invokeId) ?? [])
      }

      if (originalFetchRef.current) {
        return originalFetchRef.current(input as Parameters<typeof fetch>[0], init)
      }

      throw new Error(`Unhandled Storybook request: ${url.pathname}`)
    }

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
    }
  }, [storyMocks])

  return (
    <>
      <DashboardWorkingConversationsSection
        cards={cards}
        isLoading={false}
        error={null}
        onOpenUpstreamAccount={(accountId, accountLabel) => {
          setSelectedInvocation(null)
          setSelectedAccount({ id: accountId, label: accountLabel })
        }}
        onOpenInvocation={(selection) => {
          setSelectedAccount(null)
          setSelectedInvocation(selection)
        }}
      />
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={(accountId, accountLabel) => {
          setSelectedInvocation(null)
          setSelectedAccount({ id: accountId, label: accountLabel })
        }}
      />
      <StoryAccountDrawer
        account={selectedAccount}
        onClose={() => setSelectedAccount(null)}
      />
      <div className="rounded-xl border border-base-300/75 bg-base-100/70 px-4 py-3 text-sm text-base-content/75">
        <span className="font-semibold">Drawer state:</span>{' '}
        <span data-testid="story-drawer-state" className="font-mono">
          {selectedInvocation
            ? `invocation:${selectedInvocation.invocation.record.invokeId}`
            : selectedAccount
              ? `account:${selectedAccount.id}`
              : 'none'}
        </span>
      </div>
    </>
  )
}

const meta = {
  title: 'Dashboard/WorkingConversationsSection',
  component: DashboardWorkingConversationsSection,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardWorkingConversationsSection>

export default meta

type Story = StoryObj<typeof meta>

export const CurrentAndPrevious: Story = {
  args: {
    cards: buildCards(currentAndPreviousResponse),
    isLoading: false,
    error: null,
  },
}

export const CurrentOnlyPlaceholder: Story = {
  args: {
    cards: buildCards(currentOnlyResponse),
    isLoading: false,
    error: null,
  },
}

export const RunningOnlyConversation: Story = {
  args: {
    cards: buildCards(runningOnlyResponse),
    isLoading: false,
    error: null,
  },
}

export const InvocationDrawerOpen: Story = {
  args: {
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={failedClickableResponse}
      initialSelection={{ promptCacheKey: 'pck-failed-clickable', slotKind: 'current' }}
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          'Dashboard card section with the new invocation detail drawer opened by default, backed by stable request-id lookups and mock response-body detail data.',
      },
    },
  },
}

export const FailedWithClickableAccount: Story = {
  args: {
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const accountButtons = await canvas.findAllByRole('button', {
      name: /pool-account-77@example.com/i,
    })
    const accountButton = accountButtons[0]

    await userEvent.click(accountButton)

    await waitFor(() => {
      expect(document.body.textContent ?? '').toContain(
        'Mock shared account detail drawer used to verify',
      )
    })
    await expect(canvas.getByTestId('story-drawer-state')).toHaveTextContent('account:77')
  },
}

export const DrawerInteractionFlow: Story = {
  args: {
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    )
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error('missing current slot')
    }

    await userEvent.click(currentSlot)

    await waitFor(() => {
      expect(document.body.querySelector('[data-testid="dashboard-invocation-detail-drawer"]')).not.toBeNull()
    })

    const drawerAccountButton = document.body.querySelector(
      '[data-testid="dashboard-invocation-detail-drawer"] button[title="pool-account-77@example.com"]',
    )
    if (!(drawerAccountButton instanceof HTMLButtonElement)) {
      throw new Error('missing drawer account button')
    }

    await userEvent.click(drawerAccountButton)

    await waitFor(() => {
      expect(document.body.querySelector('[data-testid="story-account-drawer"]')).not.toBeNull()
    })
  },
}

export const StateGallery: Story = {
  args: {
    cards: buildCards(
      createResponse([
        ...currentAndPreviousResponse.conversations,
        ...currentOnlyResponse.conversations,
        ...runningOnlyResponse.conversations,
        ...failedClickableResponse.conversations,
      ]),
    ),
    isLoading: false,
    error: null,
  },
}

export const CreatedAtDescendingOrder: Story = {
  args: {
    cards: buildCards(createdAtDescendingOrderResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const cards = await canvas.findAllByTestId('dashboard-working-conversation-card')
    await expect(cards[0]).toHaveTextContent('pck-created-newest')
    await expect(cards[1]).toHaveTextContent('pck-created-middle')
    await expect(cards[2]).toHaveTextContent('pck-created-oldest')
  },
}
