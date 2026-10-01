import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, fireEvent, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../../lib/api";
import { DashboardInvocationTimeline } from "./DashboardInvocationTimeline";

const rangeStart = "2026-07-16T10:40:00.000Z";
const rangeEnd = "2026-07-16T11:10:00.000Z";

const response: TimeseriesResponse = {
  rangeStart,
  rangeEnd,
  bucketSeconds: 60,
  points: Array.from({ length: 30 }, (_, index) => {
    const start = new Date(Date.parse(rangeStart) + index * 60_000);
    return {
      bucketStart: start.toISOString(),
      bucketEnd: new Date(start.getTime() + 60_000).toISOString(),
      totalCount: index % 5 === 0 ? 4 : 2,
      successCount: 2,
      failureCount: 0,
      totalTokens: 1_000,
      totalCost: 0.01,
      firstTokenSampleCount: index % 4 === 0 ? 2 : 0,
      firstTokenAvgMs: index % 4 === 0 ? 500 + index * 12 : null,
    };
  }),
};

const records: InvocationTimelineResponse = {
  rangeStart,
  rangeEnd,
  asOf: "2026-07-16T11:05:00.000Z",
  total: 5,
  hasMore: false,
  nextCursor: null,
  records: [
    {
      id: 1,
      invokeId: "invoke-success-001",
      occurredAt: "2026-07-16T11:00:00.000Z",
      endAt: "2026-07-16T11:04:30.000Z",
      isInFlight: false,
      status: "success",
      tTotalMs: 270_000,
      firstTokenMs: 42_000,
    },
    {
      id: 2,
      invokeId: "invoke-responding-002",
      occurredAt: "2026-07-16T11:01:15.000Z",
      endAt: null,
      isInFlight: true,
      livePhase: "responding",
      tTotalMs: null,
      firstTokenMs: 54_000,
    },
    {
      id: 3,
      invokeId: "invoke-failed-003",
      occurredAt: "2026-07-16T11:02:00.000Z",
      endAt: "2026-07-16T11:03:10.000Z",
      isInFlight: false,
      status: "failed",
      tTotalMs: 70_000,
      failureClass: "service_failure",
    },
    {
      id: 4,
      invokeId: "invoke-queued-004",
      occurredAt: "2026-07-16T11:03:00.000Z",
      endAt: null,
      isInFlight: true,
      livePhase: "queued",
      tTotalMs: null,
    },
    {
      id: 5,
      invokeId: "invoke-unknown-005",
      occurredAt: "2026-07-16T11:04:15.000Z",
      endAt: null,
      isInFlight: false,
      status: "success",
      tTotalMs: null,
    },
  ],
};

const denseRecords: InvocationTimelineResponse = {
  ...records,
  total: 190,
  records: Array.from({ length: 190 }, (_, index) => ({
    id: index + 1,
    invokeId: `invoke-dense-${String(index + 1).padStart(3, "0")}`,
    occurredAt: "2026-07-16T11:00:00.000Z",
    endAt: "2026-07-16T11:01:00.000Z",
    isInFlight: false,
    status: "success" as const,
    tTotalMs: 60_000,
    firstTokenMs: index % 3 === 0 ? 2_000 : null,
  })),
};

const overflowRecords: InvocationTimelineResponse = {
  ...denseRecords,
  total: 360,
  records: Array.from({ length: 360 }, (_, index) => ({
    id: index + 1,
    invokeId: `invoke-overflow-${String(index + 1).padStart(3, "0")}`,
    occurredAt: "2026-07-16T11:00:00.000Z",
    endAt: "2026-07-16T11:01:00.000Z",
    isInFlight: false,
    status: "success" as const,
    tTotalMs: 60_000,
    firstTokenMs: null,
  })),
};

