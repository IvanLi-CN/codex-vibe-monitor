import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fireEvent, within } from "storybook/test";
import { I18nProvider } from "../i18n";
import type {
  ParallelWorkConversation,
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
} from "../lib/api";
import { ParallelWorkStatsSection } from "./ParallelWorkStatsSection";

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
    conversations: overrides.conversations ?? [],
  };
}

function isoAtUtcMinute(hour: number, minute: number) {
  return new Date(Date.UTC(2026, 2, 7, hour, minute)).toISOString();
}

const realisticShortConversations: ParallelWorkConversation[] = [
  {
    conversationId: "pck-research-import-a13f",
    start: isoAtUtcMinute(0, 18),
    end: isoAtUtcMinute(1, 6),
    requestCount: 5,
  },
  {
    conversationId: "pck-dashboard-debug-72be",
    start: isoAtUtcMinute(1, 42),
    end: isoAtUtcMinute(4, 58),
    requestCount: 23,
  },
  {
    conversationId: "pck-ci-investigation-24d0",
    start: isoAtUtcMinute(2, 15),
    end: isoAtUtcMinute(3, 2),
    requestCount: 8,
  },
  {
    conversationId: "pck-pr-review-loop-09ac",
    start: isoAtUtcMinute(5, 36),
    end: isoAtUtcMinute(7, 14),
    requestCount: 14,
  },
  {
    conversationId: "pck-usage-rollup-fix-c91b",
    start: isoAtUtcMinute(7, 50),
    end: isoAtUtcMinute(12, 26),
    requestCount: 38,
  },
  {
    conversationId: "pck-frontend-polish-518e",
    start: isoAtUtcMinute(9, 8),
    end: isoAtUtcMinute(10, 40),
    requestCount: 12,
  },
  {
    conversationId: "pck-screenshot-evidence-d0af",
    start: isoAtUtcMinute(10, 12),
    end: isoAtUtcMinute(12, 2),
    requestCount: 17,
  },
  {
    conversationId: "pck-owner-followup-6b33",
    start: isoAtUtcMinute(13, 25),
    end: isoAtUtcMinute(14, 10),
    requestCount: 4,
  },
  {
    conversationId: "pck-long-build-watch-44ef",
    start: isoAtUtcMinute(14, 45),
    end: isoAtUtcMinute(20, 36),
    requestCount: 31,
  },
  {
    conversationId: "pck-api-contract-8f71",
    start: isoAtUtcMinute(17, 20),
    end: isoAtUtcMinute(19, 5),
    requestCount: 11,
  },
  {
    conversationId: "pck-late-hotfix-3c59",
    start: isoAtUtcMinute(21, 42),
    end: isoAtUtcMinute(23, 18),
    requestCount: 9,
  },
];

function buildMinuteWindowFromConversations(
  conversations: ParallelWorkConversation[],
): ParallelWorkWindowResponse {
  const rangeStart = "2026-03-07T00:00:00Z";
  const rangeEnd = "2026-03-08T00:00:00Z";
  const startMs = Date.parse(rangeStart);
  const bucketMs = 15 * 60 * 1000;
  const points = Array.from({ length: 96 }, (_, index) => {
    const bucketStartMs = startMs + index * bucketMs;
    const bucketEndMs = bucketStartMs + bucketMs;
    const parallelCount = conversations.filter((conversation) => {
      const conversationStartMs = Date.parse(conversation.start);
      const conversationEndMs = Date.parse(conversation.end);
      return conversationStartMs < bucketEndMs && conversationEndMs > bucketStartMs;
    }).length;
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      parallelCount,
    };
  });
  const counts = points.map((point) => point.parallelCount);

  return buildWindow({
    rangeStart,
    rangeEnd,
    bucketSeconds: 900,
    completeBucketCount: points.length,
    activeBucketCount: counts.filter((count) => count > 0).length,
    minCount: Math.min(...counts),
    maxCount: Math.max(...counts),
    avgCount: counts.reduce((sum, value) => sum + value, 0) / counts.length,
    points,
    conversations,
  });
}

