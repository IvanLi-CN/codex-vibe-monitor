import type { TimeseriesResponse } from "../lib/api";
import {
  parseDateInput,
  resolveClosedNaturalDayEnd,
} from "./dashboardNaturalDayWindow";

const MINUTE_MS = 60_000;

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
  cumulativeCost: number | null;
  cumulativeTokens: number | null;
  chartCumulativeCost: number | null;
  chartCumulativeTokens: number | null;
}

export function buildTodayMinuteChartData(
  response: TimeseriesResponse | null,
  options?: { now?: Date; localeTag?: string },
): DashboardTodayMinuteDatum[] {
  const localeTag = options?.localeTag ?? "en-US";
  const fallbackNow = options?.now ?? new Date();
  const anchor = resolveRangeAnchor(response, fallbackNow);
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
    };
    current.successCount += point.successCount ?? 0;
    current.failureCount += point.failureCount ?? 0;
    current.inFlightCount += Math.max(point.inFlightCount ?? 0, 0);
    current.totalCount += point.totalCount ?? 0;
    current.totalCost += point.totalCost ?? 0;
    current.totalTokens += point.totalTokens ?? 0;
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
      cumulativeCost: isFuture ? null : cumulativeCost,
      cumulativeTokens: isFuture ? null : cumulativeTokens,
      chartCumulativeCost: isFuture ? null : cumulativeCost,
      chartCumulativeTokens: isFuture ? null : cumulativeTokens,
    });
  }

  return data;
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
) {
  const rangeEnd = parseDateInput(response?.rangeEnd);
  if (!rangeEnd) {
    return floorToMinute(fallbackNow);
  }

  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(response);
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
