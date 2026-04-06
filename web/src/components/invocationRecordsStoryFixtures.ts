import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
  ApiPoolUpstreamRequestAttempt,
  InvocationExceptionSummary,
  InvocationNetworkSummary,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
  InvocationTokenSummary,
  StatsResponse,
} from '../lib/api'

export const STORYBOOK_INVOCATION_RECORDS: ApiInvocation[] = [
  {
    id: 6101,
    invokeId: 'inv_story_6101',
    occurredAt: '2026-03-10T07:58:12.000Z',
    createdAt: '2026-03-10T07:58:12.000Z',
    source: 'pool',
    proxyDisplayName: 'tokyo-edge-01',
    routeMode: 'pool',
    upstreamAccountId: 17,
    upstreamAccountName: 'Pool Alpha 17',
    poolAttemptCount: 2,
    poolDistinctAccountCount: 1,
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 18240,
    outputTokens: 1240,
    cacheInputTokens: 15280,
    reasoningTokens: 280,
    reasoningEffort: 'high',
    totalTokens: 19760,
    cost: 0.1824,
    requesterIp: '203.0.113.24',
    promptCacheKey: 'pck_story_alpha',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    responseContentEncoding: 'gzip, br',
    tReqReadMs: 24,
    tReqParseMs: 6,
    tUpstreamConnectMs: 708,
    tUpstreamTtfbMs: 142,
    tTotalMs: 1180,
  },
  {
    id: 6102,
    invokeId: 'inv_story_6102',
    occurredAt: '2026-03-10T07:56:44.000Z',
    createdAt: '2026-03-10T07:56:44.000Z',
    source: 'pool',
    proxyDisplayName: 'osaka-edge-02',
    routeMode: 'pool',
    upstreamAccountId: 23,
    upstreamAccountName: 'Pool Beta 23',
    poolAttemptCount: 5,
    poolDistinctAccountCount: 3,
    poolAttemptTerminalReason: 'budget_exhausted_final',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 6240,
    outputTokens: 0,
    cacheInputTokens: 0,
    totalTokens: 6240,
    cost: 0.041,
    errorMessage: 'upstream timeout while waiting first byte',
    failureKind: 'upstream_timeout',
    failureClass: 'service_failure',
    isActionable: true,
    requesterIp: '198.51.100.19',
    promptCacheKey: 'pck_story_beta',
    requestedServiceTier: 'priority',
    serviceTier: 'default',
    billingServiceTier: 'priority',
    responseContentEncoding: 'identity',
    tUpstreamTtfbMs: null,
    tTotalMs: 30015,
  },
  {
    id: 6103,
    invokeId: 'inv_story_6103',
    occurredAt: '2026-03-10T07:53:08.000Z',
    createdAt: '2026-03-10T07:53:08.000Z',
    source: 'pool',
    proxyDisplayName: 'singapore-edge-03',
    routeMode: 'pool',
    upstreamAccountId: 31,
    upstreamAccountName: 'Pool Gamma 31',
    poolAttemptCount: 1,
    poolDistinctAccountCount: 1,
    poolAttemptTerminalReason: 'client_failure',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'failed',
    inputTokens: 980,
    outputTokens: 0,
    cacheInputTokens: 0,
    totalTokens: 980,
    cost: 0.0043,
    errorMessage: 'request payload missing required field: input',
    failureKind: 'invalid_request',
    failureClass: 'client_failure',
    isActionable: false,
    requesterIp: '198.51.100.77',
    promptCacheKey: 'pck_story_gamma',
    requestedServiceTier: 'auto',
    serviceTier: 'auto',
    detailLevel: 'structured_only',
    detailPrunedAt: '2026-03-10T08:30:00.000Z',
    detailPruneReason: 'success_over_30d',
    responseContentEncoding: 'gzip',
    tReqReadMs: 12,
    tReqParseMs: 5,
    tUpstreamConnectMs: 18,
    tUpstreamTtfbMs: 0,
    tTotalMs: 35,
  },
  {
    id: 6104,
    invokeId: 'inv_story_6104',
    occurredAt: '2026-03-10T07:51:02.000Z',
    createdAt: '2026-03-10T07:51:02.000Z',
    source: 'pool',
    proxyDisplayName: 'seoul-edge-04',
    routeMode: 'pool',
    upstreamAccountId: 12,
    upstreamAccountName: 'Pool Delta 12',
    poolAttemptCount: 2,
    poolDistinctAccountCount: 1,
    poolAttemptTerminalReason: 'client_abort',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'failed',
    inputTokens: 2140,
    outputTokens: 40,
    cacheInputTokens: 620,
    totalTokens: 2180,
    cost: 0.0122,
    errorMessage: 'client connection closed before upstream stream completed',
    failureKind: 'client_disconnect',
    failureClass: 'client_abort',
    isActionable: false,
    requesterIp: '203.0.113.88',
    promptCacheKey: 'pck_story_delta',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    responseContentEncoding: 'br',
    tReqReadMs: 18,
    tReqParseMs: 6,
    tUpstreamConnectMs: 300,
    tUpstreamTtfbMs: 96,
    tTotalMs: 420,
  },
  {
    id: 6105,
    invokeId: 'inv_story_6105',
    occurredAt: '2026-03-10T07:49:17.000Z',
    createdAt: '2026-03-10T07:49:17.000Z',
    source: 'pool',
    proxyDisplayName: 'taipei-edge-05',
    routeMode: 'pool',
    upstreamAccountId: 44,
    upstreamAccountName: 'Pool Echo 44',
    poolAttemptCount: 1,
    poolDistinctAccountCount: 1,
    poolAttemptTerminalReason: 'success',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 11820,
    outputTokens: 830,
    cacheInputTokens: 6440,
    reasoningTokens: 120,
    reasoningEffort: 'medium',
    totalTokens: 12650,
    cost: 0.0917,
    requesterIp: '203.0.113.129',
    promptCacheKey: 'pck_story_epsilon',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    responseContentEncoding: 'gzip',
    tReqReadMs: 22,
    tReqParseMs: 9,
    tUpstreamConnectMs: 715,
    tUpstreamTtfbMs: 184,
    tTotalMs: 930,
  },
  {
    id: 6107,
    invokeId: 'inv_story_6107',
    occurredAt: '2026-03-10T07:44:20.000Z',
    createdAt: '2026-03-10T07:44:20.000Z',
    source: 'proxy',
    proxyDisplayName: 'san-jose-edge-07',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 133604,
    outputTokens: 349,
    cacheInputTokens: 132608,
    reasoningTokens: 62,
    reasoningEffort: 'high',
    totalTokens: 133953,
    cost: 0.0409,
    requesterIp: '192.168.31.6',
    promptCacheKey: 'pck_story_semantics',
    requestedServiceTier: 'auto',
    serviceTier: 'auto',
    responseContentEncoding: 'identity',
    tReqReadMs: 31,
    tReqParseMs: 1,
    tUpstreamConnectMs: 9330,
    tUpstreamTtfbMs: 0,
    tUpstreamStreamMs: 10080,
    tRespParseMs: 1,
    tPersistMs: 1,
    tTotalMs: 19460,
  },
  {
    id: 6106,
    invokeId: 'inv_story_6106',
    occurredAt: '2026-03-10T07:46:55.000Z',
    createdAt: '2026-03-10T07:46:55.000Z',
    source: 'pool',
    proxyDisplayName: 'frankfurt-edge-06',
    routeMode: 'pool',
    upstreamAccountId: 58,
    upstreamAccountName: 'Pool Zeta 58',
    poolAttemptCount: 3,
    poolDistinctAccountCount: 2,
    poolAttemptTerminalReason: 'in_progress',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5.4',
    status: 'running',
    inputTokens: 4200,
    outputTokens: 0,
    cacheInputTokens: 2100,
    totalTokens: 4200,
    cost: 0.019,
    requesterIp: '203.0.113.141',
    promptCacheKey: 'pck_story_zeta',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    responseContentEncoding: 'gzip',
    tUpstreamTtfbMs: null,
    tTotalMs: 1450,
  },
]

