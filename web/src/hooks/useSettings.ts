import { useCallback, useEffect, useRef, useState } from "react";
import {
  type ForwardProxySettings,
  fetchPoolRoutingSettings,
  fetchSettings,
  type PoolRoutingSettings,
  type PricingSettings,
  type ProxySettings,
  type SettingsPayload,
  type UpdatePoolRoutingSettingsPayload,
  updateForwardProxySettings,
  updatePoolRoutingSettings,
  updatePricingSettings,
  updateProxySettings,
} from "../lib/api";
import { emitUpstreamAccountsChanged } from "../lib/upstreamAccountsEvents";
import { useLatestDebouncedMutation } from "./useLatestDebouncedMutation";

function toProxyUpdatePayload(proxy: ProxySettings) {
  return {
    hijackEnabled: proxy.hijackEnabled,
    mergeUpstreamEnabled: proxy.mergeUpstreamEnabled,
    fastModeRewriteMode: proxy.fastModeRewriteMode,
    upstream429MaxRetries: proxy.upstream429MaxRetries,
    websocketEnabled: proxy.websocketEnabled,
    upstreamWebsocketDefaultEnabled: proxy.upstreamWebsocketDefaultEnabled,
    requestBodyLoggingEnabled: proxy.requestBodyLoggingEnabled,
    responseBodyLoggingEnabled: proxy.responseBodyLoggingEnabled,
    encryptedSessionOwnerRoutingEnabled: proxy.encryptedSessionOwnerRoutingEnabled,
    enabledModels: proxy.enabledModels,
  };
}

function toPricingKey(pricing: PricingSettings): string {
  return JSON.stringify({
    catalogVersion: pricing.catalogVersion,
    entries: [...pricing.entries]
      .sort((a, b) => a.model.localeCompare(b.model))
      .map((entry) => ({
        model: entry.model,
        inputPer1m: entry.inputPer1m,
        outputPer1m: entry.outputPer1m,
        cacheInputPer1m: entry.cacheReadPer1m ?? entry.cacheInputPer1m ?? null,
        cacheReadPer1m: entry.cacheReadPer1m ?? entry.cacheInputPer1m ?? null,
        cacheWritePer1m: entry.cacheWritePer1m ?? null,
        reasoningPer1m: entry.reasoningPer1m ?? null,
        source: entry.source,
      })),
  });
}

function isSamePricingSettings(lhs: PricingSettings, rhs: PricingSettings): boolean {
  return toPricingKey(lhs) === toPricingKey(rhs);
}

