import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationRequestPoint,
  PromptCacheConversationsResponse,
  PromptCacheConversationSelection,
  PromptCacheConversationUpstreamAccount,
} from "./api";
import { invocationStableKey } from "./invocation";
import {
  choosePreferredInvocationRecord,
  mergeInvocationRecordCollections,
} from "./invocationLiveMerge";

const PROMPT_CACHE_COUNT_MODE_WINDOW_HOURS = 24;
const PROMPT_CACHE_ACTIVITY_MODE_LIMIT = 50;
const PROMPT_CACHE_PREVIEW_LIMIT = 5;
const PROMPT_CACHE_LIVE_RECORDS_PER_KEY_LIMIT = 8;
const PROMPT_CACHE_UPSTREAM_ACCOUNT_LIMIT = 3;

export type PromptCacheConversationHistoryByKey = Record<
  string,
  Pick<PromptCacheConversation, "createdAt" | "lastActivityAt">
>;

type PromptCacheConversationPreviewExtras = Partial<
  Pick<
    ApiInvocation,
    | "source"
    | "inputTokens"
    | "outputTokens"
    | "cacheInputTokens"
    | "reasoningTokens"
    | "reasoningEffort"
    | "errorMessage"
    | "failureKind"
    | "isActionable"
    | "responseContentEncoding"
    | "requestedServiceTier"
    | "serviceTier"
    | "tReqReadMs"
    | "tReqParseMs"
    | "tUpstreamConnectMs"
    | "tUpstreamTtfbMs"
    | "tUpstreamStreamMs"
    | "tRespParseMs"
    | "tPersistMs"
    | "tTotalMs"
  >
>;

function normalizePromptCacheKey(value: string | null | undefined) {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
}

function parseOccurredAtEpoch(raw: string | null | undefined) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

function parsePromptCacheSelectionWindowMs(
  selection: PromptCacheConversationSelection,
) {
  if (selection.mode === "count") {
    return PROMPT_CACHE_COUNT_MODE_WINDOW_HOURS * 3_600_000;
  }
  if ("activityMinutes" in selection) {
    return selection.activityMinutes * 60_000;
  }
  return selection.activityHours * 3_600_000;
}

function promptCachePreviewIsPending(
  preview: Pick<PromptCacheConversationInvocationPreview, "status">,
) {
  const normalizedStatus = preview.status?.trim().toLowerCase() ?? "";
  return normalizedStatus === "running" || normalizedStatus === "pending";
}

function promptCachePreviewIsTerminal(
  preview: Pick<PromptCacheConversationInvocationPreview, "status">,
) {
  return !promptCachePreviewIsPending(preview);
}

function withinPromptCacheSelectionWindow(
  occurredAt: string,
  selection: PromptCacheConversationSelection,
  now: number,
) {
  const occurredAtEpoch = parseOccurredAtEpoch(occurredAt);
  if (occurredAtEpoch == null) return false;
  return occurredAtEpoch >= now - parsePromptCacheSelectionWindowMs(selection);
}

function recordMatchesPromptCacheSelection(
  record: Pick<ApiInvocation, "occurredAt" | "status">,
  selection: PromptCacheConversationSelection,
  now: number,
) {
  if (
    selection.mode === "activityWindow" &&
    "activityMinutes" in selection &&
    promptCacheInvocationIsPending(record as ApiInvocation)
  ) {
    return true;
  }
  return withinPromptCacheSelectionWindow(record.occurredAt, selection, now);
}

export function getPromptCacheConversationVisibleLimit(
  selection: PromptCacheConversationSelection,
) {
  return selection.mode === "count"
    ? selection.limit
    : PROMPT_CACHE_ACTIVITY_MODE_LIMIT;
}

