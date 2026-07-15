/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { ParallelWorkStatsResponse, TimeseriesResponse } from "../../lib/api";
import { TodayStatsOverview } from "./TodayStatsOverview";

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string, values?: { timezone?: string }) => {
      const map: Record<string, string> = {
        "dashboard.today.title": "Today summary",
        "dashboard.today.subtitle": `Accumulated in natural day (${values?.timezone ?? "UTC"})`,
        "dashboard.today.dayBadge": "Today",
        "dashboard.today.tokensPerMinute": "TPM",
        "dashboard.today.spendRate": "Spend rate",
        "dashboard.today.responseTime": "Response time",
        "dashboard.today.firstResponseTime": "Time to first byte",
        "dashboard.today.responseTimeDescription":
          "Response time uses the latest 5-minute active tail.",
        "dashboard.today.inProgressConversations": "In progress",
        "dashboard.today.queuedInvocations": "Queued",
        "dashboard.today.parallelConversations": "Parallel conversations",
        "dashboard.today.todayCost": "Today cost",
        "dashboard.today.yesterdayCost": "Yesterday cost",
        "dashboard.today.todayTokens": "Today Token",
        "dashboard.today.yesterdayTokens": "Yesterday Token",
        "dashboard.today.tokensPerMinuteDescription":
          "TPM uses the active tail inside the latest 5-minute window.",
        "dashboard.today.spendRateDescription":
          "Spend rate uses the active tail inside the latest 5-minute window.",
        "dashboard.today.inProgressConversationsDescription":
          "Current running or pending invocations, counted per invocation.",
        "dashboard.today.queuedInvocationsDescription":
          "Queued invocations not yet requesting upstream.",
        "dashboard.today.parallelConversationsDescription":
          "Distinct prompt-cache conversations counted in the latest minute bucket.",
        "dashboard.today.successDescription": "Successful calls in the selected day.",
        "dashboard.today.failuresDescription": "Failed calls in the selected day.",
        "dashboard.today.totalCostDescription": "Total cost in the selected day.",
        "dashboard.today.totalTokensDescription": "Total tokens in the selected day.",
        "dashboard.usageBreakdown.title": "Usage details",
        "dashboard.today.secondary.dayAverage": "Day avg",
        "dashboard.today.secondary.previous7dAverage": "7d daily avg",
        "dashboard.today.secondary.vsYesterday": "vs yesterday",
        "dashboard.today.secondary.comparison": "Comparison",
        "dashboard.today.secondary.perConversation": "Per conversation",
        "dashboard.today.secondary.retry": "Retry",
        "dashboard.today.secondary.inProgress": "In progress",
        "dashboard.today.secondary.p95": "P95",
        "dashboard.today.secondary.failed": "Failed",
        "dashboard.today.secondary.failureRate": "Failure rate",
        "dashboard.today.secondary.cacheHitRate": "Cache hit",
        "stats.cards.loadError": "Load error",
        "stats.cards.success": "Success",
        "stats.cards.failures": "Failures",
        "stats.cards.totalCost": "Cost",
        "stats.cards.totalTokens": "Tokens",
      };
      return map[key] ?? key;
    },
  }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let metricContainerWidth = 640;
let metricTileWidth = 280;

function buildTimeseriesWithLatency(): TimeseriesResponse {
  const points = Array.from({ length: 8 }, (_, index) => {
    const bucketStart = new Date(
      Date.parse("2026-04-10T00:00:00.000Z") + index * 60_000,
    ).toISOString();
    const bucketEnd = new Date(
      Date.parse("2026-04-10T00:01:00.000Z") + index * 60_000,
    ).toISOString();
    const totalCount = index % 3 === 0 ? 0 : 2 + index;
    const sampleCount = totalCount > 0 ? 2 + index : 0;

    return {
      bucketStart,
      bucketEnd,
      totalCount,
      successCount: totalCount,
      failureCount: 0,
      totalTokens: 78000 + index * 6100,
      cacheInputTokens: 18000 + index * 1200,
      totalCost: Number((1.1 + index * 0.08).toFixed(2)),
      avgTotalMs: sampleCount > 0 ? Number((1260 + index * 73.5).toFixed(1)) : null,
      totalLatencySampleCount: sampleCount,
      firstResponseByteTotalSampleCount: sampleCount,
      firstResponseByteTotalAvgMs: sampleCount > 0 ? Number((820 + index * 41.5).toFixed(1)) : null,
      firstResponseByteTotalP95Ms: sampleCount > 0 ? Number((980 + index * 58.5).toFixed(1)) : null,
    };
  });

  return {
    rangeStart: "2026-04-10T00:00:00.000Z",
    rangeEnd: "2026-04-10T00:08:00.000Z",
    bucketSeconds: 60,
    points,
  };
}

