import { afterEach, describe, expect, it, vi } from "vitest";
import type { ApiInvocation, TimeseriesResponse } from "../lib/api";
import {
  clearTimeseriesRemountCache,
  MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
  TIMESERIES_OPEN_RESYNC_COOLDOWN_MS,
  TIMESERIES_REMOUNT_CACHE_TTL_MS,
  TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
  TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
  applyRecordsToCurrentDayBucket,
  applyRecordsToTimeseries,
  fetchAllInvocationRecordPages,
  fetchTimeseriesInFlightRecords,
  getCurrentDayBucketEndEpoch,
  getLocalDayStartEpoch,
  getNextLocalDayStartEpoch,
  getTimeseriesDayRolloverRefreshEpoch,
  getTimeseriesRemountCacheKey,
  getTimeseriesRecordsResyncDelay,
  mergePendingTimeseriesSilentOption,
  readTimeseriesRemountCache,
  resolveTimeseriesSyncPolicy,
  resolveCurrentDayLiveSeedEpoch,
  shouldEnableTimeseriesRemountCache,
  shouldReuseTimeseriesRemountCache,
  shouldPatchCurrentDayBucketOnRecordsEvent,
  shouldResyncForCurrentDayBucket,
  shouldResyncOnRecordsEvent,
  shouldTriggerTimeseriesOpenResync,
  seedCurrentDayLiveRecordDeltas,
  seedTimeseriesLiveRecordDeltas,
  shouldForceNaturalDayOpenResync,
  trackTimeseriesLiveRecordDelta,
  pruneTrackedTimeseriesLiveRecordDeltas,
  upsertCurrentDayLiveRecord,
  upsertTimeseriesLiveRecord,
  writeTimeseriesRemountCache,
} from "./useTimeseries";

afterEach(() => {
  clearTimeseriesRemountCache();
});

describe("useTimeseries records sync strategy", () => {
  it("uses explicit local incremental policy for dashboard 24h heatmap", () => {
    expect(resolveTimeseriesSyncPolicy("1d", { bucket: "1m" }).mode).toBe(
      "local",
    );
    expect(shouldResyncOnRecordsEvent("1d", { bucket: "1m" })).toBe(false);
  });

  it("uses explicit local incremental policy for dashboard 7d heatmap", () => {
    expect(resolveTimeseriesSyncPolicy("7d", { bucket: "1h" }).mode).toBe(
      "local",
    );
    expect(shouldResyncOnRecordsEvent("7d", { bucket: "1h" })).toBe(false);
  });

  it("uses current-day patch policy for dashboard history calendar", () => {
    expect(resolveTimeseriesSyncPolicy("6mo", { bucket: "1d" }).mode).toBe(
      "current-day-local",
    );
    expect(
      shouldPatchCurrentDayBucketOnRecordsEvent("6mo", { bucket: "1d" }),
    ).toBe(true);
    expect(shouldResyncOnRecordsEvent("6mo", { bucket: "1d" })).toBe(false);
  });

  it("forces server resync when backend aggregation is preferred", () => {
    expect(
      resolveTimeseriesSyncPolicy("1d", {
        bucket: "15m",
        preferServerAggregation: true,
      }).mode,
    ).toBe("server");
    expect(
      shouldResyncOnRecordsEvent("1d", {
        bucket: "15m",
        preferServerAggregation: true,
      }),
    ).toBe(true);
  });

  it("derives local-day boundaries for the today dashboard range", () => {
    const noonEpoch = Math.floor(
      new Date(2026, 3, 8, 12, 34, 56).getTime() / 1000,
    );
    expect(getLocalDayStartEpoch(noonEpoch)).toBe(
      Math.floor(new Date(2026, 3, 8, 0, 0, 0).getTime() / 1000),
    );
    expect(getNextLocalDayStartEpoch(noonEpoch)).toBe(
      Math.floor(new Date(2026, 3, 9, 0, 0, 0).getTime() / 1000),
    );
  });

  it("falls back to server resync for other daily buckets", () => {
    expect(resolveTimeseriesSyncPolicy("30d", { bucket: "1d" }).mode).toBe(
      "server",
    );
    expect(shouldResyncOnRecordsEvent("30d", { bucket: "1d" })).toBe(true);
  });
});

