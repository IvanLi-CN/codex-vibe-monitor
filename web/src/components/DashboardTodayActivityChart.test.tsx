import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import { buildTodayMinuteChartData } from "./dashboardTodayActivityChartData";

let latestChartData: Array<Record<string, unknown>> = [];

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive">{children}</div>
  ),
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: ({
    content,
  }: {
    content?: (props: {
      active: boolean;
      label: number;
      payload: Array<{ payload: Record<string, unknown> }>;
    }) => ReactNode;
  }) => {
    const point =
      latestChartData.find(
        (item) =>
          typeof item.inFlightCount === "number" &&
          Number(item.inFlightCount) > 0,
      ) ??
      latestChartData.find(
        (item) => typeof item.chartSuccessCount === "number",
      );
    return (
      <div data-testid="tooltip">
        {point
          ? content?.({
              active: true,
              label: Number(point.index ?? 0),
              payload: [{ payload: point }],
            })
          : null}
      </div>
    );
  },
  Legend: () => <div data-testid="legend" />,
  ReferenceLine: () => <div data-testid="reference-line" />,
  Area: () => <div data-testid="area-series" />,
  Bar: ({ stackId, dataKey }: { stackId?: string; dataKey?: string }) => (
    <div
      data-testid="bar-series"
      data-stack-id={stackId ?? ""}
      data-data-key={dataKey ?? ""}
    />
  ),
  AreaChart: ({ children }: { children: ReactNode }) => (
    <div data-testid="area-chart">{children}</div>
  ),
  ComposedChart: ({
    children,
    barGap,
    data,
  }: {
    children: ReactNode;
    barGap?: string | number;
    data?: Array<Record<string, unknown>>;
  }) => {
    latestChartData = data ?? [];
    return (
      <div data-testid="composed-chart" data-bar-gap={String(barGap ?? "")}>
        {children}
      </div>
    );
  },
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string) => key,
  }),
}));

vi.mock("../theme", () => ({
  useTheme: () => ({
    themeMode: "light",
  }),
}));

const response = {
  rangeStart: "2026-04-08 00:00:00",
  rangeEnd: "2026-04-08 00:03:22",
  bucketSeconds: 60,
  points: [
    {
      bucketStart: "2026-04-08 00:00:00",
      bucketEnd: "2026-04-08 00:00:59",
      totalCount: 3,
      successCount: 2,
      failureCount: 1,
      totalTokens: 120,
      totalCost: 0.5,
    },
    {
      bucketStart: "2026-04-08 00:02:00",
      bucketEnd: "2026-04-08 00:02:59",
      totalCount: 4,
      successCount: 4,
      failureCount: 0,
      totalTokens: 200,
      totalCost: 0.75,
    },
  ],
};

