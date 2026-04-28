import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fireEvent, within } from "storybook/test";
import { I18nProvider } from "../i18n";
import type {
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

const minuteWindow = buildWindow({
  rangeStart: "2026-03-07T00:00:00Z",
  rangeEnd: "2026-03-08T00:00:00Z",
  bucketSeconds: 60,
  completeBucketCount: 1_440,
  activeBucketCount: 318,
  minCount: 0,
  maxCount: 18,
  avgCount: 4.67,
  points: Array.from({ length: 96 }, (_, index) => ({
    bucketStart: new Date(Date.UTC(2026, 2, 7, 10, index)).toISOString(),
    bucketEnd: new Date(Date.UTC(2026, 2, 7, 10, index + 1)).toISOString(),
    parallelCount: Math.max(
      0,
      Math.round(
        6 +
          Math.sin(index / 5) * 4 +
          Math.cos(index / 11) * 3 +
          (index > 44 && index < 60 ? 5 : 0),
      ),
    ),
  })),
  conversations: [
    {
      conversationId: "conv-alpha",
      start: "2026-03-07T00:35:00Z",
      end: "2026-03-07T05:24:00Z",
      requestCount: 28,
    },
    {
      conversationId: "conv-bravo",
      start: "2026-03-07T02:20:00Z",
      end: "2026-03-07T09:12:00Z",
      requestCount: 41,
    },
    {
      conversationId: "conv-charlie",
      start: "2026-03-07T06:00:00Z",
      end: "2026-03-07T12:35:00Z",
      requestCount: 36,
    },
    {
      conversationId: "conv-delta",
      start: "2026-03-07T10:15:00Z",
      end: "2026-03-07T14:48:00Z",
      requestCount: 19,
    },
    {
      conversationId: "conv-echo",
      start: "2026-03-07T13:40:00Z",
      end: "2026-03-07T22:10:00Z",
      requestCount: 52,
    },
    {
      conversationId: "conv-foxtrot",
      start: "2026-03-07T18:05:00Z",
      end: "2026-03-07T23:20:00Z",
      requestCount: 24,
    },
  ],
});

const hourWindow = buildWindow({
  rangeStart: "2026-02-06T00:00:00Z",
  rangeEnd: "2026-03-08T00:00:00Z",
  bucketSeconds: 3600,
  completeBucketCount: 720,
  activeBucketCount: 321,
  minCount: 0,
  maxCount: 9,
  avgCount: 2.13,
  points: Array.from({ length: 48 }, (_, index) => ({
    bucketStart: new Date(Date.UTC(2026, 2, 7, index)).toISOString(),
    bucketEnd: new Date(Date.UTC(2026, 2, 7, index + 1)).toISOString(),
    parallelCount: Math.max(
      0,
      Math.round(2 + Math.sin(index / 4) * 2 + Math.cos(index / 7) * 2),
    ),
  })),
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
