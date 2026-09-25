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
      occurredAt: "2026-07-16T11:04:00.000Z",
      endAt: "2026-07-16T11:04:02.400Z",
      isInFlight: false,
      status: "success",
      tTotalMs: 2_400,
      firstTokenMs: 620,
    },
    {
      id: 2,
      invokeId: "invoke-responding-002",
      occurredAt: "2026-07-16T11:04:00.500Z",
      endAt: null,
      isInFlight: true,
      livePhase: "responding",
      tTotalMs: null,
      firstTokenMs: 1_120,
    },
    {
      id: 3,
      invokeId: "invoke-failed-003",
      occurredAt: "2026-07-16T11:05:15.000Z",
      endAt: "2026-07-16T11:05:15.000Z",
      isInFlight: false,
      status: "failed",
      tTotalMs: 0,
      failureClass: "service_failure",
    },
    {
      id: 4,
      invokeId: "invoke-queued-004",
      occurredAt: "2026-07-16T11:06:20.000Z",
      endAt: null,
      isInFlight: true,
      livePhase: "queued",
      tTotalMs: null,
    },
    {
      id: 5,
      invokeId: "invoke-unknown-005",
      occurredAt: "2026-07-16T11:07:00.000Z",
      endAt: null,
      isInFlight: false,
      status: "success",
      tTotalMs: null,
    },
  ],
};

const meta = {
  title: "Dashboard/DashboardInvocationTimeline",
  component: DashboardInvocationTimeline,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1440x1024" },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="vibe-dark" className="min-h-screen bg-[#08172b] px-6 py-8 text-white">
          <div data-visual-evidence-surface className="mx-auto max-w-[1280px]">
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
  },
};

export const DenseFallback: Story = {
  args: {
    response,
    loading: false,
    timelineData: { ...records, total: 2_001, overLimit: true, records: [] },
    fallback,
  },
};

export const EmptyWindow: Story = {
  args: {
    response,
    loading: false,
    timelineData: { ...records, total: 0, records: [] },
    fallback,
  },
};
