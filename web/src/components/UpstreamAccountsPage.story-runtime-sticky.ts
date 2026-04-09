import type {
  ApiInvocation,
  PromptCacheConversationInvocationPreview,
  UpstreamStickyConversationsResponse,
} from '../lib/api'

function buildStickyRequestPoints(
  points: Array<{
    occurredAt: string
    requestTokens: number
    status?: string
    isSuccess?: boolean
  }>,
) {
  let cumulativeTokens = 0
  return points.map((point) => {
    cumulativeTokens += point.requestTokens
    return {
      occurredAt: point.occurredAt,
      status: point.status ?? 'success',
      isSuccess: point.isSuccess ?? true,
      requestTokens: point.requestTokens,
      cumulativeTokens,
    }
  })
}

type StoryStickyConversationSeed = {
  stickyKey: string
  requestCount: number
  totalTokens: number
  totalCost: number
  createdAt: string
  lastActivityAt: string
  last24hRequests: Array<{
    occurredAt: string
    status: string
    isSuccess: boolean
    requestTokens: number
    cumulativeTokens: number
  }>
}

function buildStickyInvocationPreview(
  accountId: number,
  conversation: StoryStickyConversationSeed,
  point: StoryStickyConversationSeed['last24hRequests'][number],
  index: number,
): PromptCacheConversationInvocationPreview {
  const invokeId = `sticky-${accountId}-${conversation.stickyKey.slice(-6)}-${index + 1}`
  const isFailure = point.isSuccess === false
  return {
    id: accountId * 10_000 + index + 1 + conversation.stickyKey.length,
    invokeId,
    occurredAt: point.occurredAt,
    status: isFailure ? 'failed' : 'completed',
    failureClass: isFailure ? 'service_failure' : 'none',
    routeMode: 'sticky',
    model: 'gpt-5.4',
    totalTokens: point.requestTokens,
    cost: Number((point.requestTokens / 1_000_000).toFixed(4)),
    proxyDisplayName: accountId === 101 ? 'Tokyo Edge' : 'Fallback Edge',
    upstreamAccountId: accountId,
    upstreamAccountName: storyDisplayNameForAccount(accountId),
    endpoint: '/v1/responses',
    source: 'proxy',
    inputTokens: Math.max(1, Math.round(point.requestTokens * 0.68)),
    outputTokens: Math.max(1, Math.round(point.requestTokens * 0.27)),
    cacheInputTokens: Math.max(0, Math.round(point.requestTokens * 0.05)),
    reasoningTokens: Math.max(0, Math.round(point.requestTokens * 0.02)),
    reasoningEffort: isFailure ? 'xhigh' : 'high',
    errorMessage: isFailure ? '[pool_no_available_slot] Sticky fallback exhausted.' : undefined,
    failureKind: isFailure ? 'pool_no_available_slot' : undefined,
    isActionable: isFailure ? true : undefined,
    responseContentEncoding: 'br',
    requestedServiceTier: 'flex',
    serviceTier: 'scale',
    tReqReadMs: 12,
    tReqParseMs: 14,
    tUpstreamConnectMs: 18,
    tUpstreamTtfbMs: 220 + index * 11,
    tUpstreamStreamMs: 1_200 + index * 80,
    tRespParseMs: 16,
    tPersistMs: 8,
    tTotalMs: 1_580 + index * 90,
  }
}

function buildStickyInvocationRecord(
  preview: PromptCacheConversationInvocationPreview,
  stickyKey: string,
): ApiInvocation {
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    occurredAt: preview.occurredAt,
    source: preview.source,
    proxyDisplayName: preview.proxyDisplayName ?? undefined,
    model: preview.model ?? undefined,
    inputTokens: preview.inputTokens,
    outputTokens: preview.outputTokens,
    cacheInputTokens: preview.cacheInputTokens,
    reasoningTokens: preview.reasoningTokens,
    reasoningEffort: preview.reasoningEffort,
    totalTokens: preview.totalTokens,
    cost: preview.cost ?? undefined,
    status: preview.status,
    errorMessage: preview.errorMessage,
    failureKind: preview.failureKind,
    failureClass: preview.failureClass ?? undefined,
    isActionable: preview.isActionable,
    endpoint: preview.endpoint ?? undefined,
    promptCacheKey: stickyKey,
    routeMode: preview.routeMode ?? undefined,
    upstreamAccountId: preview.upstreamAccountId,
    upstreamAccountName: preview.upstreamAccountName ?? undefined,
    responseContentEncoding: preview.responseContentEncoding,
    requestedServiceTier: preview.requestedServiceTier,
    serviceTier: preview.serviceTier,
    tTotalMs: preview.tTotalMs ?? null,
    tReqReadMs: preview.tReqReadMs ?? null,
    tReqParseMs: preview.tReqParseMs ?? null,
    tUpstreamConnectMs: preview.tUpstreamConnectMs ?? null,
    tUpstreamTtfbMs: preview.tUpstreamTtfbMs ?? null,
    tUpstreamStreamMs: preview.tUpstreamStreamMs ?? null,
    tRespParseMs: preview.tRespParseMs ?? null,
    tPersistMs: preview.tPersistMs ?? null,
    createdAt: preview.occurredAt,
  }
}

