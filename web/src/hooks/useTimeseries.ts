import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchInvocationRecords, fetchTimeseries } from "../lib/api";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
} from "../lib/api";
import { invocationStableKey } from "../lib/invocation";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

export interface UseTimeseriesOptions {
  bucket?: string;
  settlementHour?: number;
  preferServerAggregation?: boolean;
}

export type TimeseriesSyncMode = "local" | "current-day-local" | "server";

export interface TimeseriesSyncPolicy {
  mode: TimeseriesSyncMode;
  recordsRefreshThrottleMs: number;
}

interface LoadOptions {
  silent?: boolean;
  force?: boolean;
}

interface PendingLoad {
  silent: boolean;
  waiters: Array<() => void>;
}

interface UpdateContext {
  range: string;
  bucketSeconds?: number;
  settlementHour?: number;
}

type LiveRecordOutcome = "success" | "failure" | "pending";

interface LiveRecordDelta {
  bucketStart: string;
  bucketEnd: string;
  bucketStartEpoch: number;
  bucketEndEpoch: number;
  totalCount: number;
  successCount: number;
  failureCount: number;
  totalTokens: number;
  totalCost: number;
  countsOnly?: boolean;
}

export const TIMESERIES_RECORDS_RESYNC_THROTTLE_MS = 3_000;
export const TIMESERIES_OPEN_RESYNC_COOLDOWN_MS = 3_000;
export const TIMESERIES_REMOUNT_CACHE_TTL_MS = 30_000;
export const TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS = 60_000;
export const MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS = 1_024;
const TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE = 500;
const TIMESERIES_IN_FLIGHT_STATUSES = ["running", "pending"] as const;

interface TimeseriesRemountCacheEntry {
  data: TimeseriesResponse;
  cachedAt: number;
  liveRecordDeltas: Map<string, LiveRecordDelta>;
  settledLiveRecordUpdatedAt: Map<string, number>;
  untrackedInFlightCounts: Map<string, number>;
}

const timeseriesRemountCache = new Map<string, TimeseriesRemountCacheEntry>();

function cloneTimeseriesResponse(
  response: TimeseriesResponse,
): TimeseriesResponse {
  return {
    ...response,
    points: response.points.map((point) => ({ ...point })),
  };
}

function cloneLiveRecordDelta(delta: LiveRecordDelta): LiveRecordDelta {
  return { ...delta };
}

function cloneLiveRecordDeltaMap(
  liveRecordDeltas?: ReadonlyMap<string, LiveRecordDelta> | null,
) {
  const cloned = new Map<string, LiveRecordDelta>();
  if (!liveRecordDeltas) {
    return cloned;
  }
  for (const [key, delta] of liveRecordDeltas) {
    cloned.set(key, cloneLiveRecordDelta(delta));
  }
  return cloned;
}

function cloneSettledLiveRecordUpdatedAtMap(
  settledLiveRecordUpdatedAt?: ReadonlyMap<string, number> | null,
) {
  return new Map(settledLiveRecordUpdatedAt);
}

function cloneTimeseriesRemountCacheEntry(entry: TimeseriesRemountCacheEntry) {
  return {
    data: cloneTimeseriesResponse(entry.data),
    cachedAt: entry.cachedAt,
    liveRecordDeltas: cloneLiveRecordDeltaMap(entry.liveRecordDeltas),
    settledLiveRecordUpdatedAt: cloneSettledLiveRecordUpdatedAtMap(
      entry.settledLiveRecordUpdatedAt,
    ),
    untrackedInFlightCounts: new Map(entry.untrackedInFlightCounts),
  };
}

export function resolveTimeseriesSyncPolicy(
  range: string,
  options?: UseTimeseriesOptions,
): TimeseriesSyncPolicy {
  const rangeSeconds = parseRangeSpec(range);

  if (options?.preferServerAggregation) {
    return {
      mode: "server",
      recordsRefreshThrottleMs: TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
    };
  }

  if (range === "1d" && options?.bucket === "1m") {
    return {
      mode: "local",
      recordsRefreshThrottleMs: 0,
    };
  }

  if (range === "7d" && options?.bucket === "1h") {
    return {
      mode: "local",
      recordsRefreshThrottleMs: 0,
    };
  }

  if (
    options?.bucket === "1d" &&
    rangeSeconds !== null &&
    rangeSeconds >= 90 * 86_400
  ) {
    return {
      mode: "current-day-local",
      recordsRefreshThrottleMs: 0,
    };
  }

  const bucketSeconds =
    guessBucketSeconds(options?.bucket) ?? defaultBucketSecondsForRange(range);
  return {
    mode: bucketSeconds >= 86_400 ? "server" : "local",
    recordsRefreshThrottleMs:
      bucketSeconds >= 86_400 ? TIMESERIES_RECORDS_RESYNC_THROTTLE_MS : 0,
  };
}

export function shouldResyncOnRecordsEvent(
  range: string,
  options?: UseTimeseriesOptions,
) {
  return resolveTimeseriesSyncPolicy(range, options).mode === "server";
}

export function shouldPatchCurrentDayBucketOnRecordsEvent(
  range: string,
  options?: UseTimeseriesOptions,
) {
  return (
    resolveTimeseriesSyncPolicy(range, options).mode === "current-day-local"
  );
}

export function getTimeseriesRecordsResyncDelay(
  lastRefreshAt: number,
  now: number,
  throttleMs = TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
) {
  return Math.max(0, throttleMs - (now - lastRefreshAt));
}

export function shouldTriggerTimeseriesOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= TIMESERIES_OPEN_RESYNC_COOLDOWN_MS;
}

export function getTimeseriesRemountCacheKey(
  range: string,
  options?: UseTimeseriesOptions,
) {
  return JSON.stringify([
    range,
    options?.bucket ?? null,
    options?.settlementHour ?? null,
    options?.preferServerAggregation ?? false,
  ]);
}

export function shouldEnableTimeseriesRemountCache(range: string) {
  return range !== "current" && range !== "today";
}

export function readTimeseriesRemountCache(
  range: string,
  options?: UseTimeseriesOptions,
  now = Date.now(),
  ttlMs = TIMESERIES_REMOUNT_CACHE_TTL_MS,
) {
  if (!shouldEnableTimeseriesRemountCache(range)) return null;
  const cached = timeseriesRemountCache.get(
    getTimeseriesRemountCacheKey(range, options),
  );
  if (!cached) return null;
  return shouldReuseTimeseriesRemountCache(cached.cachedAt, now, ttlMs)
    ? cloneTimeseriesRemountCacheEntry(cached)
    : null;
}

