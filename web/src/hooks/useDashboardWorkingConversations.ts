import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import {
  fetchPromptCacheConversationsPage,
  type ApiInvocation,
  type PromptCacheConversation,
  type PromptCacheConversationsResponse,
} from "../lib/api";
import {
  DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  DASHBOARD_WORKING_CONVERSATIONS_SELECTION,
  mapPromptCacheConversationsToDashboardCards,
} from "../lib/dashboardWorkingConversations";
import { buildPromptCachePreviewFromInvocation } from "../lib/promptCacheLive";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";
import { publishWorkingConversationPatchMetrics } from "../lib/dashboardPerformanceDiagnostics";

const DASHBOARD_WORKING_CONVERSATIONS_REFRESH_THROTTLE_MS = 1_500;
const DASHBOARD_WORKING_CONVERSATIONS_POLL_INTERVAL_MS = 15_000;
const DASHBOARD_WORKING_CONVERSATIONS_OPEN_RESYNC_COOLDOWN_MS = 3_000;
const DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_LIMIT = 2;
const WORKING_SET_WINDOW_MS = 5 * 60 * 1_000;

interface FreshSnapshotKeysState {
  snapshotAt: string | null;
  keys: Set<string>;
}

interface PatchConversationWithRecordResult {
  conversation: PromptCacheConversation;
  didPatchVisible: boolean;
  shouldRefreshHead: boolean;
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isNaN(epoch) ? null : epoch;
}

function isRecordWithinSnapshotBoundary(
  record: Pick<ApiInvocation, "createdAt" | "occurredAt">,
  snapshotAt: string | null | undefined,
) {
  const snapshotAtEpoch = parseEpoch(snapshotAt);
  const recordCreatedAtEpoch = parseEpoch(record.createdAt ?? record.occurredAt);
  if (snapshotAtEpoch == null || recordCreatedAtEpoch == null) {
    return false;
  }
  return recordCreatedAtEpoch <= snapshotAtEpoch;
}

function isInFlightStatus(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase() ?? "";
  return normalized === "running" || normalized === "pending";
}

function resolveConversationSortAnchor(
  conversation: Pick<
    PromptCacheConversation,
    "lastTerminalAt" | "lastInFlightAt" | "lastActivityAt"
  >,
) {
  return Math.max(
    parseEpoch(conversation.lastTerminalAt) ?? Number.MIN_SAFE_INTEGER,
    parseEpoch(conversation.lastInFlightAt) ?? Number.MIN_SAFE_INTEGER,
    parseEpoch(conversation.lastActivityAt) ?? Number.MIN_SAFE_INTEGER,
  );
}

function compareConversationsByVisibleSetOrder(
  left: Pick<
    PromptCacheConversation,
    | "createdAt"
    | "promptCacheKey"
    | "lastTerminalAt"
    | "lastInFlightAt"
    | "lastActivityAt"
  >,
  right: Pick<
    PromptCacheConversation,
    | "createdAt"
    | "promptCacheKey"
    | "lastTerminalAt"
    | "lastInFlightAt"
    | "lastActivityAt"
  >,
) {
  const leftSortAnchor = resolveConversationSortAnchor(left);
  const rightSortAnchor = resolveConversationSortAnchor(right);
  if (leftSortAnchor !== rightSortAnchor) {
    return rightSortAnchor - leftSortAnchor;
  }

  const createdAtCompare = right.createdAt.localeCompare(left.createdAt);
  if (createdAtCompare !== 0) return createdAtCompare;
  return right.promptCacheKey.localeCompare(left.promptCacheKey);
}

function dedupeAndSortConversations(conversations: PromptCacheConversation[]) {
  const deduped = new Map<string, PromptCacheConversation>();
  for (const conversation of conversations) {
    if (!deduped.has(conversation.promptCacheKey)) {
      deduped.set(conversation.promptCacheKey, conversation);
    }
  }
  return Array.from(deduped.values()).sort(
    compareConversationsByVisibleSetOrder,
  );
}