function storyDisplayNameForAccount(accountId: number) {
  if (accountId === 101) return 'Codex Pro - Tokyo'
  if (accountId === 102) return 'Team key - missing weekly limit'
  return `Story account #${accountId}`
}

function buildStickyConversationSeeds(accountId: number): StoryStickyConversationSeed[] {
  return (
    accountId === 101
      ? [
          {
            stickyKey: '019ce3a1-6787-7910-b0fd-c246d6f6a901',
            requestCount: 10,
            totalTokens: 455_170,
            totalCost: 0.3507,
            createdAt: '2026-03-13T04:01:20.000Z',
            lastActivityAt: '2026-03-13T04:03:02.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T10:15:00.000Z',
                requestTokens: 102_440,
              },
              {
                occurredAt: '2026-03-12T18:20:00.000Z',
                requestTokens: 154_380,
              },
              {
                occurredAt: '2026-03-13T02:03:02.000Z',
                requestTokens: 198_350,
              },
            ]),
          },
          {
            stickyKey: '019ce3a0-cf52-7740-bec5-611a0c6af442',
            requestCount: 12,
            totalTokens: 629_175,
            totalCost: 0.4101,
            createdAt: '2026-03-13T03:59:52.000Z',
            lastActivityAt: '2026-03-13T04:06:08.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T12:10:00.000Z',
                requestTokens: 140_000,
              },
              {
                occurredAt: '2026-03-12T20:45:00.000Z',
                requestTokens: 212_875,
              },
              {
                occurredAt: '2026-03-13T03:06:08.000Z',
                requestTokens: 276_300,
              },
            ]),
          },
          {
            stickyKey: '019ce3a0-10a2-7c40-ba26-6f3358f44c77',
            requestCount: 5,
            totalTokens: 398_199,
            totalCost: 0.7543,
            createdAt: '2026-03-13T03:57:28.000Z',
            lastActivityAt: '2026-03-13T04:00:52.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T09:00:00.000Z',
                requestTokens: 120_000,
              },
              {
                occurredAt: '2026-03-12T21:40:00.000Z',
                requestTokens: 131_400,
              },
              {
                occurredAt: '2026-03-12T23:00:52.000Z',
                requestTokens: 146_799,
              },
            ]),
          },
          {
            stickyKey: '019ce39e-4ab3-7452-9cc3-3c51ad9088c1',
            requestCount: 23,
            totalTokens: 1_302_244,
            totalCost: 0.7238,
            createdAt: '2026-03-13T03:55:36.000Z',
            lastActivityAt: '2026-03-13T04:01:05.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T08:25:00.000Z',
                requestTokens: 330_000,
              },
              {
                occurredAt: '2026-03-12T17:15:00.000Z',
                requestTokens: 445_120,
              },
              {
                occurredAt: '2026-03-13T01:48:00.000Z',
                requestTokens: 268_624,
              },
              {
                occurredAt: '2026-03-13T04:01:05.000Z',
                requestTokens: 258_500,
              },
            ]),
          },
          {
            stickyKey: '019ce39a-6cfa-7b90-8e96-6de7e6076b02',
            requestCount: 20,
            totalTokens: 1_289_447,
            totalCost: 0.7022,
            createdAt: '2026-03-13T03:51:19.000Z',
            lastActivityAt: '2026-03-13T03:54:08.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T07:52:00.000Z',
                requestTokens: 281_000,
              },
              {
                occurredAt: '2026-03-12T13:04:00.000Z',
                requestTokens: 309_447,
              },
              {
                occurredAt: '2026-03-12T23:15:00.000Z',
                requestTokens: 334_000,
              },
              {
                occurredAt: '2026-03-12T22:54:08.000Z',
                requestTokens: 365_000,
                status: 'failed',
                isSuccess: false,
              },
            ]),
          },
          {
            stickyKey: '019ce397-7b0c-7240-9096-0b0e2a97d57a',
            requestCount: 35,
            totalTokens: 3_241_662,
            totalCost: 1.4563,
            createdAt: '2026-03-13T03:48:11.000Z',
            lastActivityAt: '2026-03-13T03:56:06.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T06:18:00.000Z',
                requestTokens: 640_000,
              },
              {
                occurredAt: '2026-03-12T11:42:00.000Z',
                requestTokens: 722_516,
              },
              {
                occurredAt: '2026-03-12T19:36:00.000Z',
                requestTokens: 841_900,
              },
              {
                occurredAt: '2026-03-13T03:56:06.000Z',
                requestTokens: 1_037_246,
              },
            ]),
          },
          {
            stickyKey: '019ce395-2299-7641-a0d6-c2ac4b6d9184',
            requestCount: 23,
            totalTokens: 1_455_961,
            totalCost: 1.0577,
            createdAt: '2026-03-13T03:45:33.000Z',
            lastActivityAt: '2026-03-13T03:53:28.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T05:10:00.000Z',
                requestTokens: 340_000,
              },
              {
                occurredAt: '2026-03-12T15:10:00.000Z',
                requestTokens: 462_400,
              },
              {
                occurredAt: '2026-03-12T22:00:00.000Z',
                requestTokens: 299_561,
              },
              {
                occurredAt: '2026-03-13T03:53:28.000Z',
                requestTokens: 354_000,
              },
            ]),
          },
        ]
      : [
          {
            stickyKey: '019ce3f1-7aa2-74b2-a762-145ec7cfe001',
            requestCount: 8,
            totalTokens: 122_440,
            totalCost: 0.1184,
            createdAt: '2026-03-13T02:44:00.000Z',
            lastActivityAt: '2026-03-13T03:14:00.000Z',
            last24hRequests: buildStickyRequestPoints([
              { occurredAt: '2026-03-12T18:00:00.000Z', requestTokens: 28_440 },
              { occurredAt: '2026-03-13T01:00:00.000Z', requestTokens: 44_000 },
              { occurredAt: '2026-03-12T22:14:00.000Z', requestTokens: 50_000 },
            ]),
          },
          {
            stickyKey: '019ce3f1-7aa2-74b2-a762-145ec7cfe002',
            requestCount: 11,
            totalTokens: 164_920,
            totalCost: 0.1542,
            createdAt: '2026-03-13T02:21:00.000Z',
            lastActivityAt: '2026-03-13T03:09:00.000Z',
            last24hRequests: buildStickyRequestPoints([
              { occurredAt: '2026-03-12T16:45:00.000Z', requestTokens: 38_120 },
              { occurredAt: '2026-03-13T00:32:00.000Z', requestTokens: 52_400 },
              { occurredAt: '2026-03-13T03:09:00.000Z', requestTokens: 74_400 },
            ]),
          },
        ]
  )
}