function comparePromptCacheConversationOrder(
  left: Pick<
    PromptCacheConversation,
    "createdAt" | "promptCacheKey" | "recentInvocations"
  >,
  right: Pick<
    PromptCacheConversation,
    "createdAt" | "promptCacheKey" | "recentInvocations"
  >,
  selection: PromptCacheConversationSelection,
  now: number,
) {
  if (selection.mode === "activityWindow" && "activityMinutes" in selection) {
    const rangeStartMs = now - parsePromptCacheSelectionWindowMs(selection);
    const resolveSortEpoch = (
      conversation: Pick<
        PromptCacheConversation,
        "createdAt" | "recentInvocations"
      >,
    ) => {
      const recentTerminal = conversation.recentInvocations.find((preview) => {
        if (!promptCachePreviewIsTerminal(preview)) return false;
        const epoch = parseOccurredAtEpoch(preview.occurredAt);
        return epoch != null && epoch >= rangeStartMs;
      });
      if (recentTerminal) {
        return parseOccurredAtEpoch(recentTerminal.occurredAt);
      }
      const inFlight = conversation.recentInvocations.find((preview) =>
        promptCachePreviewIsPending(preview)
      );
      if (inFlight) {
        return parseOccurredAtEpoch(inFlight.occurredAt);
      }
      return null;
    };

    const leftAnchor = resolveSortEpoch(left);
    const rightAnchor = resolveSortEpoch(right);
    const normalizedLeftAnchor = leftAnchor ?? Number.MIN_SAFE_INTEGER;
    const normalizedRightAnchor = rightAnchor ?? Number.MIN_SAFE_INTEGER;
    if (normalizedLeftAnchor !== normalizedRightAnchor) {
      return normalizedRightAnchor - normalizedLeftAnchor;
    }
  }

  const leftEpoch = parseOccurredAtEpoch(left.createdAt) ?? Number.MIN_SAFE_INTEGER;
  const rightEpoch = parseOccurredAtEpoch(right.createdAt) ?? Number.MIN_SAFE_INTEGER;
  if (leftEpoch !== rightEpoch) return rightEpoch - leftEpoch;
  return right.promptCacheKey.localeCompare(left.promptCacheKey);
}

function normalizeFailureClass(
  value: ApiInvocation["failureClass"] | null | undefined,
) {
  const normalized = value?.trim().toLowerCase();
  return normalized === "none" ||
    normalized === "service_failure" ||
    normalized === "client_failure" ||
    normalized === "client_abort"
    ? normalized
    : null;
}

function isPromptCacheInvocationSuccessful(record: ApiInvocation) {
  const failureClass = normalizeFailureClass(record.failureClass);
  if (failureClass && failureClass !== "none") return false;
  const normalizedStatus = record.status?.trim().toLowerCase() ?? "";
  if (normalizedStatus === "running" || normalizedStatus === "pending") return false;
  if (normalizedStatus === "success" || normalizedStatus === "completed") return true;
  if (normalizedStatus.startsWith("http_4") || normalizedStatus.startsWith("http_5")) {
    return false;
  }
  return !record.errorMessage?.trim();
}

