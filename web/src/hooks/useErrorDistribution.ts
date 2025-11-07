import { useCallback, useEffect, useState } from 'react'
import { fetchErrorDistribution } from '../lib/api'
import type { ErrorDistributionResponse } from '../lib/api'

export function useErrorDistribution(range: string, top?: number) {
  const [data, setData] = useState<ErrorDistributionResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const res = await fetchErrorDistribution(range, top != null ? { top } : undefined)
      setData(res)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [range, top])

  useEffect(() => {
    void load()
  }, [load])

  return { data, isLoading, error, refresh: load }
}
