import type {
  ApiInvocation,
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
    source: 'proxy',
    proxyDisplayName: 'tokyo-edge-01',
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
    tUpstreamTtfbMs: 142,
    tTotalMs: 1180,
  },
  {
    id: 6102,
    invokeId: 'inv_story_6102',
    occurredAt: '2026-03-10T07:56:44.000Z',
    createdAt: '2026-03-10T07:56:44.000Z',
    source: 'proxy',
    proxyDisplayName: 'osaka-edge-02',
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
    serviceTier: 'auto',
    tUpstreamTtfbMs: null,
    tTotalMs: 30015,
  },
  {
    id: 6103,
    invokeId: 'inv_story_6103',
    occurredAt: '2026-03-10T07:53:08.000Z',
    createdAt: '2026-03-10T07:53:08.000Z',
    source: 'proxy',
    proxyDisplayName: 'singapore-edge-03',
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
    tUpstreamTtfbMs: 0,
    tTotalMs: 35,
  },
  {
    id: 6104,
    invokeId: 'inv_story_6104',
    occurredAt: '2026-03-10T07:51:02.000Z',
    createdAt: '2026-03-10T07:51:02.000Z',
    source: 'proxy',
    proxyDisplayName: 'seoul-edge-04',
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
    tUpstreamTtfbMs: 96,
    tTotalMs: 420,
  },
  {
    id: 6105,
    invokeId: 'inv_story_6105',
    occurredAt: '2026-03-10T07:49:17.000Z',
    createdAt: '2026-03-10T07:49:17.000Z',
    source: 'proxy',
    proxyDisplayName: 'taipei-edge-05',
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
    tUpstreamTtfbMs: 184,
    tTotalMs: 930,
  },
  {
    id: 6106,
    invokeId: 'inv_story_6106',
    occurredAt: '2026-03-10T07:46:55.000Z',
    createdAt: '2026-03-10T07:46:55.000Z',
    source: 'proxy',
    proxyDisplayName: 'frankfurt-edge-06',
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
    tUpstreamTtfbMs: null,
    tTotalMs: 1450,
  },
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