async function hoverTimelineAt(canvasElement: HTMLElement, xRatio: number, yRatio: number) {
  const plot = canvasElement.querySelector('[data-testid="dashboard-invocation-timeline-plot"]');
  if (!(plot instanceof HTMLElement)) throw new Error("missing invocation timeline plot");
  const rect = plot.getBoundingClientRect();
  await fireEvent.pointerMove(plot, {
    clientX: rect.left + rect.width * xRatio,
    clientY: rect.top + (rect.height - 28) * yRatio,
  });
  const tooltip = within(canvasElement).getByTestId("dashboard-invocation-timeline-hover-tooltip");
  await expect(tooltip).toBeVisible();
  return { plot, tooltip };
}

function expectDenseLaneGeometry(laneScroll: HTMLElement, totalCalls: number): void {
  const bars = Array.from(laneScroll.querySelectorAll<HTMLElement>("[data-call-value]"))
    .map((bar) => {
      const rect = bar.getBoundingClientRect();
      return { top: rect.top, height: rect.height };
    })
    .sort((left, right) => left.top - right.top);

  expect(laneScroll.scrollHeight).toBeGreaterThan(laneScroll.clientHeight);
  expect(bars.length).toBeGreaterThan(0);
  expect(bars.length).toBeLessThan(totalCalls);
  for (const bar of bars) {
    expect(bar.height).toBeGreaterThanOrEqual(8);
    expect(bar.height).toBeLessThanOrEqual(16);
  }
  for (let index = 1; index < bars.length; index += 1) {
    expect(bars[index]!.top - bars[index - 1]!.top - bars[index - 1]!.height).toBe(1);
  }
}

function RefreshableOverflowTimeline() {
  const [timelineData, setTimelineData] = useState(overflowRecords);
  return (
    <>
      <button
        type="button"
        className="sr-only"
        data-testid="refresh-overflow-fixture"
        onClick={() =>
          setTimelineData((current) => ({
            ...current,
            asOf: new Date(Date.parse(current.asOf) + 1_000).toISOString(),
            records: [...current.records],
          }))
        }
      >
        Refresh mock snapshot
      </button>
      <DashboardInvocationTimeline
        response={response}
        loading={false}
        timelineData={timelineData}
      />
    </>
  );
}