export function buildInvocationFromPromptCachePreview(
  preview: PromptCacheConversationInvocationPreview,
): ApiInvocation {
  const extras = preview as PromptCacheConversationInvocationPreview &
    PromptCacheConversationPreviewExtras;
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    occurredAt: preview.occurredAt,
    createdAt: preview.occurredAt,
    source: extras.source,
    status: preview.status,
    failureClass: preview.failureClass ?? undefined,
    failureKind: extras.failureKind,
    isActionable: extras.isActionable,
    routeMode: preview.routeMode ?? undefined,
    model: preview.model ?? undefined,
    inputTokens: extras.inputTokens,
    outputTokens: extras.outputTokens,
    cacheInputTokens: extras.cacheInputTokens,
    reasoningTokens: extras.reasoningTokens,
    reasoningEffort: extras.reasoningEffort,
    totalTokens: preview.totalTokens,
    cost: preview.cost ?? undefined,
    errorMessage: extras.errorMessage,
    endpoint: preview.endpoint ?? undefined,
    upstreamAccountId: preview.upstreamAccountId,
    upstreamAccountName: preview.upstreamAccountName ?? undefined,
    proxyDisplayName: preview.proxyDisplayName ?? undefined,
    responseContentEncoding: extras.responseContentEncoding,
    requestedServiceTier: extras.requestedServiceTier,
    serviceTier: extras.serviceTier,
    tReqReadMs: extras.tReqReadMs,
    tReqParseMs: extras.tReqParseMs,
    tUpstreamConnectMs: extras.tUpstreamConnectMs,
    tUpstreamTtfbMs: extras.tUpstreamTtfbMs,
    tUpstreamStreamMs: extras.tUpstreamStreamMs,
    tRespParseMs: extras.tRespParseMs,
    tPersistMs: extras.tPersistMs,
    tTotalMs: extras.tTotalMs,
  };
}

export function buildPromptCachePreviewFromInvocation(
  record: ApiInvocation,
): PromptCacheConversationInvocationPreview {
  return {
    id: record.id,
    invokeId: record.invokeId,
    occurredAt: record.occurredAt,
    status: record.status?.trim() || "unknown",
    failureClass: normalizeFailureClass(record.failureClass),
    routeMode: record.routeMode?.trim() || null,
    model: record.model?.trim() || null,
    totalTokens:
      typeof record.totalTokens === "number" && Number.isFinite(record.totalTokens)
        ? record.totalTokens
        : 0,
    cost:
      typeof record.cost === "number" && Number.isFinite(record.cost)
        ? record.cost
        : null,
    proxyDisplayName: record.proxyDisplayName?.trim() || null,
    upstreamAccountId:
      typeof record.upstreamAccountId === "number" &&
      Number.isFinite(record.upstreamAccountId)
        ? record.upstreamAccountId
        : null,
    upstreamAccountName: record.upstreamAccountName?.trim() || null,
    endpoint: record.endpoint?.trim() || null,
    source: record.source,
    inputTokens: record.inputTokens,
    outputTokens: record.outputTokens,
    cacheInputTokens: record.cacheInputTokens,
    reasoningTokens: record.reasoningTokens,
    reasoningEffort: record.reasoningEffort,
    errorMessage: record.errorMessage,
    failureKind: record.failureKind,
    isActionable: record.isActionable,
    responseContentEncoding: record.responseContentEncoding,
    requestedServiceTier: record.requestedServiceTier,
    serviceTier: record.serviceTier,
    tReqReadMs: record.tReqReadMs,
    tReqParseMs: record.tReqParseMs,
    tUpstreamConnectMs: record.tUpstreamConnectMs,
    tUpstreamTtfbMs: record.tUpstreamTtfbMs,
    tUpstreamStreamMs: record.tUpstreamStreamMs,
    tRespParseMs: record.tRespParseMs,
    tPersistMs: record.tPersistMs,
    tTotalMs: record.tTotalMs,
  };
}

function buildPromptCacheRequestPointFromInvocation(
  record: ApiInvocation,
): PromptCacheConversationRequestPoint {
  const requestTokens =
    typeof record.totalTokens === "number" && Number.isFinite(record.totalTokens)
      ? Math.max(0, record.totalTokens)
      : 0;
  return {
    occurredAt: record.occurredAt,
    status: record.status?.trim() || "unknown",
    isSuccess: isPromptCacheInvocationSuccessful(record),
    requestTokens,
    cumulativeTokens: 0,
  };
}

