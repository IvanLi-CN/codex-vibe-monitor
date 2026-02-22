import { useCallback, useEffect, useState } from 'react'
import { fetchProxyModelSettings, updateProxyModelSettings } from '../lib/api'
import type { ProxyModelSettings } from '../lib/api'

export function useProxyModelSettings() {
  const [settings, setSettings] = useState<ProxyModelSettings | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isSaving, setIsSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchProxyModelSettings()
      setSettings(response)
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

  const update = useCallback(
    async (next: { hijackEnabled: boolean; mergeUpstreamEnabled: boolean; enabledModels: string[] }) => {
      if (!settings) return
      const normalized = {
        hijackEnabled: next.hijackEnabled,
        mergeUpstreamEnabled: next.hijackEnabled ? next.mergeUpstreamEnabled : false,
        enabledModels: next.enabledModels,
      }
      const optimistic: ProxyModelSettings = {
        ...settings,
        ...normalized,
      }
      setSettings(optimistic)
      setIsSaving(true)
      try {
        const saved = await updateProxyModelSettings(normalized)
        setSettings(saved)
        setError(null)
      } catch (err) {
        setSettings(settings)
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsSaving(false)
      }
    },
    [settings],
  )

  return {
    settings,
    isLoading,
    isSaving,
    error,
    refresh: load,
    update,
  }
}
