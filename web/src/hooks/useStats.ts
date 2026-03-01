import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchSummary } from '../lib/api'
import type { StatsResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

interface UseSummaryOptions {
  limit?: number
}

const SUPPORTED_SSE_WINDOWS = new Set(['all', '30m', '1h', '1d', '1mo'])
export const UNSUPPORTED_SSE_REFRESH_INTERVAL_MS = 60_000
export const CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 600

interface LoadOptions {
  silent?: boolean
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
  const hasHydratedRef = useRef(false)
  const inFlightRef = useRef(false)
  const pendingLoadRef = useRef<LoadOptions | null>(null)
  const requestSeqRef = useRef(0)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastCurrentRecordsRefreshAtRef = useRef(0)

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
    if (shouldShowLoading) {
      setIsLoading(true)
    }
    try {
      const response = await fetchSummary(window, { limit: options?.limit })
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
      inFlightRef.current = false
      const pendingLoad = pendingLoadRef.current
      if (pendingLoad) {
        pendingLoadRef.current = null
        void runLoad(pendingLoad)
      }
    }
  }, [options?.limit, window])

  const load = useCallback(async (loadOptions: LoadOptions = {}) => {
    const silent = loadOptions.silent ?? false
    if (inFlightRef.current) {
      pendingLoadRef.current = {
        silent: mergePendingSummarySilentOption(pendingLoadRef.current?.silent ?? null, silent),
      }
      return
    }
    await runLoad({ silent })
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

  useEffect(() => {
    // Invalidate prior async loads when summary query context changes.
    requestSeqRef.current += 1
    hasHydratedRef.current = false
    pendingLoadRef.current = null
    lastCurrentRecordsRefreshAtRef.current = 0
    clearPendingRefreshTimer()
    void load()
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

  return {
    summary: stats,
    isLoading,
    error,
    refresh: load,
  }
}
