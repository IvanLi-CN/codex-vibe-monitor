import { useCallback, useEffect, useState } from "react";
import type { ApiInvocation, StatsResponse } from "../lib/api";
import { fetchSummary } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { getBrowserTimeZone } from "../lib/timeZone";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

interface UseSummaryOptions {
  limit?: number;
  upstreamAccountId?: number;
}

const SUPPORTED_SSE_WINDOWS = new Set(["all", "30m", "1h", "1d", "1mo"]);
const CALENDAR_SUMMARY_WINDOWS = new Set([
  "today",
  "yesterday",
  "thisWeek",
  "thisMonth",
  "previous7d",
]);
const DAY_BOUNDARY_SUMMARY_WINDOWS = new Set(["today", "yesterday", "previous7d"]);
export const UNSUPPORTED_SSE_REFRESH_INTERVAL_MS = 60_000;
export const CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 5_000;
export const CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 5_000;
export const CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS = 5_000;
export const CURRENT_SUMMARY_REQUEST_TIMEOUT_MS = 10_000;
export const CURRENT_SUMMARY_RETRY_DELAY_MS = 2_000;
export const CURRENT_SUMMARY_MAX_RETRY_ATTEMPTS = 3;
export const SUMMARY_REMOUNT_CACHE_TTL_MS = 30_000;

export interface UnsupportedRefreshGate {
  inFlight: boolean;
  lastTriggerAt: number;
}

interface SummaryRemountCacheEntry {
  stats: StatsResponse;
  cachedAt: number;
}

const summaryRemountCache = new Map<string, SummaryRemountCacheEntry>();

export function createUnsupportedRefreshGate(): UnsupportedRefreshGate {
  return { inFlight: false, lastTriggerAt: 0 };
}

export function getSummaryRemountCacheKey(
  window: string,
  limit?: number,
  upstreamAccountId?: number,
) {
  return `${window}::${limit ?? "default"}::${upstreamAccountId == null ? "global" : `account:${upstreamAccountId}`}`;
}

export function shouldEnableSummaryRemountCache(window: string) {
  return window !== "current" && !isCalendarSummaryWindow(window);
}

export function readSummaryRemountCache(
  window: string,
  limit?: number,
  now = Date.now(),
  ttlMs = SUMMARY_REMOUNT_CACHE_TTL_MS,
  upstreamAccountId?: number,
) {
  if (!shouldEnableSummaryRemountCache(window)) return null;
  const cached = summaryRemountCache.get(
    getSummaryRemountCacheKey(window, limit, upstreamAccountId),
  );
  if (!cached) return null;
  return shouldReuseSummaryRemountCache(cached.cachedAt, now, ttlMs) ? cached : null;
}

export function writeSummaryRemountCache(
  window: string,
  limit: number | undefined,
  stats: StatsResponse,
  cachedAt = Date.now(),
  upstreamAccountId?: number,
) {
  if (!shouldEnableSummaryRemountCache(window)) return;
  summaryRemountCache.set(getSummaryRemountCacheKey(window, limit, upstreamAccountId), {
    stats,
    cachedAt,
  });
}

export function clearSummaryRemountCache() {
  summaryRemountCache.clear();
}

export function shouldReuseSummaryRemountCache(
  cachedAt: number,
  now: number,
  ttlMs = SUMMARY_REMOUNT_CACHE_TTL_MS,
) {
  return now - cachedAt < ttlMs;
}

export function isCalendarSummaryWindow(window: string) {
  return CALENDAR_SUMMARY_WINDOWS.has(window);
}

function isDayBoundarySummaryWindow(window: string) {
  return DAY_BOUNDARY_SUMMARY_WINDOWS.has(window);
}

export function getCurrentSummarySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - (now - lastRefreshAt));
}

function getLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(0, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

function getNextLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000);
  value.setHours(24, 0, 0, 0);
  return Math.floor(value.getTime() / 1000);
}

export function shouldRefreshCalendarSummaryOnRecords(window: string) {
  return window === "today" || window === "thisWeek" || window === "thisMonth";
}

export function shouldRefreshYesterdaySummaryOnRecords(
  records: Array<Pick<ApiInvocation, "occurredAt">>,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const rangeEndEpoch = getLocalDayStartEpoch(nowEpochSeconds);
  const rangeStartEpoch = getLocalDayStartEpoch(rangeEndEpoch - 1);
  return records.some((record) => {
    const occurredEpochMs = Date.parse(record.occurredAt ?? "");
    if (!Number.isFinite(occurredEpochMs)) {
      return false;
    }
    const occurredEpoch = Math.floor(occurredEpochMs / 1000);
    return occurredEpoch >= rangeStartEpoch && occurredEpoch < rangeEndEpoch;
  });
}