export function buildStickyConversations(
  accountId: number,
  parsedUrl: URL,
): UpstreamStickyConversationsResponse {
  const selectionLimit = Number(parsedUrl.searchParams.get('limit') || 50)
  const selectionActivityHours = Number(parsedUrl.searchParams.get('activityHours') || 0)
  const selectionMode = selectionActivityHours > 0 ? 'activityWindow' : 'count'
  const rangeEnd = '2026-03-13T04:10:00.000Z'
  const rangeEndEpoch = Date.parse(rangeEnd)
  const conversations = buildStickyConversationSeeds(accountId)
    .map((conversation) => {
      const recentInvocations = [...conversation.last24hRequests]
        .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt))
        .slice(0, 5)
        .map((point, index) => buildStickyInvocationPreview(accountId, conversation, point, index))
      return {
        ...conversation,
        recentInvocations,
      }
    })
    .sort((left, right) => Date.parse(right.lastActivityAt) - Date.parse(left.lastActivityAt))

  const filteredConversations =
    selectionMode === 'activityWindow'
      ? conversations.filter((conversation) => {
          const lastActivityEpoch = Date.parse(conversation.lastActivityAt)
          const boundary = rangeEndEpoch - selectionActivityHours * 3_600_000
          return Number.isFinite(lastActivityEpoch) && lastActivityEpoch >= boundary
        })
      : conversations.slice(0, Math.max(1, selectionLimit))

  return {
    rangeStart: '2026-03-12T04:00:00.000Z',
    rangeEnd: '2026-03-13T04:10:00.000Z',
    selectionMode,
    selectedLimit: selectionMode === 'count' ? Math.max(1, selectionLimit) : null,
    selectedActivityHours: selectionMode === 'activityWindow' ? selectionActivityHours : null,
    implicitFilter: {
      kind: null,
      filteredCount: 0,
    },
    conversations: filteredConversations,
  }
}

export function buildStickyInvocationRecords(accountId: number) {
  return buildStickyConversationSeeds(accountId)
    .flatMap((conversation) =>
      [...conversation.last24hRequests]
        .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt))
        .map((point, index) =>
          buildStickyInvocationRecord(
            buildStickyInvocationPreview(accountId, conversation, point, index),
            conversation.stickyKey,
          ),
        ),
    )
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt))
}
