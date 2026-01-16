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
  createdAt: string
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

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>('/api/version')
}

export async function fetchSummary(window: string, options?: { limit?: number }) {
  const search = new URLSearchParams()
  search.set('window', window)
  if (options?.limit !== undefined) {
    search.set('limit', String(options.limit))
  }
  return fetchJson<StatsResponse>(`/api/stats/summary?${search.toString()}`)
}

export async function fetchTimeseries(range: string, params?: { bucket?: string; settlementHour?: number }) {
  const search = new URLSearchParams()
  search.set('range', range)
  if (params?.bucket) search.set('bucket', params.bucket)
  if (params?.settlementHour !== undefined) search.set('settlementHour', String(params.settlementHour))
  return fetchJson<TimeseriesResponse>(`/api/stats/timeseries?${search.toString()}`)
}

export async function fetchErrorDistribution(range: string, params?: { top?: number }) {
  const search = new URLSearchParams()
  search.set('range', range)
  if (params?.top != null) search.set('top', String(params.top))
  return fetchJson<ErrorDistributionResponse>(`/api/stats/errors?${search.toString()}`)
}

export async function fetchQuotaSnapshot() {
  return fetchJson<QuotaSnapshot>('/api/quota/latest')
}

export function createEventSource(path: string) {
  return new EventSource(withBase(path))
}
