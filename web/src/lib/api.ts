import { getBrowserTimeZone } from './timeZone'

const rawBase = import.meta.env.VITE_API_BASE_URL ?? ''
const API_BASE = rawBase.endsWith('/') ? rawBase.slice(0, -1) : rawBase

const withBase = (path: string) => `${API_BASE}${path}`

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(withBase(path), {
    headers: {
      'Content-Type': 'application/json',
    },
    ...init,
  })

  if (!response.ok) {
    const text = await response.text()
    throw new Error(`Request failed: ${response.status} ${text}`)
  }

  return response.json() as Promise<T>
}

export interface ApiInvocation {
  id: number
  invokeId: string
  occurredAt: string
  source?: string
  model?: string
  inputTokens?: number
  outputTokens?: number
  cacheInputTokens?: number
  reasoningTokens?: number
  totalTokens?: number
  cost?: number
  status?: string
  errorMessage?: string
  timings?: ApiInvocationTimings
  rawMetadata?: ApiInvocationRawMetadata
  proxyTimings?: ApiInvocationTimings
  proxyRawMetadata?: ApiInvocationRawMetadata
  createdAt: string
}

export interface ApiInvocationTimings {
  requestReadMs?: number | null
  requestParseMs?: number | null
  upstreamConnectMs?: number | null
  upstreamFirstByteMs?: number | null
  upstreamStreamMs?: number | null
  responseParseMs?: number | null
  persistenceMs?: number | null
  totalMs?: number | null
  [stage: string]: number | null | undefined
}

export interface ApiInvocationRawMetadata {
  request?: Record<string, unknown>
  response?: Record<string, unknown>
  [key: string]: unknown
}

export interface ListResponse {
  records: ApiInvocation[]
}

export interface StatsResponse {
  totalCount: number
  successCount: number
  failureCount: number
  totalCost: number
  totalTokens: number
}

export interface TimeseriesPoint {
  bucketStart: string
  bucketEnd: string
  totalCount: number
  successCount: number
  failureCount: number
  totalTokens: number
  totalCost: number
}

export interface TimeseriesResponse {
  rangeStart: string
  rangeEnd: string
  bucketSeconds: number
  points: TimeseriesPoint[]
}

export interface ErrorDistributionItem {
  reason: string
  count: number
}

export interface ErrorDistributionResponse {
  rangeStart: string
  rangeEnd: string
  items: ErrorDistributionItem[]
}

export interface PerfStageStats {
  stage: string
  count: number
  avgMs: number
  p50Ms: number
  p90Ms: number
  p99Ms: number
  maxMs: number
}

export interface PerfStatsResponse {
  rangeStart: string
  rangeEnd: string
  items?: PerfStageStats[]
  stages?: PerfStageStats[]
}

export interface PerfStatsQuery {
  range?: string
  bucket?: string
  settlementHour?: number
  timeZone?: string
  source?: string
  model?: string
  endpoint?: string
}

export interface QuotaSnapshot {
  capturedAt: string
  amountLimit?: number
  usedAmount?: number
  remainingAmount?: number
  period?: string
  periodResetTime?: string
  expireTime?: string
  isActive: boolean
  totalCost: number
  totalRequests: number
  totalTokens: number
  lastRequestTime?: string
  billingType?: string
  remainingCount?: number
  usedCount?: number
  subTypeName?: string
}

export type BroadcastPayload =
  | {
      type: 'records'
      records: ApiInvocation[]
    }
  | {
      type: 'summary'
      window: string
      summary: StatsResponse
    }
  | {
      type: 'quota'
      snapshot: QuotaSnapshot
    }
  | {
      type: 'version'
      version: string
    }

export async function fetchInvocations(limit: number, params?: { model?: string; status?: string }) {
  const search = new URLSearchParams()
  search.set('limit', String(limit))
  if (params?.model) search.set('model', params.model)
  if (params?.status) search.set('status', params.status)

  return fetchJson<ListResponse>(`/api/invocations?${search.toString()}`)
}

