import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ErrorDistributionResponse,
  FailureSummaryResponse,
  ParallelWorkConversation,
  ParallelWorkStatsResponse,
  StatsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
} from "../../lib/api";
import { buildTopicDescriptor, type SubscriptionTopicEnvelope } from "../../lib/sse";
import { getBrowserTimeZone } from "../../lib/timeZone";
import StatsPage from "../../pages/Stats";
import {
  FullPageStorySurface,
  StorybookPageEnvironment,
} from "../../storybook/storybookPageHelpers";
import { getStorybookPageSseController } from "../../storybook/storybookPageSse";
import { jsonResponse } from "../../storybook/storybookResponse";

type StatsScenario = "default" | "timeseries-error";

type StatsStoryParameters = {
  scenario?: StatsScenario;
};

function buildSummary(overrides: Partial<StatsResponse>): StatsResponse {
  return {
    totalCount: 0,
    successCount: 0,
    failureCount: 0,
    totalCost: 0,
    totalTokens: 0,
    ...overrides,
  };
}

function buildTimeseriesPoints({
  count,
  bucketSeconds,
  startMs,
  offset = 0,
}: {
  count: number;
  bucketSeconds: number;
  startMs: number;
  offset?: number;
}) {
  return Array.from({ length: count }, (_, index) => {
    const bucketStartMs = startMs + index * bucketSeconds * 1000;
    const bucketEndMs = bucketStartMs + bucketSeconds * 1000;
    const bucketStart = new Date(bucketStartMs);
    const hour = bucketStart.getUTCHours();
    const businessRamp =
      hour >= 1 && hour <= 10
        ? 1.6
        : hour >= 11 && hour <= 14
          ? 1.15
          : hour >= 15 && hour <= 19
            ? 0.72
            : 0.28;
    const incidentSpike =
      index >= Math.floor(count * 0.38) && index <= Math.floor(count * 0.45) ? 22 : 0;
    const releaseTail =
      index >= Math.floor(count * 0.68) && index <= Math.floor(count * 0.76) ? 14 : 0;
    const localVariation = ((index + offset) % 7) * 3 + ((index + offset) % 11 === 0 ? 9 : 0);
    const totalCount = Math.max(
      1,
      Math.round(18 * businessRamp + localVariation + incidentSpike + releaseTail),
    );
    const failureCount = Math.min(
      totalCount,
      incidentSpike > 0
        ? 4 + (index % 3)
        : releaseTail > 0
          ? 2
          : index % 13 === 0
            ? 2
            : index % 5 === 0
              ? 1
              : 0,
    );
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount,
      successCount: totalCount - failureCount,
      failureCount,
      totalTokens: totalCount * 4100,
      totalCost: Number((totalCount * 0.021).toFixed(4)),
      firstResponseByteTotalAvgMs: 520 + (index % 7) * 32,
      firstResponseByteTotalP95Ms: 710 + (index % 5) * 44,
      firstResponseByteTotalSampleCount: totalCount,
    } satisfies TimeseriesPoint;
  });
}

function buildTimeseriesResponse(options: {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  effectiveBucket: string;
  availableBuckets: string[];
  points: TimeseriesPoint[];
}): TimeseriesResponse {
  return {
    rangeStart: options.rangeStart,
    rangeEnd: options.rangeEnd,
    bucketSeconds: options.bucketSeconds,
    effectiveBucket: options.effectiveBucket,
    availableBuckets: options.availableBuckets,
    bucketLimitedToDaily: false,
    points: options.points,
  };
}

