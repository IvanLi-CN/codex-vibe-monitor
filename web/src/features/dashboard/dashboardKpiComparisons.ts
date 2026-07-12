import type {
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
  StatsResponse,
  TimeseriesResponse,
} from "../../lib/api";

export interface ActiveMinuteAverages {
  activeMinutes: number;
  tokensPerMinute: number | null;
  spendRate: number | null;
}

export interface ParallelWorkKpiSnapshot {
  currentCount: number | null;
  dayAverage: number | null;
  yesterdayAverage: number | null;
}

export interface SameProgressUsageSnapshot {
  totalCost: number | null;
  totalTokens: number | null;
  successCount: number | null;
}

export interface SameProgressUsageOptions {
  timeZone?: string | null;
}

export function percentDelta(
  current: number | null | undefined,
  baseline: number | null | undefined,
) {
  if (current == null || baseline == null || baseline === 0) return null;
  return (current - baseline) / baseline;
}

export function failureRate(successCount: number, failureCount: number) {
  const terminalCount = successCount + failureCount;
  return terminalCount > 0 ? failureCount / terminalCount : 0;
}

export function cacheHitRate(cacheInputTokens: number, totalTokens: number) {
  return totalTokens > 0 ? cacheInputTokens / totalTokens : 0;
}

export function sumCacheInputTokens(response: TimeseriesResponse | null | undefined) {
  return (response?.points ?? []).reduce((sum, point) => sum + (point.cacheInputTokens ?? 0), 0);
}

function parseEpochMs(value: string | null | undefined) {
  if (!value) return null;
  const epochMs = Date.parse(value);
  return Number.isFinite(epochMs) ? epochMs : null;
}

function rangeEndEpochMs(response: TimeseriesResponse) {
  const explicitEnd = parseEpochMs(response.rangeEnd);
  if (explicitEnd != null) return explicitEnd;
  const lastPoint = response.points[response.points.length - 1];
  return parseEpochMs(lastPoint?.bucketEnd);
}

function localClockProgressMs(value: string, timeZone: string) {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).formatToParts(new Date(value));
    const partValue = (type: Intl.DateTimeFormatPartTypes) => {
      const raw = parts.find((part) => part.type === type)?.value;
      if (raw == null) return null;
      const parsed = Number.parseInt(raw, 10);
      return Number.isFinite(parsed) ? parsed : null;
    };
    const hour = partValue("hour");
    const minute = partValue("minute");
    const second = partValue("second");
    if (hour == null || minute == null || second == null) return null;
    return ((hour * 60 + minute) * 60 + second) * 1000;
  } catch {
    return null;
  }
}

export function buildSameProgressUsageSnapshot(
  current: TimeseriesResponse | null | undefined,
  comparison: TimeseriesResponse | null | undefined,
  options: SameProgressUsageOptions = {},
): SameProgressUsageSnapshot {
  const currentStart = parseEpochMs(current?.rangeStart);
  const currentEnd = current ? rangeEndEpochMs(current) : null;
  const comparisonStart = parseEpochMs(comparison?.rangeStart);
  const comparisonEnd = parseEpochMs(comparison?.rangeEnd);
  if (
    current == null ||
    comparison == null ||
    currentStart == null ||
    currentEnd == null ||
    comparisonStart == null ||
    currentEnd < currentStart
  ) {
    return {
      totalCost: null,
      totalTokens: null,
      successCount: null,
    };
  }

  const comparisonCutoff = comparisonStart + (currentEnd - currentStart);
  const currentEndIso = new Date(currentEnd).toISOString();
  const currentProgress = options.timeZone
    ? localClockProgressMs(currentEndIso, options.timeZone)
    : null;
  let reachedLocalProgressCutoff = false;
  const comparisonPoints = [...(comparison.points ?? [])].sort((left, right) => {
    return (parseEpochMs(left.bucketStart) ?? 0) - (parseEpochMs(right.bucketStart) ?? 0);
  });
  return comparisonPoints.reduce<SameProgressUsageSnapshot>(
    (snapshot, point) => {
      const bucketStart = parseEpochMs(point.bucketStart);
      const bucketEnd = parseEpochMs(point.bucketEnd);
      const outsideComparisonRange =
        bucketStart == null ||
        bucketEnd == null ||
        bucketStart < comparisonStart ||
        (comparisonEnd != null && bucketStart >= comparisonEnd);
      let outsideSameProgress = bucketEnd == null || bucketEnd > comparisonCutoff;
      if (currentProgress != null && options.timeZone) {
        if (reachedLocalProgressCutoff) {
          outsideSameProgress = true;
        } else {
          const bucketProgress = localClockProgressMs(point.bucketEnd, options.timeZone);
          outsideSameProgress = bucketProgress == null || bucketProgress > currentProgress;
          reachedLocalProgressCutoff = outsideSameProgress;
        }
      }
      if (outsideComparisonRange || outsideSameProgress) {
        return snapshot;
      }
      return {
        totalCost: (snapshot.totalCost ?? 0) + point.totalCost,
        totalTokens: (snapshot.totalTokens ?? 0) + point.totalTokens,
        successCount: (snapshot.successCount ?? 0) + (point.successCount ?? 0),
      };
    },
    {
      totalCost: 0,
      totalTokens: 0,
      successCount: 0,
    },
  );
}

