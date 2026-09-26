import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
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
  overLimit: false,
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

const meta = {
  title: "Dashboard/DashboardInvocationTimeline",
  component: DashboardInvocationTimeline,
  tags: ["autodocs", "test"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1440x1024" },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="vibe-dark" className="min-h-screen bg-[#08172b] text-white">
          <div
            data-visual-evidence-surface
            className="mx-auto max-w-[1280px] bg-[#08172b] px-6 py-8"
          >
            <div data-visual-evidence-target>
              <Story />
            </div>
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardInvocationTimeline>;

export default meta;

type Story = StoryObj<typeof meta>;

const fallback = (
  <div className="rounded-lg border border-warning/40 bg-warning/10 p-6 text-warning">
    Aggregate chart fallback
  </div>
);

export const LiveTraffic: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: records,
    fallback,
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
    expect(callAxisTicks).toHaveLength(4);
    expect(callAxisTicks[0]?.textContent).toBe("4");
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
    expect(Math.abs(laneZeroRect.bottom - ttftZeroRect.bottom)).toBeLessThanOrEqual(0.01);
    const ttftTicks = canvasElement.querySelectorAll("[data-ttft-axis-tick]");
    expect(ttftTicks).toHaveLength(5);
    expect(ttftTicks?.[0]?.textContent).toBe("836 ms");
    expect(ttftTicks?.[ttftTicks.length - 1]?.textContent).toBe("0 ms");
    expect(canvas.queryByText("全天")).toBeNull();
    expect(canvas.queryByText(/Y1|Y2/)).toBeNull();
    await expect(canvas.getByText("调用")).toBeVisible();
    const bars = canvasElement.querySelectorAll(
      '[data-testid="dashboard-invocation-timeline-lane-scroll"] button',
    );
    expect(Array.from(bars).every((bar) => bar.textContent === "")).toBe(true);
    expect(Array.from(bars).every((bar) => bar.getBoundingClientRect().width >= 8)).toBe(true);
    const controls = canvasElement.querySelectorAll("button.icon-button");
    expect(controls).toHaveLength(4);
    const controlsAreCentered = Array.from(controls).every((control) => {
      const icon = control.querySelector("[data-icon]");
      if (!(icon instanceof HTMLElement)) return false;
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
      return (
        Math.abs(buttonCenter.x - iconCenter.x) <= 1 && Math.abs(buttonCenter.y - iconCenter.y) <= 1
      );
    });
    expect(controlsAreCentered).toBe(true);
    const firstBarRect = bars[0]?.getBoundingClientRect();
    const secondBarRect = bars[1]?.getBoundingClientRect();
    if (!firstBarRect || !secondBarRect) {
      throw new Error("missing invocation bars for linear-axis check");
    }
    expect(Math.abs(firstBarRect.bottom - xAxisRect.top)).toBeLessThanOrEqual(1);
    expect(firstBarRect.top).toBeGreaterThan(secondBarRect.top);
  },
};

export const DenseFallback: Story = {
  args: {
    response,
    loading: false,
    timelineData: { ...records, total: 2_001, overLimit: true, records: [] },
    fallback,
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText("Aggregate chart fallback")).toBeVisible();
  },
};

export const EmptyWindow: Story = {
  args: {
    response,
    loading: false,
    timelineData: { ...records, total: 0, records: [] },
    fallback,
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText(/No invocations|当前时间窗口没有/)).toBeVisible();
  },
};

export const MobileTraffic: Story = {
  ...LiveTraffic,
  parameters: {
    viewport: { defaultViewport: "mobile393" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByRole("button", { name: /invoke-success-001/ })).toBeVisible();
  },
};

export const DenseConcurrency190: Story = {
  args: {
    response,
    loading: false,
    error: null,
    timelineData: denseRecords,
    fallback,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-invocation-timeline")).toBeVisible();
    await expect(canvas.getByText(/190.*(调用|calls)/i)).toBeVisible();
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
    expect(frameElement.getBoundingClientRect().height).toBe(320);
    expect(laneScrollElement.scrollHeight).toBe(1733);
    expect(laneScrollElement.clientHeight).toBe(292);
    await expect(canvas.getByTestId("dashboard-invocation-timeline-x-axis")).toBeVisible();
  },
};