export function writeTimeseriesRemountCache(
  range: string,
  options: UseTimeseriesOptions | undefined,
  data: TimeseriesResponse,
  cachedAt = Date.now(),
  liveRecordDeltas?: ReadonlyMap<string, LiveRecordDelta> | null,
  settledLiveRecordUpdatedAt?: ReadonlyMap<string, number> | null,
  untrackedInFlightCounts?: ReadonlyMap<string, number> | null,
) {
  if (!shouldEnableTimeseriesRemountCache(range)) return;
  timeseriesRemountCache.set(getTimeseriesRemountCacheKey(range, options), {
    data: cloneTimeseriesResponse(data),
    cachedAt,
    liveRecordDeltas: cloneLiveRecordDeltaMap(liveRecordDeltas),
    settledLiveRecordUpdatedAt: cloneSettledLiveRecordUpdatedAtMap(
      settledLiveRecordUpdatedAt,
    ),
    untrackedInFlightCounts: new Map(untrackedInFlightCounts),
  });
}

export function clearTimeseriesRemountCache() {
  timeseriesRemountCache.clear();
}

export function shouldReuseTimeseriesRemountCache(
  cachedAt: number,
  now: number,
  ttlMs = TIMESERIES_REMOUNT_CACHE_TTL_MS,
) {
  return now - cachedAt < ttlMs;
}

export function mergePendingTimeseriesSilentOption(
  existingSilent: boolean | null,
  incomingSilent: boolean,
) {
  return (existingSilent ?? true) && incomingSilent;
}

