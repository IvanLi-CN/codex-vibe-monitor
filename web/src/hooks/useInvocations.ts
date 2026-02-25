import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchInvocations } from '../lib/api'
import type { ApiInvocation, BroadcastPayload } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

export interface InvocationFilters {
  model?: string
  status?: string
}

function recordKey(record: ApiInvocation) {
  return `${record.invokeId}-${record.occurredAt}`
}

function recordsChanged(next: ApiInvocation[], current: ApiInvocation[]) {
  return (
    next.length !== current.length ||
    next.some((record, index) => {
      const existing = current[index]
      return (
        !existing || existing.invokeId !== record.invokeId || existing.occurredAt !== record.occurredAt
      )
    })
  )
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
    const key = recordKey(record)
    dedupe.set(key, record)
  }

  for (const record of current) {
    if (!matchesFilters(record, filters)) continue
    const key = recordKey(record)
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
  const pendingOpenResyncRef = useRef(false)
  const lastResyncAtRef = useRef(0)
  const requestSeqRef = useRef(0)
  const recordsRef = useRef<ApiInvocation[]>([])
  const onNewRecordsRef = useRef(onNewRecords)

  useEffect(() => {
    onNewRecordsRef.current = onNewRecords
  }, [onNewRecords])

  useEffect(() => {
    recordsRef.current = records
  }, [records])

  const load = useCallback(
    async (opts?: { silent?: boolean }) => {
      const requestSeq = requestSeqRef.current + 1
      requestSeqRef.current = requestSeq
      const suppressError = Boolean(opts?.silent && hasHydratedRef.current)
      const shouldShowLoading = !(opts?.silent && hasHydratedRef.current)
      const baselineKeys = new Set(recordsRef.current.map((record) => recordKey(record)))
      if (shouldShowLoading) {
        setIsLoading(true)
      }
      try {
        const response = await fetchInvocations(limit, filters)
        if (requestSeq !== requestSeqRef.current) {
          return
        }
        setRecords((current) => {
          const authoritative = mergeRecords(response.records, [], limit, filters)
          const concurrent = current.filter(
            (record) => matchesFilters(record, filters) && !baselineKeys.has(recordKey(record)),
          )
          const next = mergeRecords(authoritative, concurrent, limit, filters)
          recordsRef.current = next
          if (onNewRecordsRef.current && recordsChanged(next, current)) {
            onNewRecordsRef.current(next)
          }
          return next
        })
        hasHydratedRef.current = true
        lastResyncAtRef.current = Date.now()
        if (pendingOpenResyncRef.current) {
          pendingOpenResyncRef.current = false
          void load({ silent: true })
        }
        setError(null)
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) {
          return
        }
        if (!suppressError) {
          setError(err instanceof Error ? err.message : String(err))
        }
      } finally {
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false)
        }
      }
    },
    [filters, limit],
  )

  useEffect(() => {
    return () => {
      requestSeqRef.current += 1
    }
  }, [])

  useEffect(() => {
    hasHydratedRef.current = false
    pendingOpenResyncRef.current = false
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

  const requestResync = useCallback(
    (opts?: { force?: boolean }) => {
      const force = opts?.force ?? false
      if (!force && typeof document !== 'undefined' && document.visibilityState !== 'visible') return
      const now = Date.now()
      if (!force && now - lastResyncAtRef.current < 3000) return
      lastResyncAtRef.current = now
      void load({ silent: true })
    },
    [load],
  )

  useEffect(() => {
    if (!enableStream) {
      return
    }

    const unsubscribe = subscribeToSse((payload: BroadcastPayload) => {
      if (payload.type !== 'records') return
      setRecords((current) => {
        const next = mergeRecords(payload.records, current, limit, filters)
        recordsRef.current = next
        if (onNewRecordsRef.current) {
          if (recordsChanged(next, current)) {
            onNewRecordsRef.current(next)
          }
        }
        return next
      })
    })

    return unsubscribe
  }, [enableStream, filters, limit])

  useEffect(() => {
    if (!enableStream) {
      return
    }
    const unsubscribe = subscribeToSseOpen(() => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true
        return
      }
      requestResync({ force: true })
    })
    return unsubscribe
  }, [enableStream, requestResync])

  useEffect(() => {
    if (typeof document === 'undefined' || !enableStream) {
      return
    }
    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return
      requestResync()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [enableStream, requestResync])

  const hasData = useMemo(() => records.length > 0, [records])

  return {
    records,
    isLoading,
    error,
    hasData,
  }
}