const meta = {
  title: "Dashboard/DashboardInvocationTimeline",
  component: DashboardInvocationTimeline,
  tags: ["autodocs", "test"],
  parameters: {
    layout: "fullscreen",
  },
  globals: { viewport: { value: "desktop1440x1024", isRotated: false } },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="vibe-dark" className="min-h-screen bg-[#08172b] text-white">
          <div data-visual-evidence-surface className="mx-auto max-w-[1280px] bg-[#08172b] p-8">
            <div data-visual-evidence-target>
              <Story />
            </div>
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  argTypes: {
    timelineStatusOverride: { control: false },
  },
} satisfies Meta<typeof DashboardInvocationTimeline>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LiveTraffic: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: records,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /拉长|Lengthen/ }));
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-invocation-timeline-ttft-overlay")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-invocation-timeline-x-axis")).toBeVisible();
    const xAxis = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-x-axis"]',
    );
    const callsAxis = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-calls-axis"]',
    );
    const laneZero = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-lane-zero"]',
    );
    const ttftZero = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-ttft-zero"]',
    );
    if (
      !(xAxis instanceof HTMLElement) ||
      !(callsAxis instanceof HTMLElement) ||
      !(laneZero instanceof HTMLElement) ||
      !(ttftZero instanceof HTMLElement)
    ) {
      throw new Error("missing timeline origin elements");
    }
    const xAxisRect = xAxis.getBoundingClientRect();
    const laneZeroRect = laneZero.getBoundingClientRect();
    const ttftZeroRect = ttftZero.getBoundingClientRect();
    expect(Math.abs(laneZeroRect.bottom - xAxisRect.top)).toBeLessThanOrEqual(1);
    const callAxisTicks = callsAxis.querySelectorAll("[data-call-axis-tick]");
    expect(callAxisTicks.length).toBeGreaterThan(4);
    expect(Number(callAxisTicks[0]?.textContent)).toBeGreaterThanOrEqual(4);
    expect(callAxisTicks[callAxisTicks.length - 1]?.textContent).toBe("1");
    const firstCallTickRect = callAxisTicks[0]?.getBoundingClientRect();
    const lastCallTickRect = callAxisTicks[callAxisTicks.length - 1]?.getBoundingClientRect();
    expect(firstCallTickRect).toBeDefined();
    expect(lastCallTickRect).toBeDefined();
    if (firstCallTickRect && lastCallTickRect) {
      expect(firstCallTickRect.top - callsAxis.getBoundingClientRect().top).toBeLessThan(24);
      expect(lastCallTickRect.top).toBeGreaterThan(
        callsAxis.getBoundingClientRect().top + callsAxis.getBoundingClientRect().height * 0.7,
      );
    }
    expect(Math.abs(ttftZeroRect.bottom - xAxisRect.top)).toBeLessThanOrEqual(1);
    expect(Math.abs(laneZeroRect.bottom - ttftZeroRect.bottom)).toBeLessThanOrEqual(1);
    const ttftTicks = canvasElement.querySelectorAll("[data-ttft-axis-tick]");
    expect(ttftTicks).toHaveLength(5);
    expect(ttftTicks?.[0]?.textContent).toBe("836 ms");
    expect(ttftTicks?.[ttftTicks.length - 1]?.textContent).toBe("0 ms");
    expect(canvas.queryByText("全天")).toBeNull();
    expect(canvas.queryByText(/Y1|Y2/)).toBeNull();
    await expect(canvas.getByText("并发调用数")).toBeVisible();
    const bars = canvasElement.querySelectorAll(
      '[data-testid="dashboard-invocation-timeline-lane-scroll"] [data-call-value]',
    );
    expect(bars).toHaveLength(5);
    expect(Array.from(bars).every((bar) => bar.getAttribute("role") === "img")).toBe(true);
    expect(bars[0]).toHaveAttribute("aria-label", expect.stringContaining("开始时间"));
    expect(
      canvasElement.querySelectorAll(
        '[data-testid="dashboard-invocation-timeline-lane-scroll"] button',
      ),
    ).toHaveLength(0);
    await expect(canvas.getByTestId("dashboard-invocation-timeline-legend")).toHaveClass(
      "justify-center",
    );
    expect(Array.from(bars).every((bar) => bar.textContent === "")).toBe(true);
    expect(Array.from(bars).every((bar) => bar.getBoundingClientRect().width >= 8)).toBe(true);
    for (const bar of Array.from(bars)) {
      const value = bar.getAttribute("data-call-value");
      const grid = value
        ? Number(value) === 1
          ? xAxis
          : canvasElement.querySelector(
              `[data-call-axis-grid][data-call-axis-value="${Number(value) - 1}"]`,
            )
        : null;
      if (!(grid instanceof HTMLElement)) {
        throw new Error(`missing call-axis grid for value ${value}`);
      }
      expect(
        Math.abs(bar.getBoundingClientRect().bottom - grid.getBoundingClientRect().top),
      ).toBeLessThanOrEqual(1);
    }
    const controls = canvasElement.querySelectorAll("button.icon-button");
    expect(controls).toHaveLength(4);
    const controlAlignmentIssues: string[] = [];
    for (const control of controls) {
      const icon = control.querySelector("svg, [data-icon]");
      const label = control.getAttribute("aria-label") ?? "unlabeled control";
      if (!icon) {
        controlAlignmentIssues.push(`${label}: missing icon`);
        continue;
      }
      const buttonRect = control.getBoundingClientRect();
      const iconRect = icon.getBoundingClientRect();
      const buttonCenter = {
        x: buttonRect.left + buttonRect.width / 2,
        y: buttonRect.top + buttonRect.height / 2,
      };
      const iconCenter = {
        x: iconRect.left + iconRect.width / 2,
        y: iconRect.top + iconRect.height / 2,
      };
      const offset = {
        x: buttonCenter.x - iconCenter.x,
        y: buttonCenter.y - iconCenter.y,
      };
      if (Math.abs(offset.x) > 1 || Math.abs(offset.y) > 1) {
        controlAlignmentIssues.push(
          `${label}: icon center offset (${offset.x.toFixed(2)}, ${offset.y.toFixed(2)})`,
        );
      }
    }
    expect(controlAlignmentIssues).toEqual([]);
    const firstBarRect = bars[0]?.getBoundingClientRect();
    const secondBarRect = bars[1]?.getBoundingClientRect();
    if (!firstBarRect || !secondBarRect) {
      throw new Error("missing invocation bars for linear-axis check");
    }
    expect(Math.abs(firstBarRect.bottom - xAxisRect.top)).toBeLessThanOrEqual(1);
    expect(firstBarRect.top).toBeGreaterThan(secondBarRect.top);

    const timeline = canvas.getByTestId("dashboard-invocation-timeline");
    const chartFrame = canvas.getByTestId("dashboard-invocation-timeline-lanes");
    const frameHeight = chartFrame.getBoundingClientRect().height;
    const { plot, tooltip } = await hoverTimelineAt(canvasElement, 0.75, 0.45);
    expect(tooltip.textContent).toMatch(/并行 3|parallel 3/);
    expect(tooltip.textContent).toMatch(/运行 3|running 3/);
    expect(tooltip.textContent).toMatch(/排队 0|queued 0/);
    expect(Number(tooltip.getAttribute("data-hover-time-ms"))).toBeGreaterThan(0);
    expect(chartFrame.getBoundingClientRect().height).toBe(frameHeight);
    expect(timeline.lastElementChild).toBe(
      canvas.getByTestId("dashboard-invocation-timeline-legend"),
    );

    const plotRect = plot.getBoundingClientRect();
    await hoverTimelineAt(canvasElement, 0.99, 0.99);
    const edgeRect = canvas
      .getByTestId("dashboard-invocation-timeline-hover-tooltip")
      .getBoundingClientRect();
    expect(edgeRect.left).toBeGreaterThanOrEqual(plotRect.left + 7);
    expect(edgeRect.right).toBeLessThanOrEqual(plotRect.right - 7);
    expect(edgeRect.top).toBeGreaterThanOrEqual(plotRect.top + 7);
    expect(edgeRect.bottom).toBeLessThanOrEqual(plotRect.top + chartFrame.clientHeight - 28 - 7);

    await fireEvent.pointerOut(
      canvas.getByTestId("dashboard-invocation-timeline-interaction-area"),
      { relatedTarget: document.body },
    );
    await expect(canvas.queryByRole("tooltip")).toBeNull();
    await hoverTimelineAt(canvasElement, 0.75, 0.45);
  },
};