export function getLocalDayStartEpoch(
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(0, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

export function getNextLocalDayStartEpoch(
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(24, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

function getRangeStartEpoch(range: string, rangeEndEpoch: number) {
  if (range === "today") {
    return getLocalDayStartEpoch(rangeEndEpoch);
  }

  const rangeSeconds = parseRangeSpec(range);
  return rangeSeconds != null ? rangeEndEpoch - rangeSeconds : null;
}

function resolvePendingLoad(pending: PendingLoad | null) {
  if (!pending) return;
  pending.waiters.forEach((resolve) => resolve());
}

function createSeededTimeseries(range: string, bucket?: string) {
  const bucketSeconds =
    guessBucketSeconds(bucket) ?? defaultBucketSecondsForRange(range);
  const nowEpochSeconds = Math.floor(Date.now() / 1000);
  const rangeStartEpoch =
    getRangeStartEpoch(range, nowEpochSeconds) ?? nowEpochSeconds - 86_400;
  const start = formatEpochToIso(rangeStartEpoch);
  const end = formatEpochToIso(nowEpochSeconds);
  return {
    rangeStart: start,
    rangeEnd: end,
    bucketSeconds,
    points: [] satisfies TimeseriesPoint[],
  };
}

function normalizeLiveRecordOutcome(record: ApiInvocation): LiveRecordOutcome {
  const status = record.status?.trim().toLowerCase() ?? "";
  const failureClass = record.failureClass?.trim().toLowerCase() ?? "";
  const hasFailureDownstreamStatus =
    typeof record.downstreamStatusCode === "number" &&
    Number.isFinite(record.downstreamStatusCode) &&
    (record.downstreamStatusCode < 200 || record.downstreamStatusCode >= 300);
  const hasFailureMetadata =
    (failureClass.length > 0 && failureClass !== "none") ||
    (record.failureKind?.trim().length ?? 0) > 0 ||
    (record.errorMessage?.trim().length ?? 0) > 0 ||
    (record.downstreamErrorMessage?.trim().length ?? 0) > 0 ||
    hasFailureDownstreamStatus;

  if (status === "success" || status === "http_200") {
    return hasFailureMetadata ? "failure" : "success";
  }
  if (status === "running" || status === "pending") {
    return "pending";
  }
  if (status.length > 0) return "failure";
  return hasFailureMetadata ? "failure" : "pending";
}

function createLiveRecordDelta(
  record: ApiInvocation,
  bucketStartEpoch: number,
  bucketEndEpoch: number,
): LiveRecordDelta {
  const outcome = normalizeLiveRecordOutcome(record);
  return {
    bucketStart: formatEpochToIso(bucketStartEpoch),
    bucketEnd: formatEpochToIso(bucketEndEpoch),
    bucketStartEpoch,
    bucketEndEpoch,
    totalCount: 1,
    successCount: outcome === "success" ? 1 : 0,
    failureCount: outcome === "failure" ? 1 : 0,
    totalTokens: record.totalTokens ?? 0,
    totalCost: record.cost ?? 0,
  };
}

function getTimeseriesPointInFlightCount(
  point: Pick<TimeseriesPoint, "totalCount" | "successCount" | "failureCount">,
) {
  return Math.max(
    point.totalCount - point.successCount - point.failureCount,
    0,
  );
}

function buildUntrackedInFlightCounts(
  current: TimeseriesResponse,
  liveRecordDeltas: ReadonlyMap<string, LiveRecordDelta>,
) {
  const remaining = new Map<string, number>();
  for (const point of current.points) {
    const inFlightCount = getTimeseriesPointInFlightCount(point);
    if (inFlightCount > 0) {
      remaining.set(point.bucketStart, inFlightCount);
    }
  }
  for (const delta of liveRecordDeltas.values()) {
    const currentCount = remaining.get(delta.bucketStart) ?? 0;
    const nextCount = currentCount - delta.totalCount;
    if (nextCount > 0) {
      remaining.set(delta.bucketStart, nextCount);
    } else {
      remaining.delete(delta.bucketStart);
    }
  }
  return remaining;
}

function claimUntrackedInFlightDelta(
  record: ApiInvocation,
  current: TimeseriesResponse,
  nextDelta: LiveRecordDelta | null,
  untrackedInFlightCounts: Map<string, number>,
) {
  if (!nextDelta) {
    return null;
  }
  const occurredEpoch = parseIsoEpoch(record.occurredAt);
  const currentRangeEndEpoch = parseIsoEpoch(current.rangeEnd);
  if (
    occurredEpoch != null &&
    currentRangeEndEpoch != null &&
    occurredEpoch > currentRangeEndEpoch
  ) {
    return null;
  }
  const remainingCount =
    untrackedInFlightCounts.get(nextDelta.bucketStart) ?? 0;
  if (remainingCount <= 0) {
    return null;
  }
  if (remainingCount === 1) {
    untrackedInFlightCounts.delete(nextDelta.bucketStart);
  } else {
    untrackedInFlightCounts.set(nextDelta.bucketStart, remainingCount - 1);
  }
  return createCountsOnlyLiveRecordDelta({
    ...nextDelta,
    successCount: 0,
    failureCount: 0,
  });
}

function createCountsOnlyLiveRecordDelta(
  delta: LiveRecordDelta,
): LiveRecordDelta {
  // Anonymous in-flight placeholders only prove that this bucket already counted
  // one live invocation. They do not carry enough information to safely back out
  // provisional token/cost totals, so local patching is limited to count fields.
  return {
    ...delta,
    totalTokens: 0,
    totalCost: 0,
    countsOnly: true,
  };
}

function shouldKeepCountsOnlyDelta(
  previousDelta: LiveRecordDelta | null,
  nextDelta: LiveRecordDelta | null,
  currentPoint: Pick<
    TimeseriesPoint,
    "totalCount" | "successCount" | "failureCount" | "totalTokens" | "totalCost"
  > | null,
) {
  if (!previousDelta?.countsOnly || !nextDelta) {
    return false;
  }
  if (nextDelta.successCount === 0 && nextDelta.failureCount === 0) {
    return true;
  }
  if (!currentPoint) {
    return false;
  }
  const bucketOnlyContainsPlaceholder =
    currentPoint.totalCount === previousDelta.totalCount &&
    currentPoint.successCount === previousDelta.successCount &&
    currentPoint.failureCount === previousDelta.failureCount;
  if (!bucketOnlyContainsPlaceholder) {
    return false;
  }
  return currentPoint.totalTokens !== 0 || currentPoint.totalCost !== 0;
}

async function fetchTimeseriesInFlightRecords(
  current: TimeseriesResponse,
  signal?: AbortSignal,
) {
  const batches = await Promise.all(
    TIMESERIES_IN_FLIGHT_STATUSES.map(async (status) => {
      return fetchAllInvocationRecordPages(
        {
          from: current.rangeStart,
          to: current.rangeEnd,
          status,
          sortBy: "occurredAt",
          sortOrder: "desc",
          signal,
        },
        fetchInvocationRecords,
        TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE,
      );
    }),
  );
  const deduped = new Map<string, ApiInvocation>();
  for (const batch of batches) {
    for (const record of batch) {
      deduped.set(invocationStableKey(record), record);
    }
  }
  return Array.from(deduped.values());
}

export async function fetchAllInvocationRecordPages(
  query: Omit<InvocationRecordsQuery, "page" | "pageSize">,
  fetchPage: (
    query: InvocationRecordsQuery,
  ) => Promise<InvocationRecordsResponse> = fetchInvocationRecords,
  pageSize = TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE,
) {
  const requestedPageSize = Math.max(1, pageSize);
  const records: ApiInvocation[] = [];
  let page = 1;
  let snapshotId: number | undefined;

  while (true) {
    const response = await fetchPage({
      ...query,
      page,
      pageSize: requestedPageSize,
      ...(snapshotId != null ? { snapshotId } : {}),
    });
    if (snapshotId == null && typeof response.snapshotId === "number") {
      snapshotId = response.snapshotId;
    }
    const batch = response.records ?? [];
    records.push(...batch);

    const resolvedPageSize =
      typeof response.pageSize === "number" && response.pageSize > 0
        ? response.pageSize
        : requestedPageSize;
    const resolvedTotal =
      typeof response.total === "number" && response.total >= 0
        ? response.total
        : null;

    if (batch.length === 0) {
      break;
    }
    if (resolvedTotal != null && page * resolvedPageSize >= resolvedTotal) {
      break;
    }
    if (batch.length < resolvedPageSize) {
      break;
    }
    page += 1;
  }

  return records;
}

function sameLiveRecordDelta(
  left: LiveRecordDelta | null,
  right: LiveRecordDelta | null,
) {
  if (left === right) return true;
  if (!left || !right) return false;
  return (
    left.bucketStart === right.bucketStart &&
    left.bucketEnd === right.bucketEnd &&
    left.totalCount === right.totalCount &&
    left.successCount === right.successCount &&
    left.failureCount === right.failureCount &&
    left.totalTokens === right.totalTokens &&
    left.totalCost === right.totalCost &&
    (left.countsOnly ?? false) === (right.countsOnly ?? false)
  );
}

function isPendingLiveRecord(record: ApiInvocation) {
  return normalizeLiveRecordOutcome(record) === "pending";
}

export function pruneTrackedTimeseriesLiveRecordDeltas(
  liveRecordDeltas: Map<string, LiveRecordDelta>,
  settledLiveRecordUpdatedAt: Map<string, number>,
  now = Date.now(),
  ttlMs = TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
  maxEntries = MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
) {
  for (const [key, updatedAt] of settledLiveRecordUpdatedAt) {
    if (now - updatedAt < ttlMs) continue;
    settledLiveRecordUpdatedAt.delete(key);
    liveRecordDeltas.delete(key);
  }

  const overflow = settledLiveRecordUpdatedAt.size - maxEntries;
  if (overflow <= 0) {
    return;
  }

  const oldestSettledEntries = Array.from(settledLiveRecordUpdatedAt.entries())
    .sort((left, right) => left[1] - right[1])
    .slice(0, overflow);

  for (const [key] of oldestSettledEntries) {
    settledLiveRecordUpdatedAt.delete(key);
    liveRecordDeltas.delete(key);
  }
}

export function trackTimeseriesLiveRecordDelta(
  liveRecordDeltas: Map<string, LiveRecordDelta>,
  settledLiveRecordUpdatedAt: Map<string, number>,
  key: string,
  record: ApiInvocation,
  delta: LiveRecordDelta | null,
  now = Date.now(),
  ttlMs = TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
  maxEntries = MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
) {
  pruneTrackedTimeseriesLiveRecordDeltas(
    liveRecordDeltas,
    settledLiveRecordUpdatedAt,
    now,
    ttlMs,
    maxEntries,
  );

  if (!delta) {
    liveRecordDeltas.delete(key);
    settledLiveRecordUpdatedAt.delete(key);
    return;
  }

  liveRecordDeltas.set(key, delta);
  if (isPendingLiveRecord(record)) {
    settledLiveRecordUpdatedAt.delete(key);
    return;
  }

  settledLiveRecordUpdatedAt.set(key, now);
  pruneTrackedTimeseriesLiveRecordDeltas(
    liveRecordDeltas,
    settledLiveRecordUpdatedAt,
    now,
    ttlMs,
    maxEntries,
  );
}

function adjustTimeseriesPoint(
  point: TimeseriesPoint,
  delta: LiveRecordDelta,
  sign: 1 | -1,
) {
  point.bucketEnd = delta.bucketEnd;
  point.totalCount += sign * delta.totalCount;
  point.successCount += sign * delta.successCount;
  point.failureCount += sign * delta.failureCount;
  point.totalTokens += sign * delta.totalTokens;
  point.totalCost += sign * delta.totalCost;
}

function buildCurrentDayLiveRecordDelta(
  record: ApiInvocation,
  currentBucket: TimeseriesPoint,
): LiveRecordDelta | null {
  const occurredEpoch = parseIsoEpoch(record.occurredAt);
  const bucketStartEpoch = parseIsoEpoch(currentBucket.bucketStart);
  const bucketEndEpoch = parseIsoEpoch(currentBucket.bucketEnd);
  if (
    occurredEpoch == null ||
    bucketStartEpoch == null ||
    bucketEndEpoch == null ||
    occurredEpoch < bucketStartEpoch ||
    occurredEpoch >= bucketEndEpoch
  ) {
    return null;
  }
  return createLiveRecordDelta(record, bucketStartEpoch, bucketEndEpoch);
}

export function seedCurrentDayLiveRecordDeltas(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const seeded = new Map<string, LiveRecordDelta>();
  if (!current || records.length === 0) {
    return seeded;
  }

  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds);
  if (!currentBucket) {
    return seeded;
  }

  for (const record of records) {
    const delta = buildCurrentDayLiveRecordDelta(record, currentBucket);
    if (!delta) continue;
    seeded.set(invocationStableKey(record), delta);
  }
  return seeded;
}

export function upsertCurrentDayLiveRecord(
  current: TimeseriesResponse | null,
  record: ApiInvocation,
  previousDelta: LiveRecordDelta | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (!current) {
    return { next: current, delta: null as LiveRecordDelta | null };
  }

  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds);
  if (!currentBucket) {
    return { next: current, delta: null as LiveRecordDelta | null };
  }

  let nextDelta = buildCurrentDayLiveRecordDelta(record, currentBucket);
  if (
    nextDelta &&
    shouldKeepCountsOnlyDelta(previousDelta, nextDelta, currentBucket)
  ) {
    nextDelta = createCountsOnlyLiveRecordDelta(nextDelta);
  }
  if (sameLiveRecordDelta(previousDelta, nextDelta)) {
    return { next: current, delta: nextDelta };
  }

  const nextPoints = current.points.map((point) => ({ ...point }));
  const point = nextPoints.find(
    (item) => item.bucketStart === currentBucket.bucketStart,
  );
  if (!point) {
    return { next: current, delta: previousDelta };
  }

  if (previousDelta) {
    adjustTimeseriesPoint(point, previousDelta, -1);
  }
  if (nextDelta) {
    adjustTimeseriesPoint(point, nextDelta, 1);
  }

  return {
    next: {
      ...current,
      points: nextPoints,
    },
    delta: nextDelta,
  };
}

function buildTimeseriesLiveRecordDelta(
  record: ApiInvocation,
  current: TimeseriesResponse,
  context: UpdateContext,
): LiveRecordDelta | null {
  const bucketSeconds = context.bucketSeconds;
  if (!bucketSeconds || bucketSeconds <= 0) {
    return null;
  }

  const occurredEpoch = parseIsoEpoch(record.occurredAt);
  if (occurredEpoch == null) return null;

  const latestRangeEndEpoch =
    parseIsoEpoch(current.rangeEnd) ?? occurredEpoch + bucketSeconds;
  const earliestAllowed = getRangeStartEpoch(
    context.range,
    latestRangeEndEpoch,
  );
  if (earliestAllowed != null && occurredEpoch < earliestAllowed) {
    return null;
  }

  const offsetSeconds =
    bucketSeconds >= 86_400 ? (context.settlementHour ?? 0) * 3_600 : 0;
  const bucketStartEpoch = alignBucketEpoch(
    occurredEpoch,
    bucketSeconds,
    offsetSeconds,
  );
  const bucketEndEpoch = bucketStartEpoch + bucketSeconds;
  return createLiveRecordDelta(record, bucketStartEpoch, bucketEndEpoch);
}

export function seedTimeseriesLiveRecordDeltas(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  context: UpdateContext,
) {
  const seeded = new Map<string, LiveRecordDelta>();
  if (!current || records.length === 0) {
    return seeded;
  }

  for (const record of records) {
    const delta = buildTimeseriesLiveRecordDelta(record, current, context);
    if (!delta) continue;
    seeded.set(invocationStableKey(record), delta);
  }
  return seeded;
}

export function resolveCurrentDayLiveSeedEpoch(
  current: TimeseriesResponse,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const rangeStartEpoch = parseIsoEpoch(current.rangeStart) ?? 0;
  const rangeEndEpoch = parseIsoEpoch(current.rangeEnd);
  if (rangeEndEpoch != null && rangeEndEpoch > rangeStartEpoch) {
    return Math.max(rangeStartEpoch, rangeEndEpoch - 1);
  }
  return nowEpochSeconds;
}

async function loadSeededLiveRecordDeltas(
  current: TimeseriesResponse,
  syncMode: TimeseriesSyncMode,
  context: UpdateContext,
  signal?: AbortSignal,
) {
  if (syncMode === "server") {
    return new Map<string, LiveRecordDelta>();
  }

  const inFlightRecords = await fetchTimeseriesInFlightRecords(current, signal);
  if (syncMode === "current-day-local") {
    const seedEpoch = resolveCurrentDayLiveSeedEpoch(current);
    return seedCurrentDayLiveRecordDeltas(current, inFlightRecords, seedEpoch);
  }

  return seedTimeseriesLiveRecordDeltas(current, inFlightRecords, context);
}

export function upsertTimeseriesLiveRecord(
  current: TimeseriesResponse | null,
  record: ApiInvocation,
  previousDelta: LiveRecordDelta | null,
  context: UpdateContext,
) {
  if (!current) {
    return { next: current, delta: null as LiveRecordDelta | null };
  }

  const currentPoint = previousDelta
    ? (current.points.find(
        (point) => point.bucketStart === previousDelta.bucketStart,
      ) ?? null)
    : null;
  let nextDelta = buildTimeseriesLiveRecordDelta(record, current, context);
  if (
    nextDelta &&
    shouldKeepCountsOnlyDelta(previousDelta, nextDelta, currentPoint)
  ) {
    nextDelta = createCountsOnlyLiveRecordDelta(nextDelta);
  }
  if (sameLiveRecordDelta(previousDelta, nextDelta)) {
    return { next: current, delta: nextDelta };
  }

  const points = new Map<string, TimeseriesPoint>(
    current.points.map((point) => [point.bucketStart, { ...point }]),
  );

  const applyDelta = (delta: LiveRecordDelta, sign: 1 | -1) => {
    const existing = points.get(delta.bucketStart);
    if (!existing && sign < 0) {
      return;
    }
    const point = existing ?? {
      bucketStart: delta.bucketStart,
      bucketEnd: delta.bucketEnd,
      totalCount: 0,
      successCount: 0,
      failureCount: 0,
      totalTokens: 0,
      totalCost: 0,
    };
    adjustTimeseriesPoint(point, delta, sign);
    const isEmpty =
      point.totalCount === 0 &&
      point.successCount === 0 &&
      point.failureCount === 0 &&
      point.totalTokens === 0 &&
      point.totalCost === 0;
    if (isEmpty) {
      points.delete(delta.bucketStart);
    } else {
      points.set(delta.bucketStart, point);
    }
  };

  if (previousDelta) {
    applyDelta(previousDelta, -1);
  }
  if (nextDelta) {
    applyDelta(nextDelta, 1);
  }

  const sortedPoints = Array.from(points.values()).sort((a, b) => {
    const aEpoch = parseIsoEpoch(a.bucketStart) ?? 0;
    const bEpoch = parseIsoEpoch(b.bucketStart) ?? 0;
    return aEpoch - bEpoch;
  });

  const nextRangeEndEpoch = Math.max(
    parseIsoEpoch(current.rangeEnd) ?? 0,
    nextDelta?.bucketEndEpoch ?? 0,
  );
  const nextRangeEnd =
    nextRangeEndEpoch > 0
      ? formatEpochToIso(nextRangeEndEpoch)
      : current.rangeEnd;
  const nextRangeStartEpoch =
    nextRangeEndEpoch > 0
      ? getRangeStartEpoch(context.range, nextRangeEndEpoch)
      : null;
  const nextRangeStart =
    nextRangeStartEpoch != null
      ? formatEpochToIso(nextRangeStartEpoch)
      : current.rangeStart;

  while (nextRangeStartEpoch != null && sortedPoints.length > 0) {
    const first = sortedPoints[0];
    const firstEndEpoch = parseIsoEpoch(first.bucketEnd);
    if (firstEndEpoch != null && firstEndEpoch <= nextRangeStartEpoch) {
      sortedPoints.shift();
      continue;
    }
    break;
  }

  return {
    next: {
      ...current,
      rangeStart: nextRangeStart,
      rangeEnd: nextRangeEnd,
      points: sortedPoints,
    },
    delta: nextDelta,
  };
}

export function useTimeseries(range: string, options?: UseTimeseriesOptions) {
  const initialCachedTimeseries = readTimeseriesRemountCache(range, options);
  const initialLiveRecordDeltas =
    initialCachedTimeseries?.liveRecordDeltas ??
    new Map<string, LiveRecordDelta>();
  const [data, setData] = useState<TimeseriesResponse | null>(
    () => initialCachedTimeseries?.data ?? null,
  );
  const [isLoading, setIsLoading] = useState(
    () => initialCachedTimeseries == null,
  );
  const [error, setError] = useState<string | null>(null);
  const bucket = options?.bucket;
  const settlementHour = options?.settlementHour;
  const preferServerAggregation = options?.preferServerAggregation ?? false;
  const hasHydratedRef = useRef(initialCachedTimeseries != null);
  const activeLoadCountRef = useRef(0);
  const pendingLoadRef = useRef<PendingLoad | null>(null);
  const pendingOpenResyncRef = useRef(false);
  const requestSeqRef = useRef(0);
  const activeRequestControllerRef = useRef<AbortController | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dayRolloverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const lastRecordsRefreshAtRef = useRef(0);
  const lastOpenResyncAtRef = useRef(0);
  const localRevisionRef = useRef(0);
  const dataRef = useRef<TimeseriesResponse | null>(
    initialCachedTimeseries?.data ?? null,
  );
  const liveRecordDeltaRef = useRef<Map<string, LiveRecordDelta>>(
    initialLiveRecordDeltas,
  );
  const settledLiveRecordUpdatedAtRef = useRef<Map<string, number>>(
    initialCachedTimeseries?.settledLiveRecordUpdatedAt ?? new Map(),
  );
  const untrackedInFlightCountsRef = useRef<Map<string, number>>(
    initialCachedTimeseries?.untrackedInFlightCounts ?? new Map(),
  );

  const normalizedOptions = useMemo<UseTimeseriesOptions>(
    () => ({
      bucket,
      settlementHour,
      preferServerAggregation,
    }),
    [bucket, settlementHour, preferServerAggregation],
  );

  const syncPolicy = useMemo(
    () => resolveTimeseriesSyncPolicy(range, normalizedOptions),
    [normalizedOptions, range],
  );

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const clearDayRolloverTimer = useCallback(() => {
    if (!dayRolloverTimerRef.current) return;
    clearTimeout(dayRolloverTimerRef.current);
    dayRolloverTimerRef.current = null;
  }, []);

  const clearPendingLoad = useCallback(() => {
    resolvePendingLoad(pendingLoadRef.current);
    pendingLoadRef.current = null;
  }, []);

  const runLoad = useCallback(
    async ({ silent = false }: LoadOptions = {}) => {
      activeLoadCountRef.current += 1;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      const baselineLocalRevision = localRevisionRef.current;
      const controller = new AbortController();
      activeRequestControllerRef.current = controller;
      const shouldShowLoading = !(silent && hasHydratedRef.current);
      if (shouldShowLoading) {
        setIsLoading(true);
      }

      try {
        const response = await fetchTimeseries(range, {
          ...normalizedOptions,
          signal: controller.signal,
        });
        let seededLiveRecordDeltas = new Map<string, LiveRecordDelta>();
        let untrackedInFlightCounts = new Map<string, number>();
        if (syncPolicy.mode !== "server") {
          try {
            seededLiveRecordDeltas = await loadSeededLiveRecordDeltas(
              response,
              syncPolicy.mode,
              {
                range,
                bucketSeconds: response.bucketSeconds,
                settlementHour: normalizedOptions.settlementHour,
              },
              controller.signal,
            );
          } catch (seedErr) {
            if (seedErr instanceof Error && seedErr.name === "AbortError") {
              throw seedErr;
            }
            console.warn(
              "Failed to seed in-flight timeseries deltas after load",
              seedErr,
            );
          }
          untrackedInFlightCounts = buildUntrackedInFlightCounts(
            response,
            seededLiveRecordDeltas,
          );
        }
        if (requestSeq !== requestSeqRef.current) {
          return;
        }

        const shouldPreserveLocallyPatchedData =
          syncPolicy.mode !== "server" &&
          baselineLocalRevision !== localRevisionRef.current;

        if (shouldPreserveLocallyPatchedData) {
          if (pendingLoadRef.current) {
            pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
              pendingLoadRef.current.silent,
              true,
            );
          } else {
            pendingLoadRef.current = { silent: true, waiters: [] };
          }
        } else {
          liveRecordDeltaRef.current = seededLiveRecordDeltas;
          settledLiveRecordUpdatedAtRef.current = new Map();
          untrackedInFlightCountsRef.current = untrackedInFlightCounts;
          dataRef.current = response;
          setData(response);
          writeTimeseriesRemountCache(
            range,
            normalizedOptions,
            response,
            Date.now(),
            seededLiveRecordDeltas,
            settledLiveRecordUpdatedAtRef.current,
            untrackedInFlightCounts,
          );
        }

        hasHydratedRef.current = true;
        setError(null);

        if (pendingOpenResyncRef.current) {
          pendingOpenResyncRef.current = false;
          lastOpenResyncAtRef.current = Date.now();
          if (pendingLoadRef.current) {
            pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
              pendingLoadRef.current.silent,
              true,
            );
          } else {
            pendingLoadRef.current = { silent: true, waiters: [] };
          }
        }
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) {
          return;
        }
        if (err instanceof Error && err.name === "AbortError") {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
        if (dataRef.current != null) {
          hasHydratedRef.current = true;
          return;
        }
        const fallback = createSeededTimeseries(
          range,
          normalizedOptions.bucket,
        );
        liveRecordDeltaRef.current = new Map();
        settledLiveRecordUpdatedAtRef.current = new Map();
        dataRef.current = fallback;
        setData(fallback);
        hasHydratedRef.current = true;
      } finally {
        if (activeRequestControllerRef.current === controller) {
          activeRequestControllerRef.current = null;
        }
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
        activeLoadCountRef.current = Math.max(
          0,
          activeLoadCountRef.current - 1,
        );
        if (activeLoadCountRef.current === 0) {
          const pendingLoad = pendingLoadRef.current;
          if (pendingLoad) {
            pendingLoadRef.current = null;
            void runLoad({ silent: pendingLoad.silent }).finally(() => {
              pendingLoad.waiters.forEach((resolve) => resolve());
            });
          }
        }
      }
    },
    [normalizedOptions, range, syncPolicy.mode],
  );

  const load = useCallback(
    async ({ silent = false, force = false }: LoadOptions = {}) => {
      if (force) {
        activeRequestControllerRef.current?.abort();
        clearPendingLoad();
        clearPendingRefreshTimer();
      }

      if (!force && activeLoadCountRef.current > 0) {
        return new Promise<void>((resolve) => {
          if (pendingLoadRef.current) {
            pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
              pendingLoadRef.current.silent,
              silent,
            );
            pendingLoadRef.current.waiters.push(resolve);
            return;
          }
          pendingLoadRef.current = { silent, waiters: [resolve] };
        });
      }

      if (syncPolicy.mode === "server") {
        lastRecordsRefreshAtRef.current = Date.now();
      }

      await runLoad({ silent });
    },
    [clearPendingLoad, clearPendingRefreshTimer, runLoad, syncPolicy.mode],
  );

  const triggerRecordsResync = useCallback(() => {
    if (
      typeof document !== "undefined" &&
      document.visibilityState !== "visible"
    )
      return;
    const now = Date.now();
    const delay = getTimeseriesRecordsResyncDelay(
      lastRecordsRefreshAtRef.current,
      now,
      syncPolicy.recordsRefreshThrottleMs,
    );
    const run = () => {
      refreshTimerRef.current = null;
      lastRecordsRefreshAtRef.current = Date.now();
      void load({ silent: true });
    };

    if (delay === 0) {
      clearPendingRefreshTimer();
      run();
      return;
    }

    if (refreshTimerRef.current) {
      return;
    }
    refreshTimerRef.current = setTimeout(run, delay);
  }, [clearPendingRefreshTimer, load, syncPolicy.recordsRefreshThrottleMs]);

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true;
        return;
      }
      const now = Date.now();
      if (
        !shouldTriggerTimeseriesOpenResync(
          lastOpenResyncAtRef.current,
          now,
          force,
        )
      ) {
        return;
      }
      lastOpenResyncAtRef.current = now;
      void load({ silent: true, force: true });
    },
    [load],
  );

  useEffect(() => {
    const cachedTimeseries = readTimeseriesRemountCache(
      range,
      normalizedOptions,
    );
    requestSeqRef.current += 1;
    activeRequestControllerRef.current?.abort();
    activeRequestControllerRef.current = null;
    setData(cachedTimeseries?.data ?? null);
    setError(null);
    setIsLoading(cachedTimeseries == null);
    hasHydratedRef.current = cachedTimeseries != null;
    pendingOpenResyncRef.current = false;
    lastRecordsRefreshAtRef.current = 0;
    lastOpenResyncAtRef.current = 0;
    localRevisionRef.current = 0;
    dataRef.current = cachedTimeseries?.data ?? null;
    liveRecordDeltaRef.current =
      cachedTimeseries?.liveRecordDeltas ?? new Map();
    settledLiveRecordUpdatedAtRef.current =
      cachedTimeseries?.settledLiveRecordUpdatedAt ?? new Map();
    untrackedInFlightCountsRef.current =
      cachedTimeseries?.untrackedInFlightCounts ?? new Map();
    clearPendingLoad();
    clearPendingRefreshTimer();
    clearDayRolloverTimer();
    if (!cachedTimeseries) {
      void load({ force: true });
      return;
    }
    void load({ silent: true, force: true });
  }, [
    clearDayRolloverTimer,
    clearPendingLoad,
    clearPendingRefreshTimer,
    load,
    normalizedOptions,
    range,
  ]);

  useEffect(() => {
    if (!error) return;
    const id = setTimeout(() => {
      void load();
    }, 2000);
    return () => clearTimeout(id);
  }, [error, load]);

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;

      if (syncPolicy.mode === "server") {
        triggerRecordsResync();
        return;
      }

      if (syncPolicy.mode === "current-day-local") {
        const nowEpochSeconds = Math.floor(Date.now() / 1000);
        if (shouldResyncForCurrentDayBucket(dataRef.current, nowEpochSeconds)) {
          triggerOpenResync(true);
          return;
        }
        setData((current) => {
          let next = current;
          for (const record of payload.records) {
            const key = invocationStableKey(record);
            const currentBucket =
              next != null ? getCurrentDayBucket(next, nowEpochSeconds) : null;
            const previousDelta =
              liveRecordDeltaRef.current.get(key) ??
              (next != null
                ? claimUntrackedInFlightDelta(
                    record,
                    next,
                    currentBucket != null
                      ? buildCurrentDayLiveRecordDelta(record, currentBucket)
                      : null,
                    untrackedInFlightCountsRef.current,
                  )
                : null);
            const result = upsertCurrentDayLiveRecord(
              next,
              record,
              previousDelta,
              nowEpochSeconds,
            );
            next = result.next;
            trackTimeseriesLiveRecordDelta(
              liveRecordDeltaRef.current,
              settledLiveRecordUpdatedAtRef.current,
              key,
              record,
              result.delta,
            );
          }
          if (next !== current) {
            dataRef.current = next;
            localRevisionRef.current += 1;
            if (next) {
              writeTimeseriesRemountCache(
                range,
                normalizedOptions,
                next,
                Date.now(),
                liveRecordDeltaRef.current,
                settledLiveRecordUpdatedAtRef.current,
                untrackedInFlightCountsRef.current,
              );
            }
          }
          return next;
        });
        return;
      }

      setData((current) => {
        let next =
          current ?? createSeededTimeseries(range, normalizedOptions.bucket);
        for (const record of payload.records) {
          const key = invocationStableKey(record);
          const previousDelta =
            liveRecordDeltaRef.current.get(key) ??
            claimUntrackedInFlightDelta(
              record,
              next,
              buildTimeseriesLiveRecordDelta(record, next, {
                range,
                bucketSeconds: next.bucketSeconds,
                settlementHour: normalizedOptions.settlementHour,
              }),
              untrackedInFlightCountsRef.current,
            );
          const result = upsertTimeseriesLiveRecord(
            next,
            record,
            previousDelta,
            {
              range,
              bucketSeconds: next.bucketSeconds,
              settlementHour: normalizedOptions.settlementHour,
            },
          );
          next = result.next ?? next;
          trackTimeseriesLiveRecordDelta(
            liveRecordDeltaRef.current,
            settledLiveRecordUpdatedAtRef.current,
            key,
            record,
            result.delta,
          );
        }
        if (next !== current) {
          dataRef.current = next;
          localRevisionRef.current += 1;
          if (next) {
            writeTimeseriesRemountCache(
              range,
              normalizedOptions,
              next,
              Date.now(),
              liveRecordDeltaRef.current,
              settledLiveRecordUpdatedAtRef.current,
              untrackedInFlightCountsRef.current,
            );
          }
        }
        return next;
      });
    });
    return unsubscribe;
  }, [
    normalizedOptions,
    normalizedOptions.bucket,
    normalizedOptions.settlementHour,
    range,
    syncPolicy.mode,
    triggerOpenResync,
    triggerRecordsResync,
  ]);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onVisibilityChange = () => {
      if (document.visibilityState !== "visible") return;
      triggerOpenResync(
        range === "today" || syncPolicy.mode === "current-day-local",
      );
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [range, syncPolicy.mode, triggerOpenResync]);

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync();
    });
    return unsubscribe;
  }, [triggerOpenResync]);

  useEffect(() => {
    clearDayRolloverTimer();
    if (range !== "today" && syncPolicy.mode !== "current-day-local") {
      return;
    }
    const refreshEpoch =
      range === "today"
        ? getNextLocalDayStartEpoch()
        : getCurrentDayBucketEndEpoch(data);
    if (refreshEpoch == null) {
      return;
    }
    const delay = Math.max(0, refreshEpoch * 1000 - Date.now() + 50);
    dayRolloverTimerRef.current = setTimeout(() => {
      void load({ silent: true, force: true });
    }, delay);
    return clearDayRolloverTimer;
  }, [clearDayRolloverTimer, data, load, range, syncPolicy.mode]);

  useEffect(
    () => () => {
      requestSeqRef.current += 1;
      activeRequestControllerRef.current?.abort();
      activeRequestControllerRef.current = null;
      clearPendingLoad();
      clearPendingRefreshTimer();
      clearDayRolloverTimer();
      pendingOpenResyncRef.current = false;
    },
    [clearDayRolloverTimer, clearPendingLoad, clearPendingRefreshTimer],
  );

  return {
    data,
    isLoading,
    error,
    refresh: load,
  };
}

