import { useCallback, useEffect, useMemo, useState } from 'react'
import { fetchSummary } from '../lib/api'
import type { StatsResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

interface UseSummaryOptions {
  limit?: number
}

const SUPPORTED_SSE_WINDOWS = new Set(['all', '30m', '1h', '1d', '1mo'])

export function useSummary(window: string, options?: UseSummaryOptions) {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchSummary(window, { limit: options?.limit })
      setStats(response)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
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
        } else if (!supportsSse && window !== 'current') {
          void load()
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
