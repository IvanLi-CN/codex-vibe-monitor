import type { ApiInvocation } from "./api";
import { invocationStableKey } from "./invocation";

function normalizeStatus(value: string | null | undefined) {
  return value?.trim().toLowerCase() ?? "";
}

function comparableNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function recordLifecycleRank(record: ApiInvocation) {
  const status = normalizeStatus(record.status);
  if (status === "running" || status === "pending") return 1;
  return 2;
}

function recordCompletenessScore(record: ApiInvocation) {
  let score = 0;
  if (record.model?.trim()) score += 1;
  if (record.proxyDisplayName?.trim()) score += 1;
  if (record.endpoint?.trim()) score += 1;
  if (record.promptCacheKey?.trim()) score += 1;
  if (record.requesterIp?.trim()) score += 1;
  if (record.upstreamAccountName?.trim()) score += 1;
  if (
    typeof record.upstreamAccountId === "number" &&
    Number.isFinite(record.upstreamAccountId)
  ) {
    score += 1;
  }
  if (record.responseContentEncoding?.trim()) score += 1;
  if (
    typeof record.tUpstreamConnectMs === "number" &&
    Number.isFinite(record.tUpstreamConnectMs) &&
    record.tUpstreamConnectMs > 0
  ) {
    score += 1;
  }
  if (
    typeof record.tUpstreamTtfbMs === "number" &&
    Number.isFinite(record.tUpstreamTtfbMs) &&
    record.tUpstreamTtfbMs > 0
  ) {
    score += 2;
  }
  if (
    typeof record.tTotalMs === "number" &&
    Number.isFinite(record.tTotalMs) &&
    record.tTotalMs > 0
  ) {
    score += 3;
  }
  if (typeof record.totalTokens === "number" && Number.isFinite(record.totalTokens)) {
    score += 2;
  }
  if (typeof record.cost === "number" && Number.isFinite(record.cost)) {
    score += 2;
  }
  if (record.upstreamRequestId?.trim()) score += 2;
  if (record.failureKind?.trim()) score += 2;
  if (record.poolAttemptTerminalReason?.trim()) score += 2;
  if (record.upstreamErrorCode?.trim()) score += 1;
  if (record.upstreamErrorMessage?.trim()) score += 1;
  if (record.errorMessage?.trim()) score += 2;
  return score;
}

function compareRecordRuntimeProgress(current: ApiInvocation, next: ApiInvocation) {
  const fields: Array<[number | null, number | null]> = [
    [
      comparableNumber(current.poolAttemptCount),
      comparableNumber(next.poolAttemptCount),
    ],
    [
      comparableNumber(current.poolDistinctAccountCount),
      comparableNumber(next.poolDistinctAccountCount),
    ],
    [comparableNumber(current.tUpstreamTtfbMs), comparableNumber(next.tUpstreamTtfbMs)],
    [
      comparableNumber(current.tUpstreamStreamMs),
      comparableNumber(next.tUpstreamStreamMs),
    ],
    [comparableNumber(current.tRespParseMs), comparableNumber(next.tRespParseMs)],
    [comparableNumber(current.tPersistMs), comparableNumber(next.tPersistMs)],
    [comparableNumber(current.tTotalMs), comparableNumber(next.tTotalMs)],
  ];

  for (const [currentValue, nextValue] of fields) {
    if (currentValue === nextValue) continue;
    if (currentValue === null) return 1;
    if (nextValue === null) return -1;
    return nextValue > currentValue ? 1 : -1;
  }

  return 0;
}

export function choosePreferredInvocationRecord(
  current: ApiInvocation | undefined,
  next: ApiInvocation,
) {
  if (!current) return next;

  const currentRank = recordLifecycleRank(current);
  const nextRank = recordLifecycleRank(next);
  if (nextRank !== currentRank) {
    return nextRank > currentRank ? next : current;
  }

  const runtimeProgress = compareRecordRuntimeProgress(current, next);
  if (runtimeProgress !== 0) {
    return runtimeProgress > 0 ? next : current;
  }

  const currentScore = recordCompletenessScore(current);
  const nextScore = recordCompletenessScore(next);
  if (nextScore !== currentScore) {
    return nextScore > currentScore ? next : current;
  }

  return current;
}

export function sortInvocationRecords(records: ApiInvocation[]) {
  return [...records].sort(
    (left, right) =>
      new Date(right.occurredAt).getTime() - new Date(left.occurredAt).getTime(),
  );
}

export function mergeInvocationRecordCollections(...collections: ApiInvocation[][]) {
  const dedupe = new Map<string, ApiInvocation>();

  for (const records of collections) {
    for (const record of records) {
      const key = invocationStableKey(record);
      dedupe.set(key, choosePreferredInvocationRecord(dedupe.get(key), record));
    }
  }

  return sortInvocationRecords(Array.from(dedupe.values()));
}
