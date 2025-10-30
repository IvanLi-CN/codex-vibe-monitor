import { useEffect, useMemo, useState } from 'react'
import { fetchInvocations } from '../lib/api'
import type { ApiInvocation, BroadcastPayload } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

export interface InvocationFilters {
  model?: string
  status?: string
}

function matchesFilters(record: ApiInvocation, filters?: InvocationFilters) {
  if (!filters) return true
  if (filters.model && record.model !== filters.model) return false
  if (filters.status && record.status !== filters.status) return false
  return true
}

function sortRecords(records: ApiInvocation[]) {
  return [...records].sort(
    (a, b) => new Date(b.occurredAt).getTime() - new Date(a.occurredAt).getTime(),
  )
}

function mergeRecords(
  incoming: ApiInvocation[],
  current: ApiInvocation[],
  limit: number,
  filters?: InvocationFilters,
) {
  const dedupe = new Map<string, ApiInvocation>()

  for (const record of incoming) {
    if (!matchesFilters(record, filters)) continue
    const key = `${record.invokeId}-${record.occurredAt}`
    dedupe.set(key, record)
  }

  for (const record of current) {
    if (!matchesFilters(record, filters)) continue
    const key = `${record.invokeId}-${record.occurredAt}`
    if (!dedupe.has(key)) {
      dedupe.set(key, record)
    }
  }

  const merged = sortRecords(Array.from(dedupe.values()))
  return merged.slice(0, limit)
}

export function useInvocationStream(
  limit: number,
  filters?: InvocationFilters,
  onNewRecords?: (records: ApiInvocation[]) => void,
  options?: { enableStream?: boolean },
) {
  const [records, setRecords] = useState<ApiInvocation[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const enableStream = options?.enableStream ?? true

  useEffect(() => {
    let isMounted = true
    setIsLoading(true)
    fetchInvocations(limit, filters)
      .then((response) => {
        if (!isMounted) return
        const next = mergeRecords(response.records, [], limit, filters)
        setRecords(next)
        setError(null)
      })
      .catch((err) => {
        if (!isMounted) return
        setError(err.message)
      })
      .finally(() => {
        if (isMounted) setIsLoading(false)
      })

    return () => {
      isMounted = false
    }
  }, [filters?.model, filters?.status, limit])

  useEffect(() => {
    if (!enableStream) {
      return
    }

    const unsubscribe = subscribeToSse((payload: BroadcastPayload) => {
      if (payload.type !== 'records') return
      setRecords((current) => {
        const next = mergeRecords(payload.records, current, limit, filters)
        if (onNewRecords) {
          const changed =
            next.length !== current.length ||
            next.some((record, index) => {
              const existing = current[index]
              return (
                !existing ||
                existing.invokeId !== record.invokeId ||
                existing.occurredAt !== record.occurredAt
              )
            })
          if (changed) {
            onNewRecords(next)
          }
        }
        return next
      })
    })

    return unsubscribe
  }, [enableStream, filters?.model, filters?.status, limit, onNewRecords])

  const hasData = useMemo(() => records.length > 0, [records])

  return {
    records,
    isLoading,
    error,
    hasData,
  }
}
