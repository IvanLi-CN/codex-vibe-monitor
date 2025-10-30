import { useCallback, useEffect, useState } from 'react'
import { fetchStats } from '../lib/api'
import type { StatsResponse } from '../lib/api'

export function useStats() {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchStats()
      setStats(response)
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

  return {
    stats,
    isLoading,
    error,
    refresh: load,
  }
}
