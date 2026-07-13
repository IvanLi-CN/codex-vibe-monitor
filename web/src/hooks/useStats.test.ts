import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
  CURRENT_SUMMARY_MAX_RETRY_ATTEMPTS,
  CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS,
  CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
  CURRENT_SUMMARY_RETRY_DELAY_MS,
  clearSummaryRemountCache,
  createUnsupportedRefreshGate,
  getCalendarSummaryDayRolloverRefreshEpoch,
  getCurrentSummarySseRefreshDelay,
  getSummaryRemountCacheKey,
  isCalendarSummaryWindow,
  mergePendingSummarySilentOption,
  readSummaryRemountCache,
  runCalendarSummaryRefresh,
  runUnsupportedSummaryRefresh,
  SUMMARY_REMOUNT_CACHE_TTL_MS,
  shouldEnableSummaryRemountCache,
  shouldForceCalendarSummaryOpenResync,
  shouldHandleUnsupportedSummaryRefresh,
  shouldRefreshCalendarSummaryOnRecords,
  shouldRefreshScopedSummaryOnRecords,
  shouldRefreshYesterdaySummaryOnRecords,
  shouldRetryCurrentSummaryError,
  shouldReuseSummaryRemountCache,
  shouldTriggerCurrentSummaryOpenResync,
  UNSUPPORTED_SSE_REFRESH_INTERVAL_MS,
  writeSummaryRemountCache,
} from "./useStats";

afterEach(() => {
  clearSummaryRemountCache();
});