describe("useTimeseries current-day bucket patching", () => {
  const base: TimeseriesResponse = {
    rangeStart: "2026-03-05T00:00:00Z",
    rangeEnd: "2026-03-07T00:00:00Z",
    bucketSeconds: 86_400,
    points: [
      {
        bucketStart: "2026-03-05T00:00:00Z",
        bucketEnd: "2026-03-06T00:00:00Z",
        totalCount: 4,
        successCount: 3,
        failureCount: 1,
        totalTokens: 400,
        totalCost: 2,
      },
      {
        bucketStart: "2026-03-06T00:00:00Z",
        bucketEnd: "2026-03-07T00:00:00Z",
        totalCount: 1,
        successCount: 1,
        failureCount: 0,
        totalTokens: 100,
        totalCost: 0.5,
      },
    ],
  };

  it("patches only the active local-day bucket for repeated records events", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 1,
          invokeId: "older",
          occurredAt: "2026-03-05T10:00:00Z",
          status: "success",
          totalTokens: 10,
          cost: 0.1,
          createdAt: "2026-03-05T10:00:00Z",
        },
        {
          id: 2,
          invokeId: "today",
          occurredAt: "2026-03-06T08:30:00Z",
          status: "failed",
          totalTokens: 25,
          cost: 0.2,
          createdAt: "2026-03-06T08:30:00Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[0]).toEqual(base.points[0]);
    expect(next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 1,
      failureCount: 1,
      totalTokens: 125,
      totalCost: 0.7,
    });
  });

  it("does not treat running or pending snapshots with failure metadata as temporary failures", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 3,
          invokeId: "today-running",
          occurredAt: "2026-03-06T08:31:00Z",
          status: "running",
          errorMessage:
            "[upstream_response_failed] upstream response stream reported failure",
          failureKind: "upstream_response_failed",
          failureClass: "service_failure",
          totalTokens: 0,
          cost: 0,
          createdAt: "2026-03-06T08:31:00Z",
        },
        {
          id: 4,
          invokeId: "today-pending",
          occurredAt: "2026-03-06T08:32:00Z",
          status: "pending",
          errorMessage:
            "[downstream_closed] downstream closed while streaming upstream response",
          failureKind: "downstream_closed",
          failureClass: "client_abort",
          totalTokens: 0,
          cost: 0,
          createdAt: "2026-03-06T08:32:00Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[1]).toMatchObject({
      totalCount: 3,
      successCount: 1,
      failureCount: 0,
      inFlightCount: 2,
    });
  });

  it("keeps blank-status rows without failure metadata neutral instead of counting them as in-flight", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 41,
          invokeId: "today-neutral-blank-status",
          occurredAt: "2026-03-06T08:34:00Z",
          status: "",
          failureClass: "none",
          totalTokens: 9,
          cost: 0.09,
          createdAt: "2026-03-06T08:34:00Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 1,
      failureCount: 0,
      totalTokens: 109,
      totalCost: 0.59,
    });
  });

  it("treats legacy http_200 rows without errors as success-like", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 5,
          invokeId: "today-http-200-success-like",
          occurredAt: "2026-03-06T08:33:00Z",
          status: "http_200",
          downstreamStatusCode: 200,
          totalTokens: 15,
          cost: 0.15,
          createdAt: "2026-03-06T08:33:00Z",
        },
        {
          id: 6,
          invokeId: "today-http-200-failed",
          occurredAt: "2026-03-06T08:34:00Z",
          status: "http_200",
          errorMessage: "upstream parse failed",
          totalTokens: 9,
          cost: 0.09,
          createdAt: "2026-03-06T08:34:00Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[1]).toMatchObject({
      totalCount: 3,
      successCount: 2,
      failureCount: 1,
      totalTokens: 124,
      totalCost: 0.74,
    });
  });

  it("treats http_200 rows with downstream-only metadata as failures", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 7,
          invokeId: "today-http-200-downstream-only",
          occurredAt: "2026-03-06T08:33:30Z",
          status: "http_200",
          downstreamStatusCode: 502,
          downstreamErrorMessage: "socket closed after response",
          totalTokens: 15,
          cost: 0.15,
          createdAt: "2026-03-06T08:33:30Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 1,
      failureCount: 1,
      totalTokens: 115,
      totalCost: 0.65,
    });
  });

  it("treats blank-status rows with downstream-only metadata as failures", () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 8,
          invokeId: "today-blank-downstream-only",
          occurredAt: "2026-03-06T08:34:30Z",
          status: "",
          downstreamErrorMessage:
            "downstream closed while streaming upstream response",
          totalTokens: 12,
          cost: 0.12,
          createdAt: "2026-03-06T08:34:30Z",
        },
      ],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 1,
      failureCount: 1,
      totalTokens: 112,
      totalCost: 0.62,
    });
  });

  it("replaces a seeded current-day in-flight delta when the same invocation settles", () => {
    const runningRecord = {
      id: 30,
      invokeId: "today-running",
      occurredAt: "2026-03-06T08:31:00Z",
      status: "running",
      totalTokens: 0,
      cost: 0,
      createdAt: "2026-03-06T08:31:00Z",
    };
    const settledRecord = {
      ...runningRecord,
      status: "success",
      totalTokens: 25,
      cost: 0.2,
    };
    const current: TimeseriesResponse = {
      ...base,
      points: [
        base.points[0],
        {
          ...base.points[1],
          totalCount: 2,
          successCount: 1,
          failureCount: 0,
          totalTokens: 125,
          totalCost: 0.5,
        },
      ],
    };

    const seeded = seedCurrentDayLiveRecordDeltas(
      current,
      [runningRecord],
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );
    const result = upsertCurrentDayLiveRecord(
      current,
      settledRecord,
      seeded.get("today-running-2026-03-06T08:31:00Z") ?? null,
      Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
    );

    expect(result.next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 2,
      failureCount: 0,
      totalTokens: 150,
      totalCost: 0.7,
    });
  });

  it("subtracts a pre-midnight placeholder from its original bucket when it settles after midnight", () => {
    const current: TimeseriesResponse = {
      ...base,
      points: [
        {
          ...base.points[0],
          totalCount: 5,
          successCount: 3,
          failureCount: 1,
          totalTokens: 400,
          totalCost: 2,
        },
        base.points[1],
      ],
    };
    const previousDelta = {
      recordId: 40,
      bucketStart: base.points[0].bucketStart,
      bucketEnd: base.points[0].bucketEnd,
      bucketStartEpoch: Math.floor(
        Date.parse(base.points[0].bucketStart) / 1000,
      ),
      bucketEndEpoch: Math.floor(Date.parse(base.points[0].bucketEnd) / 1000),
      totalCount: 1,
      successCount: 0,
      failureCount: 0,
      inFlightCount: 1,
      totalTokens: 0,
      totalCost: 0,
      countsOnly: true,
    };
    const settledRecord = {
      id: 40,
      invokeId: "overnight-running",
      occurredAt: "2026-03-05T23:59:30Z",
      status: "success",
      totalTokens: 10,
      cost: 0.1,
      createdAt: "2026-03-05T23:59:30Z",
    };

    const result = upsertCurrentDayLiveRecord(
      current,
      settledRecord,
      previousDelta,
      Math.floor(Date.parse("2026-03-06T00:01:00Z") / 1000),
    );

    expect(result.delta).toBeNull();
    expect(result.next?.points[0]).toMatchObject({
      totalCount: 4,
      successCount: 3,
      failureCount: 1,
      totalTokens: 400,
      totalCost: 2,
    });
    expect(result.next?.points[1]).toEqual(base.points[1]);
  });

  it("seeds current-day in-flight deltas from the active day instead of the next bucket boundary", () => {
    const runningRecord = {
      id: 31,
      invokeId: "today-running-seed",
      occurredAt: "2026-03-06T08:31:00Z",
      status: "running",
      totalTokens: 0,
      cost: 0,
      createdAt: "2026-03-06T08:31:00Z",
    };

    const seeded = seedCurrentDayLiveRecordDeltas(
      base,
      [runningRecord],
      resolveCurrentDayLiveSeedEpoch(base),
    );

    expect(seeded.get("today-running-seed-2026-03-06T08:31:00Z")).toMatchObject(
      {
        totalCount: 1,
        successCount: 0,
        failureCount: 0,
      },
    );
  });

  it("requests a full resync when the current day is no longer covered", () => {
    expect(
      shouldResyncForCurrentDayBucket(
        base,
        Math.floor(Date.parse("2026-03-07T01:00:00Z") / 1000),
      ),
    ).toBe(true);
    expect(
      getCurrentDayBucketEndEpoch(
        base,
        Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
      ),
    ).toBe(Math.floor(Date.parse("2026-03-07T00:00:00Z") / 1000));
  });

  it("requests a full resync when there is no current-day bucket to patch yet", () => {
    expect(
      shouldResyncForCurrentDayBucket(
        null,
        Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
      ),
    ).toBe(true);
    expect(
      shouldResyncForCurrentDayBucket(
        {
          ...base,
          points: [],
        },
        Math.floor(Date.parse("2026-03-06T12:00:00Z") / 1000),
      ),
    ).toBe(true);
  });
});

