import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchSettings,
  updateForwardProxySettings,
  updatePricingSettings,
  updateProxySettings,
  type ForwardProxySettings,
  type PricingSettings,
  type ProxySettings,
  type SettingsPayload,
} from '../lib/api'

function toPricingKey(pricing: PricingSettings): string {
  return JSON.stringify({
    catalogVersion: pricing.catalogVersion,
    entries: [...pricing.entries]
      .sort((a, b) => a.model.localeCompare(b.model))
      .map((entry) => ({
        model: entry.model,
        inputPer1m: entry.inputPer1m,
        outputPer1m: entry.outputPer1m,
        cacheInputPer1m: entry.cacheInputPer1m ?? null,
        reasoningPer1m: entry.reasoningPer1m ?? null,
        source: entry.source,
      })),
  })
}

function isSamePricingSettings(lhs: PricingSettings, rhs: PricingSettings): boolean {
  return toPricingKey(lhs) === toPricingKey(rhs)
}

export function useSettings() {
  const [settings, setSettings] = useState<SettingsPayload | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isProxySaving, setIsProxySaving] = useState(false)
  const [isForwardProxySaving, setIsForwardProxySaving] = useState(false)
  const [isPricingSaving, setIsPricingSaving] = useState(false)
  const [pricingRollbackVersion, setPricingRollbackVersion] = useState(0)
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
              if (isSamePricingSettings(current.pricing, savedPricing)) {
                return current
              }
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
            setPricingRollbackVersion((version) => version + 1)
          }
          setError(err instanceof Error ? err.message : String(err))
        }
      }

      pricingSaveInFlightRef.current = false
      setIsPricingSaving(false)
    },
    [rollback],
  )

  const saveForwardProxy = useCallback(
    async (nextForwardProxy: ForwardProxySettings) => {
      if (!serverSnapshotRef.current) return
      setSettings((current) => {
        if (!current) return current
        return {
          ...current,
          forwardProxy: nextForwardProxy,
        }
      })
      setIsForwardProxySaving(true)
      try {
        const saved = await updateForwardProxySettings({
          proxyUrls: nextForwardProxy.proxyUrls,
          subscriptionUrls: nextForwardProxy.subscriptionUrls,
          subscriptionUpdateIntervalSecs: nextForwardProxy.subscriptionUpdateIntervalSecs,
          insertDirect: nextForwardProxy.insertDirect,
        })

        const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
          ? {
              ...serverSnapshotRef.current,
              forwardProxy: saved,
            }
          : null
        if (confirmedSnapshot) {
          serverSnapshotRef.current = confirmedSnapshot
        }
        setSettings((current) => {
          if (!current) return confirmedSnapshot ?? current
          return {
            ...current,
            forwardProxy: saved,
          }
        })
        setError(null)
      } catch (err) {
        rollback()
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsForwardProxySaving(false)
      }
    },
    [rollback],
  )

  return {
    settings,
    isLoading,
    isProxySaving,
    isForwardProxySaving,
    isPricingSaving,
    pricingRollbackVersion,
    error,
    refresh: load,
    saveProxy,
    saveForwardProxy,
    savePricing,
  }
}
