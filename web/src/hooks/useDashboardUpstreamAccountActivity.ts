import { useCallback, useEffect, useRef, useState } from "react";
import {
  type DashboardActivityResponse,
  fetchDashboardActivity,
  type UpstreamAccountActivityAccount,
} from "../lib/api";
import {
  recordUpstreamAccountActivityOpenResync,
  recordUpstreamAccountActivityRefresh,
} from "../lib/dashboardPerformanceDiagnostics";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";
import {
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
} from "./useDashboardWorkingConversations";

export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_REFRESH_THROTTLE_MS = 5_000;
export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_OPEN_RESYNC_COOLDOWN_MS = 5_000;

interface LoadOptions {
  silent?: boolean;
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function clampRecentInvocationLimit(value: number) {
  if (!Number.isFinite(value)) {
    return DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN;
  }
  return Math.min(
    DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
    Math.max(DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN, Math.trunc(value)),
  );
}

export function resolveUpstreamAccountRecentPreviewLimit(
  accounts: Pick<UpstreamAccountActivityAccount, "inProgressInvocationCount">[],
) {
  let maxInProgressInvocationCount = 0;
  for (const account of accounts) {
    maxInProgressInvocationCount = Math.max(
      maxInProgressInvocationCount,
      account.inProgressInvocationCount ?? 0,
    );
  }
  return clampRecentInvocationLimit(maxInProgressInvocationCount);
}

export function useDashboardUpstreamAccountActivity(
  range: string,
  enabled: boolean,
  recentInvocationLimit = DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
) {
  const snapshot = useDashboardActivitySnapshot(range, enabled, true, recentInvocationLimit);
  const data = snapshot.data
    ? {
        range: snapshot.data.range,
        rangeStart: snapshot.data.rangeStart,
        rangeEnd: snapshot.data.rangeEnd,
        accounts: snapshot.data.accounts ?? [],
      }
    : null;
  return {
    ...snapshot,
    data,
  };
}

export function useDashboardActivitySnapshot(
  range: string,
  enabled: boolean,
  includeAccounts: boolean,
  recentInvocationLimit = DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
) {
  const initialRecentInvocationLimit = clampRecentInvocationLimit(recentInvocationLimit);
  const [data, setData] = useState<DashboardActivityResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [visibleRecentInvocationLimit, setVisibleRecentInvocationLimit] = useState(
    initialRecentInvocationLimit,
  );
  const enabledRef = useRef(enabled);
  const includeAccountsRef = useRef(includeAccounts);
  const rangeRef = useRef(range);
  const recentInvocationLimitRef = useRef(initialRecentInvocationLimit);
  const previousRangeRef = useRef(range);
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
    includeAccountsRef.current = includeAccounts;
  }, [includeAccounts]);

  useEffect(() => {
    rangeRef.current = range;
  }, [range]);

  useEffect(() => {
    const nextSeedRecentInvocationLimit = clampRecentInvocationLimit(recentInvocationLimit);
    const rangeChanged = previousRangeRef.current !== range;
    previousRangeRef.current = range;
    if (!enabled) {
      recentInvocationLimitRef.current = nextSeedRecentInvocationLimit;
      setVisibleRecentInvocationLimit(nextSeedRecentInvocationLimit);
      return;
    }
    recentInvocationLimitRef.current = rangeChanged
      ? nextSeedRecentInvocationLimit
      : Math.max(recentInvocationLimitRef.current, nextSeedRecentInvocationLimit);
  }, [enabled, range, recentInvocationLimit]);

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
    const requestedRecentLimit = recentInvocationLimitRef.current;
    const controller = new AbortController();
    abortControllerRef.current = controller;
    const shouldShowLoading = !(silent && hasHydratedRef.current);
    if (shouldShowLoading) setIsLoading(true);
    try {
      const requestedIncludeAccounts = includeAccountsRef.current;
      const response = await fetchDashboardActivity(requestedRange, {
        recentLimit: requestedRecentLimit,
        includeAccounts: requestedIncludeAccounts,
        signal: controller.signal,
      });
      if (
        requestSeq !== requestSeqRef.current ||
        rangeRef.current !== requestedRange ||
        includeAccountsRef.current !== requestedIncludeAccounts ||
        recentInvocationLimitRef.current !== requestedRecentLimit ||
        !enabledRef.current
      ) {
        return;
      }
      const resolvedRecentInvocationLimit = requestedIncludeAccounts
        ? resolveUpstreamAccountRecentPreviewLimit(response.accounts ?? [])
        : requestedRecentLimit;
      const nextRecentInvocationLimit = Math.max(
        requestedRecentLimit,
        resolvedRecentInvocationLimit,
      );
      const needsExpandedReload =
        requestedIncludeAccounts && nextRecentInvocationLimit > requestedRecentLimit;
      recentInvocationLimitRef.current = nextRecentInvocationLimit;
      setVisibleRecentInvocationLimit(
        needsExpandedReload ? requestedRecentLimit : nextRecentInvocationLimit,
      );
      setData(response);
      recordUpstreamAccountActivityRefresh();
      hasHydratedRef.current = true;
      setError(null);
      if (needsExpandedReload) {
        pendingLoadRef.current = {
          silent: pendingLoadRef.current?.silent ?? true,
        };
      }
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
  }, [clearPendingRefreshTimer, enabled, invalidateCurrentRequest, load]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const now = Date.now();
      const delay = Math.max(
        0,
        DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_REFRESH_THROTTLE_MS - (now - lastRefreshAtRef.current),
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
      recordUpstreamAccountActivityOpenResync();
      void load({ silent: true });
    });
    return unsubscribe;
  }, [enabled, load]);

  return {
    data,
    isLoading,
    error,
    recentInvocationLimit: visibleRecentInvocationLimit,
    hasActivated: hasActivatedRef.current,
    reload: load,
  };
}
