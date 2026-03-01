import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchSummary } from '../lib/api'
import type { StatsResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

interface UseSummaryOptions {
  limit?: number
}

const SUPPORTED_SSE_WINDOWS = new Set(['all', '30m', '1h', '1d', '1mo'])
export const UNSUPPORTED_SSE_REFRESH_INTERVAL_MS = 60_000
export const CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 600
export const CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS = 3_000

interface LoadOptions {
  silent?: boolean
  force?: boolean
}

interface PendingLoad {
  silent: boolean
  waiters: Array<() => void>
}

export interface UnsupportedRefreshGate {
  inFlight: boolean
  lastTriggerAt: number
}

export function createUnsupportedRefreshGate(): UnsupportedRefreshGate {
  return { inFlight: false, lastTriggerAt: 0 }
}

export function getCurrentSummarySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - (now - lastRefreshAt))
}

export function mergePendingSummarySilentOption(existingSilent: boolean | null, incomingSilent: boolean) {
  return (existingSilent ?? true) && incomingSilent
}

export function shouldTriggerCurrentSummaryOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS
}

export function shouldHandleUnsupportedSummaryRefresh(payloadWindow: string, currentWindow: string, supportsSse: boolean): boolean {
  return payloadWindow !== currentWindow && !supportsSse && currentWindow !== 'current'
}

export async function runUnsupportedSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  if (gate.inFlight || now - gate.lastTriggerAt < UNSUPPORTED_SSE_REFRESH_INTERVAL_MS) {
    return false
  }

  gate.inFlight = true
  gate.lastTriggerAt = now
  try {
    await refresh()
  } catch {
    // Keep fallback refresh best-effort; hook state already records request errors.
  } finally {
    gate.inFlight = false
  }
  return true
}

export function useSummary(window: string, options?: UseSummaryOptions) {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const unsupportedRefreshRef = useRef<UnsupportedRefreshGate>(createUnsupportedRefreshGate())
  const summaryContextRef = useRef<{ window: string; limit?: number }>({
    window,
    limit: options?.limit,
  })
  const hasHydratedRef = useRef(false)
  const activeLoadCountRef = useRef(0)
  const pendingLoadRef = useRef<PendingLoad | null>(null)
  const requestSeqRef = useRef(0)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastCurrentRecordsRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)
  summaryContextRef.current.window = window
  summaryContextRef.current.limit = options?.limit

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    activeLoadCountRef.current += 1
    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) {
      setIsLoading(true)
    }
    try {
      const response = await fetchSummary(summaryContextRef.current.window, {
        limit: summaryContextRef.current.limit,
      })
      if (requestSeq !== requestSeqRef.current) return
      setStats(response)
      hasHydratedRef.current = true
      setError(null)
    } catch (err) {
      if (requestSeq !== requestSeqRef.current) return
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (requestSeq === requestSeqRef.current && shouldShowLoading) {
        setIsLoading(false)
      }
      activeLoadCountRef.current = Math.max(0, activeLoadCountRef.current - 1)
    }
  }, [])

  const load = useCallback(async (loadOptions: LoadOptions = {}) => {
    const silent = loadOptions.silent ?? false
    const force = loadOptions.force ?? false
    if (!force && activeLoadCountRef.current > 0) {
      return new Promise<void>((resolve) => {
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingSummarySilentOption(pendingLoadRef.current.silent, silent)
          pendingLoadRef.current.waiters.push(resolve)
          return
        }
        pendingLoadRef.current = { silent, waiters: [resolve] }
      })
    }

    await runLoad({ silent })

    while (activeLoadCountRef.current === 0 && pendingLoadRef.current) {
      const pending = pendingLoadRef.current
      pendingLoadRef.current = null
      await runLoad({ silent: pending.silent })
      pending.waiters.forEach((resolve) => resolve())
    }
  }, [runLoad])

  const triggerCurrentWindowRefresh = useCallback(() => {
    const now = Date.now()
    const delay = getCurrentSummarySseRefreshDelay(lastCurrentRecordsRefreshAtRef.current, now)
    const run = () => {
      refreshTimerRef.current = null
      lastCurrentRecordsRefreshAtRef.current = Date.now()
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

  const triggerCurrentOpenResync = useCallback(() => {
    if (window !== 'current' || !hasHydratedRef.current) {
      return
    }
    const now = Date.now()
    if (!shouldTriggerCurrentSummaryOpenResync(lastOpenResyncAtRef.current, now)) {
      return
    }
    lastOpenResyncAtRef.current = now
    void load({ silent: true, force: true })
  }, [load, window])

  useEffect(() => {
    // Invalidate prior async loads when summary query context changes.
    requestSeqRef.current += 1
    hasHydratedRef.current = false
    lastCurrentRecordsRefreshAtRef.current = 0
    lastOpenResyncAtRef.current = 0
    clearPendingRefreshTimer()
    void load({ force: true })
  }, [clearPendingRefreshTimer, load, options?.limit, window])

  useEffect(
    () => () => {
      requestSeqRef.current += 1
      pendingLoadRef.current = null
      clearPendingRefreshTimer()
    },
    [clearPendingRefreshTimer],
  )

  const supportsSse = useMemo(() => SUPPORTED_SSE_WINDOWS.has(window), [window])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'summary') {
        if (payload.window === window) {
          setStats(payload.summary)
          hasHydratedRef.current = true
          setError(null)
          setIsLoading(false)
        } else if (shouldHandleUnsupportedSummaryRefresh(payload.window, window, supportsSse)) {
          // Unsupported windows (e.g. today) are refreshed at a fixed cadence to avoid request storms.
          void runUnsupportedSummaryRefresh(unsupportedRefreshRef.current, Date.now(), () => load({ silent: true }))
        }
      } else if (payload.type === 'records' && window === 'current') {
        // current 窗口通过节流静默刷新，避免高频事件导致闪烁。
        triggerCurrentWindowRefresh()
      }
    })
    return unsubscribe
  }, [load, supportsSse, triggerCurrentWindowRefresh, window])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerCurrentOpenResync()
    })
    return unsubscribe
  }, [triggerCurrentOpenResync])

  return {
    summary: stats,
    isLoading,
    error,
    refresh: load,
  }
}