function getPreviewStableKey(preview: {
  invokeId: string;
  occurredAt: string;
  id?: number;
}) {
  const invokeId = preview.invokeId.trim();
  if (invokeId) return invokeId;
  return `${preview.occurredAt}::${preview.id ?? ""}`;
}

function buildRecentInvocations(
  previews: PromptCacheConversation["recentInvocations"],
  record: ApiInvocation,
) {
  const nextPreview = buildPromptCachePreviewFromInvocation(record);
  const nextByKey = new Map<
    string,
    PromptCacheConversation["recentInvocations"][number]
  >();
  for (const preview of [nextPreview, ...previews]) {
    const key = getPreviewStableKey(preview);
    if (!nextByKey.has(key)) {
      nextByKey.set(key, preview);
    }
  }
  return Array.from(nextByKey.values())
    .sort((left, right) => {
      const occurredCompare = right.occurredAt.localeCompare(left.occurredAt);
      if (occurredCompare !== 0) return occurredCompare;
      return (
        (right.id ?? Number.MIN_SAFE_INTEGER) -
        (left.id ?? Number.MIN_SAFE_INTEGER)
      );
    })
    .slice(0, DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_LIMIT);
}

function resolveVisibleLastInFlightAt(
  previews: PromptCacheConversation["recentInvocations"],
) {
  return (
    previews
      .filter((item) => isInFlightStatus(item.status))
      .map((item) => item.occurredAt)
      .sort((left, right) => right.localeCompare(left))[0] ?? null
  );
}

function pruneExpiredWorkingConversations(
  conversations: PromptCacheConversation[],
  referenceMs: number,
) {
  return conversations.filter((conversation) => {
    const lastInFlightAtEpoch = parseEpoch(conversation.lastInFlightAt);
    if (lastInFlightAtEpoch != null) {
      return true;
    }
    const lastTerminalAtEpoch = parseEpoch(conversation.lastTerminalAt);
    if (lastTerminalAtEpoch != null) {
      return referenceMs - lastTerminalAtEpoch <= WORKING_SET_WINDOW_MS;
    }
    const lastActivityAtEpoch = parseEpoch(conversation.lastActivityAt);
    if (lastActivityAtEpoch == null) return true;
    return referenceMs - lastActivityAtEpoch <= WORKING_SET_WINDOW_MS;
  });
}

function resolveWorkingSetReferenceMs(
  snapshotAt: string | null | undefined,
  fallbackNowMs: number,
) {
  return parseEpoch(snapshotAt) ?? fallbackNowMs;
}

function publishPatchDiagnostics(
  patchedPostSnapshotInvocations: Map<
    string,
    Map<string, { totalTokens: number; cost: number }>
  >,
) {
  publishWorkingConversationPatchMetrics(patchedPostSnapshotInvocations);
}

function trackPatchedInvocation(
  patchedPostSnapshotInvocations: Map<
    string,
    {
      totalTokens: number;
      cost: number;
    }
  >,
  invokeId: string,
  value: {
    totalTokens: number;
    cost: number;
  },
) {
  patchedPostSnapshotInvocations.set(invokeId, value);
}

function getOrCreatePatchedConversationInvocations(
  patchedPostSnapshotInvocationsRef: MutableRefObject<
    Map<string, Map<string, { totalTokens: number; cost: number }>>
  >,
  promptCacheKey: string,
) {
  const existing =
    patchedPostSnapshotInvocationsRef.current.get(promptCacheKey) ?? null;
  if (existing) {
    return existing;
  }
  const created = new Map<string, { totalTokens: number; cost: number }>();
  patchedPostSnapshotInvocationsRef.current.set(promptCacheKey, created);
  return created;
}

function prunePatchedPostSnapshotInvocations(
  patchedPostSnapshotInvocations: Map<
    string,
    Map<string, { totalTokens: number; cost: number }>
  >,
  {
    retainedKeys,
    refreshedKeys,
  }: {
    retainedKeys: Set<string>;
    refreshedKeys?: Set<string>;
  },
) {
  patchedPostSnapshotInvocations.forEach((patchedInvocations, promptCacheKey) => {
    if (!retainedKeys.has(promptCacheKey)) {
      patchedPostSnapshotInvocations.delete(promptCacheKey);
      return;
    }
    if (refreshedKeys?.has(promptCacheKey) || patchedInvocations.size === 0) {
      patchedPostSnapshotInvocations.delete(promptCacheKey);
    }
  });
  publishPatchDiagnostics(patchedPostSnapshotInvocations);
}

