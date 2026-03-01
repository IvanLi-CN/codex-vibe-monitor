import { getBrowserTimeZone } from './timeZone'

const rawBase = import.meta.env.VITE_API_BASE_URL ?? ''
const API_BASE = rawBase.endsWith('/') ? rawBase.slice(0, -1) : rawBase
const FORWARD_PROXY_VALIDATION_TIMEOUT_MS = 5_000
const FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS = 60_000

const withBase = (path: string) => `${API_BASE}${path}`

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(withBase(path), {
    headers: {
      'Content-Type': 'application/json',
    },
    ...init,
  })

  if (!response.ok) {
    const rawText = await response.text()
    const compactText = rawText.replace(/\s+/g, ' ').trim()
    const detail = (compactText || response.statusText || '').slice(0, 220)
    throw new Error(detail ? `Request failed: ${response.status} ${detail}` : `Request failed: ${response.status}`)
  }

  return response.json() as Promise<T>
}

export interface ApiInvocation {
  id: number
  invokeId: string
  occurredAt: string
  source?: string
  proxyDisplayName?: string
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
  firstByteSampleCount?: number
  firstByteAvgMs?: number | null
  firstByteP95Ms?: number | null
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

export interface ForwardProxyWindowStats {
  attempts: number
  successRate?: number
  avgLatencyMs?: number
}

export interface ForwardProxyNodeStats {
  oneMinute: ForwardProxyWindowStats
  fifteenMinutes: ForwardProxyWindowStats
  oneHour: ForwardProxyWindowStats
  oneDay: ForwardProxyWindowStats
  sevenDays: ForwardProxyWindowStats
}

export interface ForwardProxyNode {
  key: string
  source: string
  displayName: string
  endpointUrl?: string
  weight: number
  penalized: boolean
  stats: ForwardProxyNodeStats
}

export interface ForwardProxySettings {
  proxyUrls: string[]
  subscriptionUrls: string[]
  subscriptionUpdateIntervalSecs: number
  insertDirect: boolean
  nodes: ForwardProxyNode[]
}

export interface ForwardProxyHourlyBucket {
  bucketStart: string
  bucketEnd: string
  successCount: number
  failureCount: number
}

export interface ForwardProxyLiveNode {
  key: string
  source: string
  displayName: string
  endpointUrl?: string
  weight: number
  penalized: boolean
  stats: ForwardProxyNodeStats
  last24h: ForwardProxyHourlyBucket[]
}

export interface ForwardProxyLiveStatsResponse {
  rangeStart: string
  rangeEnd: string
  bucketSeconds: number
  nodes: ForwardProxyLiveNode[]
}

export type ForwardProxyValidationKind = 'proxyUrl' | 'subscriptionUrl'

export interface ForwardProxyValidationResult {
  ok: boolean
  message: string
  normalizedValue?: string
  discoveredNodes?: number
  latencyMs?: number
}

function forwardProxyValidationTimeoutMs(kind: ForwardProxyValidationKind): number {
  return kind === 'subscriptionUrl'
    ? FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS
    : FORWARD_PROXY_VALIDATION_TIMEOUT_MS
}

export interface SettingsPayload {
  proxy: ProxySettings
  forwardProxy: ForwardProxySettings
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

function normalizeForwardProxyWindowStats(raw: unknown): ForwardProxyWindowStats {
  const payload = (raw ?? {}) as Record<string, unknown>
  const attempts = normalizeFiniteNumber(payload.attempts) ?? 0
  const successRate = normalizeFiniteNumber(payload.successRate)
  const avgLatencyMs = normalizeFiniteNumber(payload.avgLatencyMs)
  return {
    attempts,
    successRate,
    avgLatencyMs,
  }
}

function emptyForwardProxyNodeStats(): ForwardProxyNodeStats {
  return {
    oneMinute: { attempts: 0 },
    fifteenMinutes: { attempts: 0 },
    oneHour: { attempts: 0 },
    oneDay: { attempts: 0 },
    sevenDays: { attempts: 0 },
  }
}

function normalizeForwardProxyNode(raw: unknown): ForwardProxyNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const key = typeof payload.key === 'string' ? payload.key : ''
  if (!key) return null
  const statsPayload = (payload.stats ?? {}) as Record<string, unknown>
  return {
    key,
    source: typeof payload.source === 'string' ? payload.source : 'manual',
    displayName: typeof payload.displayName === 'string' ? payload.displayName : key,
    endpointUrl: typeof payload.endpointUrl === 'string' ? payload.endpointUrl : undefined,
    weight: normalizeFiniteNumber(payload.weight) ?? 0,
    penalized: Boolean(payload.penalized),
    stats: {
      oneMinute: normalizeForwardProxyWindowStats(statsPayload.oneMinute),
      fifteenMinutes: normalizeForwardProxyWindowStats(statsPayload.fifteenMinutes),
      oneHour: normalizeForwardProxyWindowStats(statsPayload.oneHour),
      oneDay: normalizeForwardProxyWindowStats(statsPayload.oneDay),
      sevenDays: normalizeForwardProxyWindowStats(statsPayload.sevenDays),
    },
  }
}

function normalizeForwardProxySettings(raw: unknown): ForwardProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : []
  const nodes = nodesRaw
    .map(normalizeForwardProxyNode)
    .filter((node): node is ForwardProxyNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName))
  return {
    proxyUrls: normalizeStringArray(payload.proxyUrls),
    subscriptionUrls: normalizeStringArray(payload.subscriptionUrls),
    subscriptionUpdateIntervalSecs: normalizeFiniteNumber(payload.subscriptionUpdateIntervalSecs) ?? 3600,
    insertDirect: payload.insertDirect !== false,
    nodes: nodes.map((node) => ({
      ...node,
      stats: node.stats ?? emptyForwardProxyNodeStats(),
    })),
  }
}

