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
  failureKind?: string
  failureClass?: 'service_failure' | 'client_failure' | 'client_abort' | 'none'
  isActionable?: boolean
  endpoint?: string
  requesterIp?: string
  promptCacheKey?: string
  costEstimated?: number
  priceVersion?: string
  tTotalMs?: number | null
  tReqReadMs?: number | null
  tReqParseMs?: number | null
  tUpstreamConnectMs?: number | null
  tUpstreamTtfbMs?: number | null
  tUpstreamStreamMs?: number | null
  tRespParseMs?: number | null
  tPersistMs?: number | null
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

export type FailureScope = 'all' | 'service' | 'client' | 'abort'

export interface FailureSummaryResponse {
  rangeStart: string
  rangeEnd: string
  totalFailures: number
  serviceFailureCount: number
  clientFailureCount: number
  clientAbortCount: number
  actionableFailureCount: number
  actionableFailureRate: number
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

export interface ProxySettings {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  models: string[]
  enabledModels: string[]
  defaultHijackEnabled: boolean
}

export interface PricingEntry {
  model: string
  inputPer1m: number
  outputPer1m: number
  cacheInputPer1m?: number | null
  reasoningPer1m?: number | null
  source: string
}

export interface PricingSettings {
  catalogVersion: string
  entries: PricingEntry[]
}

export interface SettingsPayload {
  proxy: ProxySettings
  pricing: PricingSettings
}

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === 'string')
}

function normalizeFiniteNumber(value: unknown): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value)) return undefined
  return value
}

function normalizeProxySettings(raw: unknown): ProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>
  const models = normalizeStringArray(payload.models)
  const hasEnabledModelsField = Object.prototype.hasOwnProperty.call(payload, 'enabledModels')
  const enabledModelsRaw = normalizeStringArray(payload.enabledModels)
  const allowSet = new Set(models)
  const enabledModels = (hasEnabledModelsField ? enabledModelsRaw : models).filter((modelId) => allowSet.has(modelId))

  return {
    hijackEnabled: Boolean(payload.hijackEnabled),
    mergeUpstreamEnabled: Boolean(payload.mergeUpstreamEnabled),
    models,
    enabledModels,
    defaultHijackEnabled: Boolean(payload.defaultHijackEnabled),
  }
}

function normalizePricingEntry(raw: unknown): PricingEntry | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const model = typeof payload.model === 'string' ? payload.model.trim() : ''
  const inputPer1m = normalizeFiniteNumber(payload.inputPer1m)
  const outputPer1m = normalizeFiniteNumber(payload.outputPer1m)
  if (!model || inputPer1m === undefined || outputPer1m === undefined) return null
  const cacheInputPer1m = normalizeFiniteNumber(payload.cacheInputPer1m)
  const reasoningPer1m = normalizeFiniteNumber(payload.reasoningPer1m)
  return {
    model,
    inputPer1m,
    outputPer1m,
    cacheInputPer1m: cacheInputPer1m ?? null,
    reasoningPer1m: reasoningPer1m ?? null,
    source: typeof payload.source === 'string' && payload.source.trim() ? payload.source.trim() : 'custom',
  }
}

function normalizePricingSettings(raw: unknown): PricingSettings {
  const payload = (raw ?? {}) as Record<string, unknown>
  const entriesRaw = Array.isArray(payload.entries) ? payload.entries : []
  const entries = entriesRaw
    .map(normalizePricingEntry)
    .filter((entry): entry is PricingEntry => entry != null)
    .sort((a, b) => a.model.localeCompare(b.model))
  return {
    catalogVersion:
      typeof payload.catalogVersion === 'string' && payload.catalogVersion.trim()
        ? payload.catalogVersion.trim()
        : 'custom',
    entries,
  }
}

function normalizeSettingsPayload(raw: unknown): SettingsPayload {
  const payload = (raw ?? {}) as Record<string, unknown>
  return {
    proxy: normalizeProxySettings(payload.proxy),
    pricing: normalizePricingSettings(payload.pricing),
  }
}

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>('/api/version')
}

export async function fetchSettings(): Promise<SettingsPayload> {
  const response = await fetchJson<unknown>('/api/settings')
  return normalizeSettingsPayload(response)
}

export async function updateProxySettings(payload: {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  enabledModels: string[]
}): Promise<ProxySettings> {
  const response = await fetchJson<unknown>('/api/settings/proxy', {
    method: 'PUT',
    body: JSON.stringify(payload),
  })
  return normalizeProxySettings(response)
}

export async function updatePricingSettings(payload: PricingSettings): Promise<PricingSettings> {
  const response = await fetchJson<unknown>('/api/settings/pricing', {
    method: 'PUT',
    body: JSON.stringify(payload),
  })
  return normalizePricingSettings(response)
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

export async function fetchErrorDistribution(
  range: string,
  params?: { top?: number; scope?: FailureScope; timeZone?: string },
) {
  const search = new URLSearchParams()
  search.set('range', range)
  search.set('timeZone', params?.timeZone ?? getBrowserTimeZone())
  if (params?.top != null) search.set('top', String(params.top))
  if (params?.scope) search.set('scope', params.scope)
  return fetchJson<ErrorDistributionResponse>(`/api/stats/errors?${search.toString()}`)
}

export async function fetchFailureSummary(range: string, params?: { timeZone?: string }) {
  const search = new URLSearchParams()
  search.set('range', range)
  search.set('timeZone', params?.timeZone ?? getBrowserTimeZone())
  return fetchJson<FailureSummaryResponse>(`/api/stats/failures/summary?${search.toString()}`)
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
