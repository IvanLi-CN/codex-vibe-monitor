import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../i18n";
import type {
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
} from "../lib/api";
import { ParallelWorkStatsSection } from "./ParallelWorkStatsSection";

function buildConversationFixtures(
  points: ParallelWorkWindowResponse["points"],
  conversationCount: number,
): ParallelWorkWindowResponse["conversations"] {
  if (points.length === 0) return [];

  return Array.from({ length: conversationCount }, (_, conversationIndex) => {
    const burstLength = Math.max(4, Math.floor(points.length / 18));
    const firstStart = Math.floor(
      (conversationIndex / Math.max(1, conversationCount)) *
        Math.max(1, points.length - burstLength),
    );
    const secondStart = Math.min(
      points.length - burstLength,
      firstStart + Math.floor(points.length / 3) + conversationIndex * 3,
    );
    const candidateIndexes = [
      ...Array.from(
        { length: burstLength },
        (_, offset) => firstStart + offset,
      ),
      ...Array.from(
        { length: Math.max(3, Math.floor(burstLength / 2)) },
        (_, offset) => secondStart + offset * 2,
      ),
    ];

    const segments = Array.from(new Set(candidateIndexes))
      .filter((index) => index >= 0 && index < points.length)
      .map((index) => {
        const point = points[index];
        return {
          bucketStart: point.bucketStart,
          bucketEnd: point.bucketEnd,
          requestCount: Math.max(
            1,
            point.parallelCount - Math.floor(conversationIndex / 2),
          ),
        };
      });

    return {
      conversationKey: `story-conversation-${conversationIndex + 1}`,
      label: `C${conversationIndex + 1}`,
      firstSeenAt: segments[0]?.bucketStart ?? points[0]?.bucketStart ?? "",
      lastSeenAt: segments.at(-1)?.bucketEnd ?? points.at(-1)?.bucketEnd ?? "",
      activeBucketCount: segments.length,
      requestCount: segments.reduce(
        (total, segment) => total + segment.requestCount,
        0,
      ),
      segments,
    };
  }).filter((conversation) => conversation.segments.length > 0);
}

function buildWindow(
  overrides: Partial<ParallelWorkWindowResponse> & {
    rangeStart: string;
    rangeEnd: string;
    bucketSeconds: number;
    completeBucketCount: number;
    activeBucketCount: number;
    points: ParallelWorkWindowResponse["points"];
  },
): ParallelWorkWindowResponse {
  return {
    rangeStart: overrides.rangeStart,
    rangeEnd: overrides.rangeEnd,
    bucketSeconds: overrides.bucketSeconds,
    completeBucketCount: overrides.completeBucketCount,
    activeBucketCount: overrides.activeBucketCount,
    minCount: overrides.minCount ?? 0,
    maxCount: overrides.maxCount ?? 0,
    avgCount: overrides.avgCount ?? 0,
    points: overrides.points,
    conversations:
      overrides.conversations ?? buildConversationFixtures(overrides.points, 8),
  };
}

const populatedStats: ParallelWorkStatsResponse = {
  current: buildWindow({
    rangeStart: "2026-03-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 60,
    completeBucketCount: 10_080,
    activeBucketCount: 4_132,
    minCount: 0,
    maxCount: 18,
    avgCount: 4.67,
    points: Array.from({ length: 16 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 2, 7, 10, index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 2, 7, 10, index + 1)).toISOString(),
      parallelCount:
        [1, 3, 2, 5, 7, 10, 8, 9, 12, 11, 15, 14, 13, 9, 6, 4][index] ?? 0,
    })),
  }),
  minute7d: buildWindow({
    rangeStart: "2026-03-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 60,
    completeBucketCount: 10_080,
    activeBucketCount: 4_132,
    minCount: 0,
    maxCount: 18,
    avgCount: 4.67,
    points: Array.from({ length: 16 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 2, 7, 10, index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 2, 7, 10, index + 1)).toISOString(),
      parallelCount:
        [1, 3, 2, 5, 7, 10, 8, 9, 12, 11, 15, 14, 13, 9, 6, 4][index] ?? 0,
    })),
  }),
  hour30d: buildWindow({
    rangeStart: "2026-02-06T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 3600,
    completeBucketCount: 720,
    activeBucketCount: 321,
    minCount: 0,
    maxCount: 9,
    avgCount: 2.13,
    points: Array.from({ length: 12 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 2, 7, index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 2, 7, index + 1)).toISOString(),
      parallelCount: [0, 1, 2, 2, 3, 4, 6, 5, 4, 3, 2, 1][index] ?? 0,
    })),
  }),
  dayAll: buildWindow({
    rangeStart: "2026-01-01T00:00:00Z",
    rangeEnd: "2026-03-08T00:00:00Z",
    bucketSeconds: 86_400,
    completeBucketCount: 67,
    activeBucketCount: 54,
    minCount: 0,
    maxCount: 6,
    avgCount: 2.04,
    points: Array.from({ length: 10 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 1, 27 + index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 1, 28 + index)).toISOString(),
      parallelCount: [1, 2, 3, 5, 4, 4, 6, 5, 3, 2][index] ?? 0,
    })),
  }),
};