function buildConversationFixture(rangeStart: string): ParallelWorkConversation[] {
  const startMs = Date.parse(rangeStart);
  const at = (minutes: number) => new Date(startMs + minutes * 60 * 1000).toISOString();
  return [
    ["import-a13f", 18, 66, 5],
    ["debug-72be", 102, 298, 23],
    ["ci-24d0", 135, 182, 8],
    ["schema-91db", 214, 270, 6],
    ["review-09ac", 336, 434, 14],
    ["triage-f62d", 386, 421, 4],
    ["rollup-c91b", 470, 746, 38],
    ["frontend-518e", 548, 640, 12],
    ["evidence-d0af", 612, 722, 17],
    ["contract-80ed", 650, 695, 7],
    ["followup-6b33", 805, 850, 4],
    ["build-44ef", 885, 1236, 31],
    ["contract-8f71", 1040, 1145, 11],
    ["visual-f15c", 1085, 1120, 5],
    ["lint-628a", 1160, 1194, 3],
    ["hotfix-3c59", 1302, 1398, 9],
    ["release-5e11", 1324, 1362, 4],
    ["metrics-477c", 72, 118, 6],
    ["cache-a24b", 156, 246, 13],
    ["storybook-89c0", 458, 505, 8],
    ["backfill-e76f", 520, 825, 27],
    ["docs-2b4a", 590, 635, 5],
    ["flake-0fe2", 704, 762, 9],
    ["density-49c3", 840, 918, 10],
    ["perf-f7a1", 932, 995, 7],
    ["long-review-147d", 970, 1208, 18],
    ["handoff-6aa8", 1255, 1300, 5],
    ["copy-25bc", 1280, 1315, 3],
    ["monitor-e31d", 140, 1320, 22],
    ["quick-73ad", 760, 778, 2],
    ["timezone-99fb", 1115, 1180, 8],
    ["final-c6d2", 1378, 1422, 4],
  ].map(([slug, startMinute, endMinute, requestCount]) => ({
    conversationId: `pck-${slug}`,
    start: at(Number(startMinute)),
    end: at(Number(endMinute)),
    requestCount: Number(requestCount),
  }));
}

function buildParallelWorkResponse(options: {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  effectiveTimeZone?: string;
}): ParallelWorkStatsResponse {
  const startMs = Date.parse(options.rangeStart);
  const endMs = Date.parse(options.rangeEnd);
  const bucketMs = options.bucketSeconds * 1000;
  const count = Math.max(0, Math.min(240, Math.floor((endMs - startMs) / bucketMs)));
  const rangeMs = Math.max(0, endMs - startMs);
  const conversations =
    rangeMs <= 24 * 60 * 60 * 1000 ? buildConversationFixture(options.rangeStart) : [];
  const points = Array.from({ length: count }, (_, index) => {
    const bucketStartMs = startMs + index * bucketMs;
    const bucketEndMs = bucketStartMs + bucketMs;
    const parallelCount =
      conversations.length > 0
        ? conversations.filter((conversation) => {
            const conversationStartMs = Date.parse(conversation.start);
            const conversationEndMs = Date.parse(conversation.end);
            return conversationStartMs < bucketEndMs && conversationEndMs > bucketStartMs;
          }).length
        : Math.max(
            0,
            index % 24 >= 9 && index % 24 <= 18 ? 3 + (index % 4) : index % 17 === 0 ? 1 : 0,
          );
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      parallelCount,
    };
  });
  const counts = points.map((point) => point.parallelCount);
  const current = {
    rangeStart: options.rangeStart,
    rangeEnd: options.rangeEnd,
    bucketSeconds: options.bucketSeconds,
    completeBucketCount: points.length,
    activeBucketCount: points.filter((point) => point.parallelCount > 0).length,
    minCount: counts.length ? Math.min(...counts) : null,
    maxCount: counts.length ? Math.max(...counts) : null,
    avgCount: counts.length ? counts.reduce((sum, value) => sum + value, 0) / counts.length : null,
    effectiveTimeZone: options.effectiveTimeZone ?? "Asia/Shanghai",
    timeZoneFallback: false,
    points,
    conversations,
  };
  return {
    current,
    minute7d: current,
    hour30d: current,
    dayAll: current,
  };
}

