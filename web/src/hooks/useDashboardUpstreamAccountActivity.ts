import { useCallback, useEffect, useRef, useState } from "react";
import {
  fetchUpstreamAccountActivity,
  type UpstreamAccountActivityResponse,
} from "../lib/api";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_REFRESH_THROTTLE_MS = 5_000;
export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_OPEN_RESYNC_COOLDOWN_MS = 3_000;

interface LoadOptions {
  silent?: boolean;
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

export function useDashboardUpstreamAccountActivity(
  range: string,
  enabled: boolean,
) {
  const [data, setData] = useState<UpstreamAccountActivityResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const enabledRef = useRef(enabled);
  const rangeRef = useRef(range);
  const hasActivatedRef = useRef(false);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const requestSeqRef = useRef(0);
  const pendingLoadRef = useRef<LoadOptions | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const lastOpenResyncAtRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    enabledRef.current = enabled;
  }, [enabled]);

  useEffect(() => {
    rangeRef.current = range;
  }, [range]);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const invalidateCurrentRequest = useCallback(() => {
    requestSeqRef.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
  }, []);

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    if (!enabledRef.current) {
      return;
    }
    inFlightRef.current = true;
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    const requestedRange = rangeRef.current;
    const controller = new AbortController();
    abortControllerRef.current = controller;
    const shouldShowLoading = !(silent && hasHydratedRef.current);
    if (shouldShowLoading) setIsLoading(true);
    try {
      const response = await fetchUpstreamAccountActivity(requestedRange, {
        recentLimit: 4,
        signal: controller.signal,
      });
      if (
        requestSeq !== requestSeqRef.current ||
        rangeRef.current !== requestedRange ||
        !enabledRef.current
      ) {
        return;
      }
      setData(response);
      hasHydratedRef.current = true;
      setError(null);
    } catch (err) {
      if (isAbortError(err)) return;
      if (requestSeq !== requestSeqRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestSeq === requestSeqRef.current) {
        abortControllerRef.current = null;
        inFlightRef.current = false;
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading) {
        setIsLoading(false);
      }
      const pendingLoad = pendingLoadRef.current;
      if (requestSeq === requestSeqRef.current && pendingLoad) {
        pendingLoadRef.current = null;
        void runLoad(pendingLoad);
      }
    }
  }, []);

  const load = useCallback(
    async (options: LoadOptions = {}) => {
      if (!enabledRef.current) return;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = {
          silent: pendingSilent && (options.silent ?? false),
        };
        return;
      }
      await runLoad(options);
    },
    [runLoad],
  );

  useEffect(() => {
    if (!enabled) {
      clearPendingRefreshTimer();
      invalidateCurrentRequest();
      return;
    }
    hasActivatedRef.current = true;
    void load({ silent: hasHydratedRef.current });
  }, [clearPendingRefreshTimer, enabled, invalidateCurrentRequest, load, range]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const now = Date.now();
      const delay = Math.max(
        0,
        DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_REFRESH_THROTTLE_MS -
          (now - lastRefreshAtRef.current),
      );
      const run = () => {
        refreshTimerRef.current = null;
        lastRefreshAtRef.current = Date.now();
        void load({ silent: true });
      };
      if (delay === 0) {
        clearPendingRefreshTimer();
        run();
        return;
      }
      if (refreshTimerRef.current) return;
      refreshTimerRef.current = setTimeout(run, delay);
    });
    return unsubscribe;
  }, [clearPendingRefreshTimer, enabled, load]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSseOpen(() => {
      if (!hasActivatedRef.current || !hasHydratedRef.current) return;
      const now = Date.now();
      if (
        now - lastOpenResyncAtRef.current <
        DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_OPEN_RESYNC_COOLDOWN_MS
      ) {
        return;
      }
      lastOpenResyncAtRef.current = now;
      void load({ silent: true });
    });
    return unsubscribe;
  }, [enabled, load]);

  return {
    data,
    isLoading,
    error,
    hasActivated: hasActivatedRef.current,
    reload: load,
  };
}
