import { useCallback, useEffect, useState } from 'react'
import { fetchQuotaSnapshot } from '../lib/api'
import type { QuotaSnapshot } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

export function useQuotaSnapshot() {
  const [snapshot, setSnapshot] = useState<QuotaSnapshot | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchQuotaSnapshot()
      setSnapshot(response)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'quota') {
        setSnapshot(payload.snapshot)
        setError(null)
        setIsLoading(false)
      }
    })
    return unsubscribe
  }, [])

  return {
    snapshot,
    isLoading,
    error,
    refresh: load,
  }
}