function mergePromptCacheRequestPoints(
  basePoints: PromptCacheConversationRequestPoint[],
  liveRecords: ApiInvocation[],
  authoritativePreviewRecords: ApiInvocation[] = [],
) {
  const authoritativePreviewKeys = new Set(
    authoritativePreviewRecords.map((record) => invocationStableKey(record)),
  );
  const combined = [
    ...basePoints.map((point, index) => ({
      ...point,
      _epoch: parseOccurredAtEpoch(point.occurredAt) ?? Number.MIN_SAFE_INTEGER,
      _order: index,
    })),
    ...liveRecords.flatMap((record, index) => {
      if (authoritativePreviewKeys.has(invocationStableKey(record))) {
        return [];
      }
      const point = buildPromptCacheRequestPointFromInvocation(record);
      return [{
        ...point,
        _epoch: parseOccurredAtEpoch(record.occurredAt) ?? Number.MIN_SAFE_INTEGER,
        _order: basePoints.length + index,
      }];
    }),
  ].sort((left, right) => left._epoch - right._epoch || left._order - right._order);

  let cumulativeTokens = 0;
  return combined.map((point) => {
    cumulativeTokens += point.requestTokens;
    return {
      occurredAt: point.occurredAt,
      status: point.status,
      isSuccess: point.isSuccess,
      requestTokens: point.requestTokens,
      cumulativeTokens,
    };
  });
}

function promptCacheInvocationIsPending(record: ApiInvocation) {
  const normalizedStatus = record.status?.trim().toLowerCase() ?? "";
  return normalizedStatus === "running" || normalizedStatus === "pending";
}

function authoritativePreviewLacksLiveExtras(
  authoritative: ApiInvocation,
  live: ApiInvocation,
) {
  const hasString = (value: string | null | undefined) =>
    typeof value === "string" && value.trim().length > 0;
  const hasNumber = (value: number | null | undefined) =>
    typeof value === "number" && Number.isFinite(value);

  if (!hasString(authoritative.source) && hasString(live.source)) return true;
  if (!hasNumber(authoritative.inputTokens) && hasNumber(live.inputTokens)) return true;
  if (!hasNumber(authoritative.outputTokens) && hasNumber(live.outputTokens)) return true;
  if (
    !hasNumber(authoritative.cacheInputTokens) &&
    hasNumber(live.cacheInputTokens)
  ) {
    return true;
  }
  if (
    !hasNumber(authoritative.reasoningTokens) &&
    hasNumber(live.reasoningTokens)
  ) {
    return true;
  }
  if (
    !hasString(authoritative.reasoningEffort) &&
    hasString(live.reasoningEffort)
  ) {
    return true;
  }
  if (!hasString(authoritative.errorMessage) && hasString(live.errorMessage)) {
    return true;
  }
  if (!hasString(authoritative.failureKind) && hasString(live.failureKind)) {
    return true;
  }
  if (authoritative.isActionable == null && typeof live.isActionable === "boolean") {
    return true;
  }
  if (
    !hasString(authoritative.responseContentEncoding) &&
    hasString(live.responseContentEncoding)
  ) {
    return true;
  }
  if (
    !hasString(authoritative.requestedServiceTier) &&
    hasString(live.requestedServiceTier)
  ) {
    return true;
  }
  if (!hasString(authoritative.serviceTier) && hasString(live.serviceTier)) {
    return true;
  }
  if (!hasNumber(authoritative.tReqReadMs) && hasNumber(live.tReqReadMs)) return true;
  if (!hasNumber(authoritative.tReqParseMs) && hasNumber(live.tReqParseMs)) return true;
  if (
    !hasNumber(authoritative.tUpstreamConnectMs) &&
    hasNumber(live.tUpstreamConnectMs)
  ) {
    return true;
  }
  if (!hasNumber(authoritative.tUpstreamTtfbMs) && hasNumber(live.tUpstreamTtfbMs)) {
    return true;
  }
  if (
    !hasNumber(authoritative.tUpstreamStreamMs) &&
    hasNumber(live.tUpstreamStreamMs)
  ) {
    return true;
  }
  if (!hasNumber(authoritative.tRespParseMs) && hasNumber(live.tRespParseMs)) {
    return true;
  }
  if (!hasNumber(authoritative.tPersistMs) && hasNumber(live.tPersistMs)) return true;
  if (!hasNumber(authoritative.tTotalMs) && hasNumber(live.tTotalMs)) return true;
  return false;
}

