/** @vitest-environment jsdom */
import { act } from "react";
import { expect, it, vi } from "vitest";

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
        "dashboard.today.firstResponseTime": "TTFT",
        "dashboard.today.responseTimeDescription":
          "TTFT and response time prefer the latest rolling 60-second success samples, then fall back to the latest valid result in the current range.",
        "dashboard.today.inProgressConversations": "In progress",
        "dashboard.today.queuedInvocations": "Queued",
        "dashboard.today.parallelConversations": "Parallel conversations",
        "dashboard.today.todayCost": "Today cost",
        "dashboard.today.yesterdayCost": "Yesterday cost",
        "dashboard.today.todayTokens": "Today Token",
        "dashboard.today.yesterdayTokens": "Yesterday Token",
        "dashboard.today.tokensPerMinuteDescription":
          "TPM uses the latest rolling 60-second window.",
        "dashboard.today.spendRateDescription":
          "Spend rate uses the latest rolling 60-second window.",
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

import { TodayStatsOverview } from "./TodayStatsOverview";
import {
  buildParallelWorkStats,
  buildTimeseriesWithLatency,
  host,
  render,
} from "./TodayStatsOverview.test-support";

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
        currentFirstTokenAvgMs: 2750,
        currentAvgTotalMs: 4900,
        currentAvgResponseMs: 4900,
      }}
      modelPerformance={{
        available: true,
        total: {
          tokensPerMinute: 1200,
          avgFirstTokenMs: 1500,
        },
        models: [
          {
            model: "gpt-5.6",
            reasoningEffort: null,
            tokensPerMinute: 1200,
            avgFirstTokenMs: 1500,
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
    host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-response"]')
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
  expect(tileLabels[3]).toContain("TTFT");
  expect(tileLabels[4]).toContain("Success");
  expect(host?.textContent).toContain("Today summary");
  expect(host?.textContent).toContain("TPM");
  expect(host?.textContent).toContain("Spend rate");
  expect(host?.textContent).toContain("TTFT");
  expect(host?.textContent).toContain("In progress");
  expect(host?.textContent).toContain("Today cost");
  expect(host?.textContent).toContain("Today Token");
  expect(
    host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')?.textContent,
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
  expect(host?.textContent).toContain("TTFT");
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
    host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')?.textContent,
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
    host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')?.textContent,
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
    host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')?.textContent,
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
    host?.querySelector('[data-testid="today-stats-value-in-progress-conversations"]')?.textContent,
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
            firstTokenSampleCount: 1,
            firstTokenAvgMs: 800,
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
            firstTokenSampleCount: 2,
            firstTokenAvgMs: 500,
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
        currentFirstTokenAvgMs: 820,
        currentAvgTotalMs: 1390,
        currentAvgResponseMs: 1390,
      }}
      loading={false}
      error={null}
    />,
  );

  expectNaturalDayKpis(host);
});
/** @vitest-environment jsdom */
function expectNaturalDayKpis(host: HTMLDivElement | null) {
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-tpm-per-conversation"]')?.textContent,
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
    host?.querySelector('[data-testid="today-stats-secondary-response-time-avg-response"]')
      ?.textContent,
  ).toContain("1.39 s");
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-cost-failed"]')?.textContent,
  ).toContain("$3.5");
  expect(
    host?.querySelector('[data-testid="today-stats-secondary-tokens-failed"]')?.textContent,
  ).toContain("420");
}
