import type { TimeseriesResponse } from "../../lib/api";
import { parseDateInput, resolveClosedNaturalDayEnd } from "./dashboardNaturalDayWindow";

const MINUTE_MS = 60_000;
interface RateBucket {
  bucketStartMs: number;
  bucketEndMs: number;
  totalTokens: number;
  totalCost: number;
}

export interface DashboardTodayRateSnapshot {
  tokensPerMinute: number;
  spendRate: number;
  windowMinutes: number;
  available: boolean;
}

export function buildDashboardTodayRateSnapshot(
  response: TimeseriesResponse | null,
  options?: { now?: Date; closedNaturalDay?: boolean },
): DashboardTodayRateSnapshot | null {
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
      tokensPerMinute: 0,
      spendRate: 0,
      windowMinutes: 0,
      available: true,
    };
  }

  const pointMap = new Map<number, { bucketEndMs: number; totalTokens: number; totalCost: number }>();

  for (const point of response.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart);
    const bucketEnd = parseDateInput(point.bucketEnd);
    if (!bucketStart || !bucketEnd) continue;

    const bucketStartMs = floorToMinute(bucketStart).getTime();
    const bucketEndMs = bucketEnd.getTime();
    if (bucketStartMs >= anchorMs || bucketEndMs <= startMs) continue;

    const current = pointMap.get(bucketStartMs) ?? {
      bucketEndMs,
      totalTokens: 0,
      totalCost: 0,
    };
    current.bucketEndMs = Math.max(current.bucketEndMs, bucketEndMs);
    current.totalTokens += point.totalTokens ?? 0;
    current.totalCost += point.totalCost ?? 0;
    pointMap.set(bucketStartMs, current);
  }

  const buckets = [...pointMap.entries()]
    .map(([bucketStartMs, bucket]) => ({ bucketStartMs, ...bucket }))
    .sort((a, b) => a.bucketStartMs - b.bucketStartMs);
  const tokensRate = computeRangeRate({
    buckets,
    anchorMs,
    startMs,
    value: (bucket) => bucket.totalTokens,
  });
  const costRate = computeRangeRate({
    buckets,
    anchorMs,
    startMs,
    value: (bucket) => bucket.totalCost,
  });

  return {
    tokensPerMinute: tokensRate.rate,
    spendRate: costRate.rate,
    windowMinutes: Math.max(tokensRate.windowMinutes, costRate.windowMinutes),
    available: true,
  };
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

function computeRangeRate({
  buckets,
  anchorMs,
  startMs,
  value,
}: {
  buckets: RateBucket[];
  anchorMs: number;
  startMs: number;
  value: (bucket: RateBucket) => number;
}) {
  const windowMinutes = Math.max((anchorMs - startMs) / MINUTE_MS, 0);
  let total = 0;
  for (const bucket of buckets) {
    if (bucket.bucketEndMs <= startMs || bucket.bucketStartMs >= anchorMs) continue;
    total += value(bucket);
  }

  return {
    rate: windowMinutes > 0 ? total / windowMinutes : 0,
    windowMinutes,
  };
}

function floorToMinute(date: Date) {
  const next = new Date(date);
  next.setSeconds(0, 0);
  return next;
}