describe("DashboardTodayActivityChart", () => {
  it("builds a continuous minute series and preserves cumulative totals", () => {
    const data = buildTodayMinuteChartData(response, {
      now: new Date(2026, 3, 8, 0, 3, 22),
      localeTag: "en-US",
    });

    expect(data).toHaveLength(24 * 60);
    expect(data[0]).toMatchObject({
      successCount: 2,
      failureCount: 1,
      inFlightCount: 0,
      failureCountNegative: -1,
      chartSuccessCount: 2,
      chartInFlightCount: 0,
      chartFailureCountNegative: -1,
      totalCount: 3,
      cumulativeCost: 0.5,
      cumulativeTokens: 120,
      chartCumulativeCost: 0.5,
      chartCumulativeTokens: 120,
    });
    expect(data[1]).toMatchObject({
      successCount: 0,
      failureCount: 0,
      inFlightCount: 0,
      totalCount: 0,
      cumulativeCost: 0.5,
      cumulativeTokens: 120,
      chartSuccessCount: 0,
      chartInFlightCount: 0,
      chartFailureCountNegative: 0,
      chartCumulativeCost: 0.5,
      chartCumulativeTokens: 120,
    });
    expect(data[2]).toMatchObject({
      successCount: 4,
      failureCount: 0,
      inFlightCount: 0,
      totalCount: 4,
      cumulativeCost: 1.25,
      cumulativeTokens: 320,
      chartSuccessCount: 4,
      chartInFlightCount: 0,
      chartFailureCountNegative: 0,
      chartCumulativeCost: 1.25,
      chartCumulativeTokens: 320,
    });
    expect(data[3]).toMatchObject({
      successCount: 0,
      failureCount: 0,
      inFlightCount: 0,
      totalCount: 0,
      cumulativeCost: 1.25,
      cumulativeTokens: 320,
      chartSuccessCount: 0,
      chartInFlightCount: 0,
      chartFailureCountNegative: 0,
      chartCumulativeCost: 1.25,
      chartCumulativeTokens: 320,
    });
    expect(data.at(-1)).toMatchObject({
      label: "23:59",
      chartSuccessCount: null,
      chartInFlightCount: null,
      chartFailureCountNegative: null,
      cumulativeCost: null,
      cumulativeTokens: null,
      chartCumulativeCost: null,
      chartCumulativeTokens: null,
    });
  });

  it("clamps a 24-hour response to the local today window and keeps the rest of today empty", () => {
    const data = buildTodayMinuteChartData(
      {
        rangeStart: "2026-04-07T00:03:00.000Z",
        rangeEnd: "2026-04-08T00:03:00.000Z",
        bucketSeconds: 60,
        points: [
          {
            bucketStart: "2026-04-07T00:03:00.000Z",
            bucketEnd: "2026-04-07T00:03:59.000Z",
            totalCount: 2,
            successCount: 2,
            failureCount: 0,
            totalTokens: 80,
            totalCost: 0.25,
          },
        ],
      },
      {
        now: new Date("2026-04-08T00:03:00.000Z"),
        localeTag: "en-US",
      },
    );

    const localRangeStart = new Date(2026, 3, 8, 0, 0, 0);
    const localRangeEnd = new Date(2026, 3, 8, 23, 59, 0);
    const labelFormatter = new Intl.DateTimeFormat("en-US", {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
      hourCycle: "h23",
    });
    const expectedHeadLabel = labelFormatter
      .format(localRangeStart)
      .replace(/(^|\D)24:(\d{2})/g, "$100:$2");
    const expectedTailLabel = labelFormatter
      .format(localRangeEnd)
      .replace(/(^|\D)24:(\d{2})/g, "$100:$2");

    expect(data[0]?.label).toBe(expectedHeadLabel);
    expect(data[0]?.epochMs).toBe(localRangeStart.getTime());
    expect(data.at(-1)?.label).toBe(expectedTailLabel);
    expect(data).toHaveLength(24 * 60);
    expect(data.at(-1)?.chartCumulativeCost).toBeNull();
  });

  it("uses explicit in-flight counts and leaves neutral residual totals unrendered", () => {
    const data = buildTodayMinuteChartData(
      {
        rangeStart: "2026-04-08 00:00:00",
        rangeEnd: "2026-04-08 00:01:10",
        bucketSeconds: 60,
        points: [
          {
            bucketStart: "2026-04-08 00:01:00",
            bucketEnd: "2026-04-08 00:01:59",
            totalCount: 4,
            successCount: 1,
            failureCount: 1,
            inFlightCount: 1,
            totalTokens: 90,
            totalCost: 0.2,
          },
        ],
      },
      {
        now: new Date(2026, 3, 8, 0, 1, 10),
        localeTag: "en-US",
      },
    );

    expect(data[1]).toMatchObject({
      totalCount: 4,
      successCount: 1,
      failureCount: 1,
      inFlightCount: 1,
      chartSuccessCount: 1,
      chartInFlightCount: 1,
      chartFailureCountNegative: -1,
    });
  });

  it("shows in-flight calls in the count tooltip without inferring neutral residuals as running", () => {
    const html = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={{
          ...response,
          points: [
            {
              bucketStart: "2026-04-08 00:00:00",
              bucketEnd: "2026-04-08 00:00:59",
              totalCount: 5,
              successCount: 2,
              failureCount: 1,
              inFlightCount: 1,
              totalTokens: 120,
              totalCost: 0.5,
            },
          ],
        }}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(html).toContain("chart.inFlight");
    expect(html).toContain("1 unit.calls");
    expect(html).toContain("5 unit.calls");
  });

  it("renders count mode as a composed chart with split success and failure bars", () => {
    const html = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(html).toContain('data-testid="dashboard-today-activity-chart"');
    expect(html).toContain('data-chart-mode="count-bars"');
    expect(html).toContain('data-testid="composed-chart"');
    expect(html).toContain('data-bar-gap="-100%"');
    expect(html).not.toContain('data-testid="area-chart"');
    expect(html).toContain('data-data-key="chartSuccessCount"');
    expect(html).toContain('data-data-key="chartInFlightCount"');
    expect(html).toContain('data-data-key="chartFailureCountNegative"');
    expect(html).toContain('data-stack-id="positive"');
    expect(html).not.toContain(
      'data-data-key="chartFailureCountNegative" data-stack-id="positive"',
    );
  });

  it("renders cost and token modes as cumulative area charts", () => {
    const costHtml = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    );
    const tokenHtml = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalTokens"
      />,
    );

    expect(costHtml).toContain('data-chart-mode="cumulative-area"');
    expect(costHtml).toContain('data-testid="area-chart"');
    expect(costHtml).not.toContain('data-testid="composed-chart"');
    expect(tokenHtml).toContain('data-chart-mode="cumulative-area"');
    expect(tokenHtml).toContain('data-testid="area-chart"');
  });
});
