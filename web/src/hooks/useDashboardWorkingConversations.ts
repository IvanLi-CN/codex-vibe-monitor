import { useCallback, useMemo, useState } from "react";
import type {
  BlockedBindingConstraintSource,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
} from "../lib/api";
import {
  DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  mapPromptCacheConversationsToDashboardCards,
} from "../lib/dashboardWorkingConversations";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const DASHBOARD_WORKING_CONVERSATIONS_VISIBLE_PATCH_BATCH_MS = 1_000;
export const DASHBOARD_WORKING_CONVERSATIONS_REFRESH_THROTTLE_MS = 5_000;
export const DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN = 4;
export const DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX = 16;

const WORKING_SET_WINDOW_MS = 5 * 60 * 1_000;

export interface DashboardWorkingConversationsBlockedBindingFilter {
  upstreamAccountId?: number | null;
  constraintSource?: BlockedBindingConstraintSource | null;
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isNaN(epoch) ? null : epoch;
}

function isInFlightStatus(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase() ?? "";
  return normalized === "running" || normalized === "pending";
}

function clampRecentPreviewLimit(value: number) {
  if (!Number.isFinite(value)) {
    return DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN;
  }
  return Math.min(
    DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
    Math.max(DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN, Math.trunc(value)),
  );
}

function resolveWorkingSetReferenceMs(
  snapshotAt: string | null | undefined,
  fallbackNowMs: number,
) {
  return parseEpoch(snapshotAt) ?? fallbackNowMs;
}

function normalizeRequestedPageSize(value: number) {
  if (!Number.isFinite(value)) {
    return DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE;
  }
  return Math.max(
    DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
    Math.ceil(Math.trunc(value) / DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE) *
      DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  );
}

function buildWorkingConversationsTopic(
  pageSize: number,
  recentInvocationLimit: number,
  blockedBindingFilter?: DashboardWorkingConversationsBlockedBindingFilter | null,
) {
  return buildTopicDescriptor("dashboard.working-conversations.current", {
    pageSize,
    recentInvocationLimit,
    blockedBindingUpstreamAccountId: blockedBindingFilter?.upstreamAccountId ?? null,
    blockedBindingConstraintSource: blockedBindingFilter?.constraintSource ?? null,
  });
}

export function resolveDashboardWorkingConversationsRecentPreviewLimit(
  conversations: Pick<PromptCacheConversation, "recentInvocations">[],
  referenceMs: number,
) {
  let maxRecentInFlightCount = 0;
  for (const conversation of conversations) {
    let recentInFlightCount = 0;
    for (const invocation of conversation.recentInvocations) {
      if (!isInFlightStatus(invocation.status)) continue;
      const occurredAtEpoch = parseEpoch(invocation.occurredAt);
      if (occurredAtEpoch == null) continue;
      const ageMs = referenceMs - occurredAtEpoch;
      if (ageMs < 0 || ageMs > WORKING_SET_WINDOW_MS) continue;
      recentInFlightCount += 1;
    }
    maxRecentInFlightCount = Math.max(maxRecentInFlightCount, recentInFlightCount);
  }
  return clampRecentPreviewLimit(maxRecentInFlightCount);
}

export function useDashboardWorkingConversations(
  blockedBindingFilter?: DashboardWorkingConversationsBlockedBindingFilter | null,
) {
  const [requestedPageSize, setRequestedPageSize] = useState(
    DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  );
  const topic = useMemo(
    () =>
      buildWorkingConversationsTopic(
        requestedPageSize,
        DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
        blockedBindingFilter,
      ),
    [blockedBindingFilter, requestedPageSize],
  );
  const { data, isLoading, error, refresh } =
    useSubscriptionTopic<PromptCacheConversationsResponse>(topic);
  const cards = useMemo(() => mapPromptCacheConversationsToDashboardCards(data), [data]);
  const hasMore = data?.hasMore === true || Boolean(data?.nextCursor);
  const recentPreviewLimit = useMemo(
    () =>
      resolveDashboardWorkingConversationsRecentPreviewLimit(
        data?.conversations ?? [],
        resolveWorkingSetReferenceMs(data?.snapshotAt, Date.now()),
      ),
    [data],
  );
  const isLoadingMore = Boolean(
    data && isLoading && hasMore && requestedPageSize > (data.conversations?.length ?? 0),
  );

  const loadMore = useCallback(() => {
    if (!hasMore) return;
    setRequestedPageSize((current) =>
      normalizeRequestedPageSize(current + DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE),
    );
  }, [hasMore]);

  const setRefreshTargetCount = useCallback((count: number) => {
    if (!Number.isFinite(count)) return;
    setRequestedPageSize((current) => Math.max(current, normalizeRequestedPageSize(count)));
  }, []);

  return {
    cards,
    stats: data,
    totalMatched: data?.totalMatched ?? cards.length,
    hasMore,
    isLoading,
    isLoadingMore,
    error,
    recentPreviewLimit,
    loadMore,
    setRefreshTargetCount,
    refresh,
  };
}
