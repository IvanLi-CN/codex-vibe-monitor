import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchSummary } from '../lib/api'
import type { StatsResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

interface UseSummaryOptions {
  limit?: number
}

const SUPPORTED_SSE_WINDOWS = new Set(['all', '30m', '1h', '1d', '1mo'])
export const UNSUPPORTED_SSE_REFRESH_INTERVAL_MS = 60_000

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

  const load = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    if (!silent) {
      setIsLoading(true)
    }
    try {
      const response = await fetchSummary(window, { limit: options?.limit })
      setStats(response)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (!silent) {
        setIsLoading(false)
      }
    }
  }, [options?.limit, window])

  useEffect(() => {
    void load()
  }, [load])

  const supportsSse = useMemo(() => SUPPORTED_SSE_WINDOWS.has(window), [window])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'summary') {
        if (payload.window === window) {
          setStats(payload.summary)
          setError(null)
          setIsLoading(false)
        } else if (shouldHandleUnsupportedSummaryRefresh(payload.window, window, supportsSse)) {
          // Unsupported windows (e.g. today) are refreshed at a fixed cadence to avoid request storms.
          void runUnsupportedSummaryRefresh(unsupportedRefreshRef.current, Date.now(), () => load({ silent: true }))
        }
      } else if (payload.type === 'records' && window === 'current') {
        // current 窗口基于前端缓存，直接刷新
        void load()
      }
    })
    return unsubscribe
  }, [load, supportsSse, window])

  return {
    summary: stats,
    isLoading,
    error,
    refresh: load,
  }
}