export const DensePagination: Story = {
  args: {
    response,
    loading: false,
    timelineData: denseRecords,
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByTestId("dashboard-invocation-timeline")).toBeVisible();
  },
};

export const LiveRefreshPending: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: records,
    timelineStatusOverride: "refreshing",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByText(/Live|实时更新/)).toBeVisible();
    expect(
      canvasElement.querySelectorAll(
        '[data-testid="dashboard-invocation-timeline-lane-scroll"] [data-call-value]',
      ),
    ).toHaveLength(5);
    expect(canvas.queryByTestId("dashboard-invocation-timeline-state")).toBeNull();
  },
};

export const LiveRefreshStale: Story = {
  ...LiveRefreshPending,
  args: {
    response,
    loading: false,
    error: null,
    timelineData: records,
    timelineStatusOverride: "stale",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByText(/Last successful snapshot|显示最近成功快照/)).toBeVisible();
    expect(
      canvasElement.querySelectorAll(
        '[data-testid="dashboard-invocation-timeline-lane-scroll"] [data-call-value]',
      ),
    ).toHaveLength(5);
  },
};

export const EmptyWindow: Story = {
  args: {
    response,
    loading: false,
    timelineData: { ...records, total: 0, records: [] },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText(/No invocations|当前时间窗口没有/)).toBeVisible();
  },
};

export const Unavailable: Story = {
  args: {
    response,
    loading: false,
    error: "snapshot unavailable",
    timelineData: null,
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByTestId("dashboard-invocation-timeline-state"),
    ).toBeVisible();
    await expect(within(canvasElement).getByText(/unavailable|无法获取/)).toBeVisible();
  },
};

