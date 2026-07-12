import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
} from "./api";
import { invocationStableKey } from "./invocation";
import {
  mergeInvocationRecordCollections,
  mergeInvocationRecordDetails,
} from "./invocationLiveMerge";
import { resolveInvocationDisplayStatus } from "./invocationStatus";

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

function resolveStickyFilterValue(record: ApiInvocation) {
  return normalizeText(record.stickyKey) ?? normalizeText(record.promptCacheKey);
}

function matchesFailedStatus(record: ApiInvocation) {
  const failureClass = normalizeText(record.failureClass);
  if (failureClass === "none") return false;
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
  if (left == null) return 1;
  if (right == null) return -1;
  if (left === right) return 0;
  return left > right ? 1 : -1;
}

function compareNullableText(left: string | null, right: string | null) {
  if (left == null && right == null) return 0;
  if (left == null) return 1;
  if (right == null) return -1;
  return left.localeCompare(right);
}

function compareNullableWithDirection(
  comparison: number,
  leftIsNull: boolean,
  rightIsNull: boolean,
  direction: number,
) {
  if (leftIsNull || rightIsNull) {
    return comparison;
  }
  return comparison * direction;
}

function isInFlightStatus(record: ApiInvocation) {
  const status = normalizeText(record.status);
  return status === "running" || status === "pending";
}

function mergeIncomingWindowRecord(current: ApiInvocation | undefined, incoming: ApiInvocation) {
  if (!current) return incoming;

  const currentInFlight = isInFlightStatus(current);
  const incomingInFlight = isInFlightStatus(incoming);

  if (currentInFlight !== incomingInFlight) {
    return mergeInvocationRecordCollections([current], [incoming])[0] ?? current;
  }

  if (!incomingInFlight) {
    return mergeInvocationRecordDetails(incoming, current);
  }

  return mergeInvocationRecordCollections([current], [incoming])[0] ?? current;
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
  if (filters.stickyKey && resolveStickyFilterValue(record) !== normalizeText(filters.stickyKey)) {
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
      {
        const leftValue = normalizeNumber(left.totalTokens);
        const rightValue = normalizeNumber(right.totalTokens);
        comparison = compareNullableWithDirection(
          compareNullableNumber(leftValue, rightValue),
          leftValue == null,
          rightValue == null,
          direction,
        );
      }
      break;
    case "cost":
      {
        const leftValue = normalizeNumber(left.cost);
        const rightValue = normalizeNumber(right.cost);
        comparison = compareNullableWithDirection(
          compareNullableNumber(leftValue, rightValue),
          leftValue == null,
          rightValue == null,
          direction,
        );
      }
      break;
    case "tTotalMs":
      {
        const leftValue = normalizeNumber(left.tTotalMs);
        const rightValue = normalizeNumber(right.tTotalMs);
        comparison = compareNullableWithDirection(
          compareNullableNumber(leftValue, rightValue),
          leftValue == null,
          rightValue == null,
          direction,
        );
      }
      break;
    case "tUpstreamTtfbMs":
      {
        const leftValue = normalizeNumber(left.tUpstreamTtfbMs);
        const rightValue = normalizeNumber(right.tUpstreamTtfbMs);
        comparison = compareNullableWithDirection(
          compareNullableNumber(leftValue, rightValue),
          leftValue == null,
          rightValue == null,
          direction,
        );
      }
      break;
    case "status":
      {
        const leftValue = resolveDisplayStatusForSort(left);
        const rightValue = resolveDisplayStatusForSort(right);
        comparison = compareNullableWithDirection(
          compareNullableText(leftValue, rightValue),
          leftValue == null,
          rightValue == null,
          direction,
        );
      }
      break;
    default: {
      const leftMs = Date.parse(left.occurredAt);
      const rightMs = Date.parse(right.occurredAt);
      const leftValue = Number.isFinite(leftMs) ? leftMs : null;
      const rightValue = Number.isFinite(rightMs) ? rightMs : null;
      comparison = compareNullableWithDirection(
        compareNullableNumber(leftValue, rightValue),
        leftValue == null,
        rightValue == null,
        direction,
      );
      break;
    }
  }

  if (comparison !== 0) {
    return comparison;
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
  return sortOrder === "asc" ? left.id - right.id : right.id - left.id;
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
  const currentByKey = new Map(
    mergeInvocationRecordCollections(current).map((record) => [
      invocationStableKey(record),
      record,
    ]),
  );

  for (const record of incoming) {
    const key = invocationStableKey(record);
    currentByKey.set(key, mergeIncomingWindowRecord(currentByKey.get(key), record));
  }

  return Array.from(currentByKey.values())
    .filter((record) => matchesInvocationLiveFilters(record, options.filters))
    .sort((left, right) =>
      compareInvocationRecordsForWindow(left, right, options.sortBy, options.sortOrder),
    )
    .slice(0, options.limit);
}
