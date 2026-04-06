import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchParallelWorkStats, type ParallelWorkStatsResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

interface LoadOptions {
  silent?: boolean
  force?: boolean
}

interface PendingLoad {
  silent: boolean
}

export const PARALLEL_WORK_REFRESH_THROTTLE_MS = 60_000
export const PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS = 60_000

export function getParallelWorkRecordsResyncDelay(
  lastRefreshAt: number,
  now: number,
  throttleMs = PARALLEL_WORK_REFRESH_THROTTLE_MS,
) {
  return Math.max(0, throttleMs - (now - lastRefreshAt))
}

export function shouldTriggerParallelWorkOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true
  return now - lastResyncAt >= PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS
}

export function useParallelWorkStats() {
  const [data, setData] = useState<ParallelWorkStatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const hasHydratedRef = useRef(false)
  const pendingLoadRef = useRef<PendingLoad | null>(null)
  const pendingOpenResyncRef = useRef(false)
  const activeLoadCountRef = useRef(0)
  const requestSeqRef = useRef(0)
  const activeRequestControllerRef = useRef<AbortController | null>(null)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRecordsRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    activeLoadCountRef.current += 1
    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    const controller = new AbortController()
    activeRequestControllerRef.current = controller
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) {
      setIsLoading(true)
    }

    try {
      const response = await fetchParallelWorkStats({ signal: controller.signal })
      if (requestSeq !== requestSeqRef.current) {
        return
      }
      setData(response)
      setError(null)
      hasHydratedRef.current = true
      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false
        lastOpenResyncAtRef.current = Date.now()
      }
    } catch (err) {
      if (requestSeq !== requestSeqRef.current) {
        return
      }
      if (err instanceof Error && err.name === 'AbortError') {
        return
      }
      setError(err instanceof Error ? err.message : String(err))
      hasHydratedRef.current = true
    } finally {
      if (activeRequestControllerRef.current === controller) {
        activeRequestControllerRef.current = null
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading) {
        setIsLoading(false)
      }
      activeLoadCountRef.current = Math.max(0, activeLoadCountRef.current - 1)
      if (activeLoadCountRef.current === 0 && pendingLoadRef.current) {
        const pending = pendingLoadRef.current
        pendingLoadRef.current = null
        void runLoad({ silent: pending.silent })
      }
    }
  }, [])

  const load = useCallback(async ({ silent = false, force = false }: LoadOptions = {}) => {
    if (force) {
      activeRequestControllerRef.current?.abort()
      activeRequestControllerRef.current = null
      pendingLoadRef.current = null
      clearPendingRefreshTimer()
    }

    if (!force && activeLoadCountRef.current > 0) {
      pendingLoadRef.current = {
        silent: (pendingLoadRef.current?.silent ?? true) && silent,
      }
      return
    }

    await runLoad({ silent })
  }, [clearPendingRefreshTimer, runLoad])

  const triggerRecordsResync = useCallback(() => {
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return
    const now = Date.now()
    const delay = getParallelWorkRecordsResyncDelay(lastRecordsRefreshAtRef.current, now)
    const run = () => {
      refreshTimerRef.current = null
      lastRecordsRefreshAtRef.current = Date.now()
      void load({ silent: true })
    }

    if (delay === 0) {
      clearPendingRefreshTimer()
      run()
      return
    }
    if (refreshTimerRef.current) {
      return
    }
    refreshTimerRef.current = setTimeout(run, delay)
  }, [clearPendingRefreshTimer, load])

  const triggerOpenResync = useCallback((force = false) => {
    if (!hasHydratedRef.current) {
      pendingOpenResyncRef.current = true
      return
    }
    const now = Date.now()
    if (!shouldTriggerParallelWorkOpenResync(lastOpenResyncAtRef.current, now, force)) {
      return
    }
    lastOpenResyncAtRef.current = now
    void load({ silent: true, force: true })
  }, [load])

  useEffect(() => {
    requestSeqRef.current += 1
    activeRequestControllerRef.current?.abort()
    activeRequestControllerRef.current = null
    pendingLoadRef.current = null
    pendingOpenResyncRef.current = false
    hasHydratedRef.current = false
    lastRecordsRefreshAtRef.current = 0
    lastOpenResyncAtRef.current = 0
    clearPendingRefreshTimer()
    void load({ force: true })
  }, [clearPendingRefreshTimer, load])

  useEffect(() => {
    if (!error) return
    const timer = setTimeout(() => {
      void load()
    }, 2_000)
    return () => clearTimeout(timer)
  }, [error, load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== 'records') return
      triggerRecordsResync()
    })
    return unsubscribe
  }, [triggerRecordsResync])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync()
    })
    return unsubscribe
  }, [triggerOpenResync])

  useEffect(() => {
    if (typeof document === 'undefined') return
    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return
      triggerOpenResync()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [triggerOpenResync])

  useEffect(
    () => () => {
      requestSeqRef.current += 1
      activeRequestControllerRef.current?.abort()
      activeRequestControllerRef.current = null
      pendingLoadRef.current = null
      pendingOpenResyncRef.current = false
      clearPendingRefreshTimer()
    },
    [clearPendingRefreshTimer],
  )

  return {
    data,
    isLoading,
    error,
    refresh: load,
  }
}
