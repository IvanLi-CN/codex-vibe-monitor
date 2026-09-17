import { expect, it, vi } from "vitest";
import { fetchForwardProxyLiveStats } from "./api";

it("normalizes live proxy stats payload", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-02T00:00:00Z",
          bucketSeconds: 3600,
          nodes: [
            {
              key: "__direct__",
              source: "direct",
              displayName: "Direct",
              weight: 1,
              penalized: false,
              stats: {
                oneMinute: {
                  attempts: 2,
                  successRate: 0.5,
                  avgLatencyMs: 123,
                },
                fifteenMinutes: {
                  attempts: 10,
                  successRate: 0.6,
                  avgLatencyMs: 130,
                },
                oneHour: {
                  attempts: 40,
                  successRate: 0.7,
                  avgLatencyMs: 140,
                },
                oneDay: {
                  attempts: 200,
                  successRate: 0.8,
                  avgLatencyMs: 150,
                },
                sevenDays: {
                  attempts: 1200,
                  successRate: 0.9,
                  avgLatencyMs: 160,
                },
              },
              last24h: [
                {
                  bucketStart: "2026-03-01T00:00:00Z",
                  bucketEnd: "2026-03-01T01:00:00Z",
                  successCount: 3,
                  failureCount: 1,
                },
                {
                  bucketStart: "",
                  bucketEnd: "",
                  successCount: 99,
                  failureCount: 99,
                },
              ],
              weight24h: [
                {
                  bucketStart: "2026-03-01T00:00:00Z",
                  bucketEnd: "2026-03-01T01:00:00Z",
                  sampleCount: 3,
                  minWeight: 0.32,
                  maxWeight: 0.95,
                  avgWeight: 0.61,
                  lastWeight: 0.88,
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

  const response = await fetchForwardProxyLiveStats();
  expect(response.bucketSeconds).toBe(3600);
  expect(response.nodes).toHaveLength(1);
  expect(response.nodes[0].displayName).toBe("Direct");
  expect(response.nodes[0].stats.oneMinute.attempts).toBe(2);
  expect(response.nodes[0].last24h).toHaveLength(1);
  expect(response.nodes[0].last24h[0].successCount).toBe(3);
  expect(response.nodes[0].last24h[0].failureCount).toBe(1);
  expect(response.nodes[0].weight24h).toHaveLength(1);
  expect(response.nodes[0].weight24h[0].sampleCount).toBe(3);
  expect(response.nodes[0].weight24h[0].lastWeight).toBe(0.88);
});
it("falls back to empty weight buckets when backend payload omits weight24h", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-01T00:00:00Z",
          rangeEnd: "2026-03-02T00:00:00Z",
          bucketSeconds: 3600,
          nodes: [
            {
              key: "__direct__",
              source: "direct",
              displayName: "Direct",
              weight: 1,
              penalized: false,
              stats: {
                oneMinute: { attempts: 0 },
                fifteenMinutes: { attempts: 0 },
                oneHour: { attempts: 0 },
                oneDay: { attempts: 0 },
                sevenDays: { attempts: 0 },
              },
              last24h: [],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchForwardProxyLiveStats();
  expect(response.nodes).toHaveLength(1);
  expect(response.nodes[0].weight24h).toEqual([]);
});