export function shouldRefreshScopedSummaryOnRecords(
  window: string,
  records: Array<Pick<ApiInvocation, "occurredAt">>,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (window === "current") return false;
  if (window === "yesterday") {
    return shouldRefreshYesterdaySummaryOnRecords(records, nowEpochSeconds);
  }
  return (
    shouldRefreshCalendarSummaryOnRecords(window) ||
    SUPPORTED_SSE_WINDOWS.has(window) ||
    window === "7d"
  );
}

export function shouldForceCalendarSummaryOpenResync(
  window: string,
  lastLoadedLocalDayStartEpoch: number | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (!isDayBoundarySummaryWindow(window)) {
    return false;
  }
  return lastLoadedLocalDayStartEpoch !== getLocalDayStartEpoch(nowEpochSeconds);
}

export function getCalendarSummaryDayRolloverRefreshEpoch(
  window: string,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (!isDayBoundarySummaryWindow(window)) {
    return null;
  }
  return getNextLocalDayStartEpoch(nowEpochSeconds);
}

export function mergePendingSummarySilentOption(
  existingSilent: boolean | null,
  incomingSilent: boolean,
) {
  return (existingSilent ?? true) && incomingSilent;
}

export function shouldTriggerCurrentSummaryOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS;
}

export function shouldHandleUnsupportedSummaryRefresh(
  payloadWindow: string,
  currentWindow: string,
  supportsSse: boolean,
): boolean {
  return (
    payloadWindow !== currentWindow &&
    !supportsSse &&
    currentWindow !== "current" &&
    !isCalendarSummaryWindow(currentWindow)
  );
}

export function shouldRetryCurrentSummaryError(error: string): boolean {
  const normalized = error.toLowerCase();
  return (
    normalized.includes("timed out") ||
    normalized.includes("timeout") ||
    normalized.includes("failed to fetch") ||
    normalized.includes("network error") ||
    normalized.includes("networkerror")
  );
}

async function runThrottledSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refreshIntervalMs: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  if (gate.inFlight || now - gate.lastTriggerAt < refreshIntervalMs) {
    return false;
  }

  gate.inFlight = true;
  gate.lastTriggerAt = now;
  try {
    await refresh();
  } catch {
    // Keep fallback refresh best-effort; hook state already records request errors.
  } finally {
    gate.inFlight = false;
  }
  return true;
}

export async function runUnsupportedSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  return runThrottledSummaryRefresh(gate, now, UNSUPPORTED_SSE_REFRESH_INTERVAL_MS, refresh);
}

export async function runCalendarSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  return runThrottledSummaryRefresh(
    gate,
    now,
    CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
    refresh,
  );
}

export function useSummary(window: string, options?: UseSummaryOptions) {
  const initialCachedSummary = readSummaryRemountCache(
    window,
    options?.limit,
    Date.now(),
    SUMMARY_REMOUNT_CACHE_TTL_MS,
    options?.upstreamAccountId,
  );
  const supportsPureSse = window !== "yesterday";
  const topic = supportsPureSse
    ? buildTopicDescriptor("stats.summary.current", {
        window,
        limit: options?.limit,
        upstreamAccountId: options?.upstreamAccountId,
        timeZone: getBrowserTimeZone(),
      })
    : null;
  const sse = useSubscriptionTopic<StatsResponse>(topic, supportsPureSse);
  const [httpSummary, setHttpSummary] = useState<StatsResponse | null>(
    () => initialCachedSummary?.stats ?? null,
  );
  const [httpLoading, setHttpLoading] = useState(
    () => !supportsPureSse && initialCachedSummary == null,
  );
  const [httpError, setHttpError] = useState<string | null>(null);

  const loadHttpSummary = useCallback(async () => {
    setHttpLoading(true);
    try {
      const response = await fetchSummary(window, {
        limit: options?.limit,
        upstreamAccountId: options?.upstreamAccountId,
      });
      setHttpSummary(response);
      writeSummaryRemountCache(
        window,
        options?.limit,
        response,
        Date.now(),
        options?.upstreamAccountId,
      );
      setHttpError(null);
    } catch (error) {
      setHttpError(error instanceof Error ? error.message : String(error));
    } finally {
      setHttpLoading(false);
    }
  }, [options?.limit, options?.upstreamAccountId, window]);

  useEffect(() => {
    if (!supportsPureSse) {
      void loadHttpSummary();
    }
  }, [loadHttpSummary, supportsPureSse]);

  useEffect(() => {
    if (supportsPureSse && sse.data) {
      writeSummaryRemountCache(
        window,
        options?.limit,
        sse.data,
        Date.now(),
        options?.upstreamAccountId,
      );
    }
  }, [options?.limit, options?.upstreamAccountId, sse.data, supportsPureSse, window]);

  const summary = supportsPureSse ? (sse.data ?? initialCachedSummary?.stats ?? null) : httpSummary;
  const isLoading = supportsPureSse ? sse.isLoading && summary == null : httpLoading;
  const error = supportsPureSse ? sse.error : httpError;
  const refresh = supportsPureSse ? sse.refresh : loadHttpSummary;

  return {
    summary,
    isLoading,
    error,
    refresh,
  };
}
