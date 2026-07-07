import type { TimeseriesResponse } from "../../lib/api";
import {
  parseDateInput,
  resolveClosedNaturalDayEnd,
} from "./dashboardNaturalDayWindow";

const MINUTE_MS = 60_000;
const TREND_CHART_BUCKET_MINUTES = 10;

export interface DashboardTodayMinuteDatum {
  index: number;
  epochMs: number;
  label: string;
  tooltipLabel: string;
  successCount: number;
  failureCount: number;
  inFlightCount: number;
  queuedInFlightCount: number;
  runningInFlightCount: number;
  failureCountNegative: number;
  chartSuccessCount: number | null;
  chartInFlightCount: number | null;
  chartQueuedInFlightCount: number | null;
  chartRunningInFlightCount: number | null;
  chartFailureCountNegative: number | null;
  totalCount: number;
  totalCost: number;
  successCost: number;
  nonSuccessCost: number;
  totalTokens: number;
  tokensPerMinute: number | null;
  spendRate: number | null;
  firstResponseByteTotalAvgMs: number | null;
  firstResponseByteTotalSampleCount: number;
  chartTokensPerMinute: number | null;
  chartSpendRate: number | null;
  chartFirstResponseByteTotalAvgMs: number | null;
  cumulativeCost: number | null;
  cumulativeSuccessCost: number | null;
  cumulativeNonSuccessCost: number | null;
  cumulativeTokens: number | null;
  chartCumulativeCost: number | null;
  chartCumulativeSuccessCost: number | null;
  chartCumulativeNonSuccessCost: number | null;
  chartCumulativeTokens: number | null;
}