function buildStatsStoryFixtures() {
  const now = Date.parse("2026-04-06T12:00:00.000Z");
  const todayStart = Date.parse("2026-04-06T00:00:00.000Z");
  const dayStart = now - 24 * 60 * 60 * 1000;
  const weekStart = now - 7 * 24 * 60 * 60 * 1000;
  const rangeStartByRange = (range: string) =>
    range === "7d" ? weekStart : range === "1d" ? dayStart : todayStart;

  const summaryByWindow: Record<string, StatsResponse> = {
    today: buildSummary({
      totalCount: 18324,
      successCount: 16510,
      failureCount: 1814,
      totalCost: 812.41,
      totalTokens: 1732145566,
    }),
    "1d": buildSummary({
      totalCount: 18324,
      successCount: 16510,
      failureCount: 1814,
      totalCost: 812.41,
      totalTokens: 1732145566,
    }),
    "7d": buildSummary({
      totalCount: 102116,
      successCount: 95514,
      failureCount: 6602,
      totalCost: 4411.18,
      totalTokens: 10021567342,
    }),
  };

  const errorDistribution: ErrorDistributionResponse = {
    rangeStart: new Date(todayStart).toISOString(),
    rangeEnd: new Date(now).toISOString(),
    items: [
      { reason: "upstream_timeout", count: 482 },
      { reason: "rate_limited", count: 316 },
      { reason: "connection_reset", count: 211 },
      { reason: "invalid_request", count: 97 },
    ],
  };

  const failureSummary: FailureSummaryResponse = {
    rangeStart: new Date(todayStart).toISOString(),
    rangeEnd: new Date(now).toISOString(),
    totalFailures: 1106,
    serviceFailureCount: 742,
    clientFailureCount: 214,
    clientAbortCount: 150,
    actionableFailureCount: 956,
    actionableFailureRate: 0.864,
  };

  const buildTimeseriesForRange = (range: string, bucket: string) => {
    const rangeStart = rangeStartByRange(range);
    if (range === "7d") {
      const bucketSeconds =
        bucket === "1d" ? 86400 : bucket === "12h" ? 43200 : bucket === "6h" ? 21600 : 3600;
      return buildTimeseriesResponse({
        rangeStart: new Date(weekStart).toISOString(),
        rangeEnd: new Date(now).toISOString(),
        bucketSeconds,
        effectiveBucket: bucket,
        availableBuckets: ["1h", "6h", "12h", "1d"],
        points: buildTimeseriesPoints({
          count: bucket === "1d" ? 7 : bucket === "12h" ? 14 : bucket === "6h" ? 28 : 7 * 24,
          bucketSeconds,
          startMs: weekStart,
          offset: 3,
        }),
      });
    }
    const bucketSeconds =
      bucket === "1m"
        ? 60
        : bucket === "5m"
          ? 300
          : bucket === "30m"
            ? 1800
            : bucket === "1h"
              ? 3600
              : 900;
    return buildTimeseriesResponse({
      rangeStart: new Date(rangeStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds,
      effectiveBucket: bucket,
      availableBuckets: ["1m", "15m", "30m", "1h", "6h"],
      points: buildTimeseriesPoints({
        count:
          bucket === "1m"
            ? 24 * 60
            : bucket === "1h"
              ? 24
              : bucket === "30m"
                ? 48
                : bucket === "5m"
                  ? 288
                  : 96,
        bucketSeconds,
        startMs: rangeStart,
      }),
    });
  };

  const buildParallelWorkForRange = (range: string, bucket: string, timeZone: string) => {
    const bucketSeconds =
      bucket === "1m" ? 60 : bucket === "30m" ? 1800 : bucket === "1h" ? 3600 : 900;
    return buildParallelWorkResponse({
      rangeStart: new Date(rangeStartByRange(range)).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds,
      effectiveTimeZone: timeZone,
    });
  };

  return {
    now,
    summaryByWindow,
    errorDistribution,
    failureSummary,
    buildTimeseriesForRange,
    buildParallelWorkForRange,
  };
}

function buildStatsRequestHandler(scenario: StatsScenario = "default") {
  const fixtures = buildStatsStoryFixtures();

  return ({ url }: { url: URL }) => {
    if (url.pathname === "/api/stats/summary") {
      const window = url.searchParams.get("window") ?? "today";
      return jsonResponse(fixtures.summaryByWindow[window] ?? fixtures.summaryByWindow.today);
    }

    if (url.pathname === "/api/stats/timeseries") {
      if (scenario === "timeseries-error") {
        return new Response("timeseries temporarily unavailable", { status: 500 });
      }
      const range = url.searchParams.get("range") ?? "today";
      const bucket = url.searchParams.get("bucket") ?? (range === "7d" ? "1h" : "15m");
      return jsonResponse(fixtures.buildTimeseriesForRange(range, bucket));
    }

    if (url.pathname === "/api/stats/errors") {
      return jsonResponse(fixtures.errorDistribution);
    }

    if (url.pathname === "/api/stats/failures/summary") {
      return jsonResponse(fixtures.failureSummary);
    }

    if (url.pathname === "/api/stats/parallel-work") {
      const range = url.searchParams.get("range") ?? "today";
      const bucket = url.searchParams.get("bucket") ?? (range === "7d" ? "1h" : "15m");
      return jsonResponse(fixtures.buildParallelWorkForRange(range, bucket, getBrowserTimeZone()));
    }

    return undefined;
  };
}

function StatsPageSseBootstrap({ scenario }: { scenario: StatsScenario }) {
  useEffect(() => {
    const controller = getStorybookPageSseController();
    if (!controller) return;

    const fixtures = buildStatsStoryFixtures();
    const timeZone = getBrowserTimeZone();
    let cursor = 1;

    const emitSnapshot = <T,>(descriptor: ReturnType<typeof buildTopicDescriptor>, payload: T) => {
      controller.emit({
        type: "snapshot",
        topic: descriptor,
        topicKey: `storybook:${descriptor.topic}:${cursor}`,
        schemaEpoch: "storybook-stats-page-v1",
        cursor,
        payload,
      } satisfies SubscriptionTopicEnvelope<T>);
      cursor += 1;
    };

    const timer = window.setTimeout(() => {
      controller.emitOpen();
      emitSnapshot(
        buildTopicDescriptor("stats.summary.current", { window: "today", timeZone }),
        fixtures.summaryByWindow.today,
      );
      emitSnapshot(
        buildTopicDescriptor("stats.summary.current", { window: "7d", timeZone }),
        fixtures.summaryByWindow["7d"],
      );
      emitSnapshot(
        buildTopicDescriptor("stats.timeseries.open-window", {
          range: "today",
          bucket: "15m",
          timeZone,
        }),
        fixtures.buildTimeseriesForRange("today", "15m"),
      );
      emitSnapshot(
        buildTopicDescriptor("stats.timeseries.open-window", {
          range: "7d",
          bucket: "1h",
          timeZone,
        }),
        fixtures.buildTimeseriesForRange("7d", "1h"),
      );
      emitSnapshot(
        buildTopicDescriptor("stats.parallel-work.current", {
          range: "today",
          bucket: "15m",
          timeZone,
        }),
        fixtures.buildParallelWorkForRange("today", "15m", timeZone),
      );
      emitSnapshot(
        buildTopicDescriptor("stats.parallel-work.current", {
          range: "7d",
          bucket: "1h",
          timeZone,
        }),
        fixtures.buildParallelWorkForRange("7d", "1h", timeZone),
      );
    }, 0);

    return () => window.clearTimeout(timer);
  }, [scenario]);

  return null;
}

const meta = {
  title: "Pages/StatsPage",
  component: StatsPage,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    scenario: "default",
  },
  decorators: [
    (Story, context) => {
      const scenario = ((context.parameters as StatsStoryParameters).scenario ??
        "default") as StatsScenario;
      return (
        <I18nProvider>
          <StorybookPageEnvironment onRequest={buildStatsRequestHandler(scenario)}>
            <StatsPageSseBootstrap scenario={scenario} />
            <FullPageStorySurface>
              <Story />
            </FullPageStorySurface>
          </StorybookPageEnvironment>
        </I18nProvider>
      );
    },
  ],
} satisfies Meta<typeof StatsPage>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("统计")).toBeVisible();
    await expect(canvas.getByTestId("stats-range-select-trigger")).toBeVisible();
    await expect(canvas.getByTestId("stats-bucket-select-trigger")).toBeVisible();
    await expect(canvas.getByTestId("stats-bucket-select-trigger")).toHaveTextContent("每 15 分钟");

    await userEvent.click(canvas.getByTestId("stats-range-select-trigger"));
    await userEvent.click(within(document.body).getByText("最近 7 天"));
    await expect(canvas.getByTestId("stats-range-select-trigger")).toHaveTextContent("最近 7 天");
    await expect(canvas.queryByTestId("parallel-work-conversation-gantt")).toBeNull();
  },
};