export const MobileTraffic: Story = {
  ...LiveTraffic,
  globals: { viewport: { value: "mobile393", isRotated: false } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByRole("img", { name: /invoke-success-001/ })).toBeVisible();
    await hoverTimelineAt(canvasElement, 0.75, 0.45);
  },
};

export const EnglishTraffic: Story = {
  ...LiveTraffic,
  decorators: [
    (Story) => (
      <I18nProvider initialLocale="en" persistLocale={false}>
        <Story />
      </I18nProvider>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("Concurrent calls")).toBeVisible();
    const { tooltip } = await hoverTimelineAt(canvasElement, 0.75, 0.45);
    expect(tooltip.textContent).toMatch(/parallel 3/);
    expect(tooltip.textContent).toMatch(/running 3/);
    expect(tooltip.textContent).toMatch(/queued 0/);
  },
};

export const MobileLiveRefreshPending: Story = {
  ...LiveRefreshPending,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};

export const DenseConcurrency190: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: denseRecords,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    const frame = canvas.getByTestId("dashboard-invocation-timeline-lanes");
    await expect(frame).toBeVisible();
    const frameElement = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-lanes"]',
    );
    const laneScrollElement = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-lane-scroll"]',
    );
    if (!(frameElement instanceof HTMLElement) || !(laneScrollElement instanceof HTMLElement)) {
      throw new Error("missing dense timeline layout elements");
    }
    const expectedChartHeight = window.matchMedia("(max-width: 768px)").matches ? 336 : 320;
    expect(frameElement.getBoundingClientRect().height).toBe(expectedChartHeight);
    expect(laneScrollElement.clientHeight).toBe(expectedChartHeight - 28);
    expect(frameElement.dataset.totalLanes).toBe("190");
    expect(laneScrollElement.scrollTop).toBe(
      laneScrollElement.scrollHeight - laneScrollElement.clientHeight,
    );
    expectDenseLaneGeometry(laneScrollElement, 190);
    expect(laneScrollElement.querySelectorAll("[data-call-axis-grid]").length).toBeLessThanOrEqual(
      5,
    );
  },
};

export const OverflowConcurrency360: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: overflowRecords,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const frame = canvas.getByTestId("dashboard-invocation-timeline-lanes");
    const laneScroll = canvas.getByTestId("dashboard-invocation-timeline-lane-scroll");
    const axisScroll = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-calls-axis"] > div',
    );
    if (!(axisScroll instanceof HTMLElement)) throw new Error("missing calls axis scroller");
    const expectedChartHeight = window.matchMedia("(max-width: 768px)").matches ? 336 : 320;
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    expect(frame.getBoundingClientRect().height).toBe(expectedChartHeight);
    expect(laneScroll.scrollHeight).toBeGreaterThan(laneScroll.clientHeight);
    expect(laneScroll.scrollTop).toBe(laneScroll.scrollHeight - laneScroll.clientHeight);
    expect(frame.scrollWidth).toBeLessThanOrEqual(frame.clientWidth);
    expect(laneScroll.scrollWidth).toBeLessThanOrEqual(laneScroll.clientWidth);
    expect(laneScroll.querySelectorAll("[data-call-axis-grid]").length).toBeLessThanOrEqual(5);
    expect(axisScroll.scrollTop).toBe(laneScroll.scrollTop);
    expectDenseLaneGeometry(laneScroll, 360);
  },
};

