/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, vi } from "vitest";
import type { ParallelWorkStatsResponse, TimeseriesResponse } from "../../lib/api";

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
let host: HTMLDivElement | null = null;
let root: Root | null = null;
let metricContainerWidth = 640;
let metricTileWidth = 280;
function setMetricContainerWidth(width: number) {
  metricContainerWidth = width;
}
function setMetricTileWidth(width: number) {
  metricTileWidth = width;
}
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
      firstTokenSampleCount: sampleCount,
      firstTokenAvgMs: sampleCount > 0 ? Number((820 + index * 41.5).toFixed(1)) : null,
      firstTokenP95Ms: sampleCount > 0 ? Number((980 + index * 58.5).toFixed(1)) : null,
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
      activeMinuteCount: currentCounts.length,
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
      activeMinuteCount: 0,
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
      activeMinuteCount: 0,
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
      activeMinuteCount: 0,
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

export {
  buildParallelWorkStats,
  buildTimeseriesWithLatency,
  host,
  metricContainerWidth,
  metricTileWidth,
  render,
  root,
  setMetricContainerWidth,
  setMetricTileWidth,
};