export function getCurrentDayBucketEndEpoch(
  current: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds);
  return parseIsoEpoch(currentBucket?.bucketEnd);
}

export function shouldResyncForCurrentDayBucket(
  current: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  // Without a current bucket, the next live record cannot be patched locally and needs a server resync.
  if (!current || current.points.length === 0) {
    return true;
  }
  return getCurrentDayBucket(current, nowEpochSeconds) == null;
}

function getCurrentDayBucket(
  current: TimeseriesResponse | null,
  nowEpochSeconds: number,
) {
  if (!current) return null;
  for (let index = current.points.length - 1; index >= 0; index -= 1) {
    const point = current.points[index];
    const bucketStartEpoch = parseIsoEpoch(point.bucketStart);
    const bucketEndEpoch = parseIsoEpoch(point.bucketEnd);
    if (bucketStartEpoch == null || bucketEndEpoch == null) continue;
    if (
      nowEpochSeconds >= bucketStartEpoch &&
      nowEpochSeconds < bucketEndEpoch
    ) {
      return point;
    }
  }
  return null;
}

export function applyRecordsToCurrentDayBucket(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (!current || records.length === 0) {
    return current;
  }

  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds);
  if (!currentBucket) {
    return current;
  }

  const bucketStartEpoch = parseIsoEpoch(currentBucket.bucketStart);
  const bucketEndEpoch = parseIsoEpoch(currentBucket.bucketEnd);
  if (bucketStartEpoch == null || bucketEndEpoch == null) {
    return current;
  }

  const nextPoints = current.points.map((point) =>
    point.bucketStart === currentBucket.bucketStart ? { ...point } : point,
  );
  const nextBucket = nextPoints.find(
    (point) => point.bucketStart === currentBucket.bucketStart,
  );
  if (!nextBucket) {
    return current;
  }

  let mutating = false;
  for (const record of records) {
    const occurredEpoch = parseIsoEpoch(record.occurredAt);
    if (
      occurredEpoch == null ||
      occurredEpoch < bucketStartEpoch ||
      occurredEpoch >= bucketEndEpoch
    ) {
      continue;
    }
    nextBucket.totalCount += 1;
    const outcome = normalizeLiveRecordOutcome(record);
    if (outcome === "success") {
      nextBucket.successCount += 1;
    } else if (outcome === "failure") {
      nextBucket.failureCount += 1;
    }
    nextBucket.totalTokens += record.totalTokens ?? 0;
    nextBucket.totalCost += record.cost ?? 0;
    mutating = true;
  }

  if (!mutating) {
    return current;
  }

  return {
    ...current,
    points: nextPoints,
  };
}

