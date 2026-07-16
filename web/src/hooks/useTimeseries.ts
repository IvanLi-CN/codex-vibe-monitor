import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
} from "../lib/api";
import { fetchInvocationRecords, fetchTimeseries } from "../lib/api";
import { invocationStableKey } from "../lib/invocation";
import { buildTopicDescriptor } from "../lib/sse";
import { getBrowserTimeZone } from "../lib/timeZone";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export interface UseTimeseriesOptions {
  bucket?: string;
  settlementHour?: number;
  preferServerAggregation?: boolean;
  upstreamAccountId?: number;
}

export type TimeseriesSyncMode = "local" | "current-day-local" | "server";

export interface TimeseriesSyncPolicy {
  mode: TimeseriesSyncMode;
  recordsRefreshThrottleMs: number;
}

export interface TimeseriesOpenResyncOptions {
  bypassCooldown?: boolean;
  forceLoad?: boolean;
}

interface UpdateContext {
  range: string;
  bucketSeconds?: number;
  settlementHour?: number;
}

type LiveRecordOutcome = "success" | "failure" | "in_flight" | "neutral";

interface LiveRecordDelta {
  recordId?: number;
  bucketStart: string;
  bucketEnd: string;
  bucketStartEpoch: number;
  bucketEndEpoch: number;
  totalCount: number;
  successCount: number;
  failureCount: number;
  inFlightCount: number;
  totalTokens: number;
  totalCost: number;
  totalLatencyMs: number;
  totalLatencySampleCount: number;
  countsOnly?: boolean;
}

export const TIMESERIES_RECORDS_RESYNC_THROTTLE_MS = 3_000;
export const TIMESERIES_OPEN_RESYNC_COOLDOWN_MS = 3_000;
export const TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS = 60_000;
export const TIMESERIES_REMOUNT_CACHE_TTL_MS = Math.max(
  30_000,
  TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
);
export const MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS = 1_024;
const TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE = 500;
const TIMESERIES_IN_FLIGHT_STATUSES = ["running", "pending"] as const;

interface TimeseriesRemountCacheEntry {
  data: TimeseriesResponse;
  cachedAt: number;
  liveRecordDeltas: Map<string, LiveRecordDelta>;
  settledLiveRecordUpdatedAt: Map<string, number>;
  untrackedInFlightCounts: Map<string, number>;
  untrackedInFlightClaimSnapshotId: number | null;
}

const timeseriesRemountCache = new Map<string, TimeseriesRemountCacheEntry>();

function cloneTimeseriesResponse(response: TimeseriesResponse): TimeseriesResponse {
  return {
    ...response,
    points: response.points.map((point) => ({ ...point })),
  };
}

function cloneLiveRecordDelta(delta: LiveRecordDelta): LiveRecordDelta {
  return { ...delta };
}

function cloneLiveRecordDeltaMap(liveRecordDeltas?: ReadonlyMap<string, LiveRecordDelta> | null) {
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
    untrackedInFlightClaimSnapshotId: entry.untrackedInFlightClaimSnapshotId,
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

  if (options?.bucket === "1d" && rangeSeconds !== null && rangeSeconds >= 90 * 86_400) {
    return {
      mode: "current-day-local",
      recordsRefreshThrottleMs: 0,
    };
  }

  const bucketSeconds = guessBucketSeconds(options?.bucket) ?? defaultBucketSecondsForRange(range);
  return {
    mode: bucketSeconds >= 86_400 ? "server" : "local",
    recordsRefreshThrottleMs: bucketSeconds >= 86_400 ? TIMESERIES_RECORDS_RESYNC_THROTTLE_MS : 0,
  };
}

export function shouldResyncOnRecordsEvent(range: string, options?: UseTimeseriesOptions) {
  return resolveTimeseriesSyncPolicy(range, options).mode === "server";
}

