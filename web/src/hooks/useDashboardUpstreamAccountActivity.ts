import { useCallback, useEffect, useRef, useState } from "react";
import {
  type DashboardActivityLiveSnapshot,
  type DashboardActivityResponse,
  fetchDashboardActivity,
  fetchDashboardActivityRecent,
  type UpstreamAccountActivityAccount,
} from "../lib/api";
import { normalizeEffectiveRoutingRule } from "../lib/api/core-upstream";
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

interface RecentLoadRequest {
  summary: DashboardActivityResponse;
  recentLimit: number;
}

function buildLiveOnlyAccount(
  live: DashboardActivityLiveSnapshot["accounts"][number],
): UpstreamAccountActivityAccount {
  return {
    accountKey: live.accountKey,
    upstreamAccountId: live.upstreamAccountId,
    displayName: live.upstreamAccountId == null ? "unassigned" : `#${live.upstreamAccountId}`,
    isUnassigned: live.upstreamAccountId == null,
    requestCount: 0,
    successCount: 0,
    failureCount: 0,
    nonSuccessCount: 0,
    totalTokens: 0,
    successTokens: 0,
    nonSuccessTokens: 0,
    failureTokens: 0,
    failureCost: 0,
    totalCost: 0,
    usageBreakdown: {
      cacheWriteTokens: 0,
      cacheReadTokens: 0,
      outputTokens: 0,
      models: [],
    },
    inProgressInvocationCount: live.inProgressInvocationCount,
    inProgressPhaseCounts: live.inProgressPhaseCounts,
    retryInvocationCount: live.retryInvocationCount,
    effectiveRoutingRule: normalizeEffectiveRoutingRule({}),
    recentInvocations: [],
  };
}

