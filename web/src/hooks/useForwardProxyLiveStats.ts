import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchForwardProxyLiveStats, type ForwardProxyLiveStatsResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

const SSE_REFRESH_THROTTLE_MS = 5_000
const POLLING_REFRESH_INTERVAL_MS = 60_000

interface LoadOptions {
  silent?: boolean
}

export function useForwardProxyLiveStats() {
  const [stats, setStats] = useState<ForwardProxyLiveStatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const hasHydratedRef = useRef(false)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRefreshAtRef = useRef(0)

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const load = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) setIsLoading(true)
    try {
      const response = await fetchForwardProxyLiveStats()
      setStats(response)
      hasHydratedRef.current = true
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (shouldShowLoading) setIsLoading(false)
    }
  }, [])

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now()
    const elapsed = now - lastRefreshAtRef.current
    const run = () => {
      refreshTimerRef.current = null
      lastRefreshAtRef.current = Date.now()
      void load({ silent: true })
    }
    if (elapsed >= SSE_REFRESH_THROTTLE_MS) {
      clearPendingRefreshTimer()
      run()
      return
    }
    if (refreshTimerRef.current) return
    refreshTimerRef.current = setTimeout(run, SSE_REFRESH_THROTTLE_MS - elapsed)
  }, [clearPendingRefreshTimer, load])

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
    const timer = setInterval(() => {
      void load({ silent: true })
    }, POLLING_REFRESH_INTERVAL_MS)
    return () => clearInterval(timer)
  }, [load])

  useEffect(
    () => () => {
      clearPendingRefreshTimer()
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
