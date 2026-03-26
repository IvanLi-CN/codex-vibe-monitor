import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchSettings,
  updateForwardProxySettings,
  updatePricingSettings,
  type ForwardProxySettings,
  type PricingSettings,
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
  const [isForwardProxySaving, setIsForwardProxySaving] = useState(false)
  const [isPricingSaving, setIsPricingSaving] = useState(false)
  const [pricingRollbackVersion, setPricingRollbackVersion] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const serverSnapshotRef = useRef<SettingsPayload | null>(null)
  const pendingPricingRef = useRef<PricingSettings | null>(null)
  const pricingSaveInFlightRef = useRef(false)
  const pendingForwardProxyRef = useRef<ForwardProxySettings | null>(null)
  const forwardProxySaveInFlightRef = useRef(false)

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
      pendingForwardProxyRef.current = nextForwardProxy

      if (forwardProxySaveInFlightRef.current) {
        return
      }

      forwardProxySaveInFlightRef.current = true
      setIsForwardProxySaving(true)
      while (pendingForwardProxyRef.current) {
        const candidate = pendingForwardProxyRef.current
        pendingForwardProxyRef.current = null

        try {
          const saved = await updateForwardProxySettings({
            proxyUrls: candidate.proxyUrls,
            subscriptionUrls: candidate.subscriptionUrls,
            subscriptionUpdateIntervalSecs: candidate.subscriptionUpdateIntervalSecs,
          })

          // Ignore stale payload when a newer draft is already queued.
          if (pendingForwardProxyRef.current == null) {
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
          } else if (serverSnapshotRef.current) {
            serverSnapshotRef.current = {
              ...serverSnapshotRef.current,
              forwardProxy: saved,
            }
          }
          setError(null)
        } catch (err) {
          if (pendingForwardProxyRef.current == null) {
            rollback()
          }
          setError(err instanceof Error ? err.message : String(err))
        }
      }

      forwardProxySaveInFlightRef.current = false
      setIsForwardProxySaving(false)
    },
    [rollback],
  )

  return {
    settings,
    isLoading,
    isForwardProxySaving,
    isPricingSaving,
    pricingRollbackVersion,
    error,
    refresh: load,
    saveForwardProxy,
    savePricing,
  }
}
