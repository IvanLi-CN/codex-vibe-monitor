import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchForwardProxyLiveStats, type ForwardProxyLiveStatsResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

export const FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS = 5_000
export const FORWARD_PROXY_POLLING_REFRESH_INTERVAL_MS = 60_000
export const FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS = 3_000

export function getForwardProxySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt))
}

export function shouldTriggerForwardProxyOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS
}

interface LoadOptions {
  silent?: boolean
}

export function useForwardProxyLiveStats() {
  const [stats, setStats] = useState<ForwardProxyLiveStatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const hasHydratedRef = useRef(false)
  const inFlightRef = useRef(false)
  const pendingLoadRef = useRef<LoadOptions | null>(null)
  const pendingOpenResyncRef = useRef(false)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)
  const requestSeqRef = useRef(0)

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    inFlightRef.current = true
    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) setIsLoading(true)
    try {
      const response = await fetchForwardProxyLiveStats()
      if (requestSeq !== requestSeqRef.current) return
      setStats(response)
      hasHydratedRef.current = true
      setError(null)
      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false
        const pendingSilent = pendingLoadRef.current?.silent ?? true
        pendingLoadRef.current = { silent: pendingSilent }
      }
    } catch (err) {
      if (requestSeq !== requestSeqRef.current) return
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (requestSeq === requestSeqRef.current && shouldShowLoading) setIsLoading(false)
      inFlightRef.current = false
      const pendingLoad = pendingLoadRef.current
      if (pendingLoad) {
        pendingLoadRef.current = null
        void runLoad(pendingLoad)
      }
    }
  }, [])

  const load = useCallback(async (options: LoadOptions = {}) => {
    const silent = options.silent ?? false
    if (inFlightRef.current) {
      const pendingSilent = pendingLoadRef.current?.silent ?? true
      pendingLoadRef.current = { silent: pendingSilent && silent }
      return
    }
    await runLoad({ silent })
  }, [runLoad])

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now()
    const delay = getForwardProxySseRefreshDelay(lastRefreshAtRef.current, now)
    const run = () => {
      refreshTimerRef.current = null
      lastRefreshAtRef.current = Date.now()
      void load({ silent: true })
    }
    if (delay === 0) {
      clearPendingRefreshTimer()
      run()
      return
    }
    if (refreshTimerRef.current) return
    refreshTimerRef.current = setTimeout(run, delay)
  }, [clearPendingRefreshTimer, load])

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true
        return
      }
      const now = Date.now()
      if (!shouldTriggerForwardProxyOpenResync(lastOpenResyncAtRef.current, now, force)) return
      lastOpenResyncAtRef.current = now
      void load({ silent: true })
    },
    [load],
  )

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== 'records') return
      triggerSseRefresh()
    })
    return unsubscribe
  }, [triggerSseRefresh])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync()
    })
    return unsubscribe
  }, [triggerOpenResync])

  useEffect(() => {
    const timer = setInterval(() => {
      void load({ silent: true })
    }, FORWARD_PROXY_POLLING_REFRESH_INTERVAL_MS)
    return () => clearInterval(timer)
  }, [load])

  useEffect(
    () => () => {
      clearPendingRefreshTimer()
      pendingLoadRef.current = null
      pendingOpenResyncRef.current = false
    },
    [clearPendingRefreshTimer],
  )

  return {
    stats,
    isLoading,
    error,
    refresh: load,
  }
}
