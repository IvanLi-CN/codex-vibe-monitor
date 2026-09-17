import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import {
  DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
  getDashboardPerformanceDiagnosticsSnapshot,
} from "../../lib/dashboardPerformanceDiagnostics";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import {
  chartSection,
  dispatchPointer,
  dispatchWheel,
  dragLayer,
  flushAnimationFrame,
  host,
  interactionLayer,
  invalidReasoningBreakdownResponse,
  latestChartData,
  render,
  rerender,
  response,
  tokenBreakdownResponse,
} from "./DashboardTodayActivityChart.test-support";
import { buildTodayMinuteChartData } from "./dashboardTodayActivityChartData";

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
  expect(
    Number(section.querySelector('[data-testid="bar-series"]')?.getAttribute("data-bar-size")),
  ).toBeGreaterThan(1);
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
  expect(Number(chartSection().dataset.visibleStartIndex)).toBe(wheelPannedStart);
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
  expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(zoomedStart);
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
  expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(zoomedStart);
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
  expect(Number(chartSection().dataset.visibleStartIndex)).toBeGreaterThan(zoomedStart);
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
  expect(bars?.length).toBe(4);
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
        typeof item.index === "number" && item.index >= visibleStart && item.index <= visibleEnd,
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
          bucketStart: String(point.bucketStart).replace("2026-04-08", "2026-04-09"),
          bucketEnd: String(point.bucketEnd).replace("2026-04-08", "2026-04-09"),
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
  expect(costHtml).toContain('data-data-key="chartCumulativeSuccessCost"');
  expect(costHtml).toContain('data-data-key="chartCumulativeNonSuccessCost"');
  expect(costHtml).toContain('data-stack-id="cost"');
  expect(costHtml).toContain('data-name="stats.cards.success"');
  expect(costHtml).toContain('data-name="chart.nonSuccess"');
  expect(tokenHtml).toContain('data-chart-mode="cumulative-area"');
  expect(tokenHtml).toContain('data-testid="area-chart"');
  expect(tokenHtml).toContain('data-data-key="chartCumulativeTokens"');
});
it("builds mutually exclusive cumulative token layers and weighted rolling cache rates", () => {
  const data = buildTodayMinuteChartData(tokenBreakdownResponse, {
    now: new Date("2026-04-08T00:03:22"),
    localeTag: "en-US",
  });
  const point = data[2];

  expect(point.cumulativeCacheReadTokens).toBe(150);
  expect(point.cumulativeCacheWriteTokens).toBe(70);
  expect(point.cumulativeOutputTokens).toBe(70);
  expect(point.cumulativeReasoningTokens).toBe(30);
  expect(
    (point.cumulativeCacheReadTokens ?? 0) +
      (point.cumulativeCacheWriteTokens ?? 0) +
      (point.cumulativeOutputTokens ?? 0) +
      (point.cumulativeReasoningTokens ?? 0),
  ).toBe(point.cumulativeTokens);
  expect(point.cacheHitRate).toBeCloseTo(150 / 320, 8);
  expect(point.hourlyCacheHitRate).toBeCloseTo(150 / 320, 8);
});
it("uses the current minute and previous 59 minutes for the hourly cache reference", () => {
  const hourlyResponse = {
    ...tokenBreakdownResponse,
    rangeEnd: "2026-04-08 01:00:22",
    points: [
      {
        ...tokenBreakdownResponse.points[0],
        bucketStart: "2026-04-08 00:05:00",
        bucketEnd: "2026-04-08 00:05:59",
        totalTokens: 100,
        inputTokens: 80,
        outputTokens: 20,
        cacheInputTokens: 0,
        reasoningTokens: 0,
      },
      {
        ...tokenBreakdownResponse.points[1],
        bucketStart: "2026-04-08 01:00:00",
        bucketEnd: "2026-04-08 01:00:59",
        totalTokens: 100,
        inputTokens: 80,
        outputTokens: 20,
        cacheInputTokens: 80,
        reasoningTokens: 0,
      },
    ],
  };
  const data = buildTodayMinuteChartData(hourlyResponse, {
    now: new Date("2026-04-08T01:00:22"),
    localeTag: "en-US",
  });
  const point = data[60];

  expect(point.cacheHitRate).toBeCloseTo(0.8, 8);
  expect(point.hourlyCacheHitRate).toBeCloseTo(0.4, 8);
  expect(data[61]?.hourlyCacheHitRate).toBeNull();
});
it("clamps negative reasoning without discarding an otherwise reconciled breakdown", () => {
  const data = buildTodayMinuteChartData(invalidReasoningBreakdownResponse, {
    now: new Date("2026-04-08T00:03:22"),
    localeTag: "en-US",
  });

  expect(data[0]).toMatchObject({
    cumulativeOutputTokens: 40,
    cumulativeReasoningTokens: 0,
  });
  expect(data[0]?.chartCumulativeCacheReadTokens).not.toBeNull();
});
it("renders four token areas and both disconnected cache-rate lines only for today", () => {
  const todayHtml = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={tokenBreakdownResponse}
      loading={false}
      error={null}
      metric="totalTokens"
    />,
  );
  const yesterdayHtml = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={tokenBreakdownResponse}
      loading={false}
      error={null}
      metric="totalTokens"
      closedNaturalDay
    />,
  );

  expect(todayHtml).toContain('data-testid="composed-chart"');
  expect(todayHtml.match(/data-stack-id="tokens"/g)).toHaveLength(4);
  expect(todayHtml).toContain('data-data-key="chartCumulativeCacheReadTokens"');
  expect(todayHtml).toContain('data-data-key="chartCumulativeCacheWriteTokens"');
  expect(todayHtml).toContain('data-data-key="chartCumulativeOutputTokens"');
  expect(todayHtml).toContain('data-data-key="chartCumulativeReasoningTokens"');
  expect(todayHtml).toContain('data-data-key="chartCacheHitRate"');
  expect(todayHtml).toContain('data-data-key="chartHourlyCacheHitRate"');
  expect(todayHtml).toContain('data-y-axis-id="cacheHitRate"');
  expect(todayHtml).toContain('data-connect-nulls="false"');
  expect(yesterdayHtml).not.toContain('data-data-key="chartCacheHitRate"');
  expect(yesterdayHtml).not.toContain('data-data-key="chartHourlyCacheHitRate"');
  expect(yesterdayHtml).not.toContain('data-y-axis-id="cacheHitRate"');
});
it("falls back to the original total-token area for legacy or unreconciled points", () => {
  const legacyHtml = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={response}
      loading={false}
      error={null}
      metric="totalTokens"
    />,
  );
  const unreconciled = {
    ...tokenBreakdownResponse,
    points: tokenBreakdownResponse.points.map((point, index) =>
      index === 0 ? { ...point, outputTokens: point.outputTokens + 1 } : point,
    ),
  };
  const unreconciledHtml = renderToStaticMarkup(
    <DashboardTodayActivityChart
      response={unreconciled}
      loading={false}
      error={null}
      metric="totalTokens"
    />,
  );
  for (const html of [legacyHtml, unreconciledHtml]) {
    expect(html).toContain('data-testid="area-chart"');
    expect(html).toContain('data-data-key="chartCumulativeTokens"');
    expect(html).not.toContain('data-stack-id="tokens"');
    expect(html).not.toContain('data-data-key="chartCacheHitRate"');
    expect(html).not.toContain('data-data-key="chartHourlyCacheHitRate"');
  }
});
it("renders trend mode as 10-minute TPM and spend-rate area charts", () => {
  const html = renderToStaticMarkup(
    <DashboardTodayActivityChart response={response} loading={false} error={null} metric="trend" />,
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
  expect(html).not.toContain('data-testid="line-series" data-data-key="chartTokensPerMinute"');
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

  expect(getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount).toBe(0);

  act(() => {
    window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
  });

  expect(getDashboardPerformanceDiagnosticsSnapshot().todayChartRenderCount).toBe(1);
});