const minuteWindow = buildMinuteWindowFromConversations(
  realisticShortConversations,
);

const hourWindow = buildWindow({
  rangeStart: "2026-02-06T00:00:00Z",
  rangeEnd: "2026-03-08T00:00:00Z",
  bucketSeconds: 3600,
  completeBucketCount: 720,
  activeBucketCount: 321,
  minCount: 0,
  maxCount: 9,
  avgCount: 2.13,
  points: [
    {
      bucketStart: "2026-03-06T00:00:00Z",
      bucketEnd: "2026-03-06T01:00:00Z",
      parallelCount: 1,
    },
    {
      bucketStart: "2026-03-06T01:00:00Z",
      bucketEnd: "2026-03-06T02:00:00Z",
      parallelCount: 2,
    },
    {
      bucketStart: "2026-03-06T02:00:00Z",
      bucketEnd: "2026-03-06T03:00:00Z",
      parallelCount: 2,
    },
    {
      bucketStart: "2026-03-06T03:00:00Z",
      bucketEnd: "2026-03-06T04:00:00Z",
      parallelCount: 4,
    },
    {
      bucketStart: "2026-03-06T04:00:00Z",
      bucketEnd: "2026-03-06T05:00:00Z",
      parallelCount: 5,
    },
    {
      bucketStart: "2026-03-06T05:00:00Z",
      bucketEnd: "2026-03-06T06:00:00Z",
      parallelCount: 3,
    },
    ...Array.from({ length: 42 }, (_, index) => {
      const hour = index + 6;
      const businessHour = hour % 24 >= 9 && hour % 24 <= 19;
      const eveningTail = hour % 24 >= 20 && hour % 24 <= 22;
      return {
        bucketStart: new Date(Date.UTC(2026, 2, 6, hour)).toISOString(),
        bucketEnd: new Date(Date.UTC(2026, 2, 6, hour + 1)).toISOString(),
        parallelCount: businessHour ? 3 + ((index + 1) % 4) : eveningTail ? 2 : index % 5 === 0 ? 1 : 0,
      };
    }),
  ],
});

const dayWindow = buildWindow({
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
});

const populatedStats: ParallelWorkStatsResponse = {
  current: minuteWindow,
  minute7d: minuteWindow,
  hour30d: hourWindow,
  dayAll: dayWindow,
};

const emptyDayWindow = buildWindow({
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

const hourCurrentStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  current: hourWindow,
};

const emptyCurrentStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  current: emptyDayWindow,
  dayAll: emptyDayWindow,
};

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
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByTestId("parallel-work-card-current"),
    ).toBeInTheDocument();
    await expect(canvas.queryByTestId("parallel-work-window-toggle")).toBeNull();
    const gantt = await canvas.findByTestId("parallel-work-conversation-gantt");
    const bar = gantt.querySelector('[data-testid="parallel-work-conversation-bar"]');
    if (!(bar instanceof HTMLElement)) {
      throw new Error("missing parallel-work conversation bar");
    }
    const rect = bar.getBoundingClientRect();
    fireEvent.mouseMove(bar, {
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + rect.height / 2,
    });
    await expect(
      within(document.body).getByRole("tooltip"),
    ).toBeInTheDocument();
    await expect(
      within(document.body).getByText(/Parallel work/i),
    ).toBeInTheDocument();
  },
};

export const WideMinuteCurrent: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
};

export const CurrentHourRange: Story = {
  args: {
    stats: hourCurrentStats,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
};

export const CurrentDayEmpty: Story = {
  args: {
    stats: emptyCurrentStats,
    isLoading: false,
    error: null,
  },
};

export const Loading: Story = {
  args: {
    stats: null,
    isLoading: true,
    error: null,
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
        stats={hourCurrentStats}
        isLoading={false}
        error={null}
      />
      <ParallelWorkStatsSection
        stats={emptyCurrentStats}
        isLoading={false}
        error={null}
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