function normalizeForwardProxyHourlyBucket(raw: unknown): ForwardProxyHourlyBucket | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const bucketStart = typeof payload.bucketStart === 'string' ? payload.bucketStart : ''
  const bucketEnd = typeof payload.bucketEnd === 'string' ? payload.bucketEnd : ''
  if (!bucketStart || !bucketEnd) return null
  return {
    bucketStart,
    bucketEnd,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
  }
}

function normalizeForwardProxyLiveNode(raw: unknown): ForwardProxyLiveNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const base = normalizeForwardProxyNode(raw)
  if (!base) return null
  const bucketsRaw = Array.isArray(payload.last24h) ? payload.last24h : []
  const last24h = bucketsRaw
    .map(normalizeForwardProxyHourlyBucket)
    .filter((item): item is ForwardProxyHourlyBucket => item != null)
  return {
    key: base.key,
    source: base.source,
    displayName: base.displayName,
    endpointUrl: base.endpointUrl,
    weight: base.weight,
    penalized: base.penalized,
    stats: base.stats,
    last24h,
  }
}

function normalizeForwardProxyLiveStatsResponse(raw: unknown): ForwardProxyLiveStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : []
  const nodes = nodesRaw
    .map(normalizeForwardProxyLiveNode)
    .filter((node): node is ForwardProxyLiveNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName))
  return {
    rangeStart: typeof payload.rangeStart === 'string' ? payload.rangeStart : '',
    rangeEnd: typeof payload.rangeEnd === 'string' ? payload.rangeEnd : '',
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    nodes,
  }
}

function normalizeForwardProxyValidationResult(raw: unknown): ForwardProxyValidationResult {
  const payload = (raw ?? {}) as Record<string, unknown>
  return {
    ok: payload.ok === true,
    message: typeof payload.message === 'string' && payload.message.trim() ? payload.message : 'validation failed',
    normalizedValue: typeof payload.normalizedValue === 'string' ? payload.normalizedValue : undefined,
    discoveredNodes: normalizeFiniteNumber(payload.discoveredNodes),
    latencyMs: normalizeFiniteNumber(payload.latencyMs),
  }
}

function normalizeSettingsPayload(raw: unknown): SettingsPayload {
  const payload = (raw ?? {}) as Record<string, unknown>
  return {
    proxy: normalizeProxySettings(payload.proxy),
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
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

export async function updateForwardProxySettings(payload: {
  proxyUrls: string[]
  subscriptionUrls: string[]
  subscriptionUpdateIntervalSecs: number
  insertDirect: boolean
}): Promise<ForwardProxySettings> {
  const response = await fetchJson<unknown>('/api/settings/forward-proxy', {
    method: 'PUT',
    body: JSON.stringify(payload),
  })
  return normalizeForwardProxySettings(response)
}

export async function validateForwardProxyCandidate(payload: {
  kind: ForwardProxyValidationKind
  value: string
}): Promise<ForwardProxyValidationResult> {
  const controller = new AbortController()
  const timeoutMs = forwardProxyValidationTimeoutMs(payload.kind)
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    const response = await fetchJson<unknown>('/api/settings/forward-proxy/validate', {
      method: 'POST',
      body: JSON.stringify(payload),
      signal: controller.signal,
    })
    return normalizeForwardProxyValidationResult(response)
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      throw new Error(
        `validation request timed out after ${Math.floor(timeoutMs / 1000)}s`,
      )
    }
    throw err
  } finally {
    clearTimeout(timer)
  }
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

export async function fetchForwardProxyLiveStats() {
  const response = await fetchJson<unknown>('/api/stats/forward-proxy')
  return normalizeForwardProxyLiveStatsResponse(response)
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
