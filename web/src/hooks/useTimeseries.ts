import { useCallback, useEffect, useMemo, useState } from 'react'
import { fetchTimeseries } from '../lib/api'
import type { TimeseriesResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

export interface UseTimeseriesOptions {
  bucket?: string
  settlementHour?: number
}

export function useTimeseries(range: string, options?: UseTimeseriesOptions) {
  const [data, setData] = useState<TimeseriesResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const bucket = options?.bucket
  const settlementHour = options?.settlementHour

  const normalizedOptions = useMemo<UseTimeseriesOptions>(
    () => ({
      bucket,
      settlementHour,
    }),
    [bucket, settlementHour],
  )

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchTimeseries(range, normalizedOptions)
      setData(response)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [normalizedOptions, range])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'records') {
        void load()
      }
    })
    return unsubscribe
  }, [load])

  return {
    data,
    isLoading,
    error,
    refresh: load,
  }
}