export function applyRecordsToTimeseries(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  context: UpdateContext,
) {
  if (!current || records.length === 0) {
    return current;
  }

  const bucketSeconds = context.bucketSeconds;
  if (!bucketSeconds || bucketSeconds <= 0) {
    return current;
  }

  const offsetSeconds =
    bucketSeconds >= 86_400 ? (context.settlementHour ?? 0) * 3_600 : 0;
  const points = new Map<string, TimeseriesPoint>();
  for (const point of current.points) {
    points.set(point.bucketStart, { ...point });
  }

  let mutating = false;
  let latestRangeEndEpoch = parseIsoEpoch(current.rangeEnd);

  for (const record of records) {
    const occurredEpoch = parseIsoEpoch(record.occurredAt);
    if (occurredEpoch == null) continue;

    if (latestRangeEndEpoch == null) {
      latestRangeEndEpoch = occurredEpoch + bucketSeconds;
    }

    if (latestRangeEndEpoch != null) {
      const earliestAllowed = getRangeStartEpoch(
        context.range,
        latestRangeEndEpoch,
      );
      if (earliestAllowed != null && occurredEpoch < earliestAllowed) {
        continue;
      }
    }

    const bucketStartEpoch = alignBucketEpoch(
      occurredEpoch,
      bucketSeconds,
      offsetSeconds,
    );
    const bucketEndEpoch = bucketStartEpoch + bucketSeconds;
    const bucketStart = formatEpochToIso(bucketStartEpoch);
    const bucketEnd = formatEpochToIso(bucketEndEpoch);

    let point = points.get(bucketStart);
    if (!point) {
      point = {
        bucketStart,
        bucketEnd,
        totalCount: 0,
        successCount: 0,
        failureCount: 0,
        totalTokens: 0,
        totalCost: 0,
      };
      points.set(bucketStart, point);
    }

    point.bucketEnd = bucketEnd;
    point.totalCount += 1;
    const outcome = normalizeLiveRecordOutcome(record);
    if (outcome === "success") {
      point.successCount += 1;
    } else if (outcome === "failure") {
      point.failureCount += 1;
    }
    point.totalTokens += record.totalTokens ?? 0;
    point.totalCost += record.cost ?? 0;
    mutating = true;

    if (latestRangeEndEpoch == null || bucketEndEpoch > latestRangeEndEpoch) {
      latestRangeEndEpoch = bucketEndEpoch;
    }
  }

  if (!mutating) {
    return current;
  }

  const sortedPoints = Array.from(points.values()).sort((a, b) => {
    const aEpoch = parseIsoEpoch(a.bucketStart) ?? 0;
    const bEpoch = parseIsoEpoch(b.bucketStart) ?? 0;
    return aEpoch - bEpoch;
  });

  if (latestRangeEndEpoch != null) {
    const earliestAllowed = getRangeStartEpoch(
      context.range,
      latestRangeEndEpoch,
    );
    while (earliestAllowed != null && sortedPoints.length > 0) {
      const first = sortedPoints[0];
      const firstEndEpoch = parseIsoEpoch(first.bucketEnd);
      if (firstEndEpoch != null && firstEndEpoch <= earliestAllowed) {
        sortedPoints.shift();
        continue;
      }
      break;
    }
  }

  const nextRangeEndEpoch =
    latestRangeEndEpoch ?? parseIsoEpoch(current.rangeEnd);
  const nextRangeEnd =
    nextRangeEndEpoch != null
      ? formatEpochToIso(nextRangeEndEpoch)
      : current.rangeEnd;
  const nextRangeStartEpoch =
    nextRangeEndEpoch != null
      ? getRangeStartEpoch(context.range, nextRangeEndEpoch)
      : null;
  const nextRangeStart =
    nextRangeStartEpoch != null
      ? formatEpochToIso(nextRangeStartEpoch)
      : current.rangeStart;

  return {
    ...current,
    rangeStart: nextRangeStart,
    rangeEnd: nextRangeEnd,
    points: sortedPoints,
  };
}