export function useSettings() {
  const [settings, setSettings] = useState<SettingsPayload | null>(null);
  const [routing, setRouting] = useState<PoolRoutingSettings | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isPricingSaving, setIsPricingSaving] = useState(false);
  const [isRoutingSaving, setIsRoutingSaving] = useState(false);
  const [pricingRollbackVersion, setPricingRollbackVersion] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const serverSnapshotRef = useRef<SettingsPayload | null>(null);
  const pendingPricingRef = useRef<PricingSettings | null>(null);
  const pricingSaveInFlightRef = useRef(false);
  const proxyMutation = useLatestDebouncedMutation<ProxySettings, ProxySettings>({
    resourceKey: "settings-proxy",
    mutate: async (candidate) => updateProxySettings(toProxyUpdatePayload(candidate)),
    onSuccess: (saved) => {
      const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
        ? { ...serverSnapshotRef.current, proxy: saved }
        : null;
      if (confirmedSnapshot) serverSnapshotRef.current = confirmedSnapshot;
      setSettings((current) =>
        current ? { ...current, proxy: saved } : (confirmedSnapshot ?? current),
      );
      setError(null);
    },
    onError: (err) => {
      setError(err instanceof Error ? err.message : String(err));
    },
    onRevert: (confirmed) => {
      if (!confirmed) return;
      const snapshot = serverSnapshotRef.current;
      if (snapshot) serverSnapshotRef.current = { ...snapshot, proxy: confirmed };
      setSettings((current) => (current ? { ...current, proxy: confirmed } : current));
      setError(null);
    },
  });

  const forwardProxyMutation = useLatestDebouncedMutation<
    ForwardProxySettings,
    ForwardProxySettings
  >({
    resourceKey: "settings-forward-proxy",
    mutate: async (candidate) =>
      updateForwardProxySettings({
        proxyUrls: candidate.proxyUrls,
        subscriptionUrls: candidate.subscriptionUrls,
        subscriptionUpdateIntervalSecs: candidate.subscriptionUpdateIntervalSecs,
      }),
    onSuccess: (saved) => {
      const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
        ? { ...serverSnapshotRef.current, forwardProxy: saved }
        : null;
      if (confirmedSnapshot) serverSnapshotRef.current = confirmedSnapshot;
      setSettings((current) =>
        current ? { ...current, forwardProxy: saved } : (confirmedSnapshot ?? current),
      );
      emitUpstreamAccountsChanged();
      setError(null);
    },
    onError: (err) => {
      setError(err instanceof Error ? err.message : String(err));
    },
    onRevert: (confirmed) => {
      if (!confirmed) return;
      const snapshot = serverSnapshotRef.current;
      if (snapshot) serverSnapshotRef.current = { ...snapshot, forwardProxy: confirmed };
      setSettings((current) => (current ? { ...current, forwardProxy: confirmed } : current));
      setError(null);
    },
  });
  const retryProxy = useCallback(() => {
    setError(null);
    proxyMutation.retry();
  }, [proxyMutation.retry]);
  const revertProxy = useCallback(() => {
    setError(null);
    proxyMutation.revert();
  }, [proxyMutation.revert]);
  const retryForwardProxy = useCallback(() => {
    setError(null);
    forwardProxyMutation.retry();
  }, [forwardProxyMutation.retry]);
  const revertForwardProxy = useCallback(() => {
    setError(null);
    forwardProxyMutation.revert();
  }, [forwardProxyMutation.revert]);
  const proxyMutationPendingRef = useRef(false);
  const forwardProxyMutationPendingRef = useRef(false);
  proxyMutationPendingRef.current = proxyMutation.hasPending;
  forwardProxyMutationPendingRef.current = forwardProxyMutation.hasPending;

  const load = useCallback(async () => {
    setIsLoading(true);
    try {
      const [settingsResult, routingResult] = await Promise.allSettled([
        fetchSettings(),
        fetchPoolRoutingSettings(),
      ]);

      if (settingsResult.status !== "fulfilled") {
        throw settingsResult.reason;
      }

      serverSnapshotRef.current = settingsResult.value;
      setSettings((current) => {
        if (!current) return settingsResult.value;
        return {
          ...settingsResult.value,
          proxy: proxyMutationPendingRef.current ? current.proxy : settingsResult.value.proxy,
          forwardProxy: forwardProxyMutationPendingRef.current
            ? current.forwardProxy
            : settingsResult.value.forwardProxy,
        };
      });
      if (!proxyMutationPendingRef.current) proxyMutation.reconcile(settingsResult.value.proxy);
      if (!forwardProxyMutationPendingRef.current) {
        forwardProxyMutation.reconcile(settingsResult.value.forwardProxy);
      }

      if (routingResult.status === "fulfilled") {
        setRouting(routingResult.value);
        setError(null);
      } else {
        setRouting(null);
        setError(
          routingResult.reason instanceof Error
            ? routingResult.reason.message
            : String(routingResult.reason),
        );
      }
    } catch (err) {
      setRouting(null);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoading(false);
    }
  }, [forwardProxyMutation.reconcile, proxyMutation.reconcile]);

  useEffect(() => {
    void load();
  }, [load]);

  const rollback = useCallback(() => {
    if (serverSnapshotRef.current) {
      setSettings(serverSnapshotRef.current);
    }
  }, []);

  const saveProxy = useCallback(
    (nextProxy: ProxySettings) => {
      if (!serverSnapshotRef.current) return;
      const normalizedProxy: ProxySettings = {
        hijackEnabled: nextProxy.hijackEnabled,
        mergeUpstreamEnabled: nextProxy.hijackEnabled ? nextProxy.mergeUpstreamEnabled : false,
        fastModeRewriteMode: nextProxy.fastModeRewriteMode,
        upstream429MaxRetries: Math.max(
          0,
          Math.min(5, Math.trunc(nextProxy.upstream429MaxRetries)),
        ),
        websocketEnabled: nextProxy.websocketEnabled,
        upstreamWebsocketDefaultEnabled: nextProxy.upstreamWebsocketDefaultEnabled,
        requestBodyLoggingEnabled: nextProxy.requestBodyLoggingEnabled,
        responseBodyLoggingEnabled: nextProxy.responseBodyLoggingEnabled,
        encryptedSessionOwnerRoutingEnabled: nextProxy.encryptedSessionOwnerRoutingEnabled,
        defaultHijackEnabled: nextProxy.defaultHijackEnabled,
        models: nextProxy.models,
        enabledModels: nextProxy.models.filter((candidate) =>
          nextProxy.enabledModels.includes(candidate),
        ),
      };

      setSettings((current) => {
        if (!current) return current;
        return {
          ...current,
          proxy: normalizedProxy,
        };
      });
      proxyMutation.schedule(normalizedProxy);
    },
    [proxyMutation.schedule],
  );

  const savePricing = useCallback(
    async (nextPricing: PricingSettings) => {
      if (!serverSnapshotRef.current) return;
      setSettings((current) => {
        if (!current) return current;
        return {
          ...current,
          pricing: nextPricing,
        };
      });
      pendingPricingRef.current = nextPricing;

      if (pricingSaveInFlightRef.current) {
        return;
      }

      pricingSaveInFlightRef.current = true;
      setIsPricingSaving(true);
      while (pendingPricingRef.current) {
        const candidate = pendingPricingRef.current;
        pendingPricingRef.current = null;

        try {
          const savedPricing = await updatePricingSettings(candidate);

          // Ignore stale response payloads when a newer draft is already queued.
          if (pendingPricingRef.current == null) {
            const confirmedSnapshot: SettingsPayload | null = serverSnapshotRef.current
              ? {
                  ...serverSnapshotRef.current,
                  pricing: savedPricing,
                }
              : null;
            if (confirmedSnapshot) {
              serverSnapshotRef.current = confirmedSnapshot;
            }
            setSettings((current) => {
              if (!current) return confirmedSnapshot ?? current;
              if (isSamePricingSettings(current.pricing, savedPricing)) {
                return current;
              }
              return {
                ...current,
                pricing: savedPricing,
              };
            });
          } else if (serverSnapshotRef.current) {
            serverSnapshotRef.current = {
              ...serverSnapshotRef.current,
              pricing: savedPricing,
            };
          }

          setError(null);
        } catch (err) {
          if (pendingPricingRef.current == null) {
            rollback();
            setPricingRollbackVersion((version) => version + 1);
          }
          setError(err instanceof Error ? err.message : String(err));
        }
      }

      pricingSaveInFlightRef.current = false;
      setIsPricingSaving(false);
    },
    [rollback],
  );

  const saveForwardProxy = useCallback(
    (nextForwardProxy: ForwardProxySettings) => {
      if (!serverSnapshotRef.current) return;
      setSettings((current) => {
        if (!current) return current;
        return {
          ...current,
          forwardProxy: nextForwardProxy,
        };
      });
      forwardProxyMutation.schedule(nextForwardProxy);
    },
    [forwardProxyMutation.schedule],
  );

  const saveRouting = useCallback(async (payload: UpdatePoolRoutingSettingsPayload) => {
    setIsRoutingSaving(true);
    try {
      const saved = await updatePoolRoutingSettings(payload);
      setRouting(saved);
      emitUpstreamAccountsChanged();
      setError(null);
      return saved;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      throw err;
    } finally {
      setIsRoutingSaving(false);
    }
  }, []);

  return {
    settings,
    routing,
    isLoading,
    isProxySaving: proxyMutation.status === "pending" || proxyMutation.status === "saving",
    isForwardProxySaving:
      forwardProxyMutation.status === "pending" || forwardProxyMutation.status === "saving",
    proxySaveError: proxyMutation.error,
    forwardProxySaveError: forwardProxyMutation.error,
    isPricingSaving,
    isRoutingSaving,
    pricingRollbackVersion,
    error,
    refresh: load,
    saveProxy,
    flushProxy: proxyMutation.flush,
    retryProxy,
    revertProxy,
    saveForwardProxy,
    flushForwardProxy: forwardProxyMutation.flush,
    retryForwardProxy,
    revertForwardProxy,
    savePricing,
    saveRouting,
  };
}