function buildOptimisticUpstreamAccounts(
  records: ApiInvocation[],
): PromptCacheConversationUpstreamAccount[] {
  const grouped = new Map<string, PromptCacheConversationUpstreamAccount>();

  for (const record of records) {
    const upstreamAccountId =
      typeof record.upstreamAccountId === "number" &&
      Number.isFinite(record.upstreamAccountId)
        ? record.upstreamAccountId
        : null;
    const upstreamAccountName = record.upstreamAccountName?.trim() || null;
    const groupKey = upstreamAccountId != null
      ? `id:${upstreamAccountId}`
      : upstreamAccountName != null
        ? `name:${upstreamAccountName}`
        : "unknown";
    const totalTokens =
      typeof record.totalTokens === "number" && Number.isFinite(record.totalTokens)
        ? Math.max(0, record.totalTokens)
        : 0;
    const totalCost =
      typeof record.cost === "number" && Number.isFinite(record.cost) ? record.cost : 0;

    const existing = grouped.get(groupKey);
    if (existing) {
      existing.requestCount += 1;
      existing.totalTokens += totalTokens;
      existing.totalCost += totalCost;
      if (record.occurredAt > existing.lastActivityAt) {
        existing.lastActivityAt = record.occurredAt;
      }
      continue;
    }

    grouped.set(groupKey, {
      upstreamAccountId,
      upstreamAccountName,
      requestCount: 1,
      totalTokens,
      totalCost,
      lastActivityAt: record.occurredAt,
    });
  }

  return Array.from(grouped.values())
    .sort((left, right) => {
      const lastActivityCompare =
        (parseOccurredAtEpoch(right.lastActivityAt) ?? Number.MIN_SAFE_INTEGER) -
        (parseOccurredAtEpoch(left.lastActivityAt) ?? Number.MIN_SAFE_INTEGER);
      if (lastActivityCompare !== 0) return lastActivityCompare;
      return (right.totalTokens ?? 0) - (left.totalTokens ?? 0);
    })
    .slice(0, PROMPT_CACHE_UPSTREAM_ACCOUNT_LIMIT);
}

function buildOptimisticConversation(
  promptCacheKey: string,
  liveRecords: ApiInvocation[],
  createdAtOverride?: string | null,
): PromptCacheConversation {
  const uniqueRecords = mergeInvocationRecordCollections(liveRecords);
  const previewRecords = uniqueRecords.slice(0, PROMPT_CACHE_PREVIEW_LIMIT);
  const derivedCreatedAt = uniqueRecords
    .map((record) => record.occurredAt)
    .reduce((earliest, occurredAt) =>
      earliest == null || occurredAt < earliest ? occurredAt : earliest,
    );
  const lastActivityAt = uniqueRecords
    .map((record) => record.occurredAt)
    .reduce((latest, occurredAt) =>
      latest == null || occurredAt > latest ? occurredAt : latest,
    );

  return {
    promptCacheKey,
    requestCount: uniqueRecords.length,
    totalTokens: uniqueRecords.reduce((sum, record) => {
      const totalTokens =
        typeof record.totalTokens === "number" && Number.isFinite(record.totalTokens)
          ? Math.max(0, record.totalTokens)
          : 0;
      return sum + totalTokens;
    }, 0),
    totalCost: uniqueRecords.reduce((sum, record) => {
      const cost =
        typeof record.cost === "number" && Number.isFinite(record.cost) ? record.cost : 0;
      return sum + cost;
    }, 0),
    createdAt:
      createdAtOverride?.trim() || derivedCreatedAt || new Date().toISOString(),
    lastActivityAt: lastActivityAt ?? new Date().toISOString(),
    upstreamAccounts: buildOptimisticUpstreamAccounts(uniqueRecords),
    recentInvocations: previewRecords.map(buildPromptCachePreviewFromInvocation),
    last24hRequests: mergePromptCacheRequestPoints([], uniqueRecords),
  };
}

