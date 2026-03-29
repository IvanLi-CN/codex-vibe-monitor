import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchInvocations } from '../lib/api'
import type { ApiInvocation, BroadcastPayload } from '../lib/api'
import {
  choosePreferredInvocationRecord,
  sortInvocationRecords,
} from '../lib/invocationLiveMerge'
import { invocationStableKey } from '../lib/invocation'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

export interface InvocationFilters {
  model?: string
  status?: string
}

function recordKey(record: ApiInvocation) {
  return invocationStableKey(record)
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

function matchesFailedStatus(record: ApiInvocation) {
  const failureClass = record.failureClass?.trim().toLowerCase()
  if (failureClass && failureClass !== 'none') return true

  const normalizedStatus = record.status?.trim().toLowerCase() ?? ''
  if (normalizedStatus === 'failed') return true
  if (normalizedStatus === 'http_429' || normalizedStatus.startsWith('http_4') || normalizedStatus.startsWith('http_5')) return true

  return Boolean(record.errorMessage?.trim())
}

function matchesFilters(record: ApiInvocation, filters?: InvocationFilters) {
  if (!filters) return true
  if (filters.model && record.model !== filters.model) return false
  if (filters.status === 'failed') return matchesFailedStatus(record)
  if (filters.status && record.status !== filters.status) return false
  return true
}

function mergeRecords(
  incoming: ApiInvocation[],
  current: ApiInvocation[],
  limit: number,
  filters?: InvocationFilters,
  hiddenKeys?: Set<string>,
) {
  const dedupe = new Map<string, ApiInvocation>()
  const currentKeys = new Set(current.map((record) => recordKey(record)))
  const nextHidden = new Set(hiddenKeys ?? [])

  const pushRecord = (record: ApiInvocation) => {
    if (!matchesFilters(record, filters)) return
    const key = recordKey(record)
    if (nextHidden.has(key) && !currentKeys.has(key)) return
    dedupe.set(key, choosePreferredInvocationRecord(dedupe.get(key), record))
  }

  for (const record of incoming) {
    pushRecord(record)
  }

  for (const record of current) {
    pushRecord(record)
  }

  const merged = sortInvocationRecords(Array.from(dedupe.values()))
  const visible = merged.slice(0, limit)
  for (const record of merged.slice(limit)) {
    nextHidden.add(recordKey(record))
  }
  return { records: visible, hiddenKeys: nextHidden }
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
  const hiddenKeysRef = useRef<Set<string>>(new Set())
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
          hiddenKeysRef.current = new Set()
          const authoritative = mergeRecords(response.records, [], limit, filters, hiddenKeysRef.current)
          const concurrent = current.filter(
            (record) => matchesFilters(record, filters) && !baselineKeys.has(recordKey(record)),
          )
          const nextState = mergeRecords(authoritative.records, concurrent, limit, filters, authoritative.hiddenKeys)
          const next = nextState.records
          hiddenKeysRef.current = nextState.hiddenKeys
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
          queueMicrotask(() => {
            void load({ silent: true })
          })
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
    hiddenKeysRef.current = new Set()
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
        const nextState = mergeRecords(payload.records, current, limit, filters, hiddenKeysRef.current)
        const next = nextState.records
        hiddenKeysRef.current = nextState.hiddenKeys
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
