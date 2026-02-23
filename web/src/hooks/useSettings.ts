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
  const pendingPricingRef = useRef<PricingSettings | null>(null)
  const pricingSaveInFlightRef = useRef(false)

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
      if (!serverSnapshotRef.current) return
      const normalizedProxy = {
        hijackEnabled: nextProxy.hijackEnabled,
        mergeUpstreamEnabled: nextProxy.hijackEnabled ? nextProxy.mergeUpstreamEnabled : false,
        enabledModels: nextProxy.enabledModels,
      }
      setSettings((current) => {
        if (!current) return current
        return {
          ...current,
          proxy: {
            ...current.proxy,
            ...normalizedProxy,
          },
        }
      })
      setIsProxySaving(true)
      try {
        const savedProxy = await updateProxySettings(normalizedProxy)
        const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
          ? {
              ...serverSnapshotRef.current,
              proxy: savedProxy,
            }
          : null
        if (confirmedSnapshot) {
          serverSnapshotRef.current = confirmedSnapshot
        }
        setSettings((current) => {
          if (!current) return confirmedSnapshot ?? current
          return {
            ...current,
            proxy: savedProxy,
          }
        })
        setError(null)
      } catch (err) {
        rollback()
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsProxySaving(false)
      }
    },
    [rollback],
  )

  const savePricing = useCallback(
    async (nextPricing: PricingSettings) => {
      if (!serverSnapshotRef.current) return
      setSettings((current) => {
        if (!current) return current
        return {
          ...current,
          pricing: nextPricing,
        }
      })
      pendingPricingRef.current = nextPricing

      if (pricingSaveInFlightRef.current) {
        return
      }

      pricingSaveInFlightRef.current = true
      setIsPricingSaving(true)
      while (pendingPricingRef.current) {
        const candidate = pendingPricingRef.current
        pendingPricingRef.current = null

        try {
          const savedPricing = await updatePricingSettings(candidate)

          // Ignore stale response payloads when a newer draft is already queued.
          if (pendingPricingRef.current == null) {
            const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
              ? {
                  ...serverSnapshotRef.current,
                  pricing: savedPricing,
                }
              : null
            if (confirmedSnapshot) {
              serverSnapshotRef.current = confirmedSnapshot
            }
            setSettings((current) => {
              if (!current) return confirmedSnapshot ?? current
              return {
                ...current,
                pricing: savedPricing,
              }
            })
          } else if (serverSnapshotRef.current) {
            serverSnapshotRef.current = {
              ...serverSnapshotRef.current,
              pricing: savedPricing,
            }
          }

          setError(null)
        } catch (err) {
          if (pendingPricingRef.current == null) {
            rollback()
          }
          setError(err instanceof Error ? err.message : String(err))
        }
      }

      pricingSaveInFlightRef.current = false
      setIsPricingSaving(false)
    },
    [rollback],
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