function resolveStoryPoolAttemptStatus(record: ApiInvocation): ApiPoolUpstreamRequestAttempt['status'] {
  if (record.status === 'success') return 'success'
  if (record.poolAttemptTerminalReason === 'budget_exhausted_final') return 'budget_exhausted_final'
  if (record.failureClass === 'client_failure') return 'http_failure'
  return 'transport_failure'
}

export function createStoryPoolAttemptsByInvokeId(records: ApiInvocation[]) {
  const attemptsByInvokeId: Record<string, ApiPoolUpstreamRequestAttempt[]> = {}

  records.forEach((record, recordIndex) => {
    if (record.routeMode !== 'pool') return

    const attemptTotal = Math.max(1, record.poolAttemptCount ?? 1)
    const distinctTotal = Math.max(1, Math.min(attemptTotal, record.poolDistinctAccountCount ?? attemptTotal))
    const finalAccountId =
      typeof record.upstreamAccountId === 'number' && Number.isFinite(record.upstreamAccountId)
        ? Math.trunc(record.upstreamAccountId)
        : 100 + recordIndex * 10 + distinctTotal
    const baseAccountId = finalAccountId - (distinctTotal - 1)
    const finalStatus = resolveStoryPoolAttemptStatus(record)

    attemptsByInvokeId[record.invokeId] = Array.from({ length: attemptTotal }, (_, attemptIndex) => {
      const isLastAttempt = attemptIndex === attemptTotal - 1
      const distinctAccountIndex = Math.min(distinctTotal, attemptIndex + 1)
      const sameAccountRetryIndex = attemptIndex < distinctTotal ? 1 : attemptIndex - distinctTotal + 2
      const upstreamAccountId = isLastAttempt ? finalAccountId : baseAccountId + distinctAccountIndex - 1
      const upstreamAccountName =
        isLastAttempt && record.upstreamAccountName?.trim()
          ? record.upstreamAccountName
          : `Pool Candidate ${upstreamAccountId}`
      const status = isLastAttempt ? finalStatus : attemptIndex % 2 === 0 ? 'transport_failure' : 'http_failure'
      const startedAtMs = Date.parse(record.occurredAt) - (attemptTotal - attemptIndex) * 900
      const finishedAtMs = startedAtMs + 240 + attemptIndex * 90

      return {
        id: record.id * 100 + attemptIndex + 1,
        invokeId: record.invokeId,
        occurredAt: record.occurredAt,
        endpoint: record.endpoint ?? '/v1/responses',
        attemptIndex: attemptIndex + 1,
        distinctAccountIndex,
        sameAccountRetryIndex,
        status,
        httpStatus:
          status === 'success' ? 200 : status === 'http_failure' ? (isLastAttempt ? 400 : 429) : null,
        failureKind:
          status === 'success'
            ? null
            : status === 'http_failure'
              ? isLastAttempt
                ? record.failureKind ?? 'invalid_request'
                : 'rate_limit'
              : 'connect_timeout',
        errorMessage:
          status === 'success'
            ? null
            : status === 'http_failure'
              ? isLastAttempt
                ? record.errorMessage ?? 'upstream rejected request'
                : 'upstream returned 429'
              : 'forward proxy connect timeout',
        connectLatencyMs: status === 'transport_failure' ? 160 + attemptIndex * 20 : 55 + attemptIndex * 12,
        firstByteLatencyMs:
          status === 'success' ? (record.tUpstreamTtfbMs ?? 180) : status === 'http_failure' ? 210 + attemptIndex * 18 : null,
        streamLatencyMs:
          status === 'success' && typeof record.tTotalMs === 'number' && Number.isFinite(record.tTotalMs)
            ? Math.max(80, record.tTotalMs - (record.tUpstreamTtfbMs ?? 180))
            : null,
        upstreamRequestId: `${record.invokeId}-attempt-${attemptIndex + 1}`,
        startedAt: new Date(startedAtMs).toISOString(),
        finishedAt: new Date(finishedAtMs).toISOString(),
        createdAt: new Date(finishedAtMs).toISOString(),
        upstreamAccountId,
        upstreamAccountName,
      }
    })
  })

  return attemptsByInvokeId
}