function parseRangeSpec(range: string) {
  if (range.endsWith("mo")) {
    const value = Number(range.slice(0, -2));
    return Number.isFinite(value) ? value * 30 * 86_400 : null;
  }
  const unit = range.slice(-1);
  const value = Number(range.slice(0, -1));
  if (!Number.isFinite(value)) return null;
  switch (unit) {
    case "d":
      return value * 86_400;
    case "h":
      return value * 3_600;
    case "m":
      return value * 60;
    default:
      return null;
  }
}

function alignBucketEpoch(
  epochSeconds: number,
  bucketSeconds: number,
  offsetSeconds: number,
) {
  const adjusted = epochSeconds - offsetSeconds;
  const aligned =
    Math.floor(adjusted / bucketSeconds) * bucketSeconds + offsetSeconds;
  return aligned;
}

function parseIsoEpoch(value?: string | null) {
  if (!value) return null;
  const t = Date.parse(value);
  if (Number.isNaN(t)) return null;
  return Math.floor(t / 1000);
}

function formatEpochToIso(epochSeconds: number) {
  return new Date(epochSeconds * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

function guessBucketSeconds(spec?: string) {
  switch (spec) {
    case "1m":
      return 60;
    case "5m":
      return 300;
    case "15m":
      return 900;
    case "30m":
      return 1800;
    case "1h":
      return 3600;
    case "6h":
      return 21600;
    case "12h":
      return 43200;
    case "1d":
      return 86400;
    default:
      return undefined;
  }
}

function defaultBucketSecondsForRange(range: string) {
  const sec = parseRangeSpec(range) ?? 86_400;
  if (sec <= 3_600) return 60;
  if (sec <= 172_800) return 1_800;
  if (sec <= 2_592_000) return 3_600;
  return 86_400;
}