describe("useTimeseries natural-day range patching", () => {
  it("keeps today-scoped local patches inside the current local day", () => {
    const now = new Date(2026, 3, 8, 12, 0, 0);
    const currentDayStart = new Date(2026, 3, 8, 0, 0, 0);
    const previousDayStart = new Date(2026, 3, 7, 0, 0, 0);
    const current: TimeseriesResponse = {
      rangeStart: currentDayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: now.toISOString().replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: new Date(2026, 3, 7, 23, 59, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          bucketEnd: new Date(2026, 3, 8, 0, 0, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 10,
          totalCost: 0.1,
        },
      ],
    };

    const next = applyRecordsToTimeseries(
      current,
      [
        {
          id: 1,
          invokeId: "yesterday",
          occurredAt: new Date(2026, 3, 7, 23, 59, 30)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          status: "success",
          totalTokens: 10,
          cost: 0.1,
          createdAt: new Date(2026, 3, 7, 23, 59, 30)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
        },
        {
          id: 2,
          invokeId: "today",
          occurredAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          status: "failed",
          totalTokens: 25,
          cost: 0.2,
          createdAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
        },
      ],
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(next?.rangeStart).toBe(
      currentDayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
    );
    expect(next?.points).toHaveLength(1);
    expect(next?.points[0]).toMatchObject({
      bucketStart: new Date(2026, 3, 8, 0, 1, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      totalTokens: 25,
      totalCost: 0.2,
    });
    expect(previousDayStart.getTime()).toBeLessThan(currentDayStart.getTime());
  });

  it("keeps yesterday-scoped local patches inside the previous local day", () => {
    const yesterdayStart = new Date(2026, 3, 7, 0, 0, 0);
    const todayStart = new Date(2026, 3, 8, 0, 0, 0);
    const current: TimeseriesResponse = {
      rangeStart: yesterdayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: todayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: new Date(2026, 3, 7, 23, 58, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          bucketEnd: new Date(2026, 3, 7, 23, 59, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 10,
          totalCost: 0.1,
        },
      ],
    };

    const next = applyRecordsToTimeseries(
      current,
      [
        {
          id: 3,
          invokeId: "yesterday-success",
          occurredAt: new Date(2026, 3, 7, 23, 59, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          status: "success",
          totalTokens: 20,
          cost: 0.3,
          createdAt: new Date(2026, 3, 7, 23, 59, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
        },
        {
          id: 4,
          invokeId: "today-should-ignore",
          occurredAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          status: "failed",
          totalTokens: 25,
          cost: 0.2,
          createdAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
        },
      ],
      {
        range: "yesterday",
        bucketSeconds: 60,
      },
    );

    expect(next?.rangeStart).toBe(
      yesterdayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
    );
    expect(next?.rangeEnd).toBe(
      todayStart.toISOString().replace(/\.\d{3}Z$/, "Z"),
    );
    expect(next?.points).toHaveLength(2);
    expect(next?.points.at(-1)).toMatchObject({
      bucketStart: new Date(2026, 3, 7, 23, 59, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      totalCount: 1,
      successCount: 1,
      failureCount: 0,
      totalTokens: 20,
      totalCost: 0.3,
    });
  });

  it("counts running local live records toward totals without painting provisional failure metadata as failures", () => {
    const current: TimeseriesResponse = {
      rangeStart: new Date(2026, 3, 8, 0, 0, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: new Date(2026, 3, 8, 0, 3, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [],
    };

    const next = applyRecordsToTimeseries(
      current,
      [
        {
          id: 11,
          invokeId: "live-running",
          occurredAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          status: "running",
          errorMessage:
            "[upstream_response_failed] upstream response stream reported failure",
          failureKind: "upstream_response_failed",
          failureClass: "service_failure",
          totalTokens: 0,
          cost: 0,
          createdAt: new Date(2026, 3, 8, 0, 1, 15)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
        },
      ],
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(next?.points[0]).toMatchObject({
      totalCount: 1,
      successCount: 0,
      failureCount: 0,
    });
  });

  it("keeps a running SSE row with provisional failure metadata out of local failure buckets", () => {
    const current: TimeseriesResponse = {
      rangeStart: new Date(2026, 3, 8, 0, 0, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: new Date(2026, 3, 8, 0, 3, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [],
    };

    const result = upsertTimeseriesLiveRecord(
      current,
      {
        id: 12,
        invokeId: "live-running-sse",
        occurredAt: new Date(2026, 3, 8, 0, 1, 30)
          .toISOString()
          .replace(/\.\d{3}Z$/, "Z"),
        status: "running",
        errorMessage:
          "[upstream_response_failed] upstream response stream reported failure",
        failureKind: "upstream_response_failed",
        failureClass: "service_failure",
        totalTokens: 0,
        cost: 0,
        createdAt: new Date(2026, 3, 8, 0, 1, 30)
          .toISOString()
          .replace(/\.\d{3}Z$/, "Z"),
      },
      null,
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(result.next?.points[0]).toMatchObject({
      totalCount: 1,
      successCount: 0,
      failureCount: 0,
      totalTokens: 0,
      totalCost: 0,
    });
  });

  it("treats downstream-only live SSE rows as failures immediately", () => {
    const current: TimeseriesResponse = {
      rangeStart: new Date(2026, 3, 8, 0, 0, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: new Date(2026, 3, 8, 0, 3, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [],
    };

    const result = upsertTimeseriesLiveRecord(
      current,
      {
        id: 13,
        invokeId: "live-http-200-downstream-only",
        occurredAt: new Date(2026, 3, 8, 0, 1, 45)
          .toISOString()
          .replace(/\.\d{3}Z$/, "Z"),
        status: "http_200",
        downstreamStatusCode: 502,
        downstreamErrorMessage: "socket closed after response",
        totalTokens: 17,
        cost: 0.17,
        createdAt: new Date(2026, 3, 8, 0, 1, 45)
          .toISOString()
          .replace(/\.\d{3}Z$/, "Z"),
      },
      null,
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(result.next?.points[0]).toMatchObject({
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      totalTokens: 17,
      totalCost: 0.17,
    });
  });

  it("replaces a seeded in-flight local delta when the same invocation settles", () => {
    const current: TimeseriesResponse = {
      rangeStart: new Date(2026, 3, 8, 0, 0, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      rangeEnd: new Date(2026, 3, 8, 0, 3, 0)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: new Date(2026, 3, 8, 0, 1, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          bucketEnd: new Date(2026, 3, 8, 0, 2, 0)
            .toISOString()
            .replace(/\.\d{3}Z$/, "Z"),
          totalCount: 1,
          successCount: 0,
          failureCount: 0,
          totalTokens: 0,
          totalCost: 0,
        },
      ],
    };
    const runningRecord = {
      id: 41,
      invokeId: "live-running",
      occurredAt: new Date(2026, 3, 8, 0, 1, 15)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
      status: "running",
      totalTokens: 0,
      cost: 0,
      createdAt: new Date(2026, 3, 8, 0, 1, 15)
        .toISOString()
        .replace(/\.\d{3}Z$/, "Z"),
    };
    const settledRecord = {
      ...runningRecord,
      status: "failed",
      totalTokens: 22,
      cost: 0.18,
      errorMessage: "upstream stream error",
      failureKind: "upstream_stream_error",
    };

    const seeded = seedTimeseriesLiveRecordDeltas(current, [runningRecord], {
      range: "today",
      bucketSeconds: 60,
    });
    const result = upsertTimeseriesLiveRecord(
      current,
      settledRecord,
      seeded.get(settledRecord.invokeId + "-" + settledRecord.occurredAt) ??
        null,
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(result.next?.points[0]).toMatchObject({
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      totalTokens: 22,
      totalCost: 0.18,
    });
  });

  it("adds settled token and cost totals when a counts-only placeholder shares a bucket with other requests", () => {
    const current: TimeseriesResponse = {
      rangeStart: "2026-03-08T00:00:00Z",
      rangeEnd: "2026-03-08T00:03:00Z",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-03-08T00:01:00Z",
          bucketEnd: "2026-03-08T00:02:00Z",
          totalCount: 2,
          successCount: 1,
          failureCount: 0,
          totalTokens: 100,
          totalCost: 1,
        },
      ],
    };
    const settledRecord: ApiInvocation = {
      id: 77,
      invokeId: "counts-only-shared-bucket",
      occurredAt: "2026-03-08T00:01:15Z",
      status: "success",
      totalTokens: 22,
      cost: 0.18,
      createdAt: "2026-03-08T00:01:15Z",
    };

    const result = upsertTimeseriesLiveRecord(
      current,
      settledRecord,
      {
        bucketStart: "2026-03-08T00:01:00Z",
        bucketEnd: "2026-03-08T00:02:00Z",
        bucketStartEpoch: Date.parse("2026-03-08T00:01:00Z") / 1000,
        bucketEndEpoch: Date.parse("2026-03-08T00:02:00Z") / 1000,
        totalCount: 1,
        successCount: 0,
        failureCount: 0,
        inFlightCount: 1,
        totalTokens: 0,
        totalCost: 0,
        countsOnly: true,
      },
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(result.next?.points[0]).toMatchObject({
      totalCount: 2,
      successCount: 2,
      failureCount: 0,
      totalTokens: 122,
      totalCost: 1.18,
    });
  });

  it("reconciles provisional token and cost totals when an anonymous placeholder bucket settles", () => {
    const current: TimeseriesResponse = {
      rangeStart: "2026-03-08T00:00:00Z",
      rangeEnd: "2026-03-08T00:03:00Z",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-03-08T00:01:00Z",
          bucketEnd: "2026-03-08T00:02:00Z",
          totalCount: 1,
          successCount: 0,
          failureCount: 0,
          inFlightCount: 1,
          totalTokens: 10,
          totalCost: 0.05,
        },
      ],
    };
    const settledRecord: ApiInvocation = {
      id: 78,
      invokeId: "counts-only-standalone-bucket",
      occurredAt: "2026-03-08T00:01:15Z",
      status: "success",
      totalTokens: 22,
      cost: 0.18,
      createdAt: "2026-03-08T00:01:15Z",
    };

    const result = upsertTimeseriesLiveRecord(
      current,
      settledRecord,
      {
        bucketStart: "2026-03-08T00:01:00Z",
        bucketEnd: "2026-03-08T00:02:00Z",
        bucketStartEpoch: Date.parse("2026-03-08T00:01:00Z") / 1000,
        bucketEndEpoch: Date.parse("2026-03-08T00:02:00Z") / 1000,
        totalCount: 1,
        successCount: 0,
        failureCount: 0,
        inFlightCount: 1,
        totalTokens: 0,
        totalCost: 0,
        countsOnly: true,
      },
      {
        range: "today",
        bucketSeconds: 60,
      },
    );

    expect(result.delta?.totalTokens).toBe(12);
    expect(result.delta?.totalCost ?? 0).toBeCloseTo(0.13);
    expect(result.next?.points[0]?.totalCount).toBe(1);
    expect(result.next?.points[0]?.successCount).toBe(1);
    expect(result.next?.points[0]?.failureCount).toBe(0);
    expect(result.next?.points[0]?.totalTokens).toBe(22);
    expect(result.next?.points[0]?.totalCost ?? 0).toBeCloseTo(0.18);
  });
});

describe("useTimeseries in-flight seeding pagination", () => {
  it("pages through all in-flight records against the first page snapshot", async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce({
        snapshotId: 1,
        total: 3,
        page: 1,
        pageSize: 2,
        records: [
          {
            id: 1,
            invokeId: "running-1",
            occurredAt: "2026-03-08T00:01:15Z",
            status: "running",
            totalTokens: 0,
            cost: 0,
            createdAt: "2026-03-08T00:01:15Z",
          },
          {
            id: 2,
            invokeId: "running-2",
            occurredAt: "2026-03-08T00:01:30Z",
            status: "running",
            totalTokens: 0,
            cost: 0,
            createdAt: "2026-03-08T00:01:30Z",
          },
        ],
      })
      .mockResolvedValueOnce({
        snapshotId: 1,
        total: 3,
        page: 2,
        pageSize: 2,
        records: [
          {
            id: 3,
            invokeId: "running-3",
            occurredAt: "2026-03-08T00:01:45Z",
            status: "running",
            totalTokens: 0,
            cost: 0,
            createdAt: "2026-03-08T00:01:45Z",
          },
        ],
      });

    const records = await fetchAllInvocationRecordPages(
      {
        from: "2026-03-08T00:00:00Z",
        to: "2026-03-08T00:03:00Z",
        status: "running",
        sortBy: "occurredAt",
        sortOrder: "desc",
      },
      fetchPage,
      2,
    );

    expect(records.map((record) => record.invokeId)).toEqual([
      "running-1",
      "running-2",
      "running-3",
    ]);
    expect(fetchPage).toHaveBeenNthCalledWith(1, {
      from: "2026-03-08T00:00:00Z",
      to: "2026-03-08T00:03:00Z",
      status: "running",
      sortBy: "occurredAt",
      sortOrder: "desc",
      page: 1,
      pageSize: 2,
    });
    expect(fetchPage).toHaveBeenNthCalledWith(2, {
      from: "2026-03-08T00:00:00Z",
      to: "2026-03-08T00:03:00Z",
      status: "running",
      sortBy: "occurredAt",
      sortOrder: "desc",
      page: 2,
      pageSize: 2,
      snapshotId: 1,
    });
  });

  it("reuses one snapshot across running and pending seed queries", async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce({
        snapshotId: 9,
        total: 1,
        page: 1,
        pageSize: 2,
        records: [
          {
            id: 1,
            invokeId: "running-1",
            occurredAt: "2026-03-08T00:01:15Z",
            status: "running",
            totalTokens: 0,
            cost: 0,
            createdAt: "2026-03-08T00:01:15Z",
          },
        ],
      })
      .mockResolvedValueOnce({
        snapshotId: 11,
        total: 1,
        page: 1,
        pageSize: 2,
        records: [
          {
            id: 2,
            invokeId: "pending-1",
            occurredAt: "2026-03-08T00:01:45Z",
            status: "pending",
            totalTokens: 0,
            cost: 0,
            createdAt: "2026-03-08T00:01:45Z",
          },
        ],
      });

    const records = await fetchTimeseriesInFlightRecords(
      {
        rangeStart: "2026-03-08T00:00:00Z",
        rangeEnd: "2026-03-08T00:03:00Z",
        bucketSeconds: 60,
        snapshotId: 9,
        points: [],
      },
      undefined,
      fetchPage,
    );

    expect(records.map((record) => record.invokeId)).toEqual([
      "running-1",
      "pending-1",
    ]);
    expect(fetchPage).toHaveBeenNthCalledWith(1, {
      from: "2026-03-08T00:00:00Z",
      to: "2026-03-08T00:03:00Z",
      status: "running",
      sortBy: "occurredAt",
      sortOrder: "desc",
      page: 1,
      pageSize: 500,
      signal: undefined,
      snapshotId: 9,
    });
    expect(fetchPage).toHaveBeenNthCalledWith(2, {
      from: "2026-03-08T00:00:00Z",
      to: "2026-03-08T00:03:00Z",
      status: "pending",
      sortBy: "occurredAt",
      sortOrder: "desc",
      page: 1,
      pageSize: 500,
      signal: undefined,
      snapshotId: 9,
    });
  });
});

describe("useTimeseries refresh coordination helpers", () => {
  it("reports the remaining resync delay inside the 3s throttle window", () => {
    expect(getTimeseriesRecordsResyncDelay(10_000, 10_250)).toBe(
      TIMESERIES_RECORDS_RESYNC_THROTTLE_MS - 250,
    );
    expect(
      getTimeseriesRecordsResyncDelay(
        20_000,
        20_000 + TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
      ),
    ).toBe(0);
  });

  it("merges pending silent loads without losing a non-silent refresh", () => {
    expect(mergePendingTimeseriesSilentOption(null, true)).toBe(true);
    expect(mergePendingTimeseriesSilentOption(true, false)).toBe(false);
    expect(mergePendingTimeseriesSilentOption(false, true)).toBe(false);
  });

  it("throttles reconnect resync unless the caller forces it", () => {
    expect(
      shouldTriggerTimeseriesOpenResync(
        30_000,
        30_000 + TIMESERIES_OPEN_RESYNC_COOLDOWN_MS - 1,
      ),
    ).toBe(false);
    expect(shouldTriggerTimeseriesOpenResync(30_000, 30_250, true)).toBe(true);
  });

  it("forces visibility reconnect resync for natural-day dashboard ranges", () => {
    expect(shouldForceNaturalDayOpenResync("today", "local")).toBe(true);
    expect(shouldForceNaturalDayOpenResync("yesterday", "local")).toBe(true);
    expect(shouldForceNaturalDayOpenResync("6mo", "current-day-local")).toBe(
      true,
    );
    expect(shouldForceNaturalDayOpenResync("1d", "local")).toBe(false);
  });

  it("schedules yesterday rollover refresh at the next local midnight", () => {
    const noonEpoch = Math.floor(
      new Date(2026, 3, 8, 12, 34, 56).getTime() / 1000,
    );

    expect(
      getTimeseriesDayRolloverRefreshEpoch(
        "yesterday",
        "local",
        null,
        noonEpoch,
      ),
    ).toBe(getNextLocalDayStartEpoch(noonEpoch));
  });

  it("stores remount cache entries by range and options", () => {
    const response: TimeseriesResponse = {
      rangeStart: "2026-04-08T00:00:00Z",
      rangeEnd: "2026-04-08T00:01:00Z",
      bucketSeconds: 60,
      points: [],
    };
    const liveRecordDeltas = new Map([
      [
        "invoke-1-2026-04-08T00:00:30Z",
        {
          bucketStart: "2026-04-08T00:00:00Z",
          bucketEnd: "2026-04-08T00:01:00Z",
          bucketStartEpoch: 1_744_070_400,
          bucketEndEpoch: 1_744_070_460,
          totalCount: 1,
          successCount: 0,
          failureCount: 0,
          inFlightCount: 1,
          totalTokens: 0,
          totalCost: 0,
        },
      ],
    ]);
    const settledLiveRecordUpdatedAt = new Map([
      ["invoke-1-2026-04-08T00:00:30Z", 12_340],
    ]);

    writeTimeseriesRemountCache(
      "1d",
      { bucket: "1m" },
      response,
      12_345,
      liveRecordDeltas,
      settledLiveRecordUpdatedAt,
      undefined,
      12_300,
    );

    expect(getTimeseriesRemountCacheKey("1d", { bucket: "1m" })).toBe(
      JSON.stringify(["1d", "1m", null, false]),
    );
    const cached = readTimeseriesRemountCache("1d", { bucket: "1m" }, 12_346);
    expect(cached).toEqual({
      data: response,
      cachedAt: 12_345,
      liveRecordDeltas,
      settledLiveRecordUpdatedAt,
      untrackedInFlightCounts: new Map(),
      untrackedInFlightClaimSnapshotId: 12_300,
    });
    expect(cached?.data).not.toBe(response);
    expect(cached?.liveRecordDeltas).not.toBe(liveRecordDeltas);
    expect(cached?.settledLiveRecordUpdatedAt).not.toBe(
      settledLiveRecordUpdatedAt,
    );
    liveRecordDeltas.get("invoke-1-2026-04-08T00:00:30Z")!.failureCount = 99;
    expect(
      cached?.liveRecordDeltas.get("invoke-1-2026-04-08T00:00:30Z")
        ?.failureCount,
    ).toBe(0);
    settledLiveRecordUpdatedAt.set("invoke-2", 999);
    expect(cached?.settledLiveRecordUpdatedAt.has("invoke-2")).toBe(false);
  });

  it("disables remount caching for current, today, and yesterday timeseries", () => {
    expect(shouldEnableTimeseriesRemountCache("current")).toBe(false);
    expect(shouldEnableTimeseriesRemountCache("today")).toBe(false);
    expect(shouldEnableTimeseriesRemountCache("yesterday")).toBe(false);
    writeTimeseriesRemountCache(
      "current",
      undefined,
      {
        rangeStart: "2026-04-08T00:00:00Z",
        rangeEnd: "2026-04-08T00:01:00Z",
        bucketSeconds: 60,
        points: [],
      },
      1_000,
    );
    expect(readTimeseriesRemountCache("current", undefined, 1_001)).toBeNull();
    writeTimeseriesRemountCache(
      "today",
      { bucket: "1m" },
      {
        rangeStart: "2026-04-08T00:00:00Z",
        rangeEnd: "2026-04-08T00:01:00Z",
        bucketSeconds: 60,
        points: [],
      },
      1_000,
    );
    expect(
      readTimeseriesRemountCache("today", { bucket: "1m" }, 1_001),
    ).toBeNull();
    writeTimeseriesRemountCache(
      "yesterday",
      { bucket: "1m" },
      {
        rangeStart: "2026-04-07T00:00:00Z",
        rangeEnd: "2026-04-08T00:00:00Z",
        bucketSeconds: 60,
        points: [],
      },
      1_000,
    );
    expect(
      readTimeseriesRemountCache("yesterday", { bucket: "1m" }, 1_001),
    ).toBeNull();
  });

  it("reuses remount cache only inside the ttl window", () => {
    expect(TIMESERIES_REMOUNT_CACHE_TTL_MS).toBeGreaterThanOrEqual(
      TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
    );
    expect(
      shouldReuseTimeseriesRemountCache(
        5_000,
        5_000 + TIMESERIES_REMOUNT_CACHE_TTL_MS - 1,
      ),
    ).toBe(true);
    expect(
      shouldReuseTimeseriesRemountCache(
        5_000,
        5_000 + TIMESERIES_REMOUNT_CACHE_TTL_MS,
      ),
    ).toBe(false);
  });

  it("does not hydrate from stale timeseries remount cache entries", () => {
    const response: TimeseriesResponse = {
      rangeStart: "2026-04-08T00:00:00Z",
      rangeEnd: "2026-04-08T00:01:00Z",
      bucketSeconds: 60,
      points: [],
    };
    writeTimeseriesRemountCache("1d", { bucket: "1m" }, response, 2_000);

    expect(
      readTimeseriesRemountCache(
        "1d",
        { bucket: "1m" },
        2_000 + TIMESERIES_REMOUNT_CACHE_TTL_MS,
      ),
    ).toBeNull();
  });

  it("preserves remount cache across the settled live-delta dedupe window", () => {
    const response: TimeseriesResponse = {
      rangeStart: "2026-04-08T00:00:00Z",
      rangeEnd: "2026-04-08T00:01:00Z",
      bucketSeconds: 60,
      points: [],
    };
    writeTimeseriesRemountCache(
      "1d",
      { bucket: "1m" },
      response,
      2_000,
      new Map([
        [
          "invoke-1",
          {
            bucketStart: "2026-04-08T00:00:00Z",
            bucketEnd: "2026-04-08T00:01:00Z",
            bucketStartEpoch: 1_744_070_400,
            bucketEndEpoch: 1_744_070_460,
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            inFlightCount: 0,
            totalTokens: 10,
            totalCost: 0.1,
          },
        ],
      ]),
      new Map([["invoke-1", 2_000]]),
    );

    expect(
      readTimeseriesRemountCache(
        "1d",
        { bucket: "1m" },
        2_000 + TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS - 1,
      ),
    ).not.toBeNull();
    expect(
      readTimeseriesRemountCache(
        "1d",
        { bucket: "1m" },
        2_000 + TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
      ),
    ).toBeNull();
  });

  it("prunes stale settled live deltas while preserving in-flight ones", () => {
    const pendingRecord: ApiInvocation = {
      id: 81,
      invokeId: "pending-live",
      occurredAt: "2026-04-08T00:00:10Z",
      status: "running",
      totalTokens: 0,
      cost: 0,
      createdAt: "2026-04-08T00:00:10Z",
    };
    const settledRecord: ApiInvocation = {
      id: 82,
      invokeId: "settled-live",
      occurredAt: "2026-04-08T00:00:20Z",
      status: "failed",
      totalTokens: 22,
      cost: 0.18,
      errorMessage: "upstream timed out",
      failureKind: "upstream_timeout",
      createdAt: "2026-04-08T00:00:20Z",
    };
    const pendingDelta = {
      bucketStart: "2026-04-08T00:00:00Z",
      bucketEnd: "2026-04-08T00:01:00Z",
      bucketStartEpoch: 1_744_070_400,
      bucketEndEpoch: 1_744_070_460,
      totalCount: 1,
      successCount: 0,
      failureCount: 0,
      inFlightCount: 1,
      totalTokens: 0,
      totalCost: 0,
    };
    const settledDelta = {
      bucketStart: "2026-04-08T00:00:00Z",
      bucketEnd: "2026-04-08T00:01:00Z",
      bucketStartEpoch: 1_744_070_400,
      bucketEndEpoch: 1_744_070_460,
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      inFlightCount: 0,
      totalTokens: 22,
      totalCost: 0.18,
    };

    const liveRecordDeltas = new Map<string, typeof pendingDelta>();
    const settledLiveRecordUpdatedAt = new Map<string, number>();

    trackTimeseriesLiveRecordDelta(
      liveRecordDeltas,
      settledLiveRecordUpdatedAt,
      "pending-live-2026-04-08T00:00:10Z",
      pendingRecord,
      pendingDelta,
      10_000,
    );
    trackTimeseriesLiveRecordDelta(
      liveRecordDeltas,
      settledLiveRecordUpdatedAt,
      "settled-live-2026-04-08T00:00:20Z",
      settledRecord,
      settledDelta,
      10_000,
    );

    expect(liveRecordDeltas.get("pending-live-2026-04-08T00:00:10Z")).toEqual(
      pendingDelta,
    );
    expect(liveRecordDeltas.get("settled-live-2026-04-08T00:00:20Z")).toEqual(
      settledDelta,
    );
    expect(settledLiveRecordUpdatedAt.size).toBe(1);
    expect(
      settledLiveRecordUpdatedAt.has("pending-live-2026-04-08T00:00:10Z"),
    ).toBe(false);
    expect(
      settledLiveRecordUpdatedAt.has("settled-live-2026-04-08T00:00:20Z"),
    ).toBe(true);

    pruneTrackedTimeseriesLiveRecordDeltas(
      liveRecordDeltas,
      settledLiveRecordUpdatedAt,
      10_000 + TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS + 1,
    );

    expect(liveRecordDeltas.has("pending-live-2026-04-08T00:00:10Z")).toBe(
      true,
    );
    expect(liveRecordDeltas.has("settled-live-2026-04-08T00:00:20Z")).toBe(
      false,
    );
    expect(settledLiveRecordUpdatedAt.size).toBe(0);
  });

  it("caps tracked settled live deltas to the newest bounded window", () => {
    const liveRecordDeltas = new Map<
      string,
      {
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
      }
    >();
    const settledLiveRecordUpdatedAt = new Map<string, number>();

    const createSettledRecord = (invokeId: string): ApiInvocation => ({
      id: Number.parseInt(invokeId.replace(/\D/g, ""), 10) || 1,
      invokeId,
      occurredAt: "2026-04-08T00:00:20Z",
      status: "failed",
      totalTokens: 1,
      cost: 0.01,
      errorMessage: "upstream timed out",
      failureKind: "upstream_timeout",
      createdAt: "2026-04-08T00:00:20Z",
    });
    const createDelta = (index: number) => ({
      bucketStart: "2026-04-08T00:00:00Z",
      bucketEnd: "2026-04-08T00:01:00Z",
      bucketStartEpoch: 1_744_070_400,
      bucketEndEpoch: 1_744_070_460,
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      inFlightCount: 0,
      totalTokens: index,
      totalCost: index / 100,
    });

    for (
      let index = 0;
      index <= MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS;
      index += 1
    ) {
      const key = `settled-${index}`;
      trackTimeseriesLiveRecordDelta(
        liveRecordDeltas,
        settledLiveRecordUpdatedAt,
        key,
        createSettledRecord(key),
        createDelta(index),
        1_000 + index,
        TIMESERIES_SETTLED_LIVE_DELTA_TTL_MS,
        MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
      );
    }

    expect(settledLiveRecordUpdatedAt.size).toBe(
      MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS,
    );
    expect(liveRecordDeltas.size).toBe(MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS);
    expect(liveRecordDeltas.has("settled-0")).toBe(false);
    expect(
      liveRecordDeltas.has(`settled-${MAX_TRACKED_SETTLED_LIVE_RECORD_DELTAS}`),
    ).toBe(true);
  });
});
