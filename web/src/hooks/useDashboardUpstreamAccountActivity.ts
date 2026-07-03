import { useCallback, useEffect, useRef, useState } from "react";
import {
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
} from "./useDashboardWorkingConversations";
import {
  type ApiInvocation,
  fetchUpstreamAccountActivity,
  type UpstreamAccountActivityAccount,
  type UpstreamAccountActivityResponse,
} from "../lib/api";
import {
  recordUpstreamAccountActivityOpenResync,
  recordUpstreamAccountActivityRefresh,
} from "../lib/dashboardPerformanceDiagnostics";
import {
  DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS,
  clearDashboardRecordPatchState,
  createDashboardRecordPatchState,
  patchUpstreamAccountActivityWithRecords,
  seedUpstreamAccountActivityPatchState,
} from "../lib/dashboardSseLocalPatch";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

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
    Math.max(
      DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
      Math.trunc(value),
    ),
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
  const initialRecentInvocationLimit = clampRecentInvocationLimit(
    recentInvocationLimit,
  );
  const [data, setData] = useState<UpstreamAccountActivityResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [visibleRecentInvocationLimit, setVisibleRecentInvocationLimit] =
    useState(initialRecentInvocationLimit);
  const enabledRef = useRef(enabled);
  const rangeRef = useRef(range);
  const recentInvocationLimitRef = useRef(initialRecentInvocationLimit);
  const previousRangeRef = useRef(range);
  const hasActivatedRef = useRef(false);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const requestSeqRef = useRef(0);
  const pendingLoadRef = useRef<LoadOptions | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const localPatchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingLocalPatchRecordsRef = useRef<ApiInvocation[]>([]);
  const localPatchStateRef = useRef(createDashboardRecordPatchState());
  const pendingPostHydrationRefreshRef = useRef(false);
  const lastRefreshAtRef = useRef(0);
  const lastOpenResyncAtRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    enabledRef.current = enabled;
  }, [enabled]);

  useEffect(() => {
    rangeRef.current = range;
  }, [range]);

  useEffect(() => {
    const nextSeedRecentInvocationLimit = clampRecentInvocationLimit(
      recentInvocationLimit,
    );
    const rangeChanged = previousRangeRef.current !== range;
    previousRangeRef.current = range;
    if (!enabled) {
      recentInvocationLimitRef.current = nextSeedRecentInvocationLimit;
      setVisibleRecentInvocationLimit(nextSeedRecentInvocationLimit);
      return;
    }
    recentInvocationLimitRef.current = rangeChanged
      ? nextSeedRecentInvocationLimit
      : Math.max(
          recentInvocationLimitRef.current,
          nextSeedRecentInvocationLimit,
        );
  }, [enabled, range, recentInvocationLimit]);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const clearLocalPatchTimer = useCallback(() => {
    if (!localPatchTimerRef.current) return;
    clearTimeout(localPatchTimerRef.current);
    localPatchTimerRef.current = null;
  }, []);

  const invalidateCurrentRequest = useCallback(() => {
    requestSeqRef.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    pendingLocalPatchRecordsRef.current = [];
    pendingPostHydrationRefreshRef.current = false;
    clearDashboardRecordPatchState(localPatchStateRef.current);
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
      const response = await fetchUpstreamAccountActivity(requestedRange, {
        recentLimit: requestedRecentLimit,
        signal: controller.signal,
      });
      if (
        requestSeq !== requestSeqRef.current ||
        rangeRef.current !== requestedRange ||
        recentInvocationLimitRef.current !== requestedRecentLimit ||
        !enabledRef.current
      ) {
        return;
      }
      const resolvedRecentInvocationLimit =
        resolveUpstreamAccountRecentPreviewLimit(response.accounts);
      const nextRecentInvocationLimit = Math.max(
        requestedRecentLimit,
        resolvedRecentInvocationLimit,
      );
      const needsExpandedReload =
        nextRecentInvocationLimit > requestedRecentLimit;
      recentInvocationLimitRef.current = nextRecentInvocationLimit;
      pendingLocalPatchRecordsRef.current = [];
      clearLocalPatchTimer();
      seedUpstreamAccountActivityPatchState(localPatchStateRef.current, response);
      setVisibleRecentInvocationLimit(
        needsExpandedReload
          ? requestedRecentLimit
          : nextRecentInvocationLimit,
      );
      setData(response);
      recordUpstreamAccountActivityRefresh();
      lastRefreshAtRef.current = Date.now();
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
  }, [clearLocalPatchTimer]);

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

  const scheduleThrottledRefresh = useCallback(() => {
    if (!hasHydratedRef.current) {
      pendingPostHydrationRefreshRef.current = true;
      return;
    }
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
  }, [clearPendingRefreshTimer, load]);

  useEffect(() => {
    if (
      !enabled ||
      !hasHydratedRef.current ||
      !pendingPostHydrationRefreshRef.current
    ) {
      return;
    }
    pendingPostHydrationRefreshRef.current = false;
    scheduleThrottledRefresh();
  }, [data, enabled, scheduleThrottledRefresh]);

  const flushPendingLocalPatches = useCallback(() => {
    localPatchTimerRef.current = null;
    const records = pendingLocalPatchRecordsRef.current;
    pendingLocalPatchRecordsRef.current = [];
    if (records.length === 0) return;

    let missedAccountRecord = false;
    setData((current) => {
      const patched = patchUpstreamAccountActivityWithRecords(
        current,
        records,
        recentInvocationLimitRef.current,
        localPatchStateRef.current,
      );
      missedAccountRecord = patched.missedAccountRecord;
      return patched.response;
    });
    if (missedAccountRecord) {
      scheduleThrottledRefresh();
    }
  }, [scheduleThrottledRefresh]);

  const enqueueLocalPatchRecords = useCallback((records: ApiInvocation[]) => {
    if (records.length === 0 || rangeRef.current !== "today") return;
    pendingLocalPatchRecordsRef.current.push(...records);
    if (localPatchTimerRef.current) return;
    localPatchTimerRef.current = setTimeout(
      flushPendingLocalPatches,
      DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS,
    );
  }, [flushPendingLocalPatches]);

  useEffect(() => {
    if (!enabled) {
      clearPendingRefreshTimer();
      clearLocalPatchTimer();
      invalidateCurrentRequest();
      return;
    }
    hasActivatedRef.current = true;
    void load({ silent: hasHydratedRef.current });
  }, [
    clearPendingRefreshTimer,
    clearLocalPatchTimer,
    enabled,
    invalidateCurrentRequest,
    load,
    range,
    recentInvocationLimit,
  ]);

  useEffect(
    () => () => {
      clearPendingRefreshTimer();
      clearLocalPatchTimer();
      pendingLocalPatchRecordsRef.current = [];
      pendingPostHydrationRefreshRef.current = false;
    },
    [clearLocalPatchTimer, clearPendingRefreshTimer],
  );

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      enqueueLocalPatchRecords(payload.records);
      scheduleThrottledRefresh();
    });
    return unsubscribe;
  }, [enabled, enqueueLocalPatchRecords, scheduleThrottledRefresh]);

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
