/** @vitest-environment jsdom */
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { LongTermMetrics, LongTermSeries, LongTermStatsOverviewResponse } from "../../lib/api";
import { LongTermStatsSection, mergeSeriesPoints } from "./LongTermStatsSection";

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  AreaChart: ({ children }: { children: ReactNode }) => (
    <div data-testid="long-term-area-chart">{children}</div>
  ),
  LineChart: ({ children }: { children: ReactNode }) => (
    <div data-testid="long-term-line-chart">{children}</div>
  ),
  Area: ({ dataKey, stackId }: { dataKey?: string; stackId?: string }) => (
    <div data-testid="long-term-area" data-data-key={dataKey} data-stack-id={stackId} />
  ),
  Line: ({ dataKey }: { dataKey?: string }) => (
    <div data-testid="long-term-line" data-data-key={dataKey} />
  ),
  CartesianGrid: () => null,
  XAxis: () => null,
  YAxis: () => null,
  Tooltip: () => null,
  Legend: () => null,
}));

vi.mock("../../theme", () => ({
  useTheme: () => ({ themeMode: "light" }),
}));

vi.mock("../../hooks/useLongTermStats", () => ({
  useLongTermStats: () => ({
    overview: null,
    series: null,
    isLoading: false,
    error: null,
    seriesError: null,
    isSeriesLoading: false,
  }),
}));

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string) =>
      ({
        "stats.longTerm.title": "Long-term usage",
        "stats.longTerm.subtitle": "Daily usage",
        "stats.longTerm.rangeLabel": "Range",
        "stats.longTerm.range.7d": "Past 7 days",
        "stats.longTerm.range.30d": "Past 30 days",
        "stats.longTerm.range.180d": "Past 180 days",
        "stats.longTerm.range.365d": "Past year",
        "stats.longTerm.tokens": "Tokens",
        "stats.longTerm.cost": "Cost",
        "stats.longTerm.calls": "Calls",
        "stats.longTerm.globalTrend": "Global trend",
        "stats.longTerm.modelTime": "Model time",
        "stats.longTerm.modelPerformance": "Model performance",
        "stats.longTerm.modelUsage": "Model usage",
        "stats.longTerm.models": "Models",
        "stats.longTerm.upstreamUsage": "Upstream accounts",
        "stats.longTerm.upstreams": "Accounts",
        "stats.longTerm.metric.tokens": "Tokens",
        "stats.longTerm.metric.cost": "Cost",
        "stats.longTerm.metric.calls": "Calls",
        "stats.longTerm.metric.usageTime": "Usage time",
        "stats.longTerm.metric.wallTime": "Wall time",
        "stats.longTerm.metric.outputSpeed": "Output speed",
        "stats.longTerm.metric.firstByte": "First byte",
        "stats.longTerm.metric.response": "Response",
        "stats.longTerm.search": "Search names",
        "stats.longTerm.select": "Select",
        "stats.longTerm.name": "Name",
        "stats.longTerm.modelAndReasoning": "Model / reasoning",
        "stats.longTerm.total": "Total",
        "stats.longTerm.unspecified": "Unspecified",
      })[key] ?? key,
  }),
}));

const metrics = (tokens: number | null, cost: number | null, calls: number): LongTermMetrics => ({
  calls,
  tokens,
  tokenSamples: calls,
  cost,
  costSamples: calls,
  usageTimeMs: 100,
  usageTimeSamples: calls,
  wallTimeMs: 80,
  wallTimeSamples: calls,
  outputSpeedTokensPerSecond: 20,
  outputSpeedSamples: calls,
  firstByteMs: 10,
  firstByteSamples: calls,
  responseMs: 90,
  responseSamples: calls,
});

function series(
  seriesKey: string,
  displayName: string,
  points: LongTermSeries["points"],
): LongTermSeries {
  return { seriesKey, displayName, points };
}

describe("mergeSeriesPoints", () => {
  it("fills missing dates with zero for stacked charts while preserving null metrics", () => {
    const points = mergeSeriesPoints(
      [
        series("a", "A", [
          { date: "2026-07-01", ...metrics(10, 1, 1) },
          { date: "2026-07-03", ...metrics(null, null, 1) },
        ]),
        series("b", "B", [{ date: "2026-07-02", ...metrics(20, 2, 2) }]),
      ],
      "tokens",
      true,
    );

    expect(points.map((point) => point.date)).toEqual(["2026-07-01", "2026-07-02", "2026-07-03"]);
    expect(points.map((point) => point.a)).toEqual([10, 0, null]);
    expect(points.map((point) => point.b)).toEqual([0, 20, 0]);
  });
});

describe("LongTermStatsSection charts", () => {
  it("uses stacked areas only for model and upstream usage charts", () => {
    const overview: LongTermStatsOverviewResponse = {
      status: "ready",
      statisticsStartDate: "2026-01-01",
      processedRows: 2,
      totalRows: 2,
      timezone: "Asia/Shanghai",
      range: "7d",
      global: metrics(30, 3, 3),
      daily: [{ date: "2026-07-01", ...metrics(30, 3, 3) }],
      models: [
        {
          seriesKey: "a",
          displayName: "gpt-5.6-sol",
          reasoningEffort: "high",
          ...metrics(10, 1, 1),
        },
      ],
      upstreams: [
        {
          seriesKey: "account:1",
          displayName: "Primary",
          reasoningEffort: null,
          ...metrics(20, 2, 2),
        },
      ],
    };
    const modelSeries = [
      series("a", "gpt-5.6-sol", [{ date: "2026-07-01", ...metrics(10, 1, 1) }]),
    ];
    const upstreamSeries = [
      series("account:1", "Primary", [{ date: "2026-07-01", ...metrics(20, 2, 2) }]),
    ];

    const html = renderToStaticMarkup(
      <LongTermStatsSection
        overviewOverride={overview}
        seriesOverride={modelSeries}
        upstreamSeriesOverride={upstreamSeries}
      />,
    );

    expect((html.match(/data-testid="long-term-area-chart"/g) ?? []).length).toBe(2);
    expect((html.match(/data-testid="long-term-area"/g) ?? []).length).toBe(2);
    expect(html).toContain('data-stack-id="long-term-usage"');
    expect((html.match(/data-testid="long-term-line-chart"/g) ?? []).length).toBe(3);
    expect(html).toContain('data-testid="long-term-model-total-row"');
    expect(html).toContain('data-testid="long-term-chart-model-usage"');
    expect(html).toContain('data-testid="long-term-chart-upstream-usage"');
  });
});