function patchConversationWithRecord(
  conversation: PromptCacheConversation,
  record: ApiInvocation,
  snapshotAt: string | null | undefined,
  patchedPostSnapshotInvocations: Map<
    string,
    {
      totalTokens: number;
      cost: number;
    }
  >,
) {
  const promptCacheKey = record.promptCacheKey?.trim();
  if (!promptCacheKey || promptCacheKey !== conversation.promptCacheKey) {
    return {
      conversation,
      didPatchVisible: false,
      shouldRefreshHead: false,
    } satisfies PatchConversationWithRecordResult;
  }

  const preview = buildPromptCachePreviewFromInvocation(record);
  const previewStableKey = getPreviewStableKey(preview);
  const existingPreview = conversation.recentInvocations.find(
    (candidate) => getPreviewStableKey(candidate) === previewStableKey,
  );
  const nextRecentInvocations = buildRecentInvocations(
    conversation.recentInvocations,
    record,
  );
  const previewTotalTokens = Math.max(0, preview.totalTokens ?? 0);
  const existingTotalTokens = Math.max(0, existingPreview?.totalTokens ?? 0);
  const previewCost = typeof preview.cost === "number" ? preview.cost : 0;
  const existingCost =
    typeof existingPreview?.cost === "number" ? existingPreview.cost : 0;
  const existingPatched =
    patchedPostSnapshotInvocations.get(record.invokeId) ?? null;
  const isWithinSnapshotRecord = isRecordWithinSnapshotBoundary(record, snapshotAt);
  const isPostSnapshotRecord = snapshotAt != null && !isWithinSnapshotRecord;
  const isVisibleAfterPatch = nextRecentInvocations.some(
    (candidate) => getPreviewStableKey(candidate) === previewStableKey,
  );
  if (!existingPreview && isWithinSnapshotRecord && !isVisibleAfterPatch) {
    return {
      conversation,
      didPatchVisible: false,
      shouldRefreshHead: true,
    } satisfies PatchConversationWithRecordResult;
  }
  let requestCountDelta = 0;
  let totalTokensDelta = 0;
  let totalCostDelta = 0;
  if (existingPreview) {
    totalTokensDelta = previewTotalTokens - existingTotalTokens;
    totalCostDelta = previewCost - existingCost;
    if (isPostSnapshotRecord || existingPatched) {
      trackPatchedInvocation(patchedPostSnapshotInvocations, record.invokeId, {
        totalTokens: previewTotalTokens,
        cost: previewCost,
      });
    }
  } else if (existingPatched) {
    totalTokensDelta = previewTotalTokens - existingPatched.totalTokens;
    totalCostDelta = previewCost - existingPatched.cost;
    trackPatchedInvocation(patchedPostSnapshotInvocations, record.invokeId, {
      totalTokens: previewTotalTokens,
      cost: previewCost,
    });
  } else if (isPostSnapshotRecord) {
    requestCountDelta = 1;
    totalTokensDelta = previewTotalTokens;
    totalCostDelta = previewCost;
    trackPatchedInvocation(patchedPostSnapshotInvocations, record.invokeId, {
      totalTokens: previewTotalTokens,
      cost: previewCost,
    });
  }
  const nextVisibleLastInFlightAt = resolveVisibleLastInFlightAt(
    nextRecentInvocations,
  );
  const currentHiddenLastInFlightAt = conversation.lastInFlightAt ?? null;
  const normalizedRecordOccurredAt = record.occurredAt?.trim() ?? "";
  const shouldPreserveHiddenInFlightAnchor =
    nextVisibleLastInFlightAt == null &&
    currentHiddenLastInFlightAt != null &&
    (isInFlightStatus(record.status) ||
      normalizedRecordOccurredAt !== currentHiddenLastInFlightAt);
  const lastInFlightAt = shouldPreserveHiddenInFlightAnchor
    ? currentHiddenLastInFlightAt
    : (nextVisibleLastInFlightAt ?? null);

  const nextConversation = {
    ...conversation,
    requestCount: conversation.requestCount + requestCountDelta,
    totalTokens: conversation.totalTokens + totalTokensDelta,
    totalCost: conversation.totalCost + totalCostDelta,
    lastActivityAt:
      conversation.lastActivityAt.localeCompare(record.occurredAt) >= 0
        ? conversation.lastActivityAt
        : record.occurredAt,
    lastTerminalAt: isInFlightStatus(record.status)
      ? (conversation.lastTerminalAt ?? null)
      : conversation.lastTerminalAt &&
          conversation.lastTerminalAt.localeCompare(record.occurredAt) >= 0
        ? conversation.lastTerminalAt
        : record.occurredAt,
    lastInFlightAt,
    recentInvocations: nextRecentInvocations,
  } satisfies PromptCacheConversation;

  return {
    conversation: nextConversation,
    didPatchVisible: true,
    shouldRefreshHead: false,
  } satisfies PatchConversationWithRecordResult;
}

