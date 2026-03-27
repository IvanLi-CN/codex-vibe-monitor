import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  fetchPromptCacheConversations,
  type ApiInvocation,
  type PromptCacheConversationSelection,
  type PromptCacheConversationsResponse,
} from "../lib/api";
import {
  mergePromptCacheConversationsResponse,
  mergePromptCacheLiveRecordMap,
  reconcilePromptCacheLiveRecordMap,
} from "../lib/promptCacheLive";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

export const PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS = 5_000;
export const PROMPT_CACHE_POLLING_REFRESH_INTERVAL_MS = 60_000;
export const PROMPT_CACHE_OPEN_RESYNC_COOLDOWN_MS = 3_000;

export function getPromptCacheSseRefreshDelay(
  lastRefreshAt: number,
  now: number,
) {
  return Math.max(
    0,
    PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt),
  );
}

export function shouldTriggerPromptCacheOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= PROMPT_CACHE_OPEN_RESYNC_COOLDOWN_MS;
}

interface LoadOptions {
  silent?: boolean;
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function isSameSelection(
  left: PromptCacheConversationSelection,
  right: PromptCacheConversationSelection,
) {
  return (
    left.mode === right.mode &&
    (left.mode === "count"
      ? right.mode === "count" && left.limit === right.limit
      : right.mode === "activityWindow" &&
        left.activityHours === right.activityHours)
  );
}

export function usePromptCacheConversations(
  selection: PromptCacheConversationSelection,
) {
  const [authoritativeStats, setAuthoritativeStats] =
    useState<PromptCacheConversationsResponse | null>(null);
  const [liveRecordsByKey, setLiveRecordsByKey] = useState<
    Record<string, ApiInvocation[]>
  >({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const selectionRef = useRef(selection);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const pendingLoadRef = useRef<LoadOptions | null>(null);
  const pendingOpenResyncRef = useRef(false);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const lastOpenResyncAtRef = useRef(0);
  const requestSeqRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  useEffect(() => {
    selectionRef.current = selection;
  }, [selection]);

  const invalidateCurrentRequest = useCallback(() => {
    requestSeqRef.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    pendingOpenResyncRef.current = false;
    clearPendingRefreshTimer();
  }, [clearPendingRefreshTimer]);

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    inFlightRef.current = true;
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    const requestedSelection = selectionRef.current;
    const controller = new AbortController();
    abortControllerRef.current = controller;
    const shouldShowLoading = !(silent && hasHydratedRef.current);
    if (shouldShowLoading) setIsLoading(true);
    try {
      const response = await fetchPromptCacheConversations(
        requestedSelection,
        controller.signal,
      );
      if (requestSeq !== requestSeqRef.current) return;
      if (!isSameSelection(selectionRef.current, requestedSelection)) return;
      setAuthoritativeStats(response);
      setLiveRecordsByKey((current) =>
        reconcilePromptCacheLiveRecordMap(current, response),
      );
      hasHydratedRef.current = true;
      setError(null);
      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false;
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = { silent: pendingSilent };
      }
    } catch (err) {
      if (isAbortError(err)) return;
      if (requestSeq !== requestSeqRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestSeq === requestSeqRef.current) {
        abortControllerRef.current = null;
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading)
        setIsLoading(false);
      if (requestSeq === requestSeqRef.current) {
        inFlightRef.current = false;
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
      const silent = options.silent ?? false;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = { silent: pendingSilent && silent };
        return;
      }
      await runLoad({ silent });
    },
    [runLoad],
  );

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now();
    const delay = getPromptCacheSseRefreshDelay(lastRefreshAtRef.current, now);
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
  }, [clearPendingRefreshTimer, load]);

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true;
        return;
      }
      const now = Date.now();
      if (
        !shouldTriggerPromptCacheOpenResync(
          lastOpenResyncAtRef.current,
          now,
          force,
        )
      )
        return;
      lastOpenResyncAtRef.current = now;
      void load({ silent: true });
    },
    [load],
  );

  useEffect(() => {
    invalidateCurrentRequest();
    void load();
  }, [invalidateCurrentRequest, load, selection]);

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      setLiveRecordsByKey((current) =>
        mergePromptCacheLiveRecordMap(current, payload.records),
      );
      triggerSseRefresh();
    });
    return unsubscribe;
  }, [triggerSseRefresh]);

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync();
    });
    return unsubscribe;
  }, [triggerOpenResync]);

  useEffect(() => {
    const timer = setInterval(() => {
      void load({ silent: true });
    }, PROMPT_CACHE_POLLING_REFRESH_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [load]);

  useEffect(
    () => () => {
      abortControllerRef.current?.abort();
      clearPendingRefreshTimer();
      pendingLoadRef.current = null;
      pendingOpenResyncRef.current = false;
    },
    [clearPendingRefreshTimer],
  );

  const stats = useMemo(
    () =>
      mergePromptCacheConversationsResponse(
        authoritativeStats,
        liveRecordsByKey,
        selection,
      ),
    [authoritativeStats, liveRecordsByKey, selection],
  );

  return {
    stats,
    isLoading,
    error,
    refresh: load,
  };
}
