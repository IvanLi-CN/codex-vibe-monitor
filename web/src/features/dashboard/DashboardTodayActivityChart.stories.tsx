import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { TimeseriesResponse } from "../../lib/api";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";

const STORY_DAY_START = "2026-04-08T00:00:00+08:00";
const MINUTE_MS = 60_000;

function deterministicNoise(index: number, salt = 0) {
  const value = Math.sin((index + 1) * (12.9898 + salt)) * 43758.5453;
  return value - Math.floor(value);
}

function gaussian(value: number, center: number, width: number) {
  return Math.exp(-((value - center) ** 2) / (2 * width ** 2));
}

function buildRealisticPoint(index: number, intensity = 1) {
  const bucketStart = new Date(STORY_DAY_START);
  bucketStart.setMinutes(bucketStart.getMinutes() + index);
  const bucketEnd = new Date(bucketStart.getTime() + MINUTE_MS);
  const hour = index / 60;

  const officeRamp = 1 / (1 + Math.exp(-(hour - 7.8) * 1.45));
  const lunchDip = 1 - 0.34 * gaussian(hour, 12.15, 0.42);
  const deployBurst =
    8 * gaussian(hour, 9.25, 0.16) +
    5 * gaussian(hour, 10.75, 0.2) +
    4 * gaussian(hour, 11.55, 0.18);
  const lowTrafficDrop =
    hour < 6 && (index % 13 === 0 || index % 17 === 0) ? 1 : 0;
  const expected =
    (0.25 + officeRamp * 8.2 * lunchDip + deployBurst) * intensity;
  const jitter =
    (deterministicNoise(index, 0.4) - 0.5) * (hour < 7 ? 2 : 4.2);
  const quietMinute =
    lowTrafficDrop > 0 ||
    (hour < 7.2 && index % 11 === 0) ||
    (hour >= 7.2 && deterministicNoise(index, 1.6) < 0.018);
  const totalCount = quietMinute
    ? 0
    : Math.max(0, Math.round(expected + jitter));
  const failureCount =
    totalCount <= 0
      ? 0
      : deterministicNoise(index, 2.8) > 0.965
        ? Math.min(2, totalCount)
        : deterministicNoise(index, 3.4) > 0.91
          ? 1
          : 0;
  const inFlightCount =
    totalCount > failureCount && index > 690 && index % 7 === 0 ? 1 : 0;
  const queuedInFlightCount = inFlightCount > 0 && index % 14 === 0 ? 1 : 0;
  const runningInFlightCount = Math.max(0, inFlightCount - queuedInFlightCount);
  const successCount = Math.max(totalCount - failureCount - inFlightCount, 0);
  const completedCount = successCount + failureCount;
  const avgTokens =
    520 +
    Math.round(gaussian(hour, 9.3, 0.9) * 460) +
    Math.round(deterministicNoise(index, 4.2) * 360);
  const totalTokens = completedCount * avgTokens;
  const latencyBase =
    410 +
    officeRamp * 90 +
    gaussian(hour, 9.25, 0.18) * 210 +
    gaussian(hour, 11.55, 0.18) * 140;

  return {
    bucketStart: bucketStart.toISOString(),
    bucketEnd: bucketEnd.toISOString(),
    totalCount,
    successCount,
    failureCount,
    inFlightCount,
    inFlightPhaseCounts: {
      queued: queuedInFlightCount,
      requesting: runningInFlightCount,
      responding: 0,
    },
    totalTokens,
    totalCost: Number((totalTokens * 0.000018).toFixed(4)),
    nonSuccessCost: Number((failureCount * avgTokens * 0.000018).toFixed(4)),
    firstResponseByteTotalSampleCount: completedCount,
    firstResponseByteTotalAvgMs:
      completedCount > 0
        ? Math.round(latencyBase + deterministicNoise(index, 5.1) * 115)
        : null,
  };
}

function buildFullNaturalDayResponse(intensity = 1): TimeseriesResponse {
  return {
    rangeStart: "2026-04-08T00:00:00+08:00",
    rangeEnd: "2026-04-09T00:00:00+08:00",
    bucketSeconds: 60,
    points: Array.from({ length: 1440 }, (_, index) =>
      buildRealisticPoint(index, intensity),
    ),
  };
}