export function createStoryInvocationRecordDetailsById(records: ApiInvocation[]) {
  const detailsById: Record<number, ApiInvocationRecordDetailResponse> = {}

  for (const record of records) {
    if (record.status !== 'failed' && !record.failureClass) continue

    const basePreview =
      record.failureClass === 'client_failure'
        ? '{"error":{"message":"invalid input payload","type":"invalid_request_error"}}'
        : record.failureClass === 'client_abort'
          ? '{"error":{"message":"downstream client disconnected while streaming","type":"client_abort"}}'
          : '{"error":{"message":"upstream request timed out after 30s","type":"server_error"},"request_id":"req_story_timeout_6102"}'

    detailsById[record.id] = {
      id: record.id,
      abnormalResponseBody: {
        available: true,
        previewText: basePreview,
        hasMore: record.failureClass !== 'client_failure',
      },
    }

    if (record.detailLevel === 'structured_only') {
      detailsById[record.id] = {
        id: record.id,
        abnormalResponseBody: {
          available: false,
          previewText: null,
          hasMore: false,
          unavailableReason: 'detail_pruned',
        },
      }
    }
  }

  return detailsById
}

export function createStoryInvocationResponseBodiesById(records: ApiInvocation[]) {
  const responseBodiesById: Record<number, ApiInvocationResponseBodyResponse> = {}

  for (const record of records) {
    if (record.status !== 'failed' && !record.failureClass) continue

    if (record.detailLevel === 'structured_only') {
      responseBodiesById[record.id] = {
        available: false,
        unavailableReason: 'detail_pruned',
      }
      continue
    }

    responseBodiesById[record.id] = {
      available: true,
      bodyText:
        record.failureClass === 'client_failure'
          ? '{"error":{"message":"invalid input payload","type":"invalid_request_error","param":"input"}}'
          : record.failureClass === 'client_abort'
            ? '{"error":{"message":"downstream client disconnected while streaming","type":"client_abort","request_id":"req_story_abort_6104"}}'
            : '{\n  "error": {\n    "message": "upstream request timed out after 30s",\n    "type": "server_error",\n    "code": "upstream_timeout"\n  },\n  "request_id": "req_story_timeout_6102",\n  "trace": "edge-timeout-6102-full"\n}',
    }
  }

  return responseBodiesById
}

