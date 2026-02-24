import { useCallback, useEffect, useState } from 'react'
import { fetchFailureSummary } from '../lib/api'
import type { FailureSummaryResponse } from '../lib/api'

export function useFailureSummary(range: string) {
  const [data, setData] = useState<FailureSummaryResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const res = await fetchFailureSummary(range)
      setData(res)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [range])

  useEffect(() => {
    void load()
  }, [load])

  return { data, isLoading, error, refresh: load }
}