export function buildActiveMinuteAverages(
  stats: StatsResponse | null | undefined,
  response: TimeseriesResponse | null | undefined,
): ActiveMinuteAverages {
  const activeMinutes = (response?.points ?? []).filter(
    (point) => (point.totalCount ?? 0) > 0,
  ).length;
  if (activeMinutes <= 0) {
    return {
      activeMinutes: 0,
      tokensPerMinute: null,
      spendRate: null,
    };
  }
  return {
    activeMinutes,
    tokensPerMinute: (stats?.totalTokens ?? 0) / activeMinutes,
    spendRate: (stats?.totalCost ?? 0) / activeMinutes,
  };
}

function latestParallelCount(window: ParallelWorkWindowResponse | null | undefined) {
  const points = window?.points ?? [];
  if (points.length === 0) return null;
  return points[points.length - 1]?.parallelCount ?? null;
}

export function buildParallelWorkKpiSnapshot(
  currentSummary: StatsResponse | null | undefined,
  currentParallelWork: ParallelWorkStatsResponse | null | undefined,
  yesterdayParallelWork: ParallelWorkStatsResponse | null | undefined,
  options: {
    preferSummaryCurrentCount?: boolean;
    allowParallelFallback?: boolean;
  } = {},
): ParallelWorkKpiSnapshot {
  const preferSummaryCurrentCount = options.preferSummaryCurrentCount ?? false;
  const allowParallelFallback = options.allowParallelFallback ?? true;
  const summaryCurrentCount = currentSummary?.inProgressConversationCount ?? null;

  return {
    currentCount:
      preferSummaryCurrentCount && summaryCurrentCount != null
        ? summaryCurrentCount
        : allowParallelFallback
          ? latestParallelCount(currentParallelWork?.current)
          : null,
    dayAverage: currentParallelWork?.current.avgCount ?? null,
    yesterdayAverage: yesterdayParallelWork?.current.avgCount ?? null,
  };
}

export function dividePerConversation(
  numerator: number | null | undefined,
  inProgressConversationCount: number | null | undefined,
) {
  if (
    numerator == null ||
    !Number.isFinite(numerator) ||
    inProgressConversationCount == null ||
    inProgressConversationCount <= 0
  ) {
    return null;
  }
  return numerator / inProgressConversationCount;
}

export function ratioOfCurrentToBaseline(
  current: number | null | undefined,
  baseline: number | null | undefined,
) {
  if (
    current == null ||
    !Number.isFinite(current) ||
    baseline == null ||
    !Number.isFinite(baseline) ||
    baseline <= 0
  ) {
    return null;
  }
  return current / baseline;
}
