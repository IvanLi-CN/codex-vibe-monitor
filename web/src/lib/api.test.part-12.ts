import { expect, it, vi } from "vitest";
import { fetchForwardProxyTimeseries } from "./api";

it("normalizes historical proxy timeseries payload", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          rangeStart: "2026-01-01T00:00:00Z",
          rangeEnd: "2026-01-08T00:00:00Z",
          bucketSeconds: 3600,
          effectiveBucket: "1h",
          availableBuckets: ["1h"],
          nodes: [
            {
              key: "__archived__",
              source: "archived",
              displayName: "Archived Proxy",
              weight: 0.8,
              penalized: false,
              buckets: [
                {
                  bucketStart: "2026-01-01T00:00:00Z",
                  bucketEnd: "2026-01-01T01:00:00Z",
                  successCount: 4,
                  failureCount: 1,
                },
                {
                  bucketStart: "",
                  bucketEnd: "",
                  successCount: 99,
                  failureCount: 99,
                },
              ],
              weightBuckets: [
                {
                  bucketStart: "2026-01-01T00:00:00Z",
                  bucketEnd: "2026-01-01T01:00:00Z",
                  sampleCount: 2,
                  minWeight: 0.5,
                  maxWeight: 0.9,
                  avgWeight: 0.7,
                  lastWeight: 0.8,
                },
                {
                  bucketStart: "",
                  bucketEnd: "",
                  sampleCount: 99,
                  minWeight: 1,
                  maxWeight: 1,
                  avgWeight: 1,
                  lastWeight: 1,
                },
              ],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchForwardProxyTimeseries("7d", {
    bucket: "1h",
    timeZone: "UTC",
  });

  expect(response.rangeStart).toBe("2026-01-01T00:00:00Z");
  expect(response.effectiveBucket).toBe("1h");
  expect(response.availableBuckets).toEqual(["1h"]);
  expect(response.nodes).toHaveLength(1);
  expect(response.nodes[0].displayName).toBe("Archived Proxy");
  expect(response.nodes[0].buckets).toHaveLength(1);
  expect(response.nodes[0].buckets[0].successCount).toBe(4);
  expect(response.nodes[0].weightBuckets).toHaveLength(1);
  expect(response.nodes[0].weightBuckets[0].lastWeight).toBe(0.8);
});
it("rejects non-whole-hour proxy history time zones instead of rewriting them", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await expect(
    fetchForwardProxyTimeseries("today", {
      bucket: "1h",
      timeZone: "Asia/Kolkata",
    }),
  ).rejects.toThrow("whole-hour UTC offsets");
  expect(fetchMock).not.toHaveBeenCalled();
});
it("rejects seasonal half-hour proxy history time zones when the queried range crosses them", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-10T00:00:00Z"));
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await expect(
    fetchForwardProxyTimeseries("1mo", {
      bucket: "1h",
      timeZone: "Australia/Lord_Howe",
    }),
  ).rejects.toThrow("whole-hour UTC offsets");
  expect(fetchMock).not.toHaveBeenCalled();
});
it("rejects short proxy history ranges that cross a sub-hour DST transition", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-03T15:40:00Z"));
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await expect(
    fetchForwardProxyTimeseries("30m", {
      bucket: "1h",
      timeZone: "Australia/Lord_Howe",
    }),
  ).rejects.toThrow("whole-hour UTC offsets");
  expect(fetchMock).not.toHaveBeenCalled();
});
