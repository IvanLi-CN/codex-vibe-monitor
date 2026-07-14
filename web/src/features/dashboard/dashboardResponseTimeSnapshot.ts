import type { TimeseriesResponse } from "../../lib/api";
import { parseDateInput, resolveClosedNaturalDayEnd } from "./dashboardNaturalDayWindow";

const MINUTE_MS = 60_000;

interface LatencyBucket {
  bucketStartMs: number;
  bucketEndMs: number;
  sampleCount: number;
  totalWeightedMs: number;
}

export interface DashboardResponseTimeSnapshot {
  responseTimeMs: number | null;
  dayAverageMs: number | null;
  available: boolean;
}

export function buildDashboardResponseTimeSnapshot(
  response: TimeseriesResponse | null,
  options?: { now?: Date; closedNaturalDay?: boolean },
): DashboardResponseTimeSnapshot | null {
  if (!response) {
    return null;
  }

  const fallbackNow = options?.now ?? new Date();
  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(
    response,
    options?.closedNaturalDay ?? false,
  );
  const responseEnd = parseDateInput(response.rangeEnd);
  const anchor = closedNaturalDayEnd ?? resolveLiveNaturalDayAnchor(responseEnd, fallbackNow);
  const start = floorToMinute(
    parseDateInput(response.rangeStart) ?? new Date(anchor.getTime() - 24 * 60 * MINUTE_MS),
  );
  const startMs = start.getTime();
  const anchorMs = anchor.getTime();

  if (anchorMs <= startMs) {
    return {
      responseTimeMs: null,
      dayAverageMs: null,
      available: true,
    };
  }

  const pointMap = new Map<number, LatencyBucket>();

  for (const point of response.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart);
    const bucketEnd = parseDateInput(point.bucketEnd);
    if (!bucketStart || !bucketEnd) continue;

    const bucketStartMs = floorToMinute(bucketStart).getTime();
    const bucketEndMs = bucketEnd.getTime();
    if (bucketStartMs >= anchorMs || bucketEndMs <= startMs) continue;

    const avgMs = point.firstResponseByteTotalAvgMs ?? null;
    const pointCallCount = Math.max(
      point.totalCount ?? 0,
      (point.successCount ?? 0) + (point.failureCount ?? 0) + Math.max(point.inFlightCount ?? 0, 0),
      0,
    );
    if (pointCallCount <= 0 || avgMs == null || !Number.isFinite(avgMs)) continue;

    const sampleCount = Math.max(point.firstResponseByteTotalSampleCount ?? 1, 1);

    const current = pointMap.get(bucketStartMs) ?? {
      bucketStartMs,
      bucketEndMs,
      sampleCount: 0,
      totalWeightedMs: 0,
    };
    current.bucketEndMs = Math.max(current.bucketEndMs, bucketEndMs);
    current.sampleCount += sampleCount;
    current.totalWeightedMs += avgMs * sampleCount;
    pointMap.set(bucketStartMs, current);
  }

  const buckets = [...pointMap.values()].sort(
    (left, right) => left.bucketStartMs - right.bucketStartMs,
  );

  return {
    responseTimeMs: computeWeightedAverage(buckets, startMs, anchorMs),
    dayAverageMs: computeWeightedAverage(buckets, startMs, anchorMs),
    available: true,
  };
}

function computeWeightedAverage(buckets: LatencyBucket[], startMs: number, endMs: number) {
  let totalWeightedMs = 0;
  let sampleCount = 0;

  for (const bucket of buckets) {
    if (bucket.bucketEndMs <= startMs || bucket.bucketStartMs >= endMs) {
      continue;
    }
    totalWeightedMs += bucket.totalWeightedMs;
    sampleCount += bucket.sampleCount;
  }

  if (sampleCount <= 0) return null;
  return totalWeightedMs / sampleCount;
}

function resolveLiveNaturalDayAnchor(responseEnd: Date | null, now: Date) {
  if (!responseEnd) return now;
  if (isSameLocalDay(responseEnd, now) && now.getTime() > responseEnd.getTime()) {
    return now;
  }
  return responseEnd;
}

function isSameLocalDay(left: Date, right: Date) {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

function floorToMinute(date: Date) {
  const next = new Date(date);
  next.setSeconds(0, 0);
  return next;
}
