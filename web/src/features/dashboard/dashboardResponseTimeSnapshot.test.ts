import { describe, expect, it } from "vitest";
import type { TimeseriesResponse } from "../../lib/api";
import { buildDashboardResponseTimeSnapshot } from "./dashboardResponseTimeSnapshot";

function buildPoint(
  minute: number,
  avgMs: number | null,
  sampleCount: number,
  totalCount = sampleCount,
) {
  const start = Date.parse("2026-04-10T00:00:00.000Z") + minute * 60_000;
  return {
    bucketStart: new Date(start).toISOString(),
    bucketEnd: new Date(start + 60_000).toISOString(),
    totalCount,
    successCount: totalCount,
    failureCount: 0,
    totalTokens: 0,
    totalCost: 0,
    firstResponseByteTotalAvgMs: avgMs,
    firstResponseByteTotalSampleCount: sampleCount,
  };
}

function buildResponse(points: TimeseriesResponse["points"]): TimeseriesResponse {
  return {
    rangeStart: "2026-04-10T00:00:00.000Z",
    rangeEnd: "2026-04-10T00:08:00.000Z",
    bucketSeconds: 60,
    points,
  };
}

describe("buildDashboardResponseTimeSnapshot", () => {
  it("uses the latest five-minute rolling window and a sample-weighted day average", () => {
    const snapshot = buildDashboardResponseTimeSnapshot(
      buildResponse([
        buildPoint(0, 500, 2),
        buildPoint(4, 1000, 1),
        buildPoint(5, 2000, 3),
        buildPoint(7, 5000, 1),
      ]),
      { now: new Date("2026-04-10T00:08:00.000Z") },
    );

    expect(snapshot?.responseTimeMs).toBeCloseTo(2400, 6);
    expect(snapshot?.dayAverageMs).toBeCloseTo(1857.142857, 6);
  });

  it("drops zero-call latency samples and reports no current value when the recent window is empty", () => {
    const snapshot = buildDashboardResponseTimeSnapshot(
      buildResponse([buildPoint(0, 500, 2), buildPoint(6, 2400, 3, 0)]),
      { now: new Date("2026-04-10T00:08:00.000Z") },
    );

    expect(snapshot?.responseTimeMs).toBeNull();
    expect(snapshot?.dayAverageMs).toBeCloseTo(500, 6);
  });

  it("does not fall back to older same-day latency when the active tail is empty", () => {
    const snapshot = buildDashboardResponseTimeSnapshot(
      buildResponse([buildPoint(1, 600, 2), buildPoint(2, 800, 2)]),
      { now: new Date("2026-04-10T00:12:00.000Z") },
    );

    expect(snapshot?.responseTimeMs).toBeNull();
    expect(snapshot?.dayAverageMs).toBeCloseTo(700, 6);
  });
});