export function buildTodayMinuteChartData(
  response: TimeseriesResponse | null,
  options?: { now?: Date; localeTag?: string; closedNaturalDay?: boolean },
): DashboardTodayMinuteDatum[] {
  const localeTag = options?.localeTag ?? "en-US";
  const fallbackNow = options?.now ?? new Date();
  const anchor = resolveRangeAnchor(
    response,
    fallbackNow,
    options?.closedNaturalDay ?? false,
  );
  const start = startOfLocalDay(anchor);
  const end = endOfLocalDay(anchor);

  const startMs = start.getTime();
  const endMs = end.getTime();
  if (endMs < startMs) return [];

  const timeFormatter = new Intl.DateTimeFormat(localeTag, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  });
  const tooltipFormatter = new Intl.DateTimeFormat(localeTag, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  });

  const pointMap = new Map<
    number,
    {
      successCount: number;
      failureCount: number;
      inFlightCount: number;
      queuedInFlightCount: number;
      runningInFlightCount: number;
      totalCount: number;
      totalCost: number;
      nonSuccessCost: number;
      totalTokens: number;
      firstResponseByteTotalWeightedMs: number;
      firstResponseByteTotalSampleCount: number;
    }
  >();

  for (const point of response?.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart);
    if (!bucketStart) continue;
    const bucketEpoch = floorToMinute(bucketStart).getTime();
    if (bucketEpoch < startMs || bucketEpoch > endMs) continue;
    const current = pointMap.get(bucketEpoch) ?? {
      successCount: 0,
      failureCount: 0,
      inFlightCount: 0,
      queuedInFlightCount: 0,
      runningInFlightCount: 0,
      totalCount: 0,
      totalCost: 0,
      nonSuccessCost: 0,
      totalTokens: 0,
      firstResponseByteTotalWeightedMs: 0,
      firstResponseByteTotalSampleCount: 0,
    };
    current.successCount += point.successCount ?? 0;
    current.failureCount += point.failureCount ?? 0;
    const pointInFlightCount = Math.max(point.inFlightCount ?? 0, 0);
    const phaseCounts = point.inFlightPhaseCounts ?? null;
    const queuedInFlightCount = Math.max(phaseCounts?.queued ?? 0, 0);
    const explicitRunningInFlightCount =
      Math.max(phaseCounts?.requesting ?? 0, 0) +
      Math.max(phaseCounts?.responding ?? 0, 0);
    const phaseTotal = queuedInFlightCount + explicitRunningInFlightCount;
    const runningInFlightCount =
      phaseTotal > 0 || pointInFlightCount <= 0
        ? explicitRunningInFlightCount
        : pointInFlightCount;
    current.queuedInFlightCount += queuedInFlightCount;
    current.runningInFlightCount += runningInFlightCount;
    current.inFlightCount += Math.max(
      pointInFlightCount,
      queuedInFlightCount + runningInFlightCount,
    );
    current.totalCount += point.totalCount ?? 0;
    current.totalCost += point.totalCost ?? 0;
    current.nonSuccessCost += point.nonSuccessCost ?? 0;
    current.totalTokens += point.totalTokens ?? 0;
    const firstResponseByteTotalAvgMs = point.firstResponseByteTotalAvgMs ?? null;
    const pointCallCount = Math.max(
      point.totalCount ?? 0,
      (point.successCount ?? 0) + (point.failureCount ?? 0) + pointInFlightCount,
      0,
    );
    const firstResponseByteTotalSampleCount =
      pointCallCount <= 0 || firstResponseByteTotalAvgMs == null
        ? 0
        : Math.max(point.firstResponseByteTotalSampleCount ?? 1, 1);
    if (firstResponseByteTotalAvgMs != null && firstResponseByteTotalSampleCount > 0) {
      current.firstResponseByteTotalWeightedMs +=
        firstResponseByteTotalAvgMs * firstResponseByteTotalSampleCount;
      current.firstResponseByteTotalSampleCount += firstResponseByteTotalSampleCount;
    }
    pointMap.set(bucketEpoch, current);
  }

  const data: DashboardTodayMinuteDatum[] = [];
  let cumulativeCost = 0;
  let cumulativeSuccessCost = 0;
  let cumulativeNonSuccessCost = 0;
  let cumulativeTokens = 0;

  for (
    let epochMs = startMs, index = 0;
    epochMs <= endMs;
    epochMs += MINUTE_MS, index += 1
  ) {
    const point = pointMap.get(epochMs);
    const isFuture = epochMs > anchor.getTime();
    const successCount = point?.successCount ?? 0;
    const failureCount = point?.failureCount ?? 0;
    const inFlightCount = Math.max(point?.inFlightCount ?? 0, 0);
    const queuedInFlightCount = Math.max(point?.queuedInFlightCount ?? 0, 0);
    const runningInFlightCount =
      point == null
        ? 0
        : Math.max(
            point.runningInFlightCount,
            inFlightCount > 0 && queuedInFlightCount + point.runningInFlightCount <= 0
              ? inFlightCount
              : 0,
          );
    const totalCount = Math.max(
      point?.totalCount ?? successCount + failureCount + inFlightCount,
      successCount + failureCount + inFlightCount,
    );
    const totalCost = point?.totalCost ?? 0;
    const nonSuccessCost = Math.max(0, point?.nonSuccessCost ?? 0);
    // Some sources (for example CRS relay deltas) only report total cost plus
    // success/failure counts, so the success-side layer is the remaining
    // cumulative cost after subtracting explicit non-success usage.
    const successCost = Math.max(0, totalCost - nonSuccessCost);
    const totalTokens = point?.totalTokens ?? 0;
    const firstResponseByteTotalAvgMs =
      point == null || point.firstResponseByteTotalSampleCount <= 0
        ? null
        : point.firstResponseByteTotalWeightedMs /
          point.firstResponseByteTotalSampleCount;
    cumulativeCost += totalCost;
    cumulativeSuccessCost += successCost;
    cumulativeNonSuccessCost += nonSuccessCost;
    cumulativeTokens += totalTokens;

    const currentDate = new Date(epochMs);
    data.push({
      index,
      epochMs,
      label: normalizeFormattedMidnight(timeFormatter.format(currentDate)),
      tooltipLabel: normalizeFormattedMidnight(
        tooltipFormatter.format(currentDate),
      ),
      successCount,
      failureCount,
      inFlightCount,
      queuedInFlightCount,
      runningInFlightCount,
      failureCountNegative: failureCount > 0 ? -failureCount : 0,
      chartSuccessCount: isFuture ? null : successCount,
      chartInFlightCount: isFuture ? null : inFlightCount,
      chartQueuedInFlightCount: isFuture ? null : queuedInFlightCount,
      chartRunningInFlightCount: isFuture ? null : runningInFlightCount,
      chartFailureCountNegative: isFuture
        ? null
        : failureCount > 0
          ? -failureCount
          : 0,
      totalCount,
      totalCost,
      successCost,
      nonSuccessCost,
      totalTokens,
      tokensPerMinute: isFuture ? null : totalTokens,
      spendRate: isFuture ? null : totalCost,
      firstResponseByteTotalAvgMs: isFuture ? null : firstResponseByteTotalAvgMs,
      firstResponseByteTotalSampleCount: isFuture
        ? 0
        : (point?.firstResponseByteTotalSampleCount ?? 0),
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: isFuture
        ? null
        : firstResponseByteTotalAvgMs,
      cumulativeCost: isFuture ? null : cumulativeCost,
      cumulativeSuccessCost: isFuture ? null : cumulativeSuccessCost,
      cumulativeNonSuccessCost: isFuture ? null : cumulativeNonSuccessCost,
      cumulativeTokens: isFuture ? null : cumulativeTokens,
      chartCumulativeCost: isFuture ? null : cumulativeCost,
      chartCumulativeSuccessCost: isFuture ? null : cumulativeSuccessCost,
      chartCumulativeNonSuccessCost: isFuture ? null : cumulativeNonSuccessCost,
      chartCumulativeTokens: isFuture ? null : cumulativeTokens,
    });
  }

  applyTenMinuteChartBuckets(data);

  return data;
}