function mergeFreshSnapshotKeysState(
  current: FreshSnapshotKeysState,
  snapshotAt: string | null | undefined,
  conversations: PromptCacheConversation[],
): FreshSnapshotKeysState {
  const normalizedSnapshotAt = snapshotAt ?? null;
  const nextKeys =
    current.snapshotAt === normalizedSnapshotAt
      ? new Set(current.keys)
      : new Set<string>();
  for (const conversation of conversations) {
    nextKeys.add(conversation.promptCacheKey);
  }
  return {
    snapshotAt: normalizedSnapshotAt,
    keys: nextKeys,
  };
}

function resolveDesiredFreshSnapshotCount(
  response: PromptCacheConversationsResponse,
  loadedFreshSnapshotCount: number,
  refreshTargetCount: number,
) {
  const totalMatched = response.totalMatched ?? response.conversations.length;
  return Math.min(
    totalMatched,
    Math.max(refreshTargetCount, loadedFreshSnapshotCount),
  );
}

function shouldBackfillFreshSnapshot(
  response: PromptCacheConversationsResponse,
  freshSnapshotKeys: FreshSnapshotKeysState,
  refreshTargetCount: number,
) {
  const normalizedSnapshotAt = response.snapshotAt ?? null;
  if (freshSnapshotKeys.snapshotAt !== normalizedSnapshotAt) return false;
  if (!response.hasMore || !response.nextCursor) return false;
  return (
    freshSnapshotKeys.keys.size <
    resolveDesiredFreshSnapshotCount(
      response,
      freshSnapshotKeys.keys.size,
      refreshTargetCount,
    )
  );
}

function reconcileFreshSnapshotRows(
  response: PromptCacheConversationsResponse,
  freshSnapshotKeys: FreshSnapshotKeysState,
  refreshTargetCount: number,
  nowMs: number,
) {
  const normalizedSnapshotAt = response.snapshotAt ?? null;
  if (freshSnapshotKeys.snapshotAt !== normalizedSnapshotAt) {
    return response;
  }

  const shouldKeepRetainedTail =
    response.hasMore &&
    freshSnapshotKeys.keys.size <
      resolveDesiredFreshSnapshotCount(
        response,
        freshSnapshotKeys.keys.size,
        refreshTargetCount,
      );
  if (shouldKeepRetainedTail) {
    return response;
  }

  const referenceMs = resolveWorkingSetReferenceMs(response.snapshotAt, nowMs);
  const conversations = pruneExpiredWorkingConversations(
    dedupeAndSortConversations(
      response.conversations.filter((conversation) =>
        freshSnapshotKeys.keys.has(conversation.promptCacheKey),
      ),
    ),
    referenceMs,
  );

  return {
    ...response,
    conversations,
    nextCursor: response.hasMore
      ? (response.nextCursor ??
        conversations[conversations.length - 1]?.cursor ??
        null)
      : null,
  } satisfies PromptCacheConversationsResponse;
}

