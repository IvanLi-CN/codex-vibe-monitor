/** @vitest-environment jsdom */
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import {
  DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
  getDashboardPerformanceDiagnosticsSnapshot,
  resetDashboardPerformanceDiagnostics,
} from "../lib/dashboardPerformanceDiagnostics";
import { buildTodayMinuteChartData } from "./dashboardTodayActivityChartData";

let latestChartData: Array<Record<string, unknown>> = [];
const storage = new Map<string, string>();
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => {
    storage.set(key, value);
  },
  removeItem: (key: string) => {
    storage.delete(key);
  },
  clear: () => {
    storage.clear();
  },
};
let host: HTMLDivElement | null = null;
let root: Root | null = null;

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive">{children}</div>
  ),
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: ({ domain }: { domain?: [number, number] }) => (
    <div
      data-testid="x-axis"
      data-domain={domain == null ? "" : domain.join(":")}
    />
  ),
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
  Area: ({
    dataKey,
    yAxisId,
    name,
    strokeWidth,
  }: {
    dataKey?: string;
    yAxisId?: string;
    name?: string;
    strokeWidth?: number;
  }) => (
    <div
      data-testid="area-series"
      data-data-key={dataKey ?? ""}
      data-y-axis-id={yAxisId ?? ""}
      data-name={name ?? ""}
      data-stroke-width={String(strokeWidth ?? "")}
    />
  ),
  Line: ({
    data,
    dataKey,
    dot,
    yAxisId,
    name,
    strokeWidth,
    strokeOpacity,
  }: {
    data?: Array<Record<string, unknown>>;
    dataKey?: string;
    dot?: false | Record<string, unknown>;
    yAxisId?: string;
    name?: string;
    strokeWidth?: number;
    strokeOpacity?: number;
  }) => (
    <div
      data-testid="line-series"
      data-data-key={dataKey ?? ""}
      data-y-axis-id={yAxisId ?? ""}
      data-name={name ?? ""}
      data-stroke-width={String(strokeWidth ?? "")}
      data-stroke-opacity={String(strokeOpacity ?? "")}
      data-dot={dot === false ? "false" : dot ? "visible" : ""}
      data-data-length={String(data?.length ?? "")}
    />
  ),
  Bar: ({
    stackId,
    dataKey,
    barSize,
  }: {
    stackId?: string;
    dataKey?: string;
    barSize?: number;
  }) => (
    <div
      data-testid="bar-series"
      data-stack-id={stackId ?? ""}
      data-data-key={dataKey ?? ""}
      data-bar-size={String(barSize ?? "")}
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
      <div
        data-testid="composed-chart"
        data-bar-gap={String(barGap ?? "")}
        data-data-length={String(latestChartData.length)}
      >
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

beforeAll(() => {
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  latestChartData = [];
  window.localStorage.clear();
  resetDashboardPerformanceDiagnostics();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

function chartSection() {
  const section = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart"]',
  ) as HTMLElement | null;
  if (!section) throw new Error("missing chart section");
  return section;
}

function interactionLayer() {
  const layer = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart-interaction-layer"]',
  ) as HTMLElement | null;
  if (!layer) throw new Error("missing chart interaction layer");
  layer.getBoundingClientRect = () =>
    ({
      x: 0,
      y: 0,
      top: 0,
      right: 1000,
      bottom: 320,
      left: 0,
      width: 1000,
      height: 320,
      toJSON: () => ({}),
    }) as DOMRect;
  layer.setPointerCapture = vi.fn();
  layer.releasePointerCapture = vi.fn();
  layer.hasPointerCapture = vi.fn(() => true);
  return layer;
}

function dragLayer() {
  const layer = host?.querySelector(
    '[data-testid="dashboard-today-activity-chart-drag-layer"]',
  ) as HTMLElement | null;
  if (!layer) throw new Error("missing chart drag layer");
  return layer;
}

function dispatchWheel(
  element: HTMLElement,
  init: WheelEventInit & { clientX?: number },
) {
  const event = new WheelEvent("wheel", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  if (init.clientX != null) {
    Object.defineProperty(event, "clientX", {
      configurable: true,
      value: init.clientX,
    });
  }
  act(() => {
    element.dispatchEvent(event);
  });
  return event;
}

function dispatchPointer(
  element: HTMLElement,
  type: string,
  init: MouseEventInit & { pointerId?: number },
) {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  Object.defineProperty(event, "pointerId", {
    configurable: true,
    value: init.pointerId ?? 1,
  });
  act(() => {
    element.dispatchEvent(event);
  });
}

async function flushAnimationFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
  });
}

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
            firstResponseByteTotalSampleCount: 2,
            firstResponseByteTotalAvgMs: 450,
          },
          {
            bucketStart: "2026-04-08 00:09:00",
            bucketEnd: "2026-04-08 00:09:59",
            totalCount: 3,
            successCount: 3,
            failureCount: 0,
            totalTokens: 1800,
            totalCost: 0.36,
            firstResponseByteTotalSampleCount: 6,
            firstResponseByteTotalAvgMs: 750,
          },
          {
            bucketStart: "2026-04-08 00:10:00",
            bucketEnd: "2026-04-08 00:10:59",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 900,
            totalCost: 0.18,
            firstResponseByteTotalSampleCount: 1,
            firstResponseByteTotalAvgMs: 300,
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
      firstResponseByteTotalAvgMs: 450,
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: 450,
    });
    expect(data[0]).toMatchObject({
      chartTokensPerMinute: 300,
      chartSpendRate: 0.06,
      chartFirstResponseByteTotalAvgMs: null,
    });
    expect(data[9]).toMatchObject({
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: 750,
    });
    expect(data[10]).toMatchObject({
      chartTokensPerMinute: 300,
      chartSpendRate: 0.06,
      chartFirstResponseByteTotalAvgMs: 300,
    });
    expect(data[11]).toMatchObject({
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: null,
    });
    expect(data.at(-1)).toMatchObject({
      tokensPerMinute: null,
      spendRate: null,
      firstResponseByteTotalAvgMs: null,
      chartTokensPerMinute: null,
      chartSpendRate: null,
      chartFirstResponseByteTotalAvgMs: null,
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
            firstResponseByteTotalSampleCount: 2,
            firstResponseByteTotalAvgMs: 450,
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
      chartFirstResponseByteTotalAvgMs: null,
    });
    expect(data[1]).toMatchObject({
      totalCount: 2,
      chartFirstResponseByteTotalAvgMs: 450,
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

    expect(html).toContain("chart.inFlight");
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
              firstResponseByteTotalSampleCount: 1,
              firstResponseByteTotalAvgMs: 450,
            },
          ],
        }}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(html).toContain("0 unit.calls");
    expect(html).toContain("chart.inFlight");
    expect(html).toContain("chart.firstResponseByteTotal");
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
            firstResponseByteTotalSampleCount: 1,
            firstResponseByteTotalAvgMs: 18225.02,
          },
        ],
      },
      { now: new Date("2026-04-08T00:03:22") },
    );

    expect(data[1]).toMatchObject({
      totalCount: 0,
      firstResponseByteTotalSampleCount: 0,
      firstResponseByteTotalAvgMs: null,
      chartFirstResponseByteTotalAvgMs: null,
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
              firstResponseByteTotalSampleCount: 1,
              firstResponseByteTotalAvgMs: 43890,
            },
          ],
        }}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(html).toContain('data-data-key="chartFirstResponseByteTotalAvgMs"');
    expect(html).toContain('data-name="chart.firstResponseByteTotal"');
    expect(html).toContain('data-stroke-width="1.25"');
    expect(html).toContain('data-stroke-opacity="0.72"');
    expect(html).toContain('data-dot="visible"');
    expect(html).toContain('data-data-length=""');
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
    expect(html).toContain('data-data-length="1440"');
    expect(html).not.toContain('data-testid="area-chart"');
    expect(html).toContain('data-data-key="chartSuccessCount"');
    expect(html).toContain('data-data-key="chartInFlightCount"');
    expect(html).toContain('data-data-key="chartFailureCountNegative"');
    expect(html).toContain('data-bar-size="1"');
    expect(html).toContain('data-stack-id="positive"');
    expect(html).toContain('data-domain="0:1439"');
    expect(html).not.toContain(
      'data-data-key="chartFailureCountNegative" data-stack-id="positive"',
    );
  });

  it("zooms horizontally around the wheel pointer and keeps the view clamped", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { ctrlKey: true, deltaY: -600, clientX: 500 });
    await flushAnimationFrame();
    const section = chartSection();

    expect(section.dataset.zoomed).toBe("true");
    expect(Number(section.dataset.visibleSpan)).toBeLessThan(1440);
    expect(Number(section.dataset.visibleStartIndex)).toBeGreaterThan(0);
    expect(Number(section.dataset.visibleEndIndex)).toBeLessThan(1439);
    expect(Number(section.querySelector('[data-testid="bar-series"]')?.getAttribute("data-bar-size"))).toBeGreaterThan(1);
    expect(latestChartData).toHaveLength(Number(section.dataset.visibleSpan));

    dispatchWheel(layer, { ctrlKey: true, deltaY: -5000, clientX: 500 });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleSpan)).toBe(30);

    dispatchWheel(layer, { ctrlKey: true, deltaY: 5000, clientX: 500 });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleSpan)).toBe(1440);
    expect(chartSection().dataset.zoomed).toBe("false");
  });

  it("zooms with ordinary vertical wheel scrolling inside the chart", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    const layer = interactionLayer();
    const event = dispatchWheel(layer, { deltaY: -600, clientX: 500 });

    expect(event.defaultPrevented).toBe(true);
    await flushAnimationFrame();
    expect(chartSection().dataset.zoomed).toBe("true");
    expect(Number(chartSection().dataset.visibleSpan)).toBeLessThan(1440);
  });

  it("pans horizontally with trackpad wheel deltas and pointer dragging", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { ctrlKey: true, deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    const zoomedStart = Number(chartSection().dataset.visibleStartIndex);

    const horizontalWheel = dispatchWheel(layer, {
      deltaX: 260,
      deltaY: 8,
      clientX: 500,
    });
    expect(horizontalWheel.defaultPrevented).toBe(true);
    await flushAnimationFrame();
    const wheelPannedStart = Number(chartSection().dataset.visibleStartIndex);
    expect(wheelPannedStart).toBeGreaterThan(zoomedStart);

    dispatchPointer(layer, "pointerdown", {
      button: 0,
      clientX: 500,
      pointerId: 8,
    });
    dispatchPointer(layer, "pointermove", {
      clientX: 220,
      pointerId: 8,
    });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(
      wheelPannedStart,
    );
    expect(dragLayer().style.transform).toContain("translate3d");

    dispatchPointer(layer, "pointerup", {
      clientX: 220,
      pointerId: 8,
    });
    await flushAnimationFrame();
    const draggedStart = Number(chartSection().dataset.visibleStartIndex);
    expect(draggedStart).toBeGreaterThan(wheelPannedStart);

    dispatchWheel(layer, { deltaX: -100_000, deltaY: 0, clientX: 500 });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(0);
  });

  it("axis-locks pointer drags so vertical gestures do not pan the chart", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    const zoomedStart = Number(chartSection().dataset.visibleStartIndex);

    dispatchPointer(layer, "pointerdown", {
      button: 0,
      clientX: 500,
      clientY: 100,
      pointerId: 12,
    });
    dispatchPointer(layer, "pointermove", {
      clientX: 505,
      clientY: 180,
      pointerId: 12,
    });
    await flushAnimationFrame();

    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(zoomedStart);
    expect(layer.releasePointerCapture).toHaveBeenCalledWith(12);
  });

  it("keeps horizontal pointer drags locked even with small vertical drift", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    const zoomedStart = Number(chartSection().dataset.visibleStartIndex);

    dispatchPointer(layer, "pointerdown", {
      button: 0,
      clientX: 500,
      clientY: 100,
      pointerId: 13,
    });
    dispatchPointer(layer, "pointermove", {
      clientX: 220,
      clientY: 125,
      pointerId: 13,
    });
    await flushAnimationFrame();

    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(zoomedStart);
    expect(dragLayer().style.transform).toContain("translate3d");

    dispatchPointer(layer, "pointerup", {
      clientX: 220,
      clientY: 125,
      pointerId: 13,
    });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(
      zoomedStart,
    );
  });

  it("allows large diagonal pointer drags to pan with the horizontal component", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    const zoomedStart = Number(chartSection().dataset.visibleStartIndex);

    dispatchPointer(layer, "pointerdown", {
      button: 0,
      clientX: 500,
      clientY: 100,
      pointerId: 14,
    });
    dispatchPointer(layer, "pointermove", {
      clientX: 240,
      clientY: 340,
      pointerId: 14,
    });
    await flushAnimationFrame();

    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(zoomedStart);
    expect(dragLayer().style.transform).toContain("translate3d");

    dispatchPointer(layer, "pointerup", {
      clientX: 240,
      clientY: 340,
      pointerId: 14,
    });
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(
      zoomedStart,
    );
  });

  it("pans when horizontal wheel intent dominates vertical drift", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { ctrlKey: true, deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    const zoomedStart = Number(chartSection().dataset.visibleStartIndex);

    const event = dispatchWheel(layer, {
      deltaX: 120,
      deltaY: 18,
      clientX: 500,
    });

    expect(event.defaultPrevented).toBe(true);
    await flushAnimationFrame();
    expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(
      zoomedStart,
    );
  });

  it("widens count bars as the viewport zooms in", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { deltaY: -600, clientX: 500 });
    await flushAnimationFrame();

    const bars = host?.querySelectorAll('[data-testid="bar-series"]');
    expect(bars?.length).toBe(3);
    expect(Number(bars?.[0]?.getAttribute("data-bar-size"))).toBeGreaterThan(1);
  });

  it("applies the same horizontal viewport to trend mode data", async () => {
    render(
      <DashboardTodayActivityChart
        response={{
          rangeStart: "2026-04-08 00:00:00",
          rangeEnd: "2026-04-08 00:22:00",
          bucketSeconds: 60,
          points: Array.from({ length: 22 }, (_, index) => ({
            bucketStart: `2026-04-08 00:${String(index).padStart(2, "0")}:00`,
            bucketEnd: `2026-04-08 00:${String(index).padStart(2, "0")}:59`,
            totalCount: 2,
            successCount: 2,
            failureCount: 0,
            totalTokens: 1000 + index * 10,
            totalCost: 0.2 + index * 0.01,
          })),
        }}
        loading={false}
        error={null}
        metric="trend"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { ctrlKey: true, deltaY: -800, clientX: 0 });
    await flushAnimationFrame();
    const section = chartSection();
    const visibleStart = Number(section.dataset.visibleStartIndex);
    const visibleEnd = Number(section.dataset.visibleEndIndex);

    expect(section.dataset.chartMode).toBe("trend-area");
    expect(latestChartData.length).toBeGreaterThan(0);
    expect(
      latestChartData.every(
        (item) =>
          typeof item.index === "number" &&
          item.index >= visibleStart &&
          item.index <= visibleEnd,
      ),
    ).toBe(true);
  });

  it("resets the horizontal viewport when the displayed day changes", async () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    const layer = interactionLayer();
    dispatchWheel(layer, { ctrlKey: true, deltaY: -700, clientX: 500 });
    await flushAnimationFrame();
    expect(chartSection().dataset.zoomed).toBe("true");

    rerender(
      <DashboardTodayActivityChart
        response={{
          ...response,
          rangeStart: "2026-04-09 00:00:00",
          rangeEnd: "2026-04-09 00:03:22",
          points: response.points.map((point) => ({
            ...point,
            bucketStart: String(point.bucketStart).replace(
              "2026-04-08",
              "2026-04-09",
            ),
            bucketEnd: String(point.bucketEnd).replace(
              "2026-04-08",
              "2026-04-09",
            ),
          })),
        }}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(chartSection().dataset.zoomed).toBe("false");
    expect(Number(chartSection().dataset.visibleStartIndex)).toBe(0);
    expect(Number(chartSection().dataset.visibleEndIndex)).toBe(1439);
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

  it("renders trend mode as 10-minute TPM and spend-rate area charts", () => {
    const html = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="trend"
      />,
    );

    expect(html).toContain('data-chart-mode="trend-area"');
    expect(html).toContain('data-testid="composed-chart"');
    expect(html).toContain('data-testid="area-series"');
    expect(html).toContain('data-data-key="chartTokensPerMinute"');
    expect(html).toContain('data-data-key="chartSpendRate"');
    expect(html).toContain('data-y-axis-id="tokens"');
    expect(html).toContain('data-y-axis-id="spend"');
    expect(html).toContain('data-name="chart.tokensPerMinute"');
    expect(html).toContain('data-name="chart.spendRate"');
    expect(html).not.toContain(
      'data-testid="line-series" data-data-key="chartTokensPerMinute"',
    );
  });

  it("starts chart diagnostics immediately after toggling debug on in an open tab", () => {
    render(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    );

    expect(getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount).toBe(
      0,
    );

    act(() => {
      window.localStorage.setItem(
        DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
        "1",
      );
    });

    expect(getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount).toBe(
      1,
    );
  });
});