describe("useSummary unsupported window fallback", () => {
  it("throttles summary event storms for unsupported windows", async () => {
    const gate = createUnsupportedRefreshGate();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS;

    const firstTrigger = await runUnsupportedSummaryRefresh(gate, base, refresh);
    expect(firstTrigger).toBe(true);

    for (let i = 1; i <= 6; i += 1) {
      const triggered = await runUnsupportedSummaryRefresh(gate, base + i * 500, refresh);
      expect(triggered).toBe(false);
    }

    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("allows refresh again once the 60s gate expires", async () => {
    const gate = createUnsupportedRefreshGate();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS;

    await runUnsupportedSummaryRefresh(gate, base, refresh);

    const tooEarly = await runUnsupportedSummaryRefresh(
      gate,
      base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS - 1,
      refresh,
    );
    const reopened = await runUnsupportedSummaryRefresh(
      gate,
      base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS,
      refresh,
    );

    expect(tooEarly).toBe(false);
    expect(reopened).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("recovers after a silent refresh failure and can retry after interval", async () => {
    const gate = createUnsupportedRefreshGate();
    const refresh = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValue(undefined);
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS;

    const first = await runUnsupportedSummaryRefresh(gate, base, refresh);
    const second = await runUnsupportedSummaryRefresh(
      gate,
      base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS,
      refresh,
    );

    expect(first).toBe(true);
    expect(second).toBe(true);
    expect(gate.inFlight).toBe(false);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("keeps supported-window summary behavior unchanged", () => {
    expect(shouldHandleUnsupportedSummaryRefresh("1d", "1d", true)).toBe(false);
    expect(shouldHandleUnsupportedSummaryRefresh("30m", "1d", true)).toBe(false);
    expect(shouldHandleUnsupportedSummaryRefresh("1h", "current", false)).toBe(false);
    expect(shouldHandleUnsupportedSummaryRefresh("1h", "today", false)).toBe(false);
    expect(shouldHandleUnsupportedSummaryRefresh("1h", "7d", false)).toBe(true);
  });

  it("recognizes calendar windows that should refresh from records", () => {
    expect(isCalendarSummaryWindow("today")).toBe(true);
    expect(isCalendarSummaryWindow("yesterday")).toBe(true);
    expect(isCalendarSummaryWindow("thisWeek")).toBe(true);
    expect(isCalendarSummaryWindow("thisMonth")).toBe(true);
    expect(isCalendarSummaryWindow("previous7d")).toBe(true);
    expect(isCalendarSummaryWindow("1d")).toBe(false);
  });

  it("throttles calendar-window records reconciles to 5 seconds", async () => {
    const gate = createUnsupportedRefreshGate();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const base = CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS;

    const first = await runCalendarSummaryRefresh(gate, base, refresh);
    const tooEarly = await runCalendarSummaryRefresh(
      gate,
      base + CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - 1,
      refresh,
    );
    const reopened = await runCalendarSummaryRefresh(
      gate,
      base + CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
      refresh,
    );

    expect(first).toBe(true);
    expect(tooEarly).toBe(false);
    expect(reopened).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("returns zero delay when current summary refresh is outside throttle window", () => {
    const delay = getCurrentSummarySseRefreshDelay(
      10_000,
      10_000 + CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
    );
    expect(delay).toBe(0);
  });

  it("returns remaining delay when current summary refresh is still throttled", () => {
    const delay = getCurrentSummarySseRefreshDelay(20_000, 20_250);
    expect(delay).toBe(CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - 250);
  });

  it("merges pending silent options to preserve non-silent requests", () => {
    expect(mergePendingSummarySilentOption(null, true)).toBe(true);
    expect(mergePendingSummarySilentOption(true, false)).toBe(false);
    expect(mergePendingSummarySilentOption(false, true)).toBe(false);
  });

  it("throttles current summary reconnect resync in cooldown window", () => {
    const allowed = shouldTriggerCurrentSummaryOpenResync(
      30_000,
      30_000 + CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS - 1,
    );
    expect(allowed).toBe(false);
  });

  it("allows forced reconnect resync regardless of cooldown", () => {
    const allowed = shouldTriggerCurrentSummaryOpenResync(40_000, 40_500, true);
    expect(allowed).toBe(true);
  });

  it("forces natural-day summary reconnect resync for day-boundary windows", () => {
    const currentDayStartEpoch = Math.floor(new Date(2026, 3, 8, 0, 0, 0).getTime() / 1000);
    const nextDayNoonEpoch = Math.floor(new Date(2026, 3, 9, 12, 0, 0).getTime() / 1000);

    expect(
      shouldForceCalendarSummaryOpenResync("today", currentDayStartEpoch, nextDayNoonEpoch),
    ).toBe(true);
    expect(
      shouldForceCalendarSummaryOpenResync("yesterday", currentDayStartEpoch, nextDayNoonEpoch),
    ).toBe(true);
    expect(
      shouldForceCalendarSummaryOpenResync("previous7d", currentDayStartEpoch, nextDayNoonEpoch),
    ).toBe(true);
    expect(
      shouldForceCalendarSummaryOpenResync(
        "today",
        currentDayStartEpoch,
        currentDayStartEpoch + 3_600,
      ),
    ).toBe(false);
    expect(
      shouldForceCalendarSummaryOpenResync("thisWeek", currentDayStartEpoch, nextDayNoonEpoch),
    ).toBe(false);
  });

  it("schedules closed natural-day summary rollover refresh at the next local midnight", () => {
    const noonEpoch = Math.floor(new Date(2026, 3, 8, 12, 34, 56).getTime() / 1000);
    const nextMidnightEpoch = Math.floor(new Date(2026, 3, 9, 0, 0, 0).getTime() / 1000);

    expect(getCalendarSummaryDayRolloverRefreshEpoch("yesterday", noonEpoch)).toBe(
      nextMidnightEpoch,
    );
    expect(getCalendarSummaryDayRolloverRefreshEpoch("previous7d", noonEpoch)).toBe(
      nextMidnightEpoch,
    );
    expect(getCalendarSummaryDayRolloverRefreshEpoch("thisWeek", noonEpoch)).toBeNull();
  });

  it("skips record-driven refreshes for the closed yesterday summary window", () => {
    expect(shouldRefreshCalendarSummaryOnRecords("today")).toBe(true);
    expect(shouldRefreshCalendarSummaryOnRecords("thisWeek")).toBe(true);
    expect(shouldRefreshCalendarSummaryOnRecords("thisMonth")).toBe(true);
    expect(shouldRefreshCalendarSummaryOnRecords("yesterday")).toBe(false);
    expect(shouldRefreshCalendarSummaryOnRecords("previous7d")).toBe(false);
  });

  it("refreshes yesterday summary only when records settle inside the previous local day", () => {
    const nowEpoch = Math.floor(new Date(2026, 3, 9, 12, 0, 0).getTime() / 1000);

    expect(
      shouldRefreshYesterdaySummaryOnRecords(
        [{ occurredAt: new Date(2026, 3, 8, 23, 59, 30).toISOString() }],
        nowEpoch,
      ),
    ).toBe(true);
    expect(
      shouldRefreshYesterdaySummaryOnRecords(
        [{ occurredAt: new Date(2026, 3, 9, 0, 1, 0).toISOString() }],
        nowEpoch,
      ),
    ).toBe(false);
  });

  it("refreshes account-scoped rolling summaries from matching records", () => {
    const nowEpoch = Math.floor(new Date(2026, 3, 9, 12, 0, 0).getTime() / 1000);
    const todayRecord = [{ occurredAt: new Date(2026, 3, 9, 11, 59, 0).toISOString() }];
    const yesterdayRecord = [{ occurredAt: new Date(2026, 3, 8, 23, 59, 0).toISOString() }];

    expect(shouldRefreshScopedSummaryOnRecords("1d", todayRecord, nowEpoch)).toBe(true);
    expect(shouldRefreshScopedSummaryOnRecords("7d", todayRecord, nowEpoch)).toBe(true);
    expect(shouldRefreshScopedSummaryOnRecords("today", todayRecord, nowEpoch)).toBe(true);
    expect(shouldRefreshScopedSummaryOnRecords("yesterday", yesterdayRecord, nowEpoch)).toBe(true);
    expect(shouldRefreshScopedSummaryOnRecords("yesterday", todayRecord, nowEpoch)).toBe(false);
    expect(shouldRefreshScopedSummaryOnRecords("current", todayRecord, nowEpoch)).toBe(false);
  });

  it("retries current summary only for transient network-like errors", () => {
    expect(shouldRetryCurrentSummaryError("summary request timed out after 10s")).toBe(true);
    expect(shouldRetryCurrentSummaryError("Failed to fetch")).toBe(true);
    expect(shouldRetryCurrentSummaryError("Network error: ECONNRESET")).toBe(true);
    expect(shouldRetryCurrentSummaryError("HTTP 400: bad request")).toBe(false);
  });

  it("keeps retry policy bounded by defaults", () => {
    expect(CURRENT_SUMMARY_RETRY_DELAY_MS).toBe(2_000);
    expect(CURRENT_SUMMARY_MAX_RETRY_ATTEMPTS).toBeGreaterThan(0);
  });

  it("stores remount cache entries by window and limit", () => {
    const summary = {
      totalCount: 12,
      successCount: 10,
      failureCount: 2,
      totalCost: 0.5,
      totalTokens: 120,
    };

    writeSummaryRemountCache("7d", undefined, summary, 1_000);

    expect(getSummaryRemountCacheKey("7d")).toBe("7d::default::global");
    expect(readSummaryRemountCache("7d", undefined, 1_001)).toEqual({
      stats: summary,
      cachedAt: 1_000,
    });
  });

  it("stores global and account-scoped remount cache entries separately", () => {
    const globalSummary = {
      totalCount: 12,
      successCount: 10,
      failureCount: 2,
      totalCost: 0.5,
      totalTokens: 120,
    };
    const accountSummary = {
      totalCount: 3,
      successCount: 2,
      failureCount: 1,
      totalCost: 0.2,
      totalTokens: 48,
    };

    writeSummaryRemountCache("7d", undefined, globalSummary, 1_000);
    writeSummaryRemountCache("7d", undefined, accountSummary, 2_000, 42);

    expect(getSummaryRemountCacheKey("7d", undefined, 42)).toBe("7d::default::account:42");
    expect(readSummaryRemountCache("7d", undefined, 2_001)).toEqual({
      stats: globalSummary,
      cachedAt: 1_000,
    });
    expect(
      readSummaryRemountCache("7d", undefined, 2_001, SUMMARY_REMOUNT_CACHE_TTL_MS, 42),
    ).toEqual({
      stats: accountSummary,
      cachedAt: 2_000,
    });
  });

  it("disables remount caching for current and calendar summary windows", () => {
    expect(shouldEnableSummaryRemountCache("current")).toBe(false);
    expect(shouldEnableSummaryRemountCache("today")).toBe(false);
    expect(shouldEnableSummaryRemountCache("yesterday")).toBe(false);
    expect(shouldEnableSummaryRemountCache("previous7d")).toBe(false);
    writeSummaryRemountCache(
      "current",
      undefined,
      {
        totalCount: 1,
        successCount: 1,
        failureCount: 0,
        totalCost: 0,
        totalTokens: 1,
      },
      1_000,
    );
    expect(readSummaryRemountCache("current", undefined, 1_001)).toBeNull();
    writeSummaryRemountCache(
      "today",
      undefined,
      {
        totalCount: 2,
        successCount: 2,
        failureCount: 0,
        totalCost: 0,
        totalTokens: 2,
      },
      1_000,
    );
    expect(readSummaryRemountCache("today", undefined, 1_001)).toBeNull();
    writeSummaryRemountCache(
      "yesterday",
      undefined,
      {
        totalCount: 3,
        successCount: 3,
        failureCount: 0,
        totalCost: 0,
        totalTokens: 3,
      },
      1_000,
    );
    expect(readSummaryRemountCache("yesterday", undefined, 1_001)).toBeNull();
    writeSummaryRemountCache(
      "previous7d",
      undefined,
      {
        totalCount: 4,
        successCount: 4,
        failureCount: 0,
        totalCost: 0,
        totalTokens: 4,
      },
      1_000,
    );
    expect(readSummaryRemountCache("previous7d", undefined, 1_001)).toBeNull();
  });

  it("treats remount cache as reusable only inside the ttl window", () => {
    expect(shouldReuseSummaryRemountCache(10_000, 10_000 + SUMMARY_REMOUNT_CACHE_TTL_MS - 1)).toBe(
      true,
    );
    expect(shouldReuseSummaryRemountCache(10_000, 10_000 + SUMMARY_REMOUNT_CACHE_TTL_MS)).toBe(
      false,
    );
  });

  it("does not hydrate from stale summary remount cache entries", () => {
    const summary = {
      totalCount: 12,
      successCount: 10,
      failureCount: 2,
      totalCost: 0.5,
      totalTokens: 120,
    };
    writeSummaryRemountCache("7d", undefined, summary, 1_000);

    expect(
      readSummaryRemountCache("7d", undefined, 1_000 + SUMMARY_REMOUNT_CACHE_TTL_MS),
    ).toBeNull();
  });
});