function mergeHeadPage(
  current: PromptCacheConversationsResponse | null,
  incoming: PromptCacheConversationsResponse,
  nowMs: number,
) {
  const totalMatched =
    incoming.totalMatched ??
    current?.totalMatched ??
    incoming.conversations.length;
  const snapshotChanged =
    current?.snapshotAt != null &&
    incoming.snapshotAt != null &&
    current.snapshotAt !== incoming.snapshotAt;
  const incomingKeys = new Set(
    incoming.conversations.map((conversation) => conversation.promptCacheKey),
  );
  const retainedCapacity = Math.max(
    0,
    totalMatched - incoming.conversations.length,
  );
  const retainedTail = (current?.conversations ?? [])
    .filter((conversation) => !incomingKeys.has(conversation.promptCacheKey))
    .slice(0, retainedCapacity);
  const referenceMs = resolveWorkingSetReferenceMs(incoming.snapshotAt, nowMs);
  const conversations = pruneExpiredWorkingConversations(
    dedupeAndSortConversations([...incoming.conversations, ...retainedTail]),
    referenceMs,
  );
  const serverHasMore = incoming.hasMore ?? incoming.nextCursor != null;
  const hasMore = snapshotChanged
    ? serverHasMore || conversations.length < totalMatched
    : serverHasMore && conversations.length < totalMatched;
  return {
    response: {
      ...incoming,
      totalMatched,
      hasMore,
      nextCursor: hasMore
        ? (incoming.nextCursor ??
          (snapshotChanged ? null : (current?.nextCursor ?? null)))
        : null,
      conversations,
    } satisfies PromptCacheConversationsResponse,
    snapshotChanged,
  };
}

function appendPage(
  current: PromptCacheConversationsResponse | null,
  incoming: PromptCacheConversationsResponse,
  nowMs: number,
) {
  if (!current) {
    return mergeHeadPage(current, incoming, nowMs).response;
  }
  const referenceMs = resolveWorkingSetReferenceMs(
    incoming.snapshotAt ?? current.snapshotAt,
    nowMs,
  );
  const conversations = pruneExpiredWorkingConversations(
    dedupeAndSortConversations([
      ...incoming.conversations,
      ...current.conversations,
    ]),
    referenceMs,
  );
  const totalMatched =
    incoming.totalMatched ?? current.totalMatched ?? conversations.length;
  const hasMore = incoming.hasMore ?? incoming.nextCursor != null;
  return {
    ...current,
    rangeStart: incoming.rangeStart,
    rangeEnd: incoming.rangeEnd,
    snapshotAt: incoming.snapshotAt ?? current.snapshotAt ?? null,
    totalMatched,
    hasMore,
    nextCursor: hasMore ? (incoming.nextCursor ?? null) : null,
    conversations,
  } satisfies PromptCacheConversationsResponse;
}