function buildDenseMinutePoints(
  length: number,
): ParallelWorkWindowResponse["points"] {
  return Array.from({ length }, (_, index) => {
    const wave = Math.max(0, Math.round(Math.sin(index / 8) * 4 + 5));
    const burst = index % 37 < 6 ? 5 : 0;
    const idle = index % 53 > 45 ? -4 : 0;
    const parallelCount = Math.max(0, Math.min(16, wave + burst + idle));
    return {
      bucketStart: new Date(Date.UTC(2026, 3, 18, 0, index * 42)).toISOString(),
      bucketEnd: new Date(
        Date.UTC(2026, 3, 18, 0, index * 42 + 1),
      ).toISOString(),
      parallelCount,
    };
  });
}

const wideMinuteWindow = buildWindow({
  rangeStart: "2026-04-18T00:00:00Z",
  rangeEnd: "2026-04-25T00:00:00Z",
  bucketSeconds: 60,
  completeBucketCount: 10_080,
  activeBucketCount: 6_420,
  minCount: 0,
  maxCount: 16,
  avgCount: 4.82,
  points: buildDenseMinutePoints(240),
});

const wideResponsiveStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  current: wideMinuteWindow,
  minute7d: wideMinuteWindow,
};

const todayFiveMinuteWindow = buildWindow({
  rangeStart: "2026-04-25T00:00:00Z",
  rangeEnd: "2026-04-25T23:59:59Z",
  bucketSeconds: 300,
  completeBucketCount: 288,
  activeBucketCount: 42,
  minCount: 0,
  maxCount: 7,
  avgCount: 1.92,
  points: Array.from({ length: 48 }, (_, index) => ({
    bucketStart: new Date(Date.UTC(2026, 3, 25, 8, index * 5)).toISOString(),
    bucketEnd: new Date(Date.UTC(2026, 3, 25, 8, index * 5 + 5)).toISOString(),
    parallelCount: [0, 1, 2, 4, 6, 5, 3, 2][index % 8] ?? 0,
  })),
});

const todayFiveMinuteStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  current: todayFiveMinuteWindow,
};

const emptyDayAllWindow = buildWindow({
  rangeStart: "2026-03-08T00:00:00Z",
  rangeEnd: "2026-03-08T00:00:00Z",
  bucketSeconds: 86_400,
  completeBucketCount: 0,
  activeBucketCount: 0,
  minCount: null,
  maxCount: null,
  avgCount: null,
  points: [],
});

const emptyDayAllStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  current: emptyDayAllWindow,
  dayAll: emptyDayAllWindow,
};

populatedStats.current = populatedStats.minute7d;

const meta = {
  title: "Stats/ParallelWorkStatsSection",
  component: ParallelWorkStatsSection,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-7xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof ParallelWorkStatsSection>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByTestId("parallel-work-card-minute7d"),
    ).toBeInTheDocument();
    await expect(canvas.queryByTestId("parallel-work-card-hour30d")).toBeNull();
    await expect(canvas.queryByTestId("parallel-work-card-dayAll")).toBeNull();
    const card = await canvas.findByTestId("parallel-work-card-minute7d");
    const chart = card.querySelector('[data-chart-library="recharts"]');
    await expect(chart).toBeInTheDocument();
    await expect(
      canvas.queryByTestId("parallel-work-window-toggle"),
    ).toBeNull();
  },
};

export const Hour30dSelected: Story = {
  args: {
    stats: { ...populatedStats, current: populatedStats.hour30d },
    isLoading: false,
    error: null,
    defaultWindowKey: "hour30d",
  },
};

export const DayAllEmpty: Story = {
  args: {
    stats: emptyDayAllStats,
    isLoading: false,
    error: null,
    defaultWindowKey: "dayAll",
  },
};

export const Loading: Story = {
  args: {
    stats: null,
    isLoading: true,
    error: null,
    defaultWindowKey: "hour30d",
  },
};

export const LoadError: Story = {
  args: {
    stats: null,
    isLoading: false,
    error:
      "Request failed: 400 unsupported timeZone for historical parallel-work rollups",
  },
};

export const Gallery: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
  },
  render: () => (
    <div className="space-y-6">
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
      />
      <ParallelWorkStatsSection
        stats={{ ...populatedStats, current: populatedStats.hour30d }}
        isLoading={false}
        error={null}
        defaultWindowKey="hour30d"
      />
      <ParallelWorkStatsSection
        stats={emptyDayAllStats}
        isLoading={false}
        error={null}
        defaultWindowKey="dayAll"
      />
      <ParallelWorkStatsSection stats={null} isLoading={true} error={null} />
      <ParallelWorkStatsSection
        stats={null}
        isLoading={false}
        error="Request failed: 500 unable to load parallel-work stats"
      />
    </div>
  ),
};

export const WideMinute7dResponsive: Story = {
  args: {
    stats: wideResponsiveStats,
    isLoading: false,
    error: null,
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-[1920px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  parameters: {
    viewport: { defaultViewport: "responsive" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const card = await canvas.findByTestId("parallel-work-card-minute7d");
    const chart = card.querySelector('[data-chart-library="recharts"]');
    await expect(chart).toBeInTheDocument();
  },
};

export const TodayFiveMinuteGantt: Story = {
  args: {
    stats: todayFiveMinuteStats,
    isLoading: false,
    error: null,
    rangeLabel: "Today",
    bucketLabel: "Every 5 minutes",
  },
  parameters: {
    viewport: { defaultViewport: "responsive" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const card = await canvas.findByTestId("parallel-work-card-minute7d");
    const chart = card.querySelector('[data-chart-kind="parallel-work-gantt"]');
    await expect(chart).toBeInTheDocument();
  },
};
