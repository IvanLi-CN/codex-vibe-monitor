import type { TimeseriesResponse } from "../../lib/api";
import { parseDateInput, resolveClosedNaturalDayEnd } from "./dashboardNaturalDayWindow";

const MINUTE_MS = 60_000;
const TREND_CHART_BUCKET_MINUTES = 10;
const HOURLY_CACHE_HIT_WINDOW_MINUTES = 60;

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
  inputTokens: number | null;
  outputTokens: number | null;
  cacheInputTokens: number | null;
  reasoningTokens: number | null;
  tokensPerMinute: number | null;
  spendRate: number | null;
  firstTokenAvgMs: number | null;
  firstTokenSampleCount: number;
  chartTokensPerMinute: number | null;
  chartSpendRate: number | null;
  chartFirstTokenAvgMs: number | null;
  cumulativeCost: number | null;
  cumulativeSuccessCost: number | null;
  cumulativeNonSuccessCost: number | null;
  cumulativeTokens: number | null;
  cumulativeCacheReadTokens: number | null;
  cumulativeCacheWriteTokens: number | null;
  cumulativeOutputTokens: number | null;
  cumulativeReasoningTokens: number | null;
  cacheHitRate: number | null;
  hourlyCacheHitRate: number | null;
  chartCumulativeCost: number | null;
  chartCumulativeSuccessCost: number | null;
  chartCumulativeNonSuccessCost: number | null;
  chartCumulativeTokens: number | null;
  chartCumulativeCacheReadTokens: number | null;
  chartCumulativeCacheWriteTokens: number | null;
  chartCumulativeOutputTokens: number | null;
  chartCumulativeReasoningTokens: number | null;
  chartCacheHitRate: number | null;
  chartHourlyCacheHitRate: number | null;
}

