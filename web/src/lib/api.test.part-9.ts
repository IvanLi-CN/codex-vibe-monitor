import { expect, it, vi } from "vitest";
import { fetchTimeseries } from "./api";

it("normalizes first-response-byte-total fields from the timeseries payload", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-26T12:00:00Z",
          rangeEnd: "2026-03-26T13:00:00Z",
          bucketSeconds: 900,
          effectiveBucket: "15m",
          availableBuckets: ["15m", "1h"],
          bucketLimitedToDaily: false,
          points: [
            {
              bucketStart: "2026-03-26T12:00:00Z",
              bucketEnd: "2026-03-26T12:15:00Z",
              totalCount: 11,
              successCount: 10,
              failureCount: 1,
              inFlightCount: 2,
              totalTokens: 193414,
              inputTokens: 121000,
              outputTokens: 72414,
              cacheInputTokens: 83000,
              reasoningTokens: 18400,
              totalCost: 0.0543,
              firstByteSampleCount: 10,
              firstByteAvgMs: 81.7,
              firstByteP95Ms: 95.2,
              firstTokenSampleCount: 10,
              firstTokenAvgMs: 43890,
              firstTokenP95Ms: 52340,
            },
            {
              bucketStart: "2026-03-26T12:15:00Z",
              bucketEnd: "2026-03-26T12:30:00Z",
              totalCount: 0,
              successCount: 0,
              failureCount: 0,
              inFlightCount: Number.NaN,
              totalTokens: 0,
              totalCost: 0,
              firstByteSampleCount: 1,
              firstByteAvgMs: 81.7,
              firstByteP95Ms: 95.2,
              firstTokenSampleCount: 1,
              firstTokenAvgMs: 18225.02,
              firstTokenP95Ms: 18225.02,
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchTimeseries("1h", { bucket: "15m" });
  expect(response.bucketSeconds).toBe(900);
  expect(response.points).toHaveLength(2);
  expect(response.points[0].inFlightCount).toBe(2);
  expect(response.points[0]).toMatchObject({
    inputTokens: 121000,
    outputTokens: 72414,
    cacheInputTokens: 83000,
    reasoningTokens: 18400,
  });
  expect(response.points[1].inputTokens).toBeUndefined();
  expect(response.points[1].outputTokens).toBeUndefined();
  expect(response.points[1].reasoningTokens).toBeUndefined();
  expect(response.points[0].firstTokenSampleCount).toBe(10);
  expect(response.points[0].firstTokenAvgMs).toBe(43890);
  expect(response.points[0].firstTokenP95Ms).toBe(52340);
  expect(response.points[1].firstByteSampleCount).toBe(0);
  expect(response.points[1].firstByteAvgMs).toBeNull();
  expect(response.points[1].firstByteP95Ms).toBeNull();
  expect(response.points[1].firstTokenSampleCount).toBe(0);
  expect(response.points[1].firstTokenAvgMs).toBeNull();
  expect(response.points[1].firstTokenP95Ms).toBeNull();
  expect(response.points[1].inFlightCount).toBe(0);
});
it("preserves total-latency sample counts when avgTotalMs is absent", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-26T12:00:00Z",
          rangeEnd: "2026-03-26T13:00:00Z",
          bucketSeconds: 900,
          points: [
            {
              bucketStart: "2026-03-26T12:00:00Z",
              bucketEnd: "2026-03-26T12:15:00Z",
              totalCount: 4,
              successCount: 4,
              failureCount: 0,
              totalTokens: 1200,
              totalCost: 0.2,
              totalLatencySampleCount: 4,
              avgTotalMs: null,
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchTimeseries("1h");

  expect(response.points).toHaveLength(1);
  expect(response.points[0].avgTotalMs).toBeNull();
  expect(response.points[0].totalLatencySampleCount).toBe(4);
});
it("adds upstreamAccountId to timeseries query parameters", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL) => {
    return new Response(
      JSON.stringify({
        rangeStart: "2026-03-26T12:00:00Z",
        rangeEnd: "2026-03-26T13:00:00Z",
        bucketSeconds: 60,
        points: [],
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchTimeseries("today", {
    bucket: "1m",
    upstreamAccountId: 42,
  });

  const url = String(fetchMock.mock.calls[0]?.[0] ?? "");
  expect(url).toContain("range=today");
  expect(url).toContain("bucket=1m");
  expect(url).toContain("upstreamAccountId=42");
});
