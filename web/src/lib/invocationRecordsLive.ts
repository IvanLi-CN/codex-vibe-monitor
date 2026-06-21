import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
} from "./api";
import { mergeInvocationRecordCollections } from "./invocationLiveMerge";
import { resolveInvocationDisplayStatus } from "./invocationStatus";
import { invocationStableKey } from "./invocation";

function normalizeText(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase() ?? "";
  return normalized.length > 0 ? normalized : null;
}

function normalizeNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normalizeStatusFilter(value: string | null | undefined) {
  const normalized = normalizeText(value);
  return normalized;
}

function matchesFailedStatus(record: ApiInvocation) {
  const failureClass = normalizeText(record.failureClass);
  if (failureClass && failureClass !== "none") return true;

  const normalizedStatus = normalizeText(record.status) ?? "";
  if (normalizedStatus === "failed") return true;
  if (normalizedStatus === "http_429") return true;
  if (normalizedStatus.startsWith("http_4") || normalizedStatus.startsWith("http_5")) {
    return true;
  }

  return typeof record.errorMessage === "string" && record.errorMessage.trim().length > 0;
}

function resolveDisplayStatusForSort(record: ApiInvocation) {
  const displayStatus = resolveInvocationDisplayStatus(record);
  const normalized = displayStatus?.trim().toLowerCase() ?? "";
  return normalized.length > 0 ? normalized : null;
}

function resolveKeywordHaystack(record: ApiInvocation) {
  return [
    record.invokeId,
    record.model,
    record.proxyDisplayName,
    record.endpoint,
    record.failureKind,
    record.errorMessage,
    record.downstreamErrorMessage,
    record.promptCacheKey,
    record.requesterIp,
  ]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .join("\n")
    .toLowerCase();
}

function compareNullableNumber(left: number | null, right: number | null) {
  if (left == null && right == null) return 0;
  if (left == null) return -1;
  if (right == null) return 1;
  if (left === right) return 0;
  return left > right ? 1 : -1;
}

function compareNullableText(left: string | null, right: string | null) {
  if (left == null && right == null) return 0;
  if (left == null) return -1;
  if (right == null) return 1;
  return left.localeCompare(right);
}

export function matchesInvocationLiveFilters(
  record: ApiInvocation,
  filters?: Pick<
    InvocationRecordsQuery,
    | "from"
    | "to"
    | "status"
    | "model"
    | "endpoint"
    | "requestId"
    | "failureClass"
    | "failureKind"
    | "promptCacheKey"
    | "stickyKey"
    | "requesterIp"
    | "upstreamAccountId"
    | "keyword"
    | "minTotalTokens"
    | "maxTotalTokens"
    | "minTotalMs"
    | "maxTotalMs"
  >,
) {
  if (!filters) return true;

  const occurredAtMs = Date.parse(record.occurredAt);
  if (filters.from && Number.isFinite(occurredAtMs) && occurredAtMs < Date.parse(filters.from)) {
    return false;
  }
  if (filters.to && Number.isFinite(occurredAtMs) && occurredAtMs >= Date.parse(filters.to)) {
    return false;
  }

  if (filters.model && normalizeText(record.model) !== normalizeText(filters.model)) {
    return false;
  }

  const statusFilter = normalizeStatusFilter(filters.status);
  if (statusFilter) {
    if (statusFilter === "failed") {
      if (!matchesFailedStatus(record)) return false;
    } else if (resolveDisplayStatusForSort(record) !== statusFilter) {
      return false;
    }
  }

  if (filters.endpoint && normalizeText(record.endpoint) !== normalizeText(filters.endpoint)) {
    return false;
  }
  if (filters.requestId && normalizeText(record.invokeId) !== normalizeText(filters.requestId)) {
    return false;
  }
  if (
    filters.failureClass &&
    normalizeText(record.failureClass) !== normalizeText(filters.failureClass)
  ) {
    return false;
  }
  if (
    filters.failureKind &&
    normalizeText(record.failureKind) !== normalizeText(filters.failureKind)
  ) {
    return false;
  }
  if (
    filters.promptCacheKey &&
    normalizeText(record.promptCacheKey) !== normalizeText(filters.promptCacheKey)
  ) {
    return false;
  }
  if (filters.stickyKey && normalizeText(record.stickyKey) !== normalizeText(filters.stickyKey)) {
    return false;
  }
  if (
    filters.requesterIp &&
    normalizeText(record.requesterIp) !== normalizeText(filters.requesterIp)
  ) {
    return false;
  }
  if (
    typeof filters.upstreamAccountId === "number" &&
    normalizeNumber(record.upstreamAccountId) !== filters.upstreamAccountId
  ) {
    return false;
  }
  if (filters.keyword) {
    const keyword = normalizeText(filters.keyword);
    if (keyword && !resolveKeywordHaystack(record).includes(keyword)) {
      return false;
    }
  }

  const totalTokens = normalizeNumber(record.totalTokens);
  if (
    typeof filters.minTotalTokens === "number" &&
    (totalTokens == null || totalTokens < filters.minTotalTokens)
  ) {
    return false;
  }
  if (
    typeof filters.maxTotalTokens === "number" &&
    (totalTokens == null || totalTokens > filters.maxTotalTokens)
  ) {
    return false;
  }

  const totalMs = normalizeNumber(record.tTotalMs);
  if (typeof filters.minTotalMs === "number" && (totalMs == null || totalMs < filters.minTotalMs)) {
    return false;
  }
  if (typeof filters.maxTotalMs === "number" && (totalMs == null || totalMs > filters.maxTotalMs)) {
    return false;
  }

  return true;
}