export async function fetchStats() {
  return fetchJson<StatsResponse>('/api/stats')
}

export interface VersionResponse {
  backend: string
  frontend: string
}

export interface ProxyModelSettings {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  defaultHijackEnabled: boolean
  models: string[]
  enabledModels: string[]
}

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === 'string')
}

function normalizeProxyModelSettings(raw: unknown): ProxyModelSettings {
  const payload = (raw ?? {}) as Record<string, unknown>
  const models = normalizeStringArray(payload.models)
  const hasEnabledModelsField = Object.prototype.hasOwnProperty.call(payload, 'enabledModels')
  const enabledModelsRaw = normalizeStringArray(payload.enabledModels)
  const allowSet = new Set(models)
  const enabledModels = (hasEnabledModelsField ? enabledModelsRaw : models).filter((modelId) => allowSet.has(modelId))

  return {
    hijackEnabled: Boolean(payload.hijackEnabled),
    mergeUpstreamEnabled: Boolean(payload.mergeUpstreamEnabled),
    defaultHijackEnabled: Boolean(payload.defaultHijackEnabled),
    models,
    enabledModels,
  }
}

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>('/api/version')
}

export async function fetchProxyModelSettings(): Promise<ProxyModelSettings> {
  const response = await fetchJson<unknown>('/api/settings/proxy-models')
  return normalizeProxyModelSettings(response)
}

export async function updateProxyModelSettings(payload: {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  enabledModels: string[]
}): Promise<ProxyModelSettings> {
  const response = await fetchJson<unknown>('/api/settings/proxy-models', {
    method: 'PUT',
    body: JSON.stringify(payload),
  })
  return normalizeProxyModelSettings(response)
}

export async function fetchSummary(window: string, options?: { limit?: number; timeZone?: string }) {
  const search = new URLSearchParams()
  search.set('window', window)
  search.set('timeZone', options?.timeZone ?? getBrowserTimeZone())
  if (options?.limit !== undefined) {
    search.set('limit', String(options.limit))
  }
  return fetchJson<StatsResponse>(`/api/stats/summary?${search.toString()}`)
}

export async function fetchTimeseries(range: string, params?: { bucket?: string; settlementHour?: number; timeZone?: string }) {
  const search = new URLSearchParams()
  search.set('range', range)
  search.set('timeZone', params?.timeZone ?? getBrowserTimeZone())
  if (params?.bucket) search.set('bucket', params.bucket)
  if (params?.settlementHour !== undefined) search.set('settlementHour', String(params.settlementHour))
  return fetchJson<TimeseriesResponse>(`/api/stats/timeseries?${search.toString()}`)
}

export async function fetchErrorDistribution(range: string, params?: { top?: number; timeZone?: string }) {
  const search = new URLSearchParams()
  search.set('range', range)
  search.set('timeZone', params?.timeZone ?? getBrowserTimeZone())
  if (params?.top != null) search.set('top', String(params.top))
  return fetchJson<ErrorDistributionResponse>(`/api/stats/errors?${search.toString()}`)
}

export async function fetchPerfStats(params?: PerfStatsQuery) {
  const search = new URLSearchParams()
  if (params?.range) search.set('range', params.range)
  if (params?.bucket) search.set('bucket', params.bucket)
  if (params?.settlementHour !== undefined) search.set('settlementHour', String(params.settlementHour))
  search.set('timeZone', params?.timeZone ?? getBrowserTimeZone())
  if (params?.source) search.set('source', params.source)
  if (params?.model) search.set('model', params.model)
  if (params?.endpoint) search.set('endpoint', params.endpoint)

  const query = search.toString()
  return fetchJson<PerfStatsResponse>(query ? `/api/stats/perf?${query}` : '/api/stats/perf')
}

export async function fetchQuotaSnapshot() {
  return fetchJson<QuotaSnapshot>('/api/quota/latest')
}

export function createEventSource(path: string) {
  return new EventSource(withBase(path))
}