export function mergePromptCacheConversationHistory(
  current: PromptCacheConversationHistoryByKey,
  stats: PromptCacheConversationsResponse | null,
) {
  if (!stats) return current;

  let changed = false;
  const next = { ...current };
  for (const conversation of stats.conversations) {
    const existing = current[conversation.promptCacheKey];
    if (
      existing?.createdAt === conversation.createdAt &&
      existing?.lastActivityAt === conversation.lastActivityAt
    ) {
      continue;
    }
    next[conversation.promptCacheKey] = {
      createdAt: conversation.createdAt,
      lastActivityAt: conversation.lastActivityAt,
    };
    changed = true;
  }

  return changed ? next : current;
}

export function mergePromptCacheLiveRecordMap(
  current: Record<string, ApiInvocation[]>,
  incoming: ApiInvocation[],
) {
  const next = { ...current };

  for (const record of incoming) {
    const promptCacheKey = normalizePromptCacheKey(record.promptCacheKey);
    if (!promptCacheKey) continue;
    next[promptCacheKey] = mergeInvocationRecordCollections(
      [record],
      next[promptCacheKey] ?? [],
    ).slice(0, PROMPT_CACHE_LIVE_RECORDS_PER_KEY_LIMIT);
  }

  return next;
}

export function reconcilePromptCacheLiveRecordMap(
  current: Record<string, ApiInvocation[]>,
  stats: PromptCacheConversationsResponse | null,
  options: {
    requestStartedAtMs?: number;
    liveRecordObservedAtByKey?: Record<string, number>;
  } = {},
) {
  if (!stats) return current;

  const fallsOutsideAuthoritativePreviewWindow = (
    record: ApiInvocation,
    previews: PromptCacheConversation["recentInvocations"],
  ) => {
    if (previews.length < PROMPT_CACHE_PREVIEW_LIMIT) return false;
    const previewTail = previews.at(-1);
    if (!previewTail) return false;
    const recordEpoch = parseOccurredAtEpoch(record.occurredAt);
    const previewTailEpoch = parseOccurredAtEpoch(previewTail.occurredAt);
    if (recordEpoch == null || previewTailEpoch == null) return false;
    if (recordEpoch !== previewTailEpoch) {
      return recordEpoch < previewTailEpoch;
    }
    const recordId =
      typeof record.id === "number" && Number.isFinite(record.id) ? record.id : null;
    const previewTailId =
      typeof previewTail.id === "number" && Number.isFinite(previewTail.id)
        ? previewTail.id
        : null;
    if (recordId == null || previewTailId == null) return false;
    return recordId <= previewTailId;
  };

  const next: Record<string, ApiInvocation[]> = {};
  for (const [promptCacheKey, records] of Object.entries(current)) {
    const conversation = stats.conversations.find(
      (item) => item.promptCacheKey === promptCacheKey,
    );
    if (!conversation) {
      const latestObservedAt = options.liveRecordObservedAtByKey?.[promptCacheKey];
      const responsePredatesLatestLiveRecord =
        typeof latestObservedAt === "number" &&
        Number.isFinite(latestObservedAt) &&
        typeof options.requestStartedAtMs === "number" &&
        Number.isFinite(options.requestStartedAtMs) &&
        options.requestStartedAtMs <= latestObservedAt;
      if (responsePredatesLatestLiveRecord) {
        next[promptCacheKey] = records;
        continue;
      }
      const pendingRecords = records.filter(promptCacheInvocationIsPending);
      if (pendingRecords.length > 0) {
        next[promptCacheKey] = pendingRecords;
      }
      continue;
    }
    const authoritativePreviewByKey = new Map<string, ApiInvocation>();
    for (const preview of conversation.recentInvocations) {
      authoritativePreviewByKey.set(
        invocationStableKey({
          invokeId: preview.invokeId,
          occurredAt: preview.occurredAt,
        }),
        buildInvocationFromPromptCachePreview(preview),
      );
    }
    const remaining = records.filter(
      (record) => {
        const authoritative = authoritativePreviewByKey.get(invocationStableKey(record));
        if (!authoritative) {
          if (promptCacheInvocationIsPending(record)) return true;
          return !fallsOutsideAuthoritativePreviewWindow(
            record,
            conversation.recentInvocations,
          );
        }
        if (authoritativePreviewLacksLiveExtras(authoritative, record)) return true;
        return (
          choosePreferredInvocationRecord(authoritative, record) === record
        );
      },
    );
    if (remaining.length > 0) {
      next[promptCacheKey] = remaining;
    }
  }
  return next;
}

