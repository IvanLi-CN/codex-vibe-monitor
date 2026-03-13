import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchUpstreamStickyConversations, type UpstreamStickyConversationsResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

export const UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS = 5_000
export const UPSTREAM_STICKY_POLLING_REFRESH_INTERVAL_MS = 60_000
export const UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS = 3_000

export function getUpstreamStickySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt))
}

export function shouldTriggerUpstreamStickyOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS
}

interface LoadOptions {
  silent?: boolean
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === 'AbortError'
}

export function useUpstreamStickyConversations(accountId: number | null, limit: number, enabled = true) {
  const [stats, setStats] = useState<UpstreamStickyConversationsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const accountIdRef = useRef(accountId)
  const limitRef = useRef(limit)
  const enabledRef = useRef(enabled)
  const hasHydratedRef = useRef(false)
  const inFlightRef = useRef(false)
  const pendingLoadRef = useRef<LoadOptions | null>(null)
  const pendingOpenResyncRef = useRef(false)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)
  const requestSeqRef = useRef(0)
  const abortControllerRef = useRef<AbortController | null>(null)

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  useEffect(() => {
    accountIdRef.current = accountId
  }, [accountId])

  useEffect(() => {
    limitRef.current = limit
  }, [limit])

  useEffect(() => {
    enabledRef.current = enabled
  }, [enabled])

  const invalidateCurrentRequest = useCallback(() => {
    requestSeqRef.current += 1
    abortControllerRef.current?.abort()
    abortControllerRef.current = null
    inFlightRef.current = false
    pendingLoadRef.current = null
    pendingOpenResyncRef.current = false
  }, [])

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    const targetAccountId = accountIdRef.current
    if (!enabledRef.current || targetAccountId == null) {
      setStats(null)
      setError(null)
      setIsLoading(false)
      hasHydratedRef.current = false
      return
    }

    inFlightRef.current = true
    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    const requestedLimit = limitRef.current
    const controller = new AbortController()
    abortControllerRef.current = controller
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) setIsLoading(true)
    try {
      const response = await fetchUpstreamStickyConversations(targetAccountId, requestedLimit, controller.signal)
      if (
        requestSeq !== requestSeqRef.current
        || accountIdRef.current !== targetAccountId
        || limitRef.current !== requestedLimit
        || !enabledRef.current
      ) {
        return
      }
      setStats(response)
      hasHydratedRef.current = true
      setError(null)
      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false
        const pendingSilent = pendingLoadRef.current?.silent ?? true
        pendingLoadRef.current = { silent: pendingSilent }
      }
    } catch (err) {
      if (isAbortError(err)) return
      if (requestSeq !== requestSeqRef.current) return
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (requestSeq === requestSeqRef.current) {
        abortControllerRef.current = null
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading) setIsLoading(false)
      if (requestSeq === requestSeqRef.current) {
        inFlightRef.current = false
      }
      const pendingLoad = pendingLoadRef.current
      if (requestSeq === requestSeqRef.current && pendingLoad) {
        pendingLoadRef.current = null
        void runLoad(pendingLoad)
      }
    }
  }, [])

  const load = useCallback(async (options: LoadOptions = {}) => {
    const silent = options.silent ?? false
    if (!enabledRef.current || accountIdRef.current == null) {
      setStats(null)
      setError(null)
      setIsLoading(false)
      hasHydratedRef.current = false
      return
    }
    if (inFlightRef.current) {
      const pendingSilent = pendingLoadRef.current?.silent ?? true
      pendingLoadRef.current = { silent: pendingSilent && silent }
      return
    }
    await runLoad({ silent })
  }, [runLoad])

  const triggerSseRefresh = useCallback(() => {
    if (!enabledRef.current || accountIdRef.current == null) return
    const now = Date.now()
    const delay = getUpstreamStickySseRefreshDelay(lastRefreshAtRef.current, now)
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

  const triggerOpenResync = useCallback((force = false) => {
    if (!enabledRef.current || accountIdRef.current == null) return
    if (!hasHydratedRef.current) {
      pendingOpenResyncRef.current = true
      return
    }
    const now = Date.now()
    if (!shouldTriggerUpstreamStickyOpenResync(lastOpenResyncAtRef.current, now, force)) return
    lastOpenResyncAtRef.current = now
    void load({ silent: true })
  }, [load])

  useEffect(() => {
    invalidateCurrentRequest()
    if (!enabled || accountId == null) {
      setStats(null)
      setError(null)
      setIsLoading(false)
      hasHydratedRef.current = false
      return
    }
    void load()
  }, [accountId, enabled, invalidateCurrentRequest, limit, load])

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
    if (!enabled || accountId == null) return undefined
    const timer = setInterval(() => {
      void load({ silent: true })
    }, UPSTREAM_STICKY_POLLING_REFRESH_INTERVAL_MS)
    return () => clearInterval(timer)
  }, [accountId, enabled, load])

  useEffect(
    () => () => {
      abortControllerRef.current?.abort()
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