function buildZeroNonSuccessResponse(
  response: TimeseriesResponse,
): TimeseriesResponse {
  return {
    ...response,
    points: response.points.map((point) => {
      const completedCount =
        Math.max(point.totalCount ?? 0, 0) - Math.max(point.inFlightCount ?? 0, 0);
      return {
        ...point,
        successCount: completedCount,
        failureCount: 0,
        nonSuccessCost: 0,
      };
    }),
  };
}

const sampleResponse: TimeseriesResponse = {
  rangeStart: "2026-04-08T00:00:00+08:00",
  rangeEnd: "2026-04-08T12:24:00+08:00",
  bucketSeconds: 60,
  points: Array.from({ length: 745 }, (_, index) =>
    buildRealisticPoint(index, 0.85),
  ),
};

const latencyMinuteAlignmentResponse: TimeseriesResponse = {
  rangeStart: "2026-04-08T00:00:00+08:00",
  rangeEnd: "2026-04-08T12:24:00+08:00",
  bucketSeconds: 60,
  points: Array.from({ length: 745 }, (_, index) => {
    if (index >= 720 && index <= 739) {
      const bucketStart = new Date(STORY_DAY_START);
      bucketStart.setMinutes(bucketStart.getMinutes() + index);
      const bucketEnd = new Date(bucketStart.getTime() + MINUTE_MS);
      const hasCalls = index !== 723 && index !== 730;
      const hasInconsistentLatency = index === 730;
      return {
        bucketStart: bucketStart.toISOString(),
        bucketEnd: bucketEnd.toISOString(),
        totalCount: hasCalls ? 1 : 0,
        successCount: hasCalls ? 1 : 0,
        failureCount: 0,
        inFlightCount: 0,
        totalTokens: hasCalls ? 900 : 0,
        totalCost: hasCalls ? 0.0162 : 0,
        nonSuccessCost: 0,
        firstResponseByteTotalSampleCount: hasCalls || hasInconsistentLatency ? 1 : 0,
        firstResponseByteTotalAvgMs: hasCalls
          ? 820 + (index - 731) * 42
          : hasInconsistentLatency
            ? 18_225.02
            : null,
      };
    }
    return buildRealisticPoint(index, 0.72);
  }),
};

const mixedOutcomeMinuteAlignmentResponse: TimeseriesResponse = {
  rangeStart: "2026-04-08T00:00:00+08:00",
  rangeEnd: "2026-04-08T12:24:00+08:00",
  bucketSeconds: 60,
  points: Array.from({ length: 745 }, (_, index) => {
    const bucketStart = new Date(STORY_DAY_START);
    bucketStart.setMinutes(bucketStart.getMinutes() + index);
    const bucketEnd = new Date(bucketStart.getTime() + MINUTE_MS);
    const isAlignmentMinute = index >= 36 && index <= 48;
    const isFocusMinute = index === 42;
    const successCount = isAlignmentMinute ? (isFocusMinute ? 7 : 3) : 0;
    const failureCount = isAlignmentMinute ? (isFocusMinute ? 4 : 2) : 0;
    const inFlightCount = isFocusMinute ? 2 : 0;
    const queuedInFlightCount = isFocusMinute ? 1 : 0;
    const runningInFlightCount = Math.max(0, inFlightCount - queuedInFlightCount);
    const completedCount = successCount + failureCount;

    return {
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: successCount + failureCount + inFlightCount,
      successCount,
      failureCount,
      inFlightCount,
      inFlightPhaseCounts: {
        queued: queuedInFlightCount,
        requesting: runningInFlightCount,
        responding: 0,
      },
      totalTokens: completedCount * 920,
      totalCost: Number((completedCount * 0.0166).toFixed(4)),
      nonSuccessCost: Number((failureCount * 0.0166).toFixed(4)),
      firstResponseByteTotalSampleCount: completedCount,
      firstResponseByteTotalAvgMs:
        completedCount > 0 ? 760 + (index - 42) * 18 : null,
    };
  }),
};