export function mergePromptCacheConversationsResponse(
  base: PromptCacheConversationsResponse | null,
  liveRecordsByKey: Record<string, ApiInvocation[]>,
  selection: PromptCacheConversationSelection,
  now = Date.now(),
  knownConversationHistoryByKey: PromptCacheConversationHistoryByKey = {},
) {
  if (!base) return null;

  const nextConversations = base.conversations.map((conversation) => {
    const liveRecords = (liveRecordsByKey[conversation.promptCacheKey] ?? []).filter(
      (record) => recordMatchesPromptCacheSelection(record, selection, now),
    );
    if (liveRecords.length === 0) {
      return conversation;
    }

    const authoritativePreviewRecords = conversation.recentInvocations.map(
      buildInvocationFromPromptCachePreview,
    );
    const mergedPreviewRecords = mergeInvocationRecordCollections(
      liveRecords,
      authoritativePreviewRecords,
    ).slice(0, PROMPT_CACHE_PREVIEW_LIMIT);
    const latestActivityEpoch = Math.max(
      parseOccurredAtEpoch(conversation.lastActivityAt) ?? Number.MIN_SAFE_INTEGER,
      ...liveRecords.map(
        (record) => parseOccurredAtEpoch(record.occurredAt) ?? Number.MIN_SAFE_INTEGER,
      ),
    );

    return {
      ...conversation,
      lastActivityAt:
        latestActivityEpoch > Number.MIN_SAFE_INTEGER
          ? new Date(latestActivityEpoch).toISOString()
          : conversation.lastActivityAt,
      recentInvocations: mergedPreviewRecords.map(buildPromptCachePreviewFromInvocation),
      last24hRequests: mergePromptCacheRequestPoints(
        conversation.last24hRequests,
        liveRecords,
        authoritativePreviewRecords,
      ),
    };
  });

  const knownKeys = new Set(nextConversations.map((item) => item.promptCacheKey));
  const maxVisibleConversations = getPromptCacheConversationVisibleLimit(selection);
  for (const [promptCacheKey, records] of Object.entries(liveRecordsByKey)) {
    if (knownKeys.has(promptCacheKey)) continue;
    const filteredRecords = records.filter((record) =>
      recordMatchesPromptCacheSelection(record, selection, now),
    );
    if (filteredRecords.length === 0) continue;
    const knownConversation = knownConversationHistoryByKey[promptCacheKey];
    if (
      !knownConversation?.createdAt &&
      nextConversations.length >= maxVisibleConversations
    ) {
      continue;
    }
    nextConversations.push(
      buildOptimisticConversation(
        promptCacheKey,
        filteredRecords,
        knownConversation?.createdAt,
      ),
    );
  }

  nextConversations.sort((left, right) =>
    comparePromptCacheConversationOrder(left, right, selection, now),
  );

  return {
    ...base,
    conversations: nextConversations.slice(0, maxVisibleConversations),
  };
}
