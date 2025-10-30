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

export type BroadcastPayload =
  | {
      type: 'records'
      records: ApiInvocation[]
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

export function createEventSource(path: string) {
  return new EventSource(withBase(path))
}
