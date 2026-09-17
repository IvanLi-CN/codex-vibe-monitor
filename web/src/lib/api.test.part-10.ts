import { expect, it, vi } from "vitest";
import { fetchParallelWorkStats, fetchParallelWorkStatsConditional } from "./api";

it("normalizes the current page-period parallel-work window", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          current: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 60,
            completeBucketCount: 10080,
            activeBucketCount: 4132,
            activeMinuteCount: 4132,
            minCount: 0,
            maxCount: 18,
            avgCount: 4.67,
            points: [
              {
                bucketStart: "2026-03-07T10:00:00Z",
                bucketEnd: "2026-03-07T10:01:00Z",
                parallelCount: 4,
              },
            ],
          },
          minute7d: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 60,
            completeBucketCount: 10080,
            activeBucketCount: 4132,
            minCount: 0,
            maxCount: 18,
            avgCount: 4.67,
            points: [
              {
                bucketStart: "2026-03-07T10:00:00Z",
                bucketEnd: "2026-03-07T10:01:00Z",
                parallelCount: 4,
              },
            ],
          },
          hour30d: {
            rangeStart: "2026-02-06T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 3600,
            completeBucketCount: 720,
            activeBucketCount: 321,
            minCount: 0,
            maxCount: 9,
            avgCount: 2.13,
            points: [
              {
                bucketStart: "2026-03-07T00:00:00Z",
                bucketEnd: "2026-03-07T01:00:00Z",
                parallelCount: 2,
              },
            ],
          },
          dayAll: {
            rangeStart: "2026-03-08T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 86400,
            completeBucketCount: 0,
            activeBucketCount: 0,
            minCount: null,
            maxCount: null,
            avgCount: null,
            points: [],
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchParallelWorkStats();
  expect(response.current.points[0]?.parallelCount).toBe(4);
  expect(response.current.activeMinuteCount).toBe(4132);
  expect(response.minute7d.points[0]?.parallelCount).toBe(4);
  expect(response.hour30d.avgCount).toBe(2.13);
  expect(response.dayAll.completeBucketCount).toBe(0);
  expect(response.dayAll.avgCount).toBeNull();
});
it("preserves the caller time zone for fixed sub-hour offsets", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => {
    return new Response(
      JSON.stringify({
        minute7d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 60,
          completeBucketCount: 1,
          activeBucketCount: 1,
          minCount: 1,
          maxCount: 1,
          avgCount: 1,
          points: [
            {
              bucketStart: "2026-03-07T10:00:00Z",
              bucketEnd: "2026-03-07T10:01:00Z",
              parallelCount: 1,
            },
          ],
        },
        hour30d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 3600,
          completeBucketCount: 1,
          activeBucketCount: 1,
          minCount: 1,
          maxCount: 1,
          avgCount: 1,
          points: [
            {
              bucketStart: "2026-03-07T10:00:00Z",
              bucketEnd: "2026-03-07T11:00:00Z",
              parallelCount: 1,
            },
          ],
        },
        dayAll: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 86400,
          completeBucketCount: 1,
          activeBucketCount: 1,
          minCount: 1,
          maxCount: 1,
          avgCount: 1,
          points: [
            {
              bucketStart: "2026-03-07T00:00:00Z",
              bucketEnd: "2026-03-08T00:00:00Z",
              parallelCount: 1,
            },
          ],
        },
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchParallelWorkStats({
    range: "7d",
    bucket: "1h",
    timeZone: "Asia/Kolkata",
  });

  expect(fetchMock).toHaveBeenCalledTimes(1);
  const firstArg = fetchMock.mock.calls.at(0)?.at(0) as RequestInfo | URL | undefined;
  expect(firstArg).toBeDefined();
  expect(String(firstArg)).toBe(
    "/api/stats/parallel-work?range=7d&bucket=1h&timeZone=Asia%2FKolkata",
  );
});
it("preserves the caller time zone for seasonal sub-hour offsets", async () => {
  const fetchMock = vi.fn(async () => {
    return new Response(
      JSON.stringify({
        minute7d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 60,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
        hour30d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 3600,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
        dayAll: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 86400,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchParallelWorkStats({ timeZone: "Australia/Lord_Howe" });

  expect(fetchMock).toHaveBeenCalledTimes(1);
  const firstArg = fetchMock.mock.calls.at(0)?.at(0) as RequestInfo | URL | undefined;
  expect(firstArg).toBeDefined();
  expect(String(firstArg)).toBe("/api/stats/parallel-work?timeZone=Australia%2FLord_Howe");
});
it("adds upstreamAccountId to parallel-work query parameters", async () => {
  const fetchMock = vi.fn(async () => {
    return new Response(
      JSON.stringify({
        current: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 60,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
        minute7d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 60,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
        hour30d: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 3600,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
        dayAll: {
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-08T00:00:00Z",
          bucketSeconds: 86400,
          completeBucketCount: 0,
          activeBucketCount: 0,
          minCount: null,
          maxCount: null,
          avgCount: null,
          points: [],
        },
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchParallelWorkStats({
    range: "today",
    bucket: "1m",
    timeZone: "UTC",
    upstreamAccountId: 42,
  });

  expect(fetchMock).toHaveBeenCalledTimes(1);
  const firstArg = fetchMock.mock.calls.at(0)?.at(0) as RequestInfo | URL | undefined;
  expect(firstArg).toBeDefined();
  expect(String(firstArg)).toBe(
    "/api/stats/parallel-work?range=today&bucket=1m&upstreamAccountId=42&timeZone=UTC",
  );
});
it("sends If-None-Match and exposes 304 responses for cached parallel-work payloads", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => {
    return new Response(null, {
      status: 304,
      headers: { ETag: '"parallel-work-cached"' },
    });
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  const response = await fetchParallelWorkStatsConditional({
    range: "today",
    bucket: "1m",
    timeZone: "UTC",
    etag: '"parallel-work-cached"',
  });

  expect(response).toEqual({
    data: null,
    etag: '"parallel-work-cached"',
    notModified: true,
  });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  const init = fetchMock.mock.calls[0]?.[1] as RequestInit | undefined;
  expect(init?.headers).toMatchObject({
    "If-None-Match": '"parallel-work-cached"',
  });
});
