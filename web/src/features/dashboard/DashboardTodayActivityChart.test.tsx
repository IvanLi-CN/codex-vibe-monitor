import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string) => key,
  }),
}));
vi.mock("../../theme", () => ({
  useTheme: () => ({
    themeMode: "light",
  }),
}));

import {
  host,
  rechartsMock,
  render,
  response,
  setCompactViewport,
} from "./DashboardTodayActivityChart.test-support";

vi.mock("recharts", () => rechartsMock);

import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import { buildTodayMinuteChartData } from "./dashboardTodayActivityChartData";

function expectEndOfDayBucket(data: ReturnType<typeof buildTodayMinuteChartData>) {
  expect(data.at(-1)).toMatchObject({
    label: "23:59",
    chartSuccessCount: null,
    chartInFlightCount: null,
    chartQueuedInFlightCount: null,
    chartRunningInFlightCount: null,
    chartFailureCountNegative: null,
    cumulativeCost: null,
    cumulativeSuccessCost: null,
    cumulativeNonSuccessCost: null,
    cumulativeTokens: null,
    chartCumulativeCost: null,
    chartCumulativeSuccessCost: null,
    chartCumulativeNonSuccessCost: null,
    chartCumulativeTokens: null,
  });
}

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
    chartQueuedInFlightCount: 0,
    chartRunningInFlightCount: 0,
    chartFailureCountNegative: -1,
    totalCount: 3,
    successCost: 0.3,
    nonSuccessCost: 0.2,
    cumulativeCost: 0.5,
    cumulativeSuccessCost: 0.3,
    cumulativeNonSuccessCost: 0.2,
    cumulativeTokens: 120,
    chartCumulativeCost: 0.5,
    chartCumulativeSuccessCost: 0.3,
    chartCumulativeNonSuccessCost: 0.2,
    chartCumulativeTokens: 120,
  });
  expect(data[1]).toMatchObject({
    successCount: 0,
    failureCount: 0,
    inFlightCount: 0,
    totalCount: 0,
    successCost: 0,
    nonSuccessCost: 0,
    cumulativeCost: 0.5,
    cumulativeSuccessCost: 0.3,
    cumulativeNonSuccessCost: 0.2,
    cumulativeTokens: 120,
    chartSuccessCount: 0,
    chartInFlightCount: 0,
    chartQueuedInFlightCount: 0,
    chartRunningInFlightCount: 0,
    chartFailureCountNegative: 0,
    chartCumulativeCost: 0.5,
    chartCumulativeSuccessCost: 0.3,
    chartCumulativeNonSuccessCost: 0.2,
    chartCumulativeTokens: 120,
  });
  expect(data[2]).toMatchObject({
    successCount: 4,
    failureCount: 0,
    inFlightCount: 0,
    totalCount: 4,
    successCost: 0.75,
    nonSuccessCost: 0,
    cumulativeCost: 1.25,
    cumulativeSuccessCost: 1.05,
    cumulativeNonSuccessCost: 0.2,
    cumulativeTokens: 320,
    chartSuccessCount: 4,
    chartInFlightCount: 0,
    chartQueuedInFlightCount: 0,
    chartRunningInFlightCount: 0,
    chartFailureCountNegative: 0,
    chartCumulativeCost: 1.25,
    chartCumulativeSuccessCost: 1.05,
    chartCumulativeNonSuccessCost: 0.2,
    chartCumulativeTokens: 320,
  });
  expect(data[3]).toMatchObject({
    successCount: 0,
    failureCount: 0,
    inFlightCount: 0,
    totalCount: 0,
    successCost: 0,
    nonSuccessCost: 0,
    cumulativeCost: 1.25,
    cumulativeSuccessCost: 1.05,
    cumulativeNonSuccessCost: 0.2,
    cumulativeTokens: 320,
    chartSuccessCount: 0,
    chartInFlightCount: 0,
    chartQueuedInFlightCount: 0,
    chartRunningInFlightCount: 0,
    chartFailureCountNegative: 0,
    chartCumulativeCost: 1.25,
    chartCumulativeSuccessCost: 1.05,
    chartCumulativeNonSuccessCost: 0.2,
    chartCumulativeTokens: 320,
  });
  expectEndOfDayBucket(data);
});
it("keeps relay-only cost in the success-side remainder when non-success cost is absent", () => {
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:01:22",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:00:00",
          bucketEnd: "2026-04-08 00:00:59",
          totalCount: 4,
          successCount: 4,
          failureCount: 0,
          totalTokens: 480,
          totalCost: 1.2,
          nonSuccessCost: 0,
        },
      ],
    },
    {
      now: new Date(2026, 3, 8, 0, 1, 22),
      localeTag: "en-US",
    },
  );

  expect(data[0]).toMatchObject({
    totalCost: 1.2,
    successCost: 1.2,
    nonSuccessCost: 0,
    cumulativeCost: 1.2,
    cumulativeSuccessCost: 1.2,
    cumulativeNonSuccessCost: 0,
    chartCumulativeCost: 1.2,
    chartCumulativeSuccessCost: 1.2,
    chartCumulativeNonSuccessCost: 0,
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
it("anchors a completed yesterday range to the previous local day instead of the next midnight", () => {
  const localYesterdayStart = new Date(2026, 3, 7, 0, 0, 0);
  const localYesterdayEnd = new Date(2026, 3, 8, 0, 1, 0);
  const localYesterdayTail = new Date(2026, 3, 7, 23, 59, 0);
  const localNow = new Date(2026, 3, 8, 12, 3, 0);
  const data = buildTodayMinuteChartData(
    {
      rangeStart: localYesterdayStart.toISOString(),
      rangeEnd: localYesterdayEnd.toISOString(),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: localYesterdayTail.toISOString(),
          bucketEnd: new Date(2026, 3, 8, 0, 0, 0).toISOString(),
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalTokens: 80,
          totalCost: 0.25,
        },
      ],
    },
    {
      now: localNow,
      localeTag: "en-US",
      closedNaturalDay: true,
    },
  );

  expect(data[0]?.label).toBe("00:00");
  expect(data[0]?.epochMs).toBe(localYesterdayStart.getTime());
  expect(data.at(-1)?.label).toBe("23:59");
  expect(data.at(-1)).toMatchObject({
    totalCount: 2,
    successCount: 2,
    failureCount: 0,
    chartSuccessCount: 2,
    chartFailureCountNegative: 0,
    chartCumulativeCost: 0.25,
    chartCumulativeTokens: 80,
  });
});
it("does not treat a rolling 24-hour window as a closed natural day at midnight", () => {
  const localRangeStart = new Date(2026, 3, 7, 0, 0, 0);
  const localRangeEnd = new Date(2026, 3, 8, 0, 0, 0);
  const localTail = new Date(2026, 3, 7, 23, 59, 0);
  const data = buildTodayMinuteChartData(
    {
      rangeStart: localRangeStart.toISOString(),
      rangeEnd: localRangeEnd.toISOString(),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: localTail.toISOString(),
          bucketEnd: localRangeEnd.toISOString(),
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalTokens: 80,
          totalCost: 0.25,
        },
      ],
    },
    {
      now: localRangeEnd,
      localeTag: "en-US",
    },
  );

  expect(data[0]?.epochMs).toBe(localRangeEnd.getTime());
  expect(data.at(-1)?.epochMs).toBe(new Date(2026, 3, 8, 23, 59, 0).getTime());
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
    chartQueuedInFlightCount: 0,
    chartRunningInFlightCount: 1,
    chartFailureCountNegative: -1,
  });
});
it("splits in-flight phase counts into queued and running chart bars", () => {
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:01:10",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:01:00",
          bucketEnd: "2026-04-08 00:01:59",
          totalCount: 5,
          successCount: 1,
          failureCount: 1,
          inFlightCount: 3,
          inFlightPhaseCounts: {
            queued: 1,
            requesting: 1,
            responding: 1,
          },
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
    totalCount: 5,
    successCount: 1,
    failureCount: 1,
    inFlightCount: 3,
    queuedInFlightCount: 1,
    runningInFlightCount: 2,
    chartSuccessCount: 1,
    chartQueuedInFlightCount: 1,
    chartRunningInFlightCount: 2,
    chartFailureCountNegative: -1,
  });
});
it("adds 10-minute chart bucket averages for trend while keeping first-byte-total minute-aligned", () => {
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:12:30",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:01:00",
          bucketEnd: "2026-04-08 00:01:59",
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalTokens: 1200,
          totalCost: 0.24,
          firstTokenSampleCount: 2,
          firstTokenAvgMs: 450,
        },
        {
          bucketStart: "2026-04-08 00:09:00",
          bucketEnd: "2026-04-08 00:09:59",
          totalCount: 3,
          successCount: 3,
          failureCount: 0,
          totalTokens: 1800,
          totalCost: 0.36,
          firstTokenSampleCount: 6,
          firstTokenAvgMs: 750,
        },
        {
          bucketStart: "2026-04-08 00:10:00",
          bucketEnd: "2026-04-08 00:10:59",
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 900,
          totalCost: 0.18,
          firstTokenSampleCount: 1,
          firstTokenAvgMs: 300,
        },
      ],
    },
    {
      now: new Date(2026, 3, 8, 0, 12, 30),
      localeTag: "en-US",
    },
  );

  expect(data[1]).toMatchObject({
    tokensPerMinute: 1200,
    spendRate: 0.24,
    firstTokenAvgMs: 450,
    chartTokensPerMinute: null,
    chartSpendRate: null,
    chartFirstTokenAvgMs: 450,
  });
  expect(data[0]).toMatchObject({
    chartTokensPerMinute: 300,
    chartSpendRate: 0.06,
    chartFirstTokenAvgMs: null,
  });
  expect(data[9]).toMatchObject({
    chartTokensPerMinute: null,
    chartSpendRate: null,
    chartFirstTokenAvgMs: 750,
  });
  expect(data[10]).toMatchObject({
    chartTokensPerMinute: 300,
    chartSpendRate: 0.06,
    chartFirstTokenAvgMs: 300,
  });
  expect(data[11]).toMatchObject({
    chartTokensPerMinute: null,
    chartSpendRate: null,
    chartFirstTokenAvgMs: null,
  });
  expect(data.at(-1)).toMatchObject({
    tokensPerMinute: null,
    spendRate: null,
    firstTokenAvgMs: null,
    chartTokensPerMinute: null,
    chartSpendRate: null,
    chartFirstTokenAvgMs: null,
  });
});
it("does not attach first-byte-total latency to an empty 10-minute anchor minute", () => {
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:12:30",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:01:00",
          bucketEnd: "2026-04-08 00:01:59",
          totalCount: 2,
          successCount: 2,
          failureCount: 0,
          totalTokens: 1200,
          totalCost: 0.24,
          firstTokenSampleCount: 2,
          firstTokenAvgMs: 450,
        },
      ],
    },
    {
      now: new Date(2026, 3, 8, 0, 12, 30),
      localeTag: "en-US",
    },
  );

  expect(data[0]).toMatchObject({
    totalCount: 0,
    chartFirstTokenAvgMs: null,
  });
  expect(data[1]).toMatchObject({
    totalCount: 2,
    chartFirstTokenAvgMs: 450,
  });
});
it("does not display a ten-minute cost total as a per-minute spend rate", () => {
  const points = Array.from({ length: 10 }, (_, minute) => ({
    bucketStart: `2026-04-08 03:${String(10 + minute).padStart(2, "0")}:00`,
    bucketEnd: `2026-04-08 03:${String(10 + minute).padStart(2, "0")}:59`,
    totalCount: 1,
    successCount: 1,
    failureCount: 0,
    totalTokens: 1_600_000,
    totalCost: 3.567,
  }));
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 03:20:00",
      bucketSeconds: 60,
      points,
    },
    {
      now: new Date(2026, 3, 8, 3, 20, 0),
      localeTag: "en-US",
    },
  );

  expect(data[190]).toMatchObject({
    chartTokensPerMinute: 1_600_000,
    chartSpendRate: 3.567,
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

  expect(html).toContain("chart.running");
  expect(html).toContain("chart.queued");
  expect(html).toContain("1 unit.calls");
  expect(html).not.toContain("5 unit.calls");
});
it("omits first-byte-total from a zero-call minute tooltip even when a neighboring minute has latency", () => {
  const html = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={{
        rangeStart: "2026-04-08 00:00:00",
        rangeEnd: "2026-04-08 00:03:22",
        bucketSeconds: 60,
        points: [
          {
            bucketStart: "2026-04-08 00:01:00",
            bucketEnd: "2026-04-08 00:01:59",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 120,
            totalCost: 0.5,
            firstTokenSampleCount: 1,
            firstTokenAvgMs: 450,
          },
        ],
      }}
      loading={false}
      error={null}
      metric="totalCount"
    />,
  );

  expect(html).toContain("0 unit.calls");
  expect(html).toContain("chart.running");
  expect(html).toContain("chart.queued");
  expect(html).toContain("chart.firstToken");
  expect(html).not.toContain("450 ms");
});
it("drops inconsistent latency samples from zero-call minute data", () => {
  const data = buildTodayMinuteChartData(
    {
      rangeStart: "2026-04-08 00:00:00",
      rangeEnd: "2026-04-08 00:03:22",
      bucketSeconds: 60,
      points: [
        {
          bucketStart: "2026-04-08 00:01:00",
          bucketEnd: "2026-04-08 00:01:59",
          totalCount: 0,
          successCount: 0,
          failureCount: 0,
          inFlightCount: 0,
          totalTokens: 0,
          totalCost: 0,
          firstTokenSampleCount: 1,
          firstTokenAvgMs: 18225.02,
        },
      ],
    },
    { now: new Date("2026-04-08T00:03:22") },
  );

  expect(data[1]).toMatchObject({
    totalCount: 0,
    firstTokenSampleCount: 0,
    firstTokenAvgMs: null,
    chartFirstTokenAvgMs: null,
  });
});
it("overlays first-byte-total latency on the count chart", () => {
  const html = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={{
        ...response,
        points: [
          {
            ...response.points[0],
            firstTokenSampleCount: 1,
            firstTokenAvgMs: 43890,
          },
        ],
      }}
      loading={false}
      error={null}
      metric="totalCount"
    />,
  );

  expect(html).toContain('data-data-key="chartFirstTokenAvgMs"');
  expect(html).toContain('data-name="chart.firstToken"');
  expect(html).toContain('data-stroke-width="1.25"');
  expect(html).toContain('data-stroke-opacity="0.72"');
  expect(html).toContain('data-dot="visible"');
  expect(html).toContain('data-data-length=""');
});
it("renders count mode with success, running, queued, and failure bars sharing one stack", () => {
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
  expect(html).toContain('data-stack-offset="sign"');
  expect(html).toContain('data-data-length="1440"');
  expect(html).not.toContain('data-testid="area-chart"');
  expect(html).toContain('data-data-key="chartSuccessCount"');
  expect(html).toContain('data-data-key="chartRunningInFlightCount"');
  expect(html).toContain('data-data-key="chartQueuedInFlightCount"');
  expect(html).toContain('data-data-key="chartFailureCountNegative"');
  expect(html).toContain('data-bar-size="1"');
  expect(html).toContain('data-stack-id="positive"');
  expect(html).toContain('data-domain="0:1439"');
  expect(html).toContain(
    'data-stack-id="positive" data-data-key="chartFailureCountNegative" data-bar-size="1" data-radius="" data-has-shape="true"',
  );
  expect(html.match(/data-stack-id="positive"/g)).toHaveLength(4);
  expect(html.match(/data-has-shape="true"/g)).toHaveLength(1);
});
it("aggregates dense count data for compact viewports and frees chart width from the latency axis", () => {
  const restoreViewport = setCompactViewport(true);
  try {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(
      host?.querySelector('[data-testid="composed-chart"]')?.getAttribute("data-data-length"),
    ).toBe("72");
    expect(host?.querySelectorAll('[data-testid="bar-series"][data-bar-size="4"]')).toHaveLength(4);
    expect(host?.querySelector('[data-testid="y-axis"][data-y-axis-id="latency"]')).toBeNull();
    expect(host?.querySelector('[data-testid="line-series"]')).toBeNull();
    expect(
      host
        ?.querySelector('[data-testid="y-axis"][data-y-axis-id="count"]')
        ?.getAttribute("data-negative-tick"),
    ).toBe("-42");
  } finally {
    restoreViewport();
  }
});
/** @vitest-environment jsdom */