export function compareInvocationRecordsForWindow(
  left: ApiInvocation,
  right: ApiInvocation,
  sortBy: InvocationSortBy,
  sortOrder: InvocationSortOrder,
): number {
  const direction = sortOrder === "asc" ? 1 : -1;
  let comparison = 0;

  switch (sortBy) {
    case "totalTokens":
      comparison = compareNullableNumber(
        normalizeNumber(left.totalTokens),
        normalizeNumber(right.totalTokens),
      );
      break;
    case "cost":
      comparison = compareNullableNumber(normalizeNumber(left.cost), normalizeNumber(right.cost));
      break;
    case "tTotalMs":
      comparison = compareNullableNumber(
        normalizeNumber(left.tTotalMs),
        normalizeNumber(right.tTotalMs),
      );
      break;
    case "tUpstreamTtfbMs":
      comparison = compareNullableNumber(
        normalizeNumber(left.tUpstreamTtfbMs),
        normalizeNumber(right.tUpstreamTtfbMs),
      );
      break;
    case "status":
      comparison = compareNullableText(
        resolveDisplayStatusForSort(left),
        resolveDisplayStatusForSort(right),
      );
      break;
    case "occurredAt":
    default: {
      const leftMs = Date.parse(left.occurredAt);
      const rightMs = Date.parse(right.occurredAt);
      comparison = compareNullableNumber(
        Number.isFinite(leftMs) ? leftMs : null,
        Number.isFinite(rightMs) ? rightMs : null,
      );
      break;
    }
  }

  if (comparison !== 0) {
    return comparison * direction;
  }

  if (sortBy === "occurredAt") {
    if (left.id !== right.id) {
      return (left.id - right.id) * direction;
    }
    return invocationStableKey(left).localeCompare(invocationStableKey(right)) * direction;
  }

  const occurredAtComparison: number = compareInvocationRecordsForWindow(
    left,
    right,
    "occurredAt",
    "desc",
  );
  if (occurredAtComparison !== 0) {
    return occurredAtComparison;
  }
  return right.id - left.id;
}

export function mergeInvocationWindowRecords(
  current: ApiInvocation[],
  incoming: ApiInvocation[],
  options: {
    filters?: Pick<
      InvocationRecordsQuery,
      | "from"
      | "to"
      | "status"
      | "model"
      | "endpoint"
      | "requestId"
      | "failureClass"
      | "failureKind"
      | "promptCacheKey"
      | "stickyKey"
      | "requesterIp"
      | "upstreamAccountId"
      | "keyword"
      | "minTotalTokens"
      | "maxTotalTokens"
      | "minTotalMs"
      | "maxTotalMs"
    >;
    sortBy: InvocationSortBy;
    sortOrder: InvocationSortOrder;
    limit: number;
  },
) {
  const matchedIncoming = incoming.filter((record) =>
    matchesInvocationLiveFilters(record, options.filters),
  );
  const merged = mergeInvocationRecordCollections(current, matchedIncoming);
  return [...merged]
    .sort((left, right) =>
      compareInvocationRecordsForWindow(left, right, options.sortBy, options.sortOrder),
    )
    .slice(0, options.limit);
}