export function useDashboardWorkingConversations() {
  const [response, setResponse] =
    useState<PromptCacheConversationsResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const responseRef = useRef<PromptCacheConversationsResponse | null>(null);
  const refreshTargetCountRef = useRef(
    DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  );
  const freshSnapshotKeysRef = useRef<FreshSnapshotKeysState>({
    snapshotAt: null,
    keys: new Set(),
  });
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const pendingHeadRefreshRef = useRef(false);
  const pendingLoadMoreRef = useRef(false);
  const requestSeqRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const patchedPostSnapshotInvocationsRef = useRef<
    Map<string, Map<string, { totalTokens: number; cost: number }>>
  >(new Map());
  const lastHeadRefreshAtRef = useRef(0);
  const lastOpenResyncAtRef = useRef(0);

  useEffect(() => {
    responseRef.current = response;
  }, [response]);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const runNextPendingAction = useCallback(() => {
    if (pendingHeadRefreshRef.current) {
      pendingHeadRefreshRef.current = false;
      return "head" as const;
    }
    if (pendingLoadMoreRef.current) {
      pendingLoadMoreRef.current = false;
      return "loadMore" as const;
    }
    return null;
  }, []);

  const runHeadLoad = useCallback(
    async (silent = false) => {
      inFlightRef.current = true;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      const controller = new AbortController();
      abortControllerRef.current = controller;
      const shouldShowLoading = !(silent && hasHydratedRef.current);
      if (shouldShowLoading) {
        setIsLoading(true);
      }
      try {
        const nextResponse = await fetchPromptCacheConversationsPage(
          DASHBOARD_WORKING_CONVERSATIONS_SELECTION,
          {
            pageSize: DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
            detail: "compact",
            signal: controller.signal,
          },
        );
        if (requestSeq !== requestSeqRef.current) return;
        const nowMs = Date.now();
        const { response: mergedResponse } = mergeHeadPage(
          responseRef.current,
          nextResponse,
          nowMs,
        );
        freshSnapshotKeysRef.current = mergeFreshSnapshotKeysState(
          freshSnapshotKeysRef.current,
          nextResponse.snapshotAt,
          nextResponse.conversations,
        );
        const merged = reconcileFreshSnapshotRows(
          mergedResponse,
          freshSnapshotKeysRef.current,
          refreshTargetCountRef.current,
          nowMs,
        );
        if (
          shouldBackfillFreshSnapshot(
            merged,
            freshSnapshotKeysRef.current,
            refreshTargetCountRef.current,
          )
        ) {
          pendingLoadMoreRef.current = true;
        }
        prunePatchedPostSnapshotInvocations(
          patchedPostSnapshotInvocationsRef.current,
          {
            retainedKeys: new Set(
              merged.conversations.map(
                (conversation) => conversation.promptCacheKey,
              ),
            ),
            refreshedKeys: new Set(
              nextResponse.conversations.map(
                (conversation) => conversation.promptCacheKey,
              ),
            ),
          },
        );
        responseRef.current = merged;
        setResponse(merged);
        hasHydratedRef.current = true;
        setError(null);
      } catch (err) {
        if (err instanceof Error && err.name === "AbortError") {
          return;
        }
        if (requestSeq !== requestSeqRef.current) return;
        setError(err instanceof Error ? err.message : String(err));
        hasHydratedRef.current = true;
      } finally {
        if (abortControllerRef.current === controller) {
          abortControllerRef.current = null;
        }
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
        if (requestSeq === requestSeqRef.current) {
          inFlightRef.current = false;
        }
        const nextAction = runNextPendingAction();
        if (nextAction === "head") {
          void runHeadLoad(true);
        } else if (nextAction === "loadMore") {
          void runLoadMore();
        }
      }
    },
    [runNextPendingAction],
  );

  const runLoadMore = useCallback(async () => {
    const current = responseRef.current;
    if (!current?.hasMore || !current.snapshotAt || !current.nextCursor) {
      pendingLoadMoreRef.current = false;
      return;
    }
    inFlightRef.current = true;
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    const controller = new AbortController();
    abortControllerRef.current = controller;
    setIsLoadingMore(true);
    try {
      const nextResponse = await fetchPromptCacheConversationsPage(
        DASHBOARD_WORKING_CONVERSATIONS_SELECTION,
        {
          pageSize: DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
          cursor: current.nextCursor,
          snapshotAt: current.snapshotAt,
          detail: "compact",
          signal: controller.signal,
        },
      );
      if (requestSeq !== requestSeqRef.current) return;
      const nowMs = Date.now();
      freshSnapshotKeysRef.current = mergeFreshSnapshotKeysState(
        freshSnapshotKeysRef.current,
        nextResponse.snapshotAt,
        nextResponse.conversations,
      );
      const mergedResponse = appendPage(
        responseRef.current,
        nextResponse,
        nowMs,
      );
      const merged = reconcileFreshSnapshotRows(
        mergedResponse,
        freshSnapshotKeysRef.current,
        refreshTargetCountRef.current,
        nowMs,
      );
      if (
        shouldBackfillFreshSnapshot(
          merged,
          freshSnapshotKeysRef.current,
          refreshTargetCountRef.current,
        )
      ) {
        pendingLoadMoreRef.current = true;
      }
      prunePatchedPostSnapshotInvocations(
        patchedPostSnapshotInvocationsRef.current,
        {
          retainedKeys: new Set(
            merged.conversations.map((conversation) => conversation.promptCacheKey),
          ),
          refreshedKeys: new Set(
            nextResponse.conversations.map(
              (conversation) => conversation.promptCacheKey,
            ),
          ),
        },
      );
      responseRef.current = merged;
      setResponse(merged);
      hasHydratedRef.current = true;
      setError(null);
    } catch (err) {
      if (err instanceof Error && err.name === "AbortError") {
        return;
      }
      if (requestSeq !== requestSeqRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (abortControllerRef.current === controller) {
        abortControllerRef.current = null;
      }
      if (requestSeq === requestSeqRef.current) {
        inFlightRef.current = false;
        setIsLoadingMore(false);
      }
      const nextAction = runNextPendingAction();
      if (nextAction === "head") {
        void runHeadLoad(true);
      } else if (nextAction === "loadMore") {
        void runLoadMore();
      }
    }
  }, [runNextPendingAction]);

  const refreshHead = useCallback(
    (silent = true) => {
      if (inFlightRef.current) {
        pendingHeadRefreshRef.current = true;
        return;
      }
      void runHeadLoad(silent);
    },
    [runHeadLoad],
  );

  const loadMore = useCallback(() => {
    const current = responseRef.current;
    if (!current?.hasMore || !current.snapshotAt || !current.nextCursor) return;
    if (inFlightRef.current) {
      pendingLoadMoreRef.current = true;
      return;
    }
    void runLoadMore();
  }, [runLoadMore]);

  const triggerThrottledHeadRefresh = useCallback(
    (force = false) => {
      if (
        typeof document !== "undefined" &&
        document.visibilityState !== "visible"
      ) {
        return;
      }
      const now = Date.now();
      const delay = force
        ? 0
        : Math.max(
            0,
            DASHBOARD_WORKING_CONVERSATIONS_REFRESH_THROTTLE_MS -
              (now - lastHeadRefreshAtRef.current),
          );
      const run = () => {
        refreshTimerRef.current = null;
        lastHeadRefreshAtRef.current = Date.now();
        refreshHead(true);
      };
      if (delay === 0) {
        clearPendingRefreshTimer();
        run();
        return;
      }
      if (refreshTimerRef.current) return;
      refreshTimerRef.current = setTimeout(run, delay);
    },
    [clearPendingRefreshTimer, refreshHead],
  );

  const refresh = useCallback(() => {
    clearPendingRefreshTimer();
    lastHeadRefreshAtRef.current = Date.now();
    refreshHead(false);
  }, [clearPendingRefreshTimer, refreshHead]);

  useEffect(() => {
    requestSeqRef.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    responseRef.current = null;
    refreshTargetCountRef.current = DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE;
    freshSnapshotKeysRef.current = {
      snapshotAt: null,
      keys: new Set(),
    };
    patchedPostSnapshotInvocationsRef.current.clear();
    publishPatchDiagnostics(patchedPostSnapshotInvocationsRef.current);
    hasHydratedRef.current = false;
    inFlightRef.current = false;
    pendingHeadRefreshRef.current = false;
    pendingLoadMoreRef.current = false;
    lastHeadRefreshAtRef.current = 0;
    lastOpenResyncAtRef.current = 0;
    clearPendingRefreshTimer();
    setResponse(null);
    setError(null);
    setIsLoadingMore(false);
    void runHeadLoad(false);
  }, [clearPendingRefreshTimer, runHeadLoad]);

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const current = responseRef.current;
      if (!current) return;
      let shouldRefreshHead = false;
      let didPatchLoaded = false;
      const loadedKeys = new Set(
        current.conversations.map(
          (conversation) => conversation.promptCacheKey,
        ),
      );
      const nextConversations = current.conversations.map((conversation) => {
        let nextConversation = conversation;
        for (const record of payload.records) {
          const promptCacheKey = record.promptCacheKey?.trim();
          if (!promptCacheKey) continue;
          if (!loadedKeys.has(promptCacheKey)) {
            shouldRefreshHead = true;
            continue;
          }
          if (promptCacheKey !== conversation.promptCacheKey) continue;
          const patchedPostSnapshotInvocations =
            getOrCreatePatchedConversationInvocations(
              patchedPostSnapshotInvocationsRef,
              conversation.promptCacheKey,
            );
          const patchResult = patchConversationWithRecord(
            nextConversation,
            record,
            current.snapshotAt,
            patchedPostSnapshotInvocations,
          );
          nextConversation = patchResult.conversation;
          if (patchResult.didPatchVisible) {
            didPatchLoaded = true;
          }
          if (patchResult.shouldRefreshHead) {
            shouldRefreshHead = true;
          }
        }
        return nextConversation;
      });
      if (didPatchLoaded) {
        const referenceMs = resolveWorkingSetReferenceMs(
          current.snapshotAt,
          Date.now(),
        );
        const patched = {
          ...current,
          conversations: pruneExpiredWorkingConversations(
            dedupeAndSortConversations(nextConversations),
            referenceMs,
          ),
        } satisfies PromptCacheConversationsResponse;
        patched.hasMore = current.hasMore ?? false;
        patched.nextCursor = patched.hasMore
          ? (current.nextCursor ?? null)
          : null;
        responseRef.current = patched;
        setResponse(patched);
        prunePatchedPostSnapshotInvocations(
          patchedPostSnapshotInvocationsRef.current,
          {
            retainedKeys: new Set(
              patched.conversations.map((conversation) => conversation.promptCacheKey),
            ),
          },
        );
      }
      if (shouldRefreshHead) {
        triggerThrottledHeadRefresh();
      }
    });
    return unsubscribe;
  }, [triggerThrottledHeadRefresh]);

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      const now = Date.now();
      if (
        now - lastOpenResyncAtRef.current <
        DASHBOARD_WORKING_CONVERSATIONS_OPEN_RESYNC_COOLDOWN_MS
      ) {
        return;
      }
      lastOpenResyncAtRef.current = now;
      triggerThrottledHeadRefresh(true);
    });
    return unsubscribe;
  }, [triggerThrottledHeadRefresh]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      triggerThrottledHeadRefresh(true);
    }, DASHBOARD_WORKING_CONVERSATIONS_POLL_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [triggerThrottledHeadRefresh]);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onVisibilityChange = () => {
      if (document.visibilityState !== "visible") return;
      triggerThrottledHeadRefresh(true);
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [triggerThrottledHeadRefresh]);

  useEffect(
    () => () => {
      requestSeqRef.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      refreshTargetCountRef.current = DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE;
      freshSnapshotKeysRef.current = {
        snapshotAt: null,
        keys: new Set(),
      };
      patchedPostSnapshotInvocationsRef.current.clear();
      publishPatchDiagnostics(patchedPostSnapshotInvocationsRef.current);
      pendingHeadRefreshRef.current = false;
      pendingLoadMoreRef.current = false;
      inFlightRef.current = false;
      clearPendingRefreshTimer();
    },
    [clearPendingRefreshTimer],
  );

  const cards = useMemo(
    () => mapPromptCacheConversationsToDashboardCards(response),
    [response],
  );
  const setRefreshTargetCount = useCallback(
    (count: number) => {
      if (!Number.isFinite(count)) return;
      const nextCount = Math.max(0, Math.trunc(count));
      const previousCount = refreshTargetCountRef.current;
      refreshTargetCountRef.current = nextCount;
      if (nextCount <= previousCount) {
        return;
      }
      const current = responseRef.current;
      if (
        !current ||
        !shouldBackfillFreshSnapshot(
          current,
          freshSnapshotKeysRef.current,
          nextCount,
        )
      ) {
        return;
      }
      if (inFlightRef.current) {
        pendingLoadMoreRef.current = true;
        return;
      }
      void runLoadMore();
    },
    [runLoadMore],
  );

  return {
    cards,
    stats: response,
    totalMatched: response?.totalMatched ?? cards.length,
    hasMore: response?.hasMore === true,
    isLoading,
    isLoadingMore,
    error,
    loadMore,
    refresh,
    setRefreshTargetCount,
  };
}
