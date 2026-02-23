import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchSettings,
  updatePricingSettings,
  updateProxySettings,
  type PricingSettings,
  type ProxySettings,
  type SettingsPayload,
} from '../lib/api'

export function useSettings() {
  const [settings, setSettings] = useState<SettingsPayload | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isProxySaving, setIsProxySaving] = useState(false)
  const [isPricingSaving, setIsPricingSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const serverSnapshotRef = useRef<SettingsPayload | null>(null)

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchSettings()
      setSettings(response)
      serverSnapshotRef.current = response
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

  const rollback = useCallback(() => {
    if (serverSnapshotRef.current) {
      setSettings(serverSnapshotRef.current)
    }
  }, [])

  const saveProxy = useCallback(
    async (nextProxy: ProxySettings) => {
      if (!settings) return
      const optimistic: SettingsPayload = {
        ...settings,
        proxy: {
          ...settings.proxy,
          hijackEnabled: nextProxy.hijackEnabled,
          mergeUpstreamEnabled: nextProxy.hijackEnabled ? nextProxy.mergeUpstreamEnabled : false,
          enabledModels: nextProxy.enabledModels,
        },
      }
      setSettings(optimistic)
      setIsProxySaving(true)
      try {
        const savedProxy = await updateProxySettings({
          hijackEnabled: optimistic.proxy.hijackEnabled,
          mergeUpstreamEnabled: optimistic.proxy.mergeUpstreamEnabled,
          enabledModels: optimistic.proxy.enabledModels,
        })
        const merged: SettingsPayload = {
          ...optimistic,
          proxy: savedProxy,
        }
        setSettings(merged)
        serverSnapshotRef.current = merged
        setError(null)
      } catch (err) {
        rollback()
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsProxySaving(false)
      }
    },
    [rollback, settings],
  )

  const savePricing = useCallback(
    async (nextPricing: PricingSettings) => {
      if (!settings) return
      const optimistic: SettingsPayload = {
        ...settings,
        pricing: nextPricing,
      }
      setSettings(optimistic)
      setIsPricingSaving(true)
      try {
        const savedPricing = await updatePricingSettings(nextPricing)
        const merged: SettingsPayload = {
          ...optimistic,
          pricing: savedPricing,
        }
        setSettings(merged)
        serverSnapshotRef.current = merged
        setError(null)
      } catch (err) {
        rollback()
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsPricingSaving(false)
      }
    },
    [rollback, settings],
  )

  return {
    settings,
    isLoading,
    isProxySaving,
    isPricingSaving,
    error,
    refresh: load,
    saveProxy,
    savePricing,
  }
}