export function shouldPatchCurrentDayBucketOnRecordsEvent(
  range: string,
  options?: UseTimeseriesOptions,
) {
  return resolveTimeseriesSyncPolicy(range, options).mode === "current-day-local";
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

export function getTimeseriesRemountCacheKey(range: string, options?: UseTimeseriesOptions) {
  return JSON.stringify([
    range,
    options?.bucket ?? null,
    options?.settlementHour ?? null,
    options?.preferServerAggregation ?? false,
    options?.upstreamAccountId ?? null,
  ]);
}

export function shouldEnableTimeseriesRemountCache(range: string) {
  return range !== "current" && range !== "today" && range !== "yesterday";
}

export function readTimeseriesRemountCache(
  range: string,
  options?: UseTimeseriesOptions,
  now = Date.now(),
  ttlMs = TIMESERIES_REMOUNT_CACHE_TTL_MS,
) {
  if (!shouldEnableTimeseriesRemountCache(range)) return null;
  const cached = timeseriesRemountCache.get(getTimeseriesRemountCacheKey(range, options));
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
  untrackedInFlightClaimSnapshotId?: number | null,
) {
  if (!shouldEnableTimeseriesRemountCache(range)) return;
  timeseriesRemountCache.set(getTimeseriesRemountCacheKey(range, options), {
    data: cloneTimeseriesResponse(data),
    cachedAt,
    liveRecordDeltas: cloneLiveRecordDeltaMap(liveRecordDeltas),
    settledLiveRecordUpdatedAt: cloneSettledLiveRecordUpdatedAtMap(settledLiveRecordUpdatedAt),
    untrackedInFlightCounts: new Map(untrackedInFlightCounts),
    untrackedInFlightClaimSnapshotId: untrackedInFlightClaimSnapshotId ?? null,
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

export function getLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(0, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

export function getNextLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(24, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

function isYesterdayTimeseriesStale(
  data: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const rangeEndEpoch = parseIsoEpoch(data?.rangeEnd);
  if (rangeEndEpoch == null) {
    return true;
  }
  return getLocalDayStartEpoch(rangeEndEpoch) !== getLocalDayStartEpoch(nowEpochSeconds);
}

export function getVisibilityOpenResyncMode(
  range: string,
  syncMode: TimeseriesSyncMode,
  data: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (range === "yesterday") {
    return isYesterdayTimeseriesStale(data, nowEpochSeconds) ? "force" : "normal";
  }
  if (range === "today" || syncMode === "current-day-local") {
    return "force";
  }
  return "normal";
}

export function getSseOpenResyncOptions(
  range: string,
  syncMode: TimeseriesSyncMode,
  data: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (range === "yesterday") {
    return {
      bypassCooldown: true,
      forceLoad: getVisibilityOpenResyncMode(range, syncMode, data, nowEpochSeconds) === "force",
    };
  }
  return range === "today" || syncMode === "current-day-local"
    ? { bypassCooldown: true, forceLoad: true }
    : { bypassCooldown: false, forceLoad: false };
}

export function getTimeseriesDayRolloverRefreshEpoch(
  range: string,
  syncMode: TimeseriesSyncMode,
  data: TimeseriesResponse | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (range === "today" || range === "yesterday") {
    return getNextLocalDayStartEpoch(nowEpochSeconds);
  }
  if (syncMode === "current-day-local") {
    return getCurrentDayBucketEndEpoch(data, nowEpochSeconds);
  }
  return null;
}

function getYesterdayRangeEndEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  return getLocalDayStartEpoch(nowEpochSeconds);
}

function getYesterdayRangeStartEpoch(rangeEndEpoch = getYesterdayRangeEndEpoch()) {
  return getLocalDayStartEpoch(rangeEndEpoch - 1);
}

function getClosedNaturalDayRangeBounds(range: string, rangeEndEpoch: number) {
  if (range !== "yesterday") {
    return null;
  }

  const end = getLocalDayStartEpoch(rangeEndEpoch);
  return {
    start: getYesterdayRangeStartEpoch(end),
    end,
  };
}

function getRangeStartEpoch(range: string, rangeEndEpoch: number) {
  if (range === "today") {
    return getLocalDayStartEpoch(rangeEndEpoch);
  }
  if (range === "yesterday") {
    return getYesterdayRangeStartEpoch(rangeEndEpoch);
  }

  const rangeSeconds = parseRangeSpec(range);
  return rangeSeconds != null ? rangeEndEpoch - rangeSeconds : null;
}

function normalizeLiveRecordOutcome(record: ApiInvocation): LiveRecordOutcome {
  const status = record.status?.trim().toLowerCase() ?? "";
  const failureClass = record.failureClass?.trim().toLowerCase() ?? "";
  if (status === "running" || status === "pending") {
    return "in_flight";
  }
  if (failureClass.length > 0 && failureClass !== "none") {
    return "failure";
  }
  if (status === "warning_success") {
    return "success";
  }
  const hasFailureMetadata =
    (record.failureKind?.trim().length ?? 0) > 0 ||
    (record.errorMessage?.trim().length ?? 0) > 0 ||
    (record.downstreamErrorMessage?.trim().length ?? 0) > 0;

  if (status === "success" || status === "completed" || status === "http_200") {
    return hasFailureMetadata ? "failure" : "success";
  }
  if (status.length > 0) return "failure";
  return hasFailureMetadata ? "failure" : "neutral";
}

function createLiveRecordDelta(
  record: ApiInvocation,
  bucketStartEpoch: number,
  bucketEndEpoch: number,
): LiveRecordDelta {
  const outcome = normalizeLiveRecordOutcome(record);
  const rawTotalLatencyMs = record.tTotalMs;
  const isTerminalOutcome = outcome === "success" || outcome === "failure";
  const hasTotalLatencySample =
    isTerminalOutcome &&
    typeof rawTotalLatencyMs === "number" &&
    Number.isFinite(rawTotalLatencyMs) &&
    rawTotalLatencyMs >= 0;
  const totalLatencyMs = hasTotalLatencySample ? Number(rawTotalLatencyMs) : 0;
  return {
    recordId: record.id,
    bucketStart: formatEpochToIso(bucketStartEpoch),
    bucketEnd: formatEpochToIso(bucketEndEpoch),
    bucketStartEpoch,
    bucketEndEpoch,
    totalCount: 1,
    successCount: outcome === "success" ? 1 : 0,
    failureCount: outcome === "failure" ? 1 : 0,
    inFlightCount: outcome === "in_flight" ? 1 : 0,
    totalTokens: record.totalTokens ?? 0,
    totalCost: record.cost ?? 0,
    totalLatencyMs,
    totalLatencySampleCount: hasTotalLatencySample ? 1 : 0,
  };
}

function getTimeseriesPointInFlightCount(
  point: Pick<TimeseriesPoint, "inFlightCount" | "totalCount" | "successCount" | "failureCount">,
) {
  if (typeof point.inFlightCount === "number" && Number.isFinite(point.inFlightCount)) {
    return Math.max(point.inFlightCount, 0);
  }
  return Math.max(point.totalCount - point.successCount - point.failureCount, 0);
}

function getTimeseriesPointCallCount(
  point: Pick<TimeseriesPoint, "inFlightCount" | "totalCount" | "successCount" | "failureCount">,
) {
  return Math.max(
    point.totalCount,
    point.successCount + point.failureCount + getTimeseriesPointInFlightCount(point),
    0,
  );
}

function sanitizeTimeseriesPointLatency(point: TimeseriesPoint) {
  if (getTimeseriesPointCallCount(point) > 0) {
    return;
  }
  point.avgTotalMs = null;
  point.totalLatencySampleCount = 0;
  point.firstByteSampleCount = 0;
  point.firstByteAvgMs = null;
  point.firstByteP95Ms = null;
  point.firstResponseByteTotalSampleCount = 0;
  point.firstResponseByteTotalAvgMs = null;
  point.firstResponseByteTotalP95Ms = null;
}

function createCountsOnlyLiveRecordDelta(delta: LiveRecordDelta): LiveRecordDelta {
  // Anonymous in-flight placeholders only prove that this bucket already counted
  // one live invocation. They do not carry enough information to safely back out
  // provisional token/cost totals, so local patching is limited to count fields.
  return {
    ...delta,
    totalTokens: 0,
    totalCost: 0,
    totalLatencyMs: 0,
    totalLatencySampleCount: 0,
    countsOnly: true,
  };
}

function reconcileCountsOnlyDelta(
  previousDelta: LiveRecordDelta | null,
  nextDelta: LiveRecordDelta | null,
  currentPoint: Pick<
    TimeseriesPoint,
    | "totalCount"
    | "successCount"
    | "failureCount"
    | "inFlightCount"
    | "totalTokens"
    | "totalCost"
    | "avgTotalMs"
    | "totalLatencySampleCount"
  > | null,
) {
  if (!previousDelta?.countsOnly || !nextDelta) {
    return nextDelta;
  }
  if (nextDelta.inFlightCount > 0 && nextDelta.successCount === 0 && nextDelta.failureCount === 0) {
    return createCountsOnlyLiveRecordDelta(nextDelta);
  }
  if (!currentPoint) {
    return nextDelta;
  }
  const bucketOnlyContainsPlaceholder =
    currentPoint.totalCount === previousDelta.totalCount &&
    currentPoint.successCount === previousDelta.successCount &&
    currentPoint.failureCount === previousDelta.failureCount &&
    getTimeseriesPointInFlightCount(currentPoint) === previousDelta.inFlightCount;
  if (!bucketOnlyContainsPlaceholder) {
    return nextDelta;
  }
  return {
    ...nextDelta,
    totalTokens: nextDelta.totalTokens - currentPoint.totalTokens,
    totalCost: nextDelta.totalCost - currentPoint.totalCost,
    totalLatencyMs:
      nextDelta.totalLatencyMs -
      (currentPoint.avgTotalMs ?? 0) * Math.max(currentPoint.totalLatencySampleCount ?? 0, 0),
    totalLatencySampleCount:
      nextDelta.totalLatencySampleCount - Math.max(currentPoint.totalLatencySampleCount ?? 0, 0),
  };
}

interface FetchAllInvocationRecordPagesResult {
  records: ApiInvocation[];
  snapshotId: number | undefined;
}

export async function fetchTimeseriesInFlightRecords(
  current: TimeseriesResponse,
  signal?: AbortSignal,
  fetchPage: (
    query: InvocationRecordsQuery,
  ) => Promise<InvocationRecordsResponse> = fetchInvocationRecords,
  rangeOverride?: { from: string; to: string } | null,
  upstreamAccountId?: number,
) {
  const [firstStatus, ...remainingStatuses] = TIMESERIES_IN_FLIGHT_STATUSES;
  const rangeFrom = rangeOverride?.from ?? current.rangeStart;
  const rangeTo = rangeOverride?.to ?? current.rangeEnd;
  const initialSnapshotId = typeof current.snapshotId === "number" ? current.snapshotId : undefined;
  const batches: ApiInvocation[][] = [];
  const firstBatch = await fetchAllInvocationRecordPagesWithSnapshot(
    {
      from: rangeFrom,
      to: rangeTo,
      status: firstStatus,
      sortBy: "occurredAt",
      sortOrder: "desc",
      ...(upstreamAccountId != null ? { upstreamAccountId } : {}),
      signal,
    },
    fetchPage,
    TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE,
    initialSnapshotId,
  );
  batches.push(firstBatch.records);

  let snapshotId = firstBatch.snapshotId;
  for (const status of remainingStatuses) {
    const batch = await fetchAllInvocationRecordPagesWithSnapshot(
      {
        from: rangeFrom,
        to: rangeTo,
        status,
        sortBy: "occurredAt",
        sortOrder: "desc",
        ...(upstreamAccountId != null ? { upstreamAccountId } : {}),
        signal,
      },
      fetchPage,
      TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE,
      snapshotId,
    );
    if (snapshotId == null) {
      snapshotId = batch.snapshotId;
    }
    batches.push(batch.records);
  }
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
  const { records } = await fetchAllInvocationRecordPagesWithSnapshot(query, fetchPage, pageSize);
  return records;
}

async function fetchAllInvocationRecordPagesWithSnapshot(
  query: Omit<InvocationRecordsQuery, "page" | "pageSize">,
  fetchPage: (
    query: InvocationRecordsQuery,
  ) => Promise<InvocationRecordsResponse> = fetchInvocationRecords,
  pageSize = TIMESERIES_IN_FLIGHT_SEED_PAGE_SIZE,
  initialSnapshotId?: number,
): Promise<FetchAllInvocationRecordPagesResult> {
  const requestedPageSize = Math.max(1, pageSize);
  const records: ApiInvocation[] = [];
  let page = 1;
  let snapshotId = initialSnapshotId;

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
      typeof response.total === "number" && response.total >= 0 ? response.total : null;

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

  return { records, snapshotId };
}

function sameLiveRecordDelta(left: LiveRecordDelta | null, right: LiveRecordDelta | null) {
  if (left === right) return true;
  if (!left || !right) return false;
  return (
    left.bucketStart === right.bucketStart &&
    left.bucketEnd === right.bucketEnd &&
    left.recordId === right.recordId &&
    left.totalCount === right.totalCount &&
    left.successCount === right.successCount &&
    left.failureCount === right.failureCount &&
    left.inFlightCount === right.inFlightCount &&
    left.totalTokens === right.totalTokens &&
    left.totalCost === right.totalCost &&
    left.totalLatencyMs === right.totalLatencyMs &&
    left.totalLatencySampleCount === right.totalLatencySampleCount &&
    (left.countsOnly ?? false) === (right.countsOnly ?? false)
  );
}

function isPendingLiveRecord(record: ApiInvocation) {
  return normalizeLiveRecordOutcome(record) === "in_flight";
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

export function mergeFreshResponseLiveRecordDeltas(
  seededLiveRecordDeltas: Map<string, LiveRecordDelta>,
  previousLiveRecordDeltas: ReadonlyMap<string, LiveRecordDelta>,
  previousSettledLiveRecordUpdatedAt: ReadonlyMap<string, number>,
  snapshotId: number | undefined,
  now = Date.now(),
  ttlMs = TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
  maxEntries = MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
) {
  const nextLiveRecordDeltas = new Map(seededLiveRecordDeltas);
  const nextSettledLiveRecordUpdatedAt = new Map(previousSettledLiveRecordUpdatedAt);
  const preservedLiveRecordDeltas = cloneLiveRecordDeltaMap(previousLiveRecordDeltas);

  pruneTrackedTimeseriesLiveRecordDeltas(
    preservedLiveRecordDeltas,
    nextSettledLiveRecordUpdatedAt,
    now,
    ttlMs,
    maxEntries,
  );

  for (const [key, delta] of preservedLiveRecordDeltas) {
    if (nextSettledLiveRecordUpdatedAt.has(key)) {
      nextLiveRecordDeltas.set(key, cloneLiveRecordDelta(delta));
      continue;
    }

    if (
      typeof snapshotId === "number" &&
      typeof delta.recordId === "number" &&
      delta.recordId > snapshotId &&
      !nextLiveRecordDeltas.has(key)
    ) {
      nextLiveRecordDeltas.set(key, cloneLiveRecordDelta(delta));
    }
  }

  return {
    liveRecordDeltas: nextLiveRecordDeltas,
    settledLiveRecordUpdatedAt: nextSettledLiveRecordUpdatedAt,
  };
}

function adjustTimeseriesPoint(point: TimeseriesPoint, delta: LiveRecordDelta, sign: 1 | -1) {
  const previousLatencySampleCount = Math.max(point.totalLatencySampleCount ?? 0, 0);
  const previousLatencyTotal =
    previousLatencySampleCount > 0 &&
    typeof point.avgTotalMs === "number" &&
    Number.isFinite(point.avgTotalMs)
      ? point.avgTotalMs * previousLatencySampleCount
      : 0;

  point.bucketEnd = delta.bucketEnd;
  point.totalCount += sign * delta.totalCount;
  point.successCount += sign * delta.successCount;
  point.failureCount += sign * delta.failureCount;
  point.inFlightCount = Math.max((point.inFlightCount ?? 0) + sign * delta.inFlightCount, 0);
  point.totalTokens += sign * delta.totalTokens;
  point.totalCost += sign * delta.totalCost;
  const nextLatencySampleCount = Math.max(
    previousLatencySampleCount + sign * delta.totalLatencySampleCount,
    0,
  );
  const nextLatencyTotal = previousLatencyTotal + sign * delta.totalLatencyMs;
  point.totalLatencySampleCount = nextLatencySampleCount;
  point.avgTotalMs = nextLatencySampleCount > 0 ? nextLatencyTotal / nextLatencySampleCount : null;
  sanitizeTimeseriesPointLatency(point);
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
  nextDelta = reconcileCountsOnlyDelta(previousDelta, nextDelta, currentBucket);
  if (sameLiveRecordDelta(previousDelta, nextDelta)) {
    return { next: current, delta: nextDelta };
  }

  const nextPoints = current.points.map((point) => ({ ...point }));
  if (previousDelta) {
    const previousPoint = nextPoints.find((item) => item.bucketStart === previousDelta.bucketStart);
    if (previousPoint) {
      adjustTimeseriesPoint(previousPoint, previousDelta, -1);
    }
  }
  if (nextDelta) {
    const nextPoint = nextPoints.find((item) => item.bucketStart === nextDelta.bucketStart);
    if (!nextPoint) {
      return { next: current, delta: previousDelta };
    }
    adjustTimeseriesPoint(nextPoint, nextDelta, 1);
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

  const latestRangeEndEpoch = parseIsoEpoch(current.rangeEnd) ?? occurredEpoch + bucketSeconds;
  const closedRange = getClosedNaturalDayRangeBounds(context.range, latestRangeEndEpoch);
  if (closedRange && (occurredEpoch < closedRange.start || occurredEpoch >= closedRange.end)) {
    return null;
  }
  const earliestAllowed = getRangeStartEpoch(context.range, latestRangeEndEpoch);
  if (earliestAllowed != null && occurredEpoch < earliestAllowed) {
    return null;
  }

  const offsetSeconds = bucketSeconds >= 86_400 ? (context.settlementHour ?? 0) * 3_600 : 0;
  const bucketStartEpoch = alignBucketEpoch(occurredEpoch, bucketSeconds, offsetSeconds);
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

export function resolveCurrentDayLiveSeedRange(
  current: TimeseriesResponse,
  nowEpochSeconds = resolveCurrentDayLiveSeedEpoch(current),
) {
  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds);
  if (!currentBucket) {
    return null;
  }
  return {
    from: currentBucket.bucketStart,
    to: currentBucket.bucketEnd,
  };
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
    ? (current.points.find((point) => point.bucketStart === previousDelta.bucketStart) ?? null)
    : null;
  let nextDelta = buildTimeseriesLiveRecordDelta(record, current, context);
  nextDelta = reconcileCountsOnlyDelta(previousDelta, nextDelta, currentPoint);
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
      inFlightCount: 0,
      totalTokens: 0,
      totalCost: 0,
      avgTotalMs: null,
      totalLatencySampleCount: 0,
    };
    adjustTimeseriesPoint(point, delta, sign);
    const isEmpty =
      point.totalCount === 0 &&
      point.successCount === 0 &&
      point.failureCount === 0 &&
      getTimeseriesPointInFlightCount(point) === 0 &&
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
    nextRangeEndEpoch > 0 ? formatEpochToIso(nextRangeEndEpoch) : current.rangeEnd;
  const nextRangeStartEpoch =
    nextRangeEndEpoch > 0 ? getRangeStartEpoch(context.range, nextRangeEndEpoch) : null;
  const nextRangeStart =
    nextRangeStartEpoch != null ? formatEpochToIso(nextRangeStartEpoch) : current.rangeStart;

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
  const bucket = options?.bucket;
  const settlementHour = options?.settlementHour;
  const upstreamAccountId = options?.upstreamAccountId;

  const normalizedOptions = useMemo<UseTimeseriesOptions>(
    () => ({
      bucket,
      settlementHour,
      upstreamAccountId,
    }),
    [bucket, settlementHour, upstreamAccountId],
  );
  const supportsPureSse = range !== "yesterday";
  const topic = supportsPureSse
    ? buildTopicDescriptor("stats.timeseries.open-window", {
        range,
        bucket: normalizedOptions.bucket,
        settlementHour: normalizedOptions.settlementHour,
        upstreamAccountId: normalizedOptions.upstreamAccountId,
        timeZone: getBrowserTimeZone(),
      })
    : null;
  const sse = useSubscriptionTopic<TimeseriesResponse>(topic, supportsPureSse);
  const [httpData, setHttpData] = useState<TimeseriesResponse | null>(
    () => initialCachedTimeseries?.data ?? null,
  );
  const [httpLoading, setHttpLoading] = useState(
    () => !supportsPureSse && initialCachedTimeseries == null,
  );
  const [httpError, setHttpError] = useState<string | null>(null);

  const loadHttpTimeseries = useCallback(async () => {
    setHttpLoading(true);
    try {
      const response = await fetchTimeseries(range, normalizedOptions);
      setHttpData(response);
      writeTimeseriesRemountCache(range, normalizedOptions, response, Date.now());
      setHttpError(null);
    } catch (error) {
      setHttpError(error instanceof Error ? error.message : String(error));
    } finally {
      setHttpLoading(false);
    }
  }, [normalizedOptions, range]);

  useEffect(() => {
    if (!supportsPureSse) {
      void loadHttpTimeseries();
    }
  }, [loadHttpTimeseries, supportsPureSse]);

  useEffect(() => {
    if (supportsPureSse && sse.data) {
      writeTimeseriesRemountCache(range, normalizedOptions, sse.data, Date.now());
    }
  }, [normalizedOptions, range, sse.data, supportsPureSse]);

  const data = supportsPureSse ? (sse.data ?? initialCachedTimeseries?.data ?? null) : httpData;
  const isLoading = supportsPureSse ? sse.isLoading && data == null : httpLoading;
  const error = supportsPureSse ? sse.error : httpError;
  const refresh = supportsPureSse ? sse.refresh : loadHttpTimeseries;

  return {
    data,
    isLoading,
    error,
    refresh,
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

function getCurrentDayBucket(current: TimeseriesResponse | null, nowEpochSeconds: number) {
  if (!current) return null;
  for (let index = current.points.length - 1; index >= 0; index -= 1) {
    const point = current.points[index];
    const bucketStartEpoch = parseIsoEpoch(point.bucketStart);
    const bucketEndEpoch = parseIsoEpoch(point.bucketEnd);
    if (bucketStartEpoch == null || bucketEndEpoch == null) continue;
    if (nowEpochSeconds >= bucketStartEpoch && nowEpochSeconds < bucketEndEpoch) {
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
  const nextBucket = nextPoints.find((point) => point.bucketStart === currentBucket.bucketStart);
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
    } else if (outcome === "in_flight") {
      nextBucket.inFlightCount = (nextBucket.inFlightCount ?? 0) + 1;
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

  const offsetSeconds = bucketSeconds >= 86_400 ? (context.settlementHour ?? 0) * 3_600 : 0;
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
      const closedRange = getClosedNaturalDayRangeBounds(context.range, latestRangeEndEpoch);
      if (closedRange && (occurredEpoch < closedRange.start || occurredEpoch >= closedRange.end)) {
        continue;
      }
      const earliestAllowed = getRangeStartEpoch(context.range, latestRangeEndEpoch);
      if (earliestAllowed != null && occurredEpoch < earliestAllowed) {
        continue;
      }
    }

    const bucketStartEpoch = alignBucketEpoch(occurredEpoch, bucketSeconds, offsetSeconds);
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
        inFlightCount: 0,
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
    } else if (outcome === "in_flight") {
      point.inFlightCount = (point.inFlightCount ?? 0) + 1;
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
    const earliestAllowed = getRangeStartEpoch(context.range, latestRangeEndEpoch);
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

  const nextRangeEndEpoch = latestRangeEndEpoch ?? parseIsoEpoch(current.rangeEnd);
  const nextRangeEnd =
    nextRangeEndEpoch != null ? formatEpochToIso(nextRangeEndEpoch) : current.rangeEnd;
  const nextRangeStartEpoch =
    nextRangeEndEpoch != null ? getRangeStartEpoch(context.range, nextRangeEndEpoch) : null;
  const nextRangeStart =
    nextRangeStartEpoch != null ? formatEpochToIso(nextRangeStartEpoch) : current.rangeStart;

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

function alignBucketEpoch(epochSeconds: number, bucketSeconds: number, offsetSeconds: number) {
  const adjusted = epochSeconds - offsetSeconds;
  const aligned = Math.floor(adjusted / bucketSeconds) * bucketSeconds + offsetSeconds;
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