export function buildTodayMinuteChartData(
  response: TimeseriesResponse | null,
  options?: { now?: Date; localeTag?: string; closedNaturalDay?: boolean },
): DashboardTodayMinuteDatum[] {
  const localeTag = options?.localeTag ?? "en-US";
  const fallbackNow = options?.now ?? new Date();
  const anchor = resolveRangeAnchor(response, fallbackNow, options?.closedNaturalDay ?? false);
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
      inputTokens: number | null;
      outputTokens: number | null;
      cacheInputTokens: number | null;
      reasoningTokens: number | null;
      firstTokenWeightedMs: number;
      firstTokenSampleCount: number;
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
      inputTokens: 0,
      outputTokens: 0,
      cacheInputTokens: 0,
      reasoningTokens: 0,
      firstTokenWeightedMs: 0,
      firstTokenSampleCount: 0,
    };
    current.successCount += point.successCount ?? 0;
    current.failureCount += point.failureCount ?? 0;
    const pointInFlightCount = Math.max(point.inFlightCount ?? 0, 0);
    const phaseCounts = point.inFlightPhaseCounts ?? null;
    const queuedInFlightCount = Math.max(phaseCounts?.queued ?? 0, 0);
    const explicitRunningInFlightCount =
      Math.max(phaseCounts?.requesting ?? 0, 0) + Math.max(phaseCounts?.responding ?? 0, 0);
    const phaseTotal = queuedInFlightCount + explicitRunningInFlightCount;
    const runningInFlightCount =
      phaseTotal > 0 || pointInFlightCount <= 0 ? explicitRunningInFlightCount : pointInFlightCount;
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
    if (
      point.inputTokens == null ||
      point.outputTokens == null ||
      point.cacheInputTokens == null ||
      point.reasoningTokens == null
    ) {
      current.inputTokens = null;
      current.outputTokens = null;
      current.cacheInputTokens = null;
      current.reasoningTokens = null;
    } else if (
      current.inputTokens != null &&
      current.outputTokens != null &&
      current.cacheInputTokens != null &&
      current.reasoningTokens != null
    ) {
      current.inputTokens += point.inputTokens;
      current.outputTokens += point.outputTokens;
      current.cacheInputTokens += point.cacheInputTokens;
      current.reasoningTokens += point.reasoningTokens;
    }
    const firstTokenAvgMs = point.firstTokenAvgMs ?? null;
    const pointCallCount = Math.max(
      point.totalCount ?? 0,
      (point.successCount ?? 0) + (point.failureCount ?? 0) + pointInFlightCount,
      0,
    );
    const firstTokenSampleCount =
      pointCallCount <= 0 || firstTokenAvgMs == null
        ? 0
        : Math.max(point.firstTokenSampleCount ?? 1, 1);
    if (firstTokenAvgMs != null && firstTokenSampleCount > 0) {
      current.firstTokenWeightedMs += firstTokenAvgMs * firstTokenSampleCount;
      current.firstTokenSampleCount += firstTokenSampleCount;
    }
    pointMap.set(bucketEpoch, current);
  }

  const data: DashboardTodayMinuteDatum[] = [];
  let cumulativeCost = 0;
  let cumulativeSuccessCost = 0;
  let cumulativeNonSuccessCost = 0;
  let cumulativeTokens = 0;
  let cumulativeCacheReadTokens = 0;
  let cumulativeCacheWriteTokens = 0;
  let cumulativeOutputTokens = 0;
  let cumulativeReasoningTokens = 0;
  const hasCompleteTokenBreakdown = Array.from(pointMap.values()).every((point) => {
    if (point.totalTokens <= 0) return true;
    return (
      point.inputTokens != null &&
      point.outputTokens != null &&
      point.cacheInputTokens != null &&
      point.reasoningTokens != null &&
      point.inputTokens >= 0 &&
      point.outputTokens >= 0 &&
      point.inputTokens + point.outputTokens === point.totalTokens
    );
  });

  for (let epochMs = startMs, index = 0; epochMs <= endMs; epochMs += MINUTE_MS, index += 1) {
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
    const inputTokens = hasCompleteTokenBreakdown ? (point?.inputTokens ?? 0) : null;
    const outputTokens = hasCompleteTokenBreakdown ? (point?.outputTokens ?? 0) : null;
    const cacheInputTokens = hasCompleteTokenBreakdown ? (point?.cacheInputTokens ?? 0) : null;
    const reasoningTokens = hasCompleteTokenBreakdown ? (point?.reasoningTokens ?? 0) : null;
    const cacheReadTokens = Math.min(Math.max(cacheInputTokens ?? 0, 0), inputTokens ?? 0);
    const cacheWriteTokens = Math.max((inputTokens ?? 0) - cacheReadTokens, 0);
    const clampedReasoningTokens = Math.min(Math.max(reasoningTokens ?? 0, 0), outputTokens ?? 0);
    const visibleOutputTokens = Math.max((outputTokens ?? 0) - clampedReasoningTokens, 0);
    const firstTokenAvgMs =
      point == null || point.firstTokenSampleCount <= 0
        ? null
        : point.firstTokenWeightedMs / point.firstTokenSampleCount;
    cumulativeCost += totalCost;
    cumulativeSuccessCost += successCost;
    cumulativeNonSuccessCost += nonSuccessCost;
    cumulativeTokens += totalTokens;
    cumulativeCacheReadTokens += cacheReadTokens;
    cumulativeCacheWriteTokens += cacheWriteTokens;
    cumulativeOutputTokens += visibleOutputTokens;
    cumulativeReasoningTokens += clampedReasoningTokens;

    const rollingCacheHitRate = (windowMinutes: number) => {
      if (!hasCompleteTokenBreakdown || isFuture) return null;

      const rollingWindowStart = Math.max(startMs, epochMs - (windowMinutes - 1) * MINUTE_MS);
      let rollingCacheTokens = 0;
      let rollingTotalTokens = 0;
      for (let cursor = rollingWindowStart; cursor <= epochMs; cursor += MINUTE_MS) {
        const rollingPoint = pointMap.get(cursor);
        rollingCacheTokens += Math.max(rollingPoint?.cacheInputTokens ?? 0, 0);
        rollingTotalTokens += Math.max(rollingPoint?.totalTokens ?? 0, 0);
      }
      return rollingTotalTokens > 0 ? rollingCacheTokens / rollingTotalTokens : null;
    };
    const cacheHitRate = rollingCacheHitRate(TREND_CHART_BUCKET_MINUTES);
    const hourlyCacheHitRate = rollingCacheHitRate(HOURLY_CACHE_HIT_WINDOW_MINUTES);

    const currentDate = new Date(epochMs);
    data.push({
      index,
      epochMs,
      label: normalizeFormattedMidnight(timeFormatter.format(currentDate)),
      tooltipLabel: normalizeFormattedMidnight(tooltipFormatter.format(currentDate)),
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
      chartFailureCountNegative: isFuture ? null : failureCount > 0 ? -failureCount : 0,
      totalCount,
      totalCost,
      successCost,
      nonSuccessCost,
      totalTokens,
      inputTokens,
      outputTokens,
      cacheInputTokens,
      reasoningTokens,
      tokensPerMinute: isFuture ? null : totalTokens,
      spendRate: isFuture ? null : totalCost,
      firstTokenAvgMs: isFuture ? null : firstTokenAvgMs,
      firstTokenSampleCount: isFuture ? 0 : (point?.firstTokenSampleCount ?? 0),
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstTokenAvgMs: isFuture ? null : firstTokenAvgMs,
      cumulativeCost: isFuture ? null : cumulativeCost,
      cumulativeSuccessCost: isFuture ? null : cumulativeSuccessCost,
      cumulativeNonSuccessCost: isFuture ? null : cumulativeNonSuccessCost,
      cumulativeTokens: isFuture ? null : cumulativeTokens,
      cumulativeCacheReadTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeCacheReadTokens,
      cumulativeCacheWriteTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeCacheWriteTokens,
      cumulativeOutputTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeOutputTokens,
      cumulativeReasoningTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeReasoningTokens,
      cacheHitRate: isFuture || !hasCompleteTokenBreakdown ? null : cacheHitRate,
      hourlyCacheHitRate: isFuture || !hasCompleteTokenBreakdown ? null : hourlyCacheHitRate,
      chartCumulativeCost: isFuture ? null : cumulativeCost,
      chartCumulativeSuccessCost: isFuture ? null : cumulativeSuccessCost,
      chartCumulativeNonSuccessCost: isFuture ? null : cumulativeNonSuccessCost,
      chartCumulativeTokens: isFuture ? null : cumulativeTokens,
      chartCumulativeCacheReadTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeCacheReadTokens,
      chartCumulativeCacheWriteTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeCacheWriteTokens,
      chartCumulativeOutputTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeOutputTokens,
      chartCumulativeReasoningTokens:
        isFuture || !hasCompleteTokenBreakdown ? null : cumulativeReasoningTokens,
      chartCacheHitRate: isFuture || !hasCompleteTokenBreakdown ? null : cacheHitRate,
      chartHourlyCacheHitRate: isFuture || !hasCompleteTokenBreakdown ? null : hourlyCacheHitRate,
    });
  }

  applyTenMinuteChartBuckets(data);

  return data;
}

function applyTenMinuteChartBuckets(data: DashboardTodayMinuteDatum[]) {
  for (let bucketStart = 0; bucketStart < data.length; bucketStart += TREND_CHART_BUCKET_MINUTES) {
    const bucket = data.slice(bucketStart, bucketStart + TREND_CHART_BUCKET_MINUTES);
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
    bucketAnchor.chartSpendRate = rateSampleMinutes > 0 ? totalCost / rateSampleMinutes : null;
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

  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(response, closedNaturalDay);
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
