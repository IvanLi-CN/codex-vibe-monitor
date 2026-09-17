/** @vitest-environment jsdom */
import { act } from "react";
import { expect, it } from "vitest";
import type { TimeseriesResponse } from "../../lib/api";
import { TodayStatsOverview } from "./TodayStatsOverview";
import { buildTimeseriesWithLatency, host, render, root } from "./TodayStatsOverview.test-support";

it("uses the complete-range first-byte average when the latest window is idle", () => {
  render(
    <TodayStatsOverview
      stats={{
        totalCount: 10,
        successCount: 10,
        failureCount: 0,
        totalCost: 0.5,
        totalTokens: 1000,
      }}
      rate={{
        tokensPerMinute: 0,
        spendRate: 0,
        windowMinutes: 5,
        available: true,
      }}
      timeseries={{
        rangeStart: "2026-04-10T00:00:00.000Z",
        rangeEnd: "2026-04-10T00:08:00.000Z",
        bucketSeconds: 60,
        points: [
          {
            bucketStart: "2026-04-10T00:00:00.000Z",
            bucketEnd: "2026-04-10T00:01:00.000Z",
            totalCount: 2,
            successCount: 2,
            failureCount: 0,
            totalTokens: 1000,
            totalCost: 0.5,
            firstTokenSampleCount: 2,
            firstTokenAvgMs: 500,
          },
        ],
      }}
      loading={false}
      error={null}
    />,
  );

  expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe(
    "—",
  );
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')
      ?.textContent,
  ).toContain("—");
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.textContent,
  ).toContain("—");
});
it("falls back to an empty avg-total secondary metric when no completed-call total exists", () => {
  const baseTimeseries = buildTimeseriesWithLatency();
  const timeseriesWithoutAvgTotal: TimeseriesResponse = {
    ...baseTimeseries,
    points: baseTimeseries.points.map((point) => ({
      ...point,
      avgTotalMs: null,
      totalLatencySampleCount: 0,
    })),
  };

  render(
    <TodayStatsOverview
      stats={{
        totalCount: 32,
        successCount: 28,
        failureCount: 4,
        totalCost: 1.2,
        totalTokens: 3200,
      }}
      rate={{
        tokensPerMinute: 416,
        spendRate: 0.1,
        windowMinutes: 5,
        available: true,
      }}
      timeseries={timeseriesWithoutAvgTotal}
      loading={false}
      error={null}
    />,
  );

  expect(
    host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-response"]')
      ?.textContent,
  ).toContain("—");
});
it("keeps the complete-range first-byte average stable when now changes", () => {
  const timeseries: TimeseriesResponse = {
    rangeStart: "2026-04-10T00:00:00.000Z",
    rangeEnd: "2026-04-10T00:03:00.000Z",
    bucketSeconds: 60,
    points: [
      {
        bucketStart: "2026-04-10T00:02:00.000Z",
        bucketEnd: "2026-04-10T00:03:00.000Z",
        totalCount: 2,
        successCount: 2,
        failureCount: 0,
        totalTokens: 1000,
        totalCost: 0.5,
        firstTokenSampleCount: 2,
        firstTokenAvgMs: 500,
      },
    ],
  };

  const renderOverview = (now: Date) => (
    <TodayStatsOverview
      stats={{
        totalCount: 2,
        successCount: 2,
        failureCount: 0,
        totalCost: 0.5,
        totalTokens: 1000,
      }}
      rate={{
        tokensPerMinute: 0,
        spendRate: 0,
        windowMinutes: 5,
        available: true,
      }}
      timeseries={timeseries}
      loading={false}
      error={null}
      now={now}
    />
  );

  render(renderOverview(new Date("2026-04-10T00:04:00.000Z")));

  expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe(
    "—",
  );

  act(() => {
    root?.render(renderOverview(new Date("2026-04-10T00:10:00.000Z")));
  });

  expect(host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent).toBe(
    "—",
  );
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')
      ?.textContent,
  ).toContain("—");
});