export const MobileFullWidthSurface: Story = {
  name: "Mobile / Full-width working surface",
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("统计")).toBeVisible();
    await expect(canvas.getByTestId("stats-range-select-trigger")).toBeVisible();
    await expect(canvas.getByTestId("stats-bucket-select-trigger")).toBeVisible();
  },
};

export const MinuteBucketOptions: Story = {
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId("parallel-work-conversation-gantt")).toBeVisible();
    const bucketTrigger = canvas.getByTestId("stats-bucket-select-trigger");
    await expect(bucketTrigger).toHaveTextContent("每 15 分钟");
    await userEvent.click(bucketTrigger);
    await expect(within(document.body).getByText("每分钟")).toBeVisible();
  },
};

export const EvidenceTodayMinuteOptionsGantt: Story = {
  name: "Evidence / Today Minute Options + Gantt",
  tags: ["test"],
  parameters: {
    a11y: {
      test: "off",
    },
  },
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const gantt = await canvas.findByTestId("parallel-work-conversation-gantt");
    await expect(gantt).toHaveAttribute("data-chart-mode", "conversation-gantt");

    const bucketTrigger = canvas.getByTestId("stats-bucket-select-trigger");
    await expect(bucketTrigger).toHaveTextContent("每 15 分钟");
    await userEvent.click(bucketTrigger);

    const options = within(document.body).getAllByRole("option");
    await expect(options[0]).toHaveTextContent("每分钟");
    await expect(options[1]).toHaveTextContent("每 15 分钟");
  },
};

export const EvidenceSevenDayRechartsPage: Story = {
  name: "Evidence / Seven Day Recharts Page",
  tags: ["test"],
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("stats-range-select-trigger"));
    await userEvent.click(within(document.body).getByText("最近 7 天"));
    await expect(canvas.getByTestId("stats-range-select-trigger")).toHaveTextContent("最近 7 天");
    await expect(canvas.queryByTestId("parallel-work-conversation-gantt")).toBeNull();
    await waitFor(() => {
      const chart = canvasElement.querySelector('[data-chart-kind="parallel-work-sparkline"]');
      if (!(chart instanceof HTMLElement || chart instanceof SVGElement)) {
        throw new Error("parallel-work-sparkline chart unavailable");
      }
      expect(chart).toHaveAttribute("data-chart-mode", "recharts-area");
    });
  },
};

export const TimeseriesError: Story = {
  parameters: {
    scenario: "timeseries-error",
  },
  render: () => <StatsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("统计")).toBeVisible();
    await expect(canvas.getAllByRole("alert").at(0)).toBeVisible();
  },
};