export const STORYBOOK_FIRST_RESPONSE_BYTE_SEMANTICS_RECORDS: ApiInvocation[] = [
  STORYBOOK_INVOCATION_RECORDS[5]!,
  STORYBOOK_INVOCATION_RECORDS[4]!,
  STORYBOOK_INVOCATION_RECORDS[1]!,
]

function sum(values: number[]) {
  return values.reduce((total, value) => total + value, 0)
}

function percentile(values: number[], ratio: number) {
  if (values.length === 0) return null
  const sorted = [...values].sort((left, right) => left - right)
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * ratio) - 1))
  return sorted[index]
}

function resolveNumericValue(value?: number | null) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export function summarizeInvocationRecords(records: ApiInvocation[]) {
  const totalTokens = sum(records.map((record) => record.totalTokens ?? 0))
  const totalCost = sum(records.map((record) => record.cost ?? 0))
  const successCount = records.filter((record) => record.status === 'success').length
  const failureCount = records.filter((record) => record.status === 'failed').length
  const ttfbValues = records
    .map((record) => resolveNumericValue(record.tUpstreamTtfbMs))
    .filter((value): value is number => value != null)
  const totalMsValues = records
    .map((record) => resolveNumericValue(record.tTotalMs))
    .filter((value): value is number => value != null)

  const stats: StatsResponse = {
    totalCount: records.length,
    successCount,
    failureCount,
    totalCost: Number(totalCost.toFixed(4)),
    totalTokens,
  }

  const token: InvocationTokenSummary = {
    requestCount: records.length,
    totalTokens,
    avgTokensPerRequest: records.length > 0 ? Number((totalTokens / records.length).toFixed(2)) : 0,
    cacheInputTokens: sum(records.map((record) => record.cacheInputTokens ?? 0)),
    totalCost: Number(totalCost.toFixed(4)),
  }

  const network: InvocationNetworkSummary = {
    avgTtfbMs: ttfbValues.length > 0 ? Number((sum(ttfbValues) / ttfbValues.length).toFixed(2)) : null,
    p95TtfbMs: percentile(ttfbValues, 0.95),
    avgTotalMs: totalMsValues.length > 0 ? Number((sum(totalMsValues) / totalMsValues.length).toFixed(2)) : null,
    p95TotalMs: percentile(totalMsValues, 0.95),
  }

  const exception: InvocationExceptionSummary = {
    failureCount,
    serviceFailureCount: records.filter((record) => record.failureClass === 'service_failure').length,
    clientFailureCount: records.filter((record) => record.failureClass === 'client_failure').length,
    clientAbortCount: records.filter((record) => record.failureClass === 'client_abort').length,
    actionableFailureCount: records.filter((record) => record.isActionable).length,
  }

  return { stats, token, network, exception }
}

export function createStoryInvocationRecordsResponse(overrides?: Partial<InvocationRecordsResponse>): InvocationRecordsResponse {
  return {
    snapshotId: 8844,
    total: STORYBOOK_INVOCATION_RECORDS.length,
    page: 1,
    pageSize: 20,
    records: STORYBOOK_INVOCATION_RECORDS.map((record) => ({ ...record })),
    ...overrides,
  }
}

export function createStoryInvocationRecordsSummary(
  overrides?: Partial<InvocationRecordsSummaryResponse>,
): InvocationRecordsSummaryResponse {
  const base = summarizeInvocationRecords(STORYBOOK_INVOCATION_RECORDS)
  return {
    snapshotId: 8844,
    newRecordsCount: 0,
    ...base.stats,
    token: base.token,
    network: base.network,
    exception: base.exception,
    ...overrides,
  }
}
