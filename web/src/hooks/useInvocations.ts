import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchInvocations } from '../lib/api'
import type { ApiInvocation, BroadcastPayload } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

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
  const hasHydratedRef = useRef(false)
  const lastResyncAtRef = useRef(0)

  const load = useCallback(
    async (opts?: { silent?: boolean }) => {
      const shouldShowLoading = !(opts?.silent && hasHydratedRef.current)
      if (shouldShowLoading) {
        setIsLoading(true)
      }
      try {
        const response = await fetchInvocations(limit, filters)
        setRecords((current) =>
          mergeRecords(response.records, opts?.silent ? current : [], limit, filters),
        )
        hasHydratedRef.current = true
        setError(null)
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsLoading(false)
      }
    },
    [filters, limit],
  )

  useEffect(() => {
    hasHydratedRef.current = false
    lastResyncAtRef.current = 0
    void load()
  }, [load])

  // Auto-retry if initial load failed (e.g., backend temporarily unavailable)
  useEffect(() => {
    if (!error || records.length > 0) return
    const id = setTimeout(() => {
      void load()
    }, 2000)
    return () => clearTimeout(id)
  }, [error, load, records.length])

  const requestResync = useCallback(() => {
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return
    const now = Date.now()
    if (now - lastResyncAtRef.current < 3000) return
    lastResyncAtRef.current = now
    void load({ silent: true })
  }, [load])

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
  }, [enableStream, filters, limit, onNewRecords])

  useEffect(() => {
    if (!enableStream) {
      return
    }
    const unsubscribe = subscribeToSseOpen(() => {
      requestResync()
    })
    return unsubscribe
  }, [enableStream, requestResync])

  const hasData = useMemo(() => records.length > 0, [records])

  return {
    records,
    isLoading,
    error,
    hasData,
  }
}