const meta = {
  title: "Dashboard/DashboardTodayActivityChart",
  component: DashboardTodayActivityChart,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-[1560px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardTodayActivityChart>;

export default meta;

type Story = StoryObj<typeof meta>;

export const CountBars: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: "totalCount",
  },
};

export const CostCumulative: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: "totalCost",
  },
};

export const CostCumulativeMixedOutcome: Story = {
  args: {
    response: mixedOutcomeMinuteAlignmentResponse,
    loading: false,
    error: null,
    metric: "totalCost",
  },
  parameters: {
    viewport: {
      defaultViewport: "desktop1440",
    },
  },
};

export const CostCumulativeZeroNonSuccess: Story = {
  args: {
    response: buildZeroNonSuccessResponse(sampleResponse),
    loading: false,
    error: null,
    metric: "totalCost",
  },
  parameters: {
    viewport: {
      defaultViewport: "desktop1440",
    },
  },
};

export const TokensCumulative: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: "totalTokens",
  },
};

export const TrendArea: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: "trend",
  },
};

export const CountBarsDensePairing: Story = {
  args: {
    response: buildFullNaturalDayResponse(1.18),
    loading: false,
    error: null,
    metric: "totalCount",
    closedNaturalDay: true,
  },
};

export const CountBarsLatencyMinuteAlignment: Story = {
  args: {
    response: latencyMinuteAlignmentResponse,
    loading: false,
    error: null,
    metric: "totalCount",
  },
};

export const CountBarsMixedOutcomeAlignment: Story = {
  args: {
    response: mixedOutcomeMinuteAlignmentResponse,
    loading: false,
    error: null,
    metric: "totalCount",
  },
  render: () => (
    <div className="overflow-hidden" data-testid="mixed-outcome-alignment">
      <div className="w-[7200px] max-w-none">
        <DashboardTodayActivityChart
          response={mixedOutcomeMinuteAlignmentResponse}
          loading={false}
          error={null}
          metric="totalCount"
        />
      </div>
    </div>
  ),
};

export const CountBarsMinuteGranularityZoom: Story = {
  args: {
    response: buildFullNaturalDayResponse(1.18),
    loading: false,
    error: null,
    metric: "totalCount",
    closedNaturalDay: true,
  },
  render: () => (
    <div className="overflow-hidden" data-testid="minute-granularity-zoom">
      <div className="w-[7200px] max-w-none">
        <DashboardTodayActivityChart
          response={buildFullNaturalDayResponse(1.18)}
          loading={false}
          error={null}
          metric="totalCount"
          closedNaturalDay
        />
      </div>
    </div>
  ),
};

export const CountBarsInteractiveViewport: Story = {
  args: {
    response: buildFullNaturalDayResponse(1.18),
    loading: false,
    error: null,
    metric: "totalCount",
    closedNaturalDay: true,
  },
  parameters: {
    viewport: {
      defaultViewport: "desktop1440",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const chart = await canvas.findByTestId("dashboard-today-activity-chart");
    const layer = await canvas.findByTestId(
      "dashboard-today-activity-chart-interaction-layer",
    );
    const rect = layer.getBoundingClientRect();

    layer.dispatchEvent(
      new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        deltaY: -700,
        clientX: rect.left + rect.width / 2,
      }),
    );

    await expect(chart).toHaveAttribute("data-zoomed", "true");
    const zoomedStart = Number(chart.getAttribute("data-visible-start-index"));

    layer.dispatchEvent(
      new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        deltaX: 320,
        deltaY: 4,
        clientX: rect.left + rect.width / 2,
      }),
    );

    await waitFor(() => {
      expect(Number(chart.getAttribute("data-visible-start-index"))).toBeGreaterThan(
        zoomedStart,
      );
    });
  },
};

export const EmptyState: Story = {
  args: {
    response: {
      rangeStart: "2026-04-08T00:00:00+08:00",
      rangeEnd: "2026-04-08T12:24:00+08:00",
      bucketSeconds: 60,
      points: [],
    },
    loading: false,
    error: null,
    metric: "totalCount",
  },
};
