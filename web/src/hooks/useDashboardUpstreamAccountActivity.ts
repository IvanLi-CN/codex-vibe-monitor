import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type DashboardActivityLiveSnapshot,
  type DashboardActivityResponse,
  fetchDashboardActivity,
  type UpstreamAccountActivityAccount,
} from "../lib/api";
import { normalizeEffectiveRoutingRule } from "../lib/api/core-upstream";
import { buildTopicDescriptor } from "../lib/sse";
import { getBrowserTimeZone } from "../lib/timeZone";
import {
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
} from "./useDashboardWorkingConversations";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_REFRESH_THROTTLE_MS = 5_000;
export const DASHBOARD_UPSTREAM_ACCOUNT_ACTIVITY_OPEN_RESYNC_COOLDOWN_MS = 5_000;

interface LoadOptions {
  silent?: boolean;
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
    uploadBytesPerSecond: live.uploadBytesPerSecond,
    downloadBytesPerSecond: live.downloadBytesPerSecond,
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
        uploadBytesPerSecond: liveAccount?.uploadBytesPerSecond ?? 0,
        downloadBytesPerSecond: liveAccount?.downloadBytesPerSecond ?? 0,
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
    networkLiveBucket: live.networkLiveBucket ?? response.networkLiveBucket,
    networkRealtimeRate: live.networkRealtimeRate ?? response.networkRealtimeRate,
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

function buildDashboardActivityTopic(
  range: string,
  includeAccounts: boolean,
  recentInvocationLimit: number,
  includeRecent: boolean,
) {
  return buildTopicDescriptor("dashboard.activity.current", {
    range,
    timeZone: getBrowserTimeZone(),
    recentLimit: clampRecentInvocationLimit(recentInvocationLimit),
    includeAccounts,
    includeRecent,
  });
}

function useHttpDashboardActivitySnapshot(
  range: string,
  enabled: boolean,
  includeAccounts: boolean,
  recentInvocationLimit: number,
  includeRecent: boolean,
) {
  const [data, setData] = useState<DashboardActivityResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);
  const dataRef = useRef<DashboardActivityResponse | null>(null);

  useEffect(() => {
    dataRef.current = data;
  }, [data]);

  const load = useCallback(
    async (options: LoadOptions = {}) => {
      if (!enabled) return;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      abortControllerRef.current?.abort();
      const controller = new AbortController();
      abortControllerRef.current = controller;
      const silent = options.silent ?? false;
      if (silent && dataRef.current != null) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }
      try {
        const response = await fetchDashboardActivity(range, {
          recentLimit: clampRecentInvocationLimit(recentInvocationLimit),
          timeZone: getBrowserTimeZone(),
          includeAccounts,
          includeRecent,
          signal: controller.signal,
        });
        if (requestSeq !== requestSeqRef.current) return;
        setData(response);
        setError(null);
      } catch (nextError) {
        if (nextError instanceof DOMException && nextError.name === "AbortError") {
          return;
        }
        if (requestSeq !== requestSeqRef.current) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        if (abortControllerRef.current === controller) {
          abortControllerRef.current = null;
        }
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false);
          setIsRefreshing(false);
        }
      }
    },
    [enabled, includeAccounts, includeRecent, range, recentInvocationLimit],
  );

  useEffect(() => {
    requestSeqRef.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    if (!enabled) {
      setData(null);
      setIsLoading(false);
      setIsRefreshing(false);
      setError(null);
      return;
    }
    void load();
    return () => {
      requestSeqRef.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
    };
  }, [enabled, load]);

  return {
    data,
    isLoading,
    isRefreshing,
    error,
    reload: () => void load({ silent: dataRef.current != null }),
  };
}

export function useDashboardUpstreamAccountActivity(
  range: string,
  enabled: boolean,
  recentInvocationLimit = DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
) {
  const snapshot = useDashboardActivitySnapshot(range, enabled, true, recentInvocationLimit, true);
  const data = snapshot.data
    ? {
        range: snapshot.data.range,
        rangeStart: snapshot.data.rangeStart,
        rangeEnd: snapshot.data.rangeEnd,
        networkLiveBucket: snapshot.data.networkLiveBucket,
        networkRealtimeRate: snapshot.data.networkRealtimeRate,
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
  includeRecent = includeAccounts,
) {
  const useHttp = enabled && range === "yesterday";
  const topic = useMemo(
    () =>
      enabled && !useHttp
        ? buildDashboardActivityTopic(range, includeAccounts, recentInvocationLimit, includeRecent)
        : null,
    [enabled, includeAccounts, includeRecent, range, recentInvocationLimit, useHttp],
  );
  const sseState = useSubscriptionTopic<DashboardActivityResponse>(topic, topic != null);
  const httpState = useHttpDashboardActivitySnapshot(
    range,
    useHttp,
    includeAccounts,
    recentInvocationLimit,
    includeRecent,
  );

  const data = useHttp ? httpState.data : sseState.data;
  const isLoading = useHttp ? httpState.isLoading : sseState.isLoading;
  const isRefreshing = useHttp
    ? httpState.isRefreshing
    : Boolean(sseState.isLoading && sseState.data != null);
  const error = useHttp ? httpState.error : sseState.error;
  const refresh = useHttp ? httpState.reload : sseState.refresh;

  return {
    data,
    isLoading,
    isRefreshing,
    recentLoading: false,
    recentError: null as string | null,
    error,
    recentInvocationLimit: resolveUpstreamAccountRecentPreviewLimit(data?.accounts ?? []),
    reload: refresh,
    retryRecent: refresh,
  };
}