export const FitsSeventeenLanesAtViewportEdge: Story = {
  args: {
    response,
    loading: false,
    timelineData: {
      ...records,
      total: 17,
      records: [
        ...Array.from({ length: 16 }, (_, index) => ({
          id: 100 + index,
          invokeId: `invoke-edge-overlap-${index}`,
          occurredAt: "2026-07-16T10:50:00.000Z",
          endAt: "2026-07-16T11:10:00.000Z",
          isInFlight: false,
          status: "success" as const,
          tTotalMs: 20 * 60_000,
          firstTokenMs: null,
        })),
        {
          id: 117,
          invokeId: "invoke-edge-zero-duration",
          occurredAt: "2026-07-16T11:09:59.999Z",
          endAt: "2026-07-16T11:09:59.999Z",
          isInFlight: false,
          status: "success" as const,
          tTotalMs: 0,
          firstTokenMs: null,
        },
      ],
    },
  },
  play: async ({ canvasElement }) => {
    const frame = within(canvasElement).getByTestId("dashboard-invocation-timeline-lanes");
    const laneScroll = within(canvasElement).getByTestId(
      "dashboard-invocation-timeline-lane-scroll",
    );
    expect({
      laneCount: frame.getAttribute("data-total-lanes"),
      verticalOverflowPx: laneScroll.scrollHeight - laneScroll.clientHeight,
      horizontalOverflowPx: laneScroll.scrollWidth - laneScroll.clientWidth,
    }).toEqual({
      laneCount: "17",
      verticalOverflowPx: 0,
      horizontalOverflowPx: 0,
    });
  },
};

export const MobileFitsSeventeenLanesAtViewportEdge: Story = {
  ...FitsSeventeenLanesAtViewportEdge,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};

export const OverflowScrollPositionRetention: Story = {
  args: {
    response,
    loading: false,
    timelineData: overflowRecords,
  },
  render: () => <RefreshableOverflowTimeline />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const laneScroll = canvas.getByTestId("dashboard-invocation-timeline-lane-scroll");
    const axisScroll = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-calls-axis"] > div',
    );
    if (!(axisScroll instanceof HTMLElement)) throw new Error("missing calls axis scroller");
    const manualScrollTop = laneScroll.scrollHeight - laneScroll.clientHeight - 32;
    laneScroll.scrollTop = manualScrollTop;
    await fireEvent.scroll(laneScroll);
    expect(axisScroll.scrollTop).toBe(manualScrollTop);
    await fireEvent.click(canvas.getByTestId("refresh-overflow-fixture"));
    await expect(laneScroll).toHaveProperty("scrollTop", manualScrollTop);
    expect(axisScroll.scrollTop).toBe(manualScrollTop);
  },
};

export const MobileDenseConcurrency190: Story = {
  ...DenseConcurrency190,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};

export const MobileOverflowConcurrency360: Story = {
  ...OverflowConcurrency360,
  globals: { viewport: { value: "mobile393", isRotated: false } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const frame = canvas.getByTestId("dashboard-invocation-timeline-lanes");
    const laneScroll = canvas.getByTestId("dashboard-invocation-timeline-lane-scroll");
    const axisScroll = canvasElement.querySelector(
      '[data-testid="dashboard-invocation-timeline-calls-axis"] > div',
    );
    if (!(axisScroll instanceof HTMLElement)) throw new Error("missing calls axis scroller");
    expect(frame.getBoundingClientRect().height).toBe(336);
    expect(laneScroll.scrollHeight).toBeGreaterThan(laneScroll.clientHeight);
    expect(laneScroll.scrollTop).toBe(laneScroll.scrollHeight - laneScroll.clientHeight);
    expect(frame.scrollWidth).toBeLessThanOrEqual(frame.clientWidth);
    expect(laneScroll.scrollWidth).toBeLessThanOrEqual(laneScroll.clientWidth);
    expect(axisScroll.scrollTop).toBe(laneScroll.scrollTop);
    expectDenseLaneGeometry(laneScroll, 360);

    const { plot, tooltip } = await hoverTimelineAt(canvasElement, 0.669, 0.45);
    const plotRect = plot.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    expect(tooltip.textContent).toMatch(/并行 360/);
    expect(tooltipRect.left).toBeGreaterThanOrEqual(plotRect.left + 7);
    expect(tooltipRect.right).toBeLessThanOrEqual(plotRect.right - 7);
    expect(tooltipRect.top).toBeGreaterThanOrEqual(plotRect.top + 7);
    expect(tooltipRect.bottom).toBeLessThanOrEqual(plotRect.top + frame.clientHeight - 28 - 7);
  },
};
