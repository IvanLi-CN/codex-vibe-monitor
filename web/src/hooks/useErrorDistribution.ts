import { useCallback, useEffect, useState } from 'react'
import { fetchErrorDistribution } from '../lib/api'
import type { ErrorDistributionResponse, FailureScope } from '../lib/api'

export function useErrorDistribution(range: string, top?: number, scope: FailureScope = 'service') {
  const [data, setData] = useState<ErrorDistributionResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    // console.debug('[useErrorDistribution] load start', { range, top })
    try {
      const res = await fetchErrorDistribution(range, top != null ? { top, scope } : { scope })
      setData(res)
      setError(null)
      // console.debug('[useErrorDistribution] load ok', res.items?.length)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      // console.debug('[useErrorDistribution] load error', err)
    } finally {
      setIsLoading(false)
    }
  }, [range, scope, top])

  useEffect(() => {
    void load()
  }, [load])

  return { data, isLoading, error, refresh: load }
}