function applyTenMinuteChartBuckets(data: DashboardTodayMinuteDatum[]) {
  for (
    let bucketStart = 0;
    bucketStart < data.length;
    bucketStart += TREND_CHART_BUCKET_MINUTES
  ) {
    const bucket = data.slice(
      bucketStart,
      bucketStart + TREND_CHART_BUCKET_MINUTES,
    );
    const bucketAnchor = bucket[0];
    if (!bucketAnchor || bucketAnchor.tokensPerMinute == null) continue;

    let totalTokens = 0;
    let totalCost = 0;
    let rateSampleMinutes = 0;

    for (const point of bucket) {
      if (point.tokensPerMinute == null || point.spendRate == null) continue;
      rateSampleMinutes += 1;
      totalTokens += point.tokensPerMinute;
      totalCost += point.spendRate;
    }

    bucketAnchor.chartTokensPerMinute =
      rateSampleMinutes > 0 ? totalTokens / rateSampleMinutes : null;
    bucketAnchor.chartSpendRate =
      rateSampleMinutes > 0 ? totalCost / rateSampleMinutes : null;
  }
}

function startOfLocalDay(date: Date) {
  const next = new Date(date);
  next.setHours(0, 0, 0, 0);
  return next;
}

function endOfLocalDay(date: Date) {
  const next = new Date(date);
  next.setHours(23, 59, 0, 0);
  return next;
}

function floorToMinute(date: Date) {
  const next = new Date(date);
  next.setSeconds(0, 0);
  return next;
}

function resolveRangeAnchor(
  response: TimeseriesResponse | null,
  fallbackNow: Date,
  closedNaturalDay: boolean,
) {
  const rangeEnd = parseDateInput(response?.rangeEnd);
  if (!rangeEnd) {
    return floorToMinute(fallbackNow);
  }

  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(
    response,
    closedNaturalDay,
  );
  if (closedNaturalDayEnd) {
    return new Date(closedNaturalDayEnd.getTime() - MINUTE_MS);
  }

  return floorToMinute(rangeEnd);
}

function normalizeFormattedMidnight(value: string) {
  return value.replace(
    /(^|\D)24:(\d{2})/g,
    (_match, prefix: string, minutes: string) => `${prefix}00:${minutes}`,
  );
}
