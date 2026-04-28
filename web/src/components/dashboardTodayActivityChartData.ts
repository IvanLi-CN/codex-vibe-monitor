import type { TimeseriesResponse } from "../lib/api";
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
  failureCountNegative: number;
  chartSuccessCount: number | null;
  chartInFlightCount: number | null;
  chartFailureCountNegative: number | null;
  totalCount: number;
  totalCost: number;
  totalTokens: number;
  tokensPerMinute: number | null;
  spendRate: number | null;
  firstResponseByteTotalAvgMs: number | null;
  firstResponseByteTotalSampleCount: number;
  chartTokensPerMinute: number | null;
  chartSpendRate: number | null;
  chartFirstResponseByteTotalAvgMs: number | null;
  cumulativeCost: number | null;
  cumulativeTokens: number | null;
  chartCumulativeCost: number | null;
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
      totalCount: number;
      totalCost: number;
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
      totalCount: 0,
      totalCost: 0,
      totalTokens: 0,
      firstResponseByteTotalWeightedMs: 0,
      firstResponseByteTotalSampleCount: 0,
    };
    current.successCount += point.successCount ?? 0;
    current.failureCount += point.failureCount ?? 0;
    current.inFlightCount += Math.max(point.inFlightCount ?? 0, 0);
    current.totalCount += point.totalCount ?? 0;
    current.totalCost += point.totalCost ?? 0;
    current.totalTokens += point.totalTokens ?? 0;
    const firstResponseByteTotalAvgMs = point.firstResponseByteTotalAvgMs ?? null;
    const firstResponseByteTotalSampleCount =
      firstResponseByteTotalAvgMs == null
        ? 0
        : Math.max(point.firstResponseByteTotalSampleCount ?? 1, 1);
    if (firstResponseByteTotalAvgMs != null) {
      current.firstResponseByteTotalWeightedMs +=
        firstResponseByteTotalAvgMs * firstResponseByteTotalSampleCount;
      current.firstResponseByteTotalSampleCount += firstResponseByteTotalSampleCount;
    }
    pointMap.set(bucketEpoch, current);
  }

  const data: DashboardTodayMinuteDatum[] = [];
  let cumulativeCost = 0;
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
    const totalCount = Math.max(
      point?.totalCount ?? successCount + failureCount + inFlightCount,
      successCount + failureCount + inFlightCount,
    );
    const totalCost = point?.totalCost ?? 0;
    const totalTokens = point?.totalTokens ?? 0;
    const firstResponseByteTotalAvgMs =
      point == null || point.firstResponseByteTotalSampleCount <= 0
        ? null
        : point.firstResponseByteTotalWeightedMs /
          point.firstResponseByteTotalSampleCount;
    cumulativeCost += totalCost;
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
      failureCountNegative: failureCount > 0 ? -failureCount : 0,
      chartSuccessCount: isFuture ? null : successCount,
      chartInFlightCount: isFuture ? null : inFlightCount,
      chartFailureCountNegative: isFuture
        ? null
        : failureCount > 0
          ? -failureCount
          : 0,
      totalCount,
      totalCost,
      totalTokens,
      tokensPerMinute: isFuture ? null : totalTokens,
      spendRate: isFuture ? null : totalCost,
      firstResponseByteTotalAvgMs: isFuture ? null : firstResponseByteTotalAvgMs,
      firstResponseByteTotalSampleCount: isFuture
        ? 0
        : (point?.firstResponseByteTotalSampleCount ?? 0),
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: null,
      cumulativeCost: isFuture ? null : cumulativeCost,
      cumulativeTokens: isFuture ? null : cumulativeTokens,
      chartCumulativeCost: isFuture ? null : cumulativeCost,
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
    let firstByteWeightedMs = 0;
    let firstByteSampleCount = 0;

    for (const point of bucket) {
      if (point.tokensPerMinute == null || point.spendRate == null) continue;
      rateSampleMinutes += 1;
      totalTokens += point.tokensPerMinute;
      totalCost += point.spendRate;
      if (point.firstResponseByteTotalAvgMs != null) {
        const sampleCount = Math.max(point.firstResponseByteTotalSampleCount, 1);
        firstByteWeightedMs += point.firstResponseByteTotalAvgMs * sampleCount;
        firstByteSampleCount += sampleCount;
      }
    }

    bucketAnchor.chartTokensPerMinute =
      rateSampleMinutes > 0 ? totalTokens / rateSampleMinutes : null;
    bucketAnchor.chartSpendRate =
      rateSampleMinutes > 0 ? totalCost / rateSampleMinutes : null;
    bucketAnchor.chartFirstResponseByteTotalAvgMs =
      firstByteSampleCount > 0 ? firstByteWeightedMs / firstByteSampleCount : null;
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