export function mergeDashboardActivityLiveSnapshot(
  response: DashboardActivityResponse,
  live: DashboardActivityLiveSnapshot,
): DashboardActivityResponse {
  const liveAccounts = new Map(live.accounts.map((account) => [account.accountKey, account]));
  const accounts: UpstreamAccountActivityAccount[] | undefined = response.accounts?.map(
    (account) => {
      const liveAccount = liveAccounts.get(
        account.accountKey ??
          (account.upstreamAccountId == null
            ? "unassigned"
            : `upstream:${account.upstreamAccountId}`),
      );
      return {
        ...account,
        inProgressInvocationCount: liveAccount?.inProgressInvocationCount ?? 0,
        inProgressPhaseCounts: liveAccount?.inProgressPhaseCounts ?? {
          queued: 0,
          requesting: 0,
          responding: 0,
        },
        retryInvocationCount: liveAccount?.retryInvocationCount ?? 0,
      };
    },
  );
  if (accounts) {
    const existingAccountKeys = new Set(
      accounts.map(
        (account) =>
          account.accountKey ??
          (account.upstreamAccountId == null
            ? "unassigned"
            : `upstream:${account.upstreamAccountId}`),
      ),
    );
    for (const account of live.accounts) {
      if (!existingAccountKeys.has(account.accountKey)) {
        accounts.push(buildLiveOnlyAccount(account));
      }
    }
  }
  return {
    ...response,
    liveRevision: live.revision,
    summary: {
      ...response.summary,
      stats: {
        ...response.summary.stats,
        inProgressConversationCount: live.inProgressInvocationCount,
        inProgressRetryConversationCount: live.retryInvocationCount,
        inProgressPhaseCounts: live.inProgressPhaseCounts,
      },
    },
    accounts,
  };
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
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [recentLoading, setRecentLoading] = useState(false);
  const [recentError, setRecentError] = useState<string | null>(null);
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
  const recentAbortControllerRef = useRef<AbortController | null>(null);
  const pendingRecentLoadRef = useRef<RecentLoadRequest | null>(null);
  const loadRecentRef = useRef<((request: RecentLoadRequest) => Promise<void>) | null>(null);
  const latestSummaryRef = useRef<DashboardActivityResponse | null>(null);
  const latestLiveSnapshotRef = useRef<DashboardActivityLiveSnapshot | null>(null);

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
    recentAbortControllerRef.current?.abort();
    abortControllerRef.current = null;
    recentAbortControllerRef.current = null;
    pendingRecentLoadRef.current = null;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    setRecentLoading(false);
  }, []);

  const loadRecent = useCallback(async ({ summary, recentLimit }: RecentLoadRequest) => {
    if (!includeAccountsRef.current || !enabledRef.current) return;
    if (recentAbortControllerRef.current) {
      pendingRecentLoadRef.current = { summary, recentLimit };
      return;
    }
    const controller = new AbortController();
    recentAbortControllerRef.current = controller;
    const requestSeq = requestSeqRef.current;
    setRecentLoading(true);
    setRecentError(null);
    try {
      const recent = await fetchDashboardActivityRecent({
        rangeStart: summary.rangeStart,
        rangeEnd: summary.rangeEnd,
        snapshotId: summary.snapshotId,
        recentLimit,
        signal: controller.signal,
      });
      if (
        requestSeq !== requestSeqRef.current ||
        latestSummaryRef.current?.snapshotId !== recent.snapshotId ||
        latestSummaryRef.current?.rangeStart !== recent.rangeStart ||
        latestSummaryRef.current?.rangeEnd !== recent.rangeEnd
      ) {
        return;
      }
      const byAccount = new Map(
        recent.accounts.map((account) => [account.accountKey, account.recentInvocations]),
      );
      const latestSummary = latestSummaryRef.current;
      if (latestSummary?.snapshotId === recent.snapshotId) {
        latestSummaryRef.current = {
          ...latestSummary,
          accounts: latestSummary.accounts?.map((account) => ({
            ...account,
            recentInvocations: byAccount.get(account.accountKey ?? "") ?? [],
          })),
        };
      }
      setData((current) => {
        if (!current || current.snapshotId !== recent.snapshotId) return current;
        return {
          ...current,
          accounts: current.accounts?.map((account) => ({
            ...account,
            recentInvocations: byAccount.get(account.accountKey ?? "") ?? [],
          })),
        };
      });
      setVisibleRecentInvocationLimit(recentLimit);
    } catch (err) {
      if (isAbortError(err)) return;
      if (requestSeq === requestSeqRef.current) {
        setRecentError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (recentAbortControllerRef.current === controller) {
        recentAbortControllerRef.current = null;
        const pendingRequest = pendingRecentLoadRef.current;
        pendingRecentLoadRef.current = null;
        if (pendingRequest && includeAccountsRef.current && enabledRef.current) {
          void loadRecentRef.current?.(pendingRequest);
        } else {
          setRecentLoading(false);
        }
      }
    }
  }, []);
  loadRecentRef.current = loadRecent;

  const runLoad = useCallback(
    async ({ silent = false }: LoadOptions = {}) => {
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
      if (!shouldShowLoading) setIsRefreshing(true);
      try {
        const requestedIncludeAccounts = includeAccountsRef.current;
        const response = await fetchDashboardActivity(requestedRange, {
          recentLimit: requestedRecentLimit,
          includeAccounts: requestedIncludeAccounts,
          includeRecent: false,
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
        recentInvocationLimitRef.current = nextRecentInvocationLimit;
        if (!requestedIncludeAccounts) setVisibleRecentInvocationLimit(nextRecentInvocationLimit);
        const latestLiveSnapshot = latestLiveSnapshotRef.current;
        const nextData =
          requestedRange !== "yesterday" &&
          latestLiveSnapshot &&
          latestLiveSnapshot.revision > (response.liveRevision ?? 0)
            ? mergeDashboardActivityLiveSnapshot(response, latestLiveSnapshot)
            : response;
        const previousData = latestSummaryRef.current;
        const nextDataWithRetainedRecent =
          silent && requestedIncludeAccounts && previousData?.range === nextData.range
            ? {
                ...nextData,
                accounts: nextData.accounts?.map((account) => ({
                  ...account,
                  recentInvocations:
                    previousData.accounts?.find(
                      (previousAccount) => previousAccount.accountKey === account.accountKey,
                    )?.recentInvocations ?? account.recentInvocations,
                })),
              }
            : nextData;
        latestSummaryRef.current = nextDataWithRetainedRecent;
        setData(nextDataWithRetainedRecent);
        recordUpstreamAccountActivityRefresh();
        hasHydratedRef.current = true;
        setError(null);
        if (requestedIncludeAccounts) {
          void loadRecent({ summary: nextData, recentLimit: nextRecentInvocationLimit });
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
        if (requestSeq === requestSeqRef.current) setIsRefreshing(false);
        const pendingLoad = pendingLoadRef.current;
        if (requestSeq === requestSeqRef.current && pendingLoad) {
          pendingLoadRef.current = null;
          void runLoad(pendingLoad);
        }
      }
    },
    [loadRecent],
  );

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
      if (payload.type === "version") {
        latestLiveSnapshotRef.current = null;
        setData((currentData) => (currentData ? { ...currentData, liveRevision: 0 } : currentData));
        return;
      }
      if (payload.type === "dashboardActivityLive") {
        const current = latestLiveSnapshotRef.current;
        if (current && payload.snapshot.revision <= current.revision) return;
        latestLiveSnapshotRef.current = payload.snapshot;
        if (rangeRef.current === "yesterday") return;
        setData((currentData) => {
          if (
            !currentData ||
            currentData.range !== rangeRef.current ||
            payload.snapshot.revision <= (currentData.liveRevision ?? 0)
          ) {
            return currentData;
          }
          return mergeDashboardActivityLiveSnapshot(currentData, payload.snapshot);
        });
        return;
      }
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
    isRefreshing,
    recentLoading,
    recentError,
    error,
    recentInvocationLimit: visibleRecentInvocationLimit,
    hasActivated: hasActivatedRef.current,
    reload: load,
    retryRecent: () => {
      const summary = latestSummaryRef.current;
      if (summary) {
        void loadRecent({ summary, recentLimit: recentInvocationLimitRef.current });
      }
    },
  };
}