function buildParallelWorkStats(
  currentCounts: number[] = [1, 3],
  currentAverage = 2,
  yesterdayAverage = 4,
): {
  current: ParallelWorkStatsResponse["current"];
  minute7d: ParallelWorkStatsResponse["minute7d"];
  hour30d: ParallelWorkStatsResponse["hour30d"];
  dayAll: ParallelWorkStatsResponse["dayAll"];
} {
  return {
    current: {
      rangeStart: "2026-04-10T00:00:00.000Z",
      rangeEnd: "2026-04-10T00:02:00.000Z",
      bucketSeconds: 60,
      completeBucketCount: currentCounts.length,
      activeBucketCount: currentCounts.length,
      minCount: Math.min(...currentCounts),
      maxCount: Math.max(...currentCounts),
      avgCount: currentAverage,
      points: currentCounts.map((parallelCount, index) => ({
        bucketStart: new Date(
          Date.parse("2026-04-10T00:00:00.000Z") + index * 60_000,
        ).toISOString(),
        bucketEnd: new Date(Date.parse("2026-04-10T00:01:00.000Z") + index * 60_000).toISOString(),
        parallelCount,
      })),
    },
    minute7d: {
      rangeStart: "2026-04-03T00:00:00.000Z",
      rangeEnd: "2026-04-10T00:00:00.000Z",
      bucketSeconds: 60,
      completeBucketCount: 0,
      activeBucketCount: 0,
      minCount: null,
      maxCount: null,
      avgCount: null,
      points: [],
    },
    hour30d: {
      rangeStart: "2026-03-11T00:00:00.000Z",
      rangeEnd: "2026-04-10T00:00:00.000Z",
      bucketSeconds: 3600,
      completeBucketCount: 0,
      activeBucketCount: 0,
      minCount: null,
      maxCount: null,
      avgCount: null,
      points: [],
    },
    dayAll: {
      rangeStart: "2026-01-01T00:00:00.000Z",
      rangeEnd: "2026-04-10T00:00:00.000Z",
      bucketSeconds: 86400,
      completeBucketCount: 1,
      activeBucketCount: 1,
      minCount: yesterdayAverage,
      maxCount: yesterdayAverage,
      avgCount: yesterdayAverage,
      points: [
        {
          bucketStart: "2026-04-09T00:00:00.000Z",
          bucketEnd: "2026-04-10T00:00:00.000Z",
          parallelCount: yesterdayAverage,
        },
      ],
    },
  };
}

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });

  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === "true") {
        return metricContainerWidth;
      }
      if ((this as HTMLElement).dataset.testid === "today-stats-metric-tile") {
        return metricTileWidth;
      }
      return 0;
    },
  });

  Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === "true") {
        return (this.textContent?.length ?? 0) * 16;
      }
      return 0;
    },
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  metricContainerWidth = 640;
  metricTileWidth = 280;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("TodayStatsOverview", () => {
  it("prefers explicit current snapshot metrics over model-performance totals for TPM and first-byte card values", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12,
          successCount: 10,
          failureCount: 2,
          totalCost: 1.2,
          totalTokens: 9999,
        }}
        rate={{
          tokensPerMinute: 9999,
          spendRate: 0.1,
          windowMinutes: 1,
          available: true,
          currentFirstResponseByteTotalAvgMs: 2750,
          currentAvgTotalMs: 4900,
        }}
        modelPerformance={{
          available: true,
          total: {
            tokensPerMinute: 1200,
            avgFirstResponseByteTotalMs: 1500,
          },
          models: [
            {
              model: "gpt-5.6",
              reasoningEffort: null,
              tokensPerMinute: 1200,
              avgFirstResponseByteTotalMs: 1500,
            },
          ],
        }}
        loading={false}
        error={null}
      />,
    );

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent).toContain(
      "9,999",
    );
    expect(
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent,
    ).toMatch(/2\.75|2,75/);
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
        ?.textContent,
    ).toMatch(/4\.9|4,9/);

    const tpmTrigger = host?.querySelector('[aria-label="TPM dashboard.modelPerformance.title"]');
    act(() => {
      tpmTrigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(document.body.querySelector('[role="tooltip"]')?.textContent).toContain(
      "dashboard.modelPerformance.total",
    );
  });

  it("keeps the original in-progress tile and adds the queued value before success", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
          inProgressConversationCount: 11,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats()}
        comparisonParallelWorkStats={buildParallelWorkStats([4], 4, 4)}
        loading={false}
        error={null}
      />,
    );

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]');
    const tokenTile = host
      ?.querySelector('[data-testid="today-stats-value-total-tokens"]')
      ?.closest('[data-testid="today-stats-metric-tile"]');
    expect(grid?.className).toContain("min-[400px]:grid-cols-2");
    expect(grid?.className).not.toContain("sm:grid-cols-2");
    expect(grid?.className).toContain("lg:grid-cols-4");
    expect(grid?.className).toContain("xl:grid-cols-7");
    expect(tokenTile?.className).toContain("min-[400px]:col-span-2");
    expect(tokenTile?.className).toContain("lg:col-span-1");
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7);
    const tileLabels = Array.from(
      host?.querySelectorAll('[data-testid="today-stats-metric-tile"]') ?? [],
    ).map((tile) => tile.textContent ?? "");
    expect(tileLabels[2]).toContain("In progress");
    expect(tileLabels[3]).toContain("Time to first byte");
    expect(tileLabels[4]).toContain("Success");
    expect(host?.textContent).toContain("Today summary");
    expect(host?.textContent).toContain("TPM");
    expect(host?.textContent).toContain("Spend rate");
    expect(host?.textContent).toContain("Time to first byte");
    expect(host?.textContent).toContain("In progress");
    expect(host?.textContent).toContain("Today cost");
    expect(host?.textContent).toContain("Today Token");
    expect(
      host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')
        ?.textContent,
    ).toContain("11");
    expect(
      host?.querySelector('[data-testid="today-stats-value-queued-invocations"]')?.textContent,
    ).toContain("0");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-day-average"]')
        ?.textContent,
    ).toContain("2");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-delta"]')?.textContent,
    ).toContain("+175%");
  });

  it("uses a six-tile desktop grid when parallel conversations are hidden", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
          inProgressConversationCount: 11,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats()}
        loading={false}
        error={null}
        showInProgressConversations={false}
      />,
    );

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]');
    expect(grid?.className).toContain("lg:grid-cols-3");
    expect(grid?.className).toContain("xl:grid-cols-6");
    expect(grid?.className).not.toContain("xl:grid-cols-7");
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(6);
    expect(host?.textContent).not.toContain("In progress");
    expect(host?.textContent).toContain("Time to first byte");
    expect(host?.textContent).toContain("Today cost");
    expect(host?.textContent).toContain("Today Token");
  });

  it("keeps the in-progress tile secondary slots visible when bucket comparisons are unavailable", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
          inProgressConversationCount: 11,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={null}
        comparisonParallelWorkStats={null}
        loading={false}
        error={null}
      />,
    );

    const grid = host?.querySelector('[data-testid="today-stats-metrics-grid"]');
    expect(grid?.className).toContain("lg:grid-cols-4");
    expect(grid?.className).toContain("xl:grid-cols-7");
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7);
    expect(
      host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')
        ?.textContent,
    ).toContain("11");
    expect(
      host?.querySelector('[data-testid="today-stats-value-queued-invocations"]')?.textContent,
    ).toContain("0");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-delta"]')?.textContent,
    ).toContain("—");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-day-average"]')
        ?.textContent,
    ).toContain("—");
  });

  it("uses historical parallel semantics for the yesterday view while keeping the tile visible", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
          inProgressConversationCount: 11,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats([1, 3], 2, 4)}
        comparisonParallelWorkStats={null}
        loading={false}
        error={null}
        dayKind="yesterday"
      />,
    );

    expect(host?.textContent).toContain("Parallel conversations");
    expect(host?.textContent).not.toContain("In progress");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-retry"]')?.textContent,
    ).toContain("—");
    expect(
      host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')
        ?.textContent,
    ).toContain("—");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-day-average"]')
        ?.textContent,
    ).toContain("2");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-delta"]')?.textContent,
    ).toContain("—");
  });

  it("supports embedded mode without rendering the outer surface panel", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 32,
          successCount: 30,
          failureCount: 2,
          totalCost: 1.28,
          totalTokens: 4096,
          inProgressConversationCount: 3,
        }}
        rate={{
          tokensPerMinute: 320,
          spendRate: 0.13,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
        showSurface={false}
      />,
    );

    expect(host?.querySelector(".surface-panel")).toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-overview-card"]')).not.toBeNull();
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7);
  });

  it("hides the heading block when used inside the overview today tab", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12,
          successCount: 10,
          failureCount: 2,
          totalCost: 0.52,
          totalTokens: 2080,
          inProgressConversationCount: 2,
        }}
        rate={{
          tokensPerMinute: 416,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
        showSurface={false}
        showHeader={false}
        showDayBadge={false}
      />,
    );

    expect(host?.textContent).not.toContain("Today summary");
    expect(host?.textContent).not.toContain("Accumulated in natural day");
    expect(host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')).toHaveLength(7);
  });

  it("adds the queued phase value to the original in-progress tile", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
          inProgressConversationCount: 13,
          inProgressRetryConversationCount: 5,
          inProgressPhaseCounts: {
            queued: 5,
            requesting: 6,
            responding: 2,
          },
        }}
        rate={{
          tokensPerMinute: 1000.6,
          spendRate: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats([1, 3], 2, 4)}
        comparisonParallelWorkStats={buildParallelWorkStats([4], 4, 4)}
        loading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')
        ?.textContent,
    ).toContain("8");
    expect(
      host?.querySelector('[data-testid="today-stats-value-queued-invocations"]')?.textContent,
    ).toContain("5");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-delta"]')?.textContent,
    ).toContain("+100%");
  });

  it("renders partial loading only for rate tiles while summary metrics stay visible", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        comparisonStats={{
          totalCount: 176,
          successCount: 160,
          failureCount: 16,
          totalCost: 4.2,
          totalTokens: 16000,
        }}
        rate={null}
        loading={false}
        rateLoading
        parallelWorkStats={buildParallelWorkStats()}
        error={null}
      />,
    );

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')).toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate"]')).toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-value-success"]')?.textContent).toContain(
      "80",
    );
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-failures"]')?.textContent,
    ).toContain("8");
    expect(host?.textContent).toContain("vs yesterday");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-cost-delta"]')?.textContent,
    ).toContain("-50%");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent,
    ).toContain("-50%");
  });

  it("keeps the in-progress primary value from summary while secondary trend data comes from parallel buckets", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
          inProgressConversationCount: 11,
        }}
        rate={{
          tokensPerMinute: 1000.6,
          spendRate: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats([1, 3], 2, 4)}
        comparisonParallelWorkStats={buildParallelWorkStats([4], 4, 4)}
        loading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')
        ?.textContent,
    ).toContain("11");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-day-average"]')
        ?.textContent,
    ).toContain("2");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-delta"]')?.textContent,
    ).toContain("+175%");
  });

  it("keeps the yesterday retry slot empty for the closed range view", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
          inProgressConversationCount: 11,
          inProgressRetryConversationCount: 3,
        }}
        rate={{
          tokensPerMinute: 1000.6,
          spendRate: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        parallelWorkStats={buildParallelWorkStats([1, 3], 2, 4)}
        comparisonParallelWorkStats={buildParallelWorkStats([4], 4, 4)}
        loading={false}
        error={null}
        dayKind="yesterday"
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-retry"]')?.textContent,
    ).toContain("—");
  });

  it("renders TPM as a whole number even when the averaged rate is fractional", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
        }}
        rate={{
          tokensPerMinute: 1000.6,
          spendRate: 0.104,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
      />,
    );

    const tpmText = host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent ?? "";
    const spendRateText =
      host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent ?? "";

    expect(tpmText).toContain("1,001");
    expect(tpmText).not.toContain(".");
    expect(spendRateText).toContain("0.10");
    expect(spendRateText).not.toContain("$");
    expect(host?.querySelector('[data-testid="today-stats-value-tpm-icon"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate-icon"]')).not.toBeNull();
  });

  it("keeps rate currency tiles on the shared two-decimal full candidate when width allows", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
          inProgressConversationCount: 1,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={{
          rangeStart: "2026-04-10T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:01:00.000Z",
          bucketSeconds: 60,
          points: [
            {
              bucketStart: "2026-04-10T00:00:00.000Z",
              bucketEnd: "2026-04-10T00:01:00.000Z",
              totalCount: 1,
              successCount: 1,
              failureCount: 0,
              totalTokens: 1000,
              totalCost: 1,
              avgTotalMs: 1200,
              totalLatencySampleCount: 1,
              firstResponseByteTotalSampleCount: 1,
              firstResponseByteTotalAvgMs: 800,
            },
          ],
        }}
        loading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent,
    ).toContain("1.00");
    expect(
      host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent,
    ).not.toContain("$");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-spend-rate-day-average"]')
        ?.textContent,
    ).toContain("$1.48");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-spend-rate-per-conversation"]')
        ?.textContent,
    ).toContain("$1.00");
  });

  it("renders the new natural-day KPI helper semantics inline", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 100,
          successCount: 90,
          failureCount: 10,
          totalCost: 50,
          totalTokens: 12000,
          inProgressConversationCount: 6,
          inProgressRetryConversationCount: 2,
          inProgressAvgWaitMs: 1800,
          nonSuccessCost: 3.5,
          nonSuccessTokens: 420,
        }}
        comparisonStats={{
          totalCount: 80,
          successCount: 60,
          failureCount: 20,
          totalCost: 30,
          totalTokens: 8000,
          inProgressConversationCount: 4,
        }}
        timeseries={{
          rangeStart: "2026-04-10T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:03:00.000Z",
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
              avgTotalMs: 1390,
              totalLatencySampleCount: 2,
              firstResponseByteTotalSampleCount: 2,
              firstResponseByteTotalAvgMs: 500,
            },
          ],
        }}
        comparisonTimeseries={{
          rangeStart: "2026-04-09T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:00:00.000Z",
          bucketSeconds: 60,
          points: [10, 20, 30, 99].map((value, index) => ({
            bucketStart: new Date(
              Date.parse("2026-04-09T00:00:00.000Z") + index * 60_000,
            ).toISOString(),
            bucketEnd: new Date(
              Date.parse("2026-04-09T00:01:00.000Z") + index * 60_000,
            ).toISOString(),
            totalCount: value,
            successCount: value,
            failureCount: 0,
            totalTokens: value * 10,
            cacheInputTokens: 0,
            totalCost: value * 0.1,
          })),
        }}
        previous7dStats={{
          totalCount: 700,
          successCount: 630,
          failureCount: 70,
          totalCost: 140,
          totalTokens: 70000,
          inProgressAvgWaitMs: 1800,
        }}
        rate={{
          tokensPerMinute: 1200,
          spendRate: 0.6,
          windowMinutes: 1,
          available: true,
          currentFirstResponseByteTotalAvgMs: 820,
          currentAvgTotalMs: 1390,
        }}
        loading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tpm-per-conversation"]')
        ?.textContent,
    ).toContain("200");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-spend-rate-per-conversation"]')
        ?.textContent,
    ).toContain("$0.10");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-success-ratio"]')?.textContent,
    ).toContain("1.5");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-in-progress-retry"]')?.textContent,
    ).toContain("2");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
        ?.textContent,
    ).toContain("1.39 s");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-cost-failed"]')?.textContent,
    ).toContain("$3.5");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-failed"]')?.textContent,
    ).toContain("420");
  });

  it("averages overlapping response-time buckets within the recent window", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 6,
          successCount: 6,
          failureCount: 0,
          totalCost: 1.2,
          totalTokens: 3000,
        }}
        rate={{
          tokensPerMinute: 0,
          spendRate: 0,
          windowMinutes: 1,
          available: true,
          currentAvgTotalMs: 400,
        }}
        timeseries={{
          rangeStart: "2026-04-10T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:06:00.000Z",
          bucketSeconds: 60,
          points: [
            {
              bucketStart: "2026-04-10T00:00:00.000Z",
              bucketEnd: "2026-04-10T00:01:00.000Z",
              totalCount: 1,
              successCount: 1,
              failureCount: 0,
              totalTokens: 300,
              totalCost: 0.1,
              avgTotalMs: 100,
              totalLatencySampleCount: 1,
              firstResponseByteTotalSampleCount: 1,
              firstResponseByteTotalAvgMs: 50,
            },
            {
              bucketStart: "2026-04-10T00:04:00.000Z",
              bucketEnd: "2026-04-10T00:05:00.000Z",
              totalCount: 2,
              successCount: 2,
              failureCount: 0,
              totalTokens: 900,
              totalCost: 0.4,
              avgTotalMs: 200,
              totalLatencySampleCount: 2,
              firstResponseByteTotalSampleCount: 2,
              firstResponseByteTotalAvgMs: 90,
            },
            {
              bucketStart: "2026-04-10T00:05:00.000Z",
              bucketEnd: "2026-04-10T00:06:00.000Z",
              totalCount: 3,
              successCount: 3,
              failureCount: 0,
              totalTokens: 1800,
              totalCost: 0.7,
              avgTotalMs: 800,
              totalLatencySampleCount: 1,
              firstResponseByteTotalSampleCount: 1,
              firstResponseByteTotalAvgMs: 140,
            },
          ],
        }}
        loading={false}
        error={null}
        now={new Date("2026-04-10T00:06:00.000Z")}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
        ?.textContent,
    ).toContain("400 ms");
  });

  it("clips coarse response-time buckets to the recent window overlap", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 6,
          successCount: 6,
          failureCount: 0,
          totalCost: 1.2,
          totalTokens: 3000,
        }}
        rate={{
          tokensPerMinute: 0,
          spendRate: 0,
          windowMinutes: 1,
          available: true,
          currentAvgTotalMs: 340,
        }}
        timeseries={{
          rangeStart: "2026-04-10T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:17:00.000Z",
          bucketSeconds: 900,
          points: [
            {
              bucketStart: "2026-04-10T00:00:00.000Z",
              bucketEnd: "2026-04-10T00:15:00.000Z",
              totalCount: 3,
              successCount: 3,
              failureCount: 0,
              totalTokens: 1200,
              totalCost: 0.5,
              avgTotalMs: 100,
              totalLatencySampleCount: 3,
              firstResponseByteTotalSampleCount: 3,
              firstResponseByteTotalAvgMs: 50,
            },
            {
              bucketStart: "2026-04-10T00:15:00.000Z",
              bucketEnd: "2026-04-10T00:30:00.000Z",
              totalCount: 3,
              successCount: 3,
              failureCount: 0,
              totalTokens: 1800,
              totalCost: 0.7,
              avgTotalMs: 700,
              totalLatencySampleCount: 3,
              firstResponseByteTotalSampleCount: 3,
              firstResponseByteTotalAvgMs: 140,
            },
          ],
        }}
        loading={false}
        error={null}
        now={new Date("2026-04-10T00:17:00.000Z")}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
        ?.textContent,
    ).toContain("340 ms");
  });

  it("compares cost and token totals against yesterday at the same day progress", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12,
          successCount: 12,
          failureCount: 0,
          totalCost: 12,
          totalTokens: 1200,
        }}
        comparisonStats={{
          totalCount: 100,
          successCount: 100,
          failureCount: 0,
          totalCost: 100,
          totalTokens: 10000,
        }}
        timeseries={{
          rangeStart: "2026-04-10T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:03:00.000Z",
          bucketSeconds: 60,
          points: [],
        }}
        comparisonTimeseries={{
          rangeStart: "2026-04-09T00:00:00.000Z",
          rangeEnd: "2026-04-10T00:00:00.000Z",
          bucketSeconds: 60,
          points: [1, 2, 3, 99].map((value, index) => ({
            bucketStart: new Date(
              Date.parse("2026-04-09T00:00:00.000Z") + index * 60_000,
            ).toISOString(),
            bucketEnd: new Date(
              Date.parse("2026-04-09T00:01:00.000Z") + index * 60_000,
            ).toISOString(),
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: value * 100,
            cacheInputTokens: 0,
            totalCost: value,
          })),
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    expect(host?.textContent).toContain("vs yesterday");
    expect(host?.textContent).not.toContain("vs yesterday same time");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-cost-delta"]')?.textContent,
    ).toContain("+100%");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent,
    ).toContain("+100%");
  });

  it("opens field descriptions from metric titles", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        timeseries={buildTimeseriesWithLatency()}
        loading={false}
        error={null}
      />,
    );

    const tpmTitle = [...(host?.querySelectorAll('[role="button"]') ?? [])].find(
      (element) => element.textContent === "TPM",
    );
    expect(tpmTitle).toBeInstanceOf(HTMLElement);
    expect(tpmTitle?.getAttribute("aria-label")).toBeNull();

    act(() => {
      tpmTitle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const tooltip = document.body.querySelector('[role="tooltip"]');
    expect(tooltip?.textContent).toContain("active tail inside the latest 5-minute window");
  });

  it("keeps the field description tooltip when a summary has no usage breakdown", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 42,
          successCount: 40,
          failureCount: 2,
          totalCost: 1.48,
          totalTokens: 9000,
        }}
        loading={false}
        error={null}
      />,
    );

    const tokenTitle = [...(host?.querySelectorAll('[role="button"]') ?? [])].find(
      (element) => element.textContent === "Today Token",
    );
    expect(tokenTitle).toBeInstanceOf(HTMLElement);

    act(() => {
      tokenTitle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const tooltip = document.body.querySelector('[role="tooltip"]');
    expect(tooltip?.textContent).toContain("Total tokens in the selected day.");
    expect(tooltip?.textContent).not.toContain("Cache write");
  });

  it("shows unavailable placeholders for rate tiles when timeseries loading fails", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={null}
        loading={false}
        rateLoading={false}
        rateError="timeseries failed"
        timeseries={buildTimeseriesWithLatency()}
        error={null}
      />,
    );

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent).toBe("—");
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent).toBe(
      "—",
    );
    expect(
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent,
    ).toBe("—");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')
        ?.textContent,
    ).toContain("—");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.textContent,
    ).toContain("—");
    expect(host?.querySelector('[data-testid="today-stats-value-success"]')?.textContent).toContain(
      "80",
    );
  });

  it("shows an unavailable TPM when the selected range lacks billed-success detail", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={{
          tokensPerMinute: 0,
          spendRate: 0.1,
          windowMinutes: 60,
          available: false,
        }}
        loading={false}
        rateLoading={false}
        rateError={null}
        timeseries={buildTimeseriesWithLatency()}
        error={null}
      />,
    );

    expect(host?.querySelector('[data-testid="today-stats-value-tpm"]')?.textContent).toBe("—");
    expect(host?.querySelector('[data-testid="today-stats-value-spend-rate"]')?.textContent).toBe(
      "—",
    );
  });

  it("switches to compact notation when the full metric value would overflow", () => {
    metricContainerWidth = 180;

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const totalTokensValue = host?.querySelector('[data-testid="today-stats-value-total-tokens"]');
    expect(totalTokensValue?.getAttribute("data-compact")).toBe("true");
    expect(totalTokensValue?.textContent).toContain("1.31B");
    expect(totalTokensValue?.getAttribute("title")).toBe("1,314,275,579");
  });

  it("drops compact decimals before truncating the magnitude suffix in narrow tiles", () => {
    metricContainerWidth = 76;

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 281110000,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const totalTokensValue = host?.querySelector('[data-testid="today-stats-value-total-tokens"]');
    expect(totalTokensValue?.getAttribute("data-compact")).toBe("true");
    expect(totalTokensValue?.getAttribute("data-candidate-key")).toBe("compact-M-0");
    expect(totalTokensValue?.textContent).toContain("281M");
    expect(totalTokensValue?.textContent).not.toContain("281.11M");
  });

  it("keeps top-right and secondary values readable without string truncation in narrow tiles", () => {
    metricContainerWidth = 92;

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 488.96,
          totalTokens: 1_049_600_000,
          inProgressConversationCount: 11,
          inProgressRetryConversationCount: 4,
          nonSuccessCost: 60.93,
          nonSuccessTokens: 88_834_346,
        }}
        comparisonStats={{
          totalCount: 9000,
          successCount: 8200,
          failureCount: 800,
          totalCost: 295.3,
          totalTokens: 730_000_000,
          inProgressConversationCount: 7,
          inProgressRetryConversationCount: 2,
          nonSuccessCost: 50.1,
          nonSuccessTokens: 52_000_000,
        }}
        previous7dStats={{
          totalCount: 56000,
          successCount: 52000,
          failureCount: 4000,
          totalCost: 392.12,
          totalTokens: 6_200_000_000,
          inProgressConversationCount: 4,
          inProgressRetryConversationCount: 1,
          nonSuccessCost: 41.2,
          nonSuccessTokens: 320_000_000,
        }}
        timeseries={buildTimeseriesWithLatency()}
        comparisonTimeseries={buildTimeseriesWithLatency()}
        rate={{
          tokensPerMinute: 1_049_600,
          spendRate: 8.31,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const tokensValue = host?.querySelector('[data-testid="today-stats-value-total-tokens"]');
    const topRightDelta = host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]');
    const failedTokens = host?.querySelector('[data-testid="today-stats-secondary-tokens-failed"]');
    const failedCost = host?.querySelector('[data-testid="today-stats-secondary-cost-failed"]');

    expect(tokensValue?.textContent).toContain("1.05B");
    expect(tokensValue?.textContent).not.toContain("1B");
    expect(topRightDelta?.textContent).not.toContain("…");
    expect(failedTokens?.textContent).not.toContain("…");
    expect(failedCost?.textContent).not.toContain("…");
    expect(failedTokens?.textContent).toMatch(/88(\.8|\.83)?M/);
  });

  it("keeps all metric labels on one line while preserving mixed case for total tokens", () => {
    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 539.42,
          totalTokens: 1314275579,
        }}
        rate={{
          tokensPerMinute: 1000,
          spendRate: 0.1,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const totalTokensLabel = host?.querySelector('[data-testid="today-stats-label-total-tokens"]');
    const labels = Array.from(host?.querySelectorAll('[role="button"] span') ?? []);
    expect(totalTokensLabel?.textContent).toBe("Today Token");
    expect(totalTokensLabel?.className).toContain("whitespace-nowrap");
    expect(totalTokensLabel?.className).toContain("normal-case");
    for (const label of labels) {
      expect(label.className).toContain("whitespace-nowrap");
    }
  });

  it("stacks top-right and secondary meta rows below the primary value when a tile is too narrow", () => {
    metricTileWidth = 170;
    metricContainerWidth = 92;

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 488.96,
          totalTokens: 1_049_600_000,
          inProgressConversationCount: 11,
          inProgressRetryConversationCount: 4,
          inProgressAvgWaitMs: 1850,
          nonSuccessCost: 60.93,
          nonSuccessTokens: 88_834_346,
        }}
        comparisonStats={{
          totalCount: 11800,
          successCount: 9100,
          failureCount: 2700,
          totalCost: 430.15,
          totalTokens: 980_000_000,
          inProgressConversationCount: 7,
          inProgressRetryConversationCount: 3,
          nonSuccessCost: 52.8,
          nonSuccessTokens: 76_300_000,
        }}
        previous7dStats={{
          totalCount: 56000,
          successCount: 52000,
          failureCount: 4000,
          totalCost: 392.12,
          totalTokens: 6_200_000_000,
          inProgressConversationCount: 4,
          inProgressRetryConversationCount: 1,
          nonSuccessCost: 41.2,
          nonSuccessTokens: 320_000_000,
        }}
        timeseries={buildTimeseriesWithLatency()}
        comparisonTimeseries={buildTimeseriesWithLatency()}
        rate={{
          tokensPerMinute: 1_049_600,
          spendRate: 8.31,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const tokenTile = host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')?.[6] as
      | HTMLElement
      | undefined;
    const stackedMeta = host?.querySelector(
      '[data-testid="today-stats-value-total-tokens-stacked-meta"]',
    );

    expect(tokenTile?.dataset.stackMeta).toBe("true");
    expect(stackedMeta).toBeInstanceOf(HTMLElement);
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent,
    ).not.toContain("…");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-failed"]')?.textContent,
    ).not.toContain("…");
  });

  it("keeps the original split meta layout when the tile width is sufficient", () => {
    metricTileWidth = 280;
    metricContainerWidth = 92;

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 12474,
          successCount: 9949,
          failureCount: 2525,
          totalCost: 488.96,
          totalTokens: 1_049_600_000,
          inProgressConversationCount: 11,
          inProgressRetryConversationCount: 4,
          inProgressAvgWaitMs: 1850,
          nonSuccessCost: 60.93,
          nonSuccessTokens: 88_834_346,
        }}
        comparisonStats={{
          totalCount: 11800,
          successCount: 9100,
          failureCount: 2700,
          totalCost: 430.15,
          totalTokens: 980_000_000,
          inProgressConversationCount: 7,
          inProgressRetryConversationCount: 3,
          nonSuccessCost: 52.8,
          nonSuccessTokens: 76_300_000,
        }}
        previous7dStats={{
          totalCount: 56000,
          successCount: 52000,
          failureCount: 4000,
          totalCost: 392.12,
          totalTokens: 6_200_000_000,
          inProgressConversationCount: 4,
          inProgressRetryConversationCount: 1,
          nonSuccessCost: 41.2,
          nonSuccessTokens: 320_000_000,
        }}
        timeseries={buildTimeseriesWithLatency()}
        comparisonTimeseries={buildTimeseriesWithLatency()}
        rate={{
          tokensPerMinute: 1_049_600,
          spendRate: 8.31,
          windowMinutes: 5,
          available: true,
        }}
        loading={false}
        error={null}
      />,
    );

    const tokenTile = host?.querySelectorAll('[data-testid="today-stats-metric-tile"]')?.[6] as
      | HTMLElement
      | undefined;
    const stackedMeta = host?.querySelector(
      '[data-testid="today-stats-value-total-tokens-stacked-meta"]',
    );

    expect(tokenTile?.dataset.stackMeta).toBe("false");
    expect(stackedMeta).toBeNull();
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-tokens-delta"]')?.textContent,
    ).not.toContain("…");
  });

  it("uses a width-capped loading placeholder instead of a fixed narrow-tile width", () => {
    render(<TodayStatsOverview stats={null} rate={null} loading error={null} />);

    const tpmLoading = host?.querySelector('[data-testid="today-stats-value-tpm-loading"]');
    expect(tpmLoading).toBeInstanceOf(HTMLElement);
    expect((tpmLoading as HTMLElement | null)?.className).toContain("w-full");
    expect((tpmLoading as HTMLElement | null)?.className).toContain("max-w-[7.5rem]");
    expect((tpmLoading as HTMLElement | null)?.className).not.toContain("w-28");
  });

  it("renders the response-time card with recent-window and day-average latency values", () => {
    const timeseries = buildTimeseriesWithLatency();
    const comparisonTimeseries = {
      ...timeseries,
      rangeStart: "2026-04-09T00:00:00.000Z",
      rangeEnd: "2026-04-09T00:08:00.000Z",
      points: timeseries.points.map((point, index) => ({
        ...point,
        bucketStart: new Date(
          Date.parse("2026-04-09T00:00:00.000Z") + index * 60_000,
        ).toISOString(),
        bucketEnd: new Date(Date.parse("2026-04-09T00:01:00.000Z") + index * 60_000).toISOString(),
        firstResponseByteTotalAvgMs:
          point.firstResponseByteTotalAvgMs == null
            ? null
            : Number((point.firstResponseByteTotalAvgMs * 0.75).toFixed(1)),
      })),
    };

    render(
      <TodayStatsOverview
        stats={{
          totalCount: 88,
          successCount: 80,
          failureCount: 8,
          totalCost: 2.1,
          totalTokens: 8000,
        }}
        rate={{
          tokensPerMinute: 416,
          spendRate: 0.1,
          windowMinutes: 1,
          available: true,
          currentFirstResponseByteTotalAvgMs: 760,
          currentAvgTotalMs: 1180,
        }}
        timeseries={timeseries}
        comparisonTimeseries={comparisonTimeseries}
        loading={false}
        error={null}
      />,
    );

    const responseTimeValue =
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent ?? "";
    const dayAverage =
      host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')
        ?.textContent ?? "";
    const delta =
      host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')
        ?.textContent ?? "";
    const avgTotal =
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
        ?.textContent ?? "";
    expect(responseTimeValue).toMatch(/ms|s/);
    expect(dayAverage).toMatch(/ms|s/);
    expect(avgTotal).toMatch(/ms|s/);
    expect(delta).toContain("%");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-delta"]')?.className,
    ).toContain("text-error");
    expect(host?.textContent).toContain("Time to first byte");
  });

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
              firstResponseByteTotalSampleCount: 2,
              firstResponseByteTotalAvgMs: 500,
            },
          ],
        }}
        loading={false}
        error={null}
      />,
    );

    expect(
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent,
    ).toBe("—");
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
      host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-total"]')
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
          firstResponseByteTotalSampleCount: 2,
          firstResponseByteTotalAvgMs: 500,
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

    expect(
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent,
    ).toBe("—");

    act(() => {
      root?.render(renderOverview(new Date("2026-04-10T00:10:00.000Z")));
    });

    expect(
      host?.querySelector('[data-testid="today-stats-value-response-time"]')?.textContent,
    ).toBe("—");
    expect(
      host?.querySelector('[data-testid="today-stats-secondary-response-time-day-average"]')
        ?.textContent,
    ).toContain("—");
  });
});
