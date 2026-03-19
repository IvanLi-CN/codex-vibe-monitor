import { useMemo, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { useTranslation, I18nProvider } from "../i18n";
import {
  PromptCacheConversationTable,
} from "./PromptCacheConversationTable";
import type {
  PromptCacheConversation,
  PromptCacheConversationSelection,
  PromptCacheConversationsResponse,
} from "../lib/api";

type PromptCacheSelectionOption =
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      count: number;
      kind: "count";
    }
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      hours: number;
      kind: "activityWindow";
    };

const SELECTION_OPTIONS: PromptCacheSelectionOption[] = [
  {
    value: "count:20",
    selection: { mode: "count", limit: 20 },
    count: 20,
    kind: "count",
  },
  {
    value: "count:50",
    selection: { mode: "count", limit: 50 },
    count: 50,
    kind: "count",
  },
  {
    value: "count:100",
    selection: { mode: "count", limit: 100 },
    count: 100,
    kind: "count",
  },
  {
    value: "activityWindow:1",
    selection: { mode: "activityWindow", activityHours: 1 },
    hours: 1,
    kind: "activityWindow",
  },
  {
    value: "activityWindow:3",
    selection: { mode: "activityWindow", activityHours: 3 },
    hours: 3,
    kind: "activityWindow",
  },
  {
    value: "activityWindow:6",
    selection: { mode: "activityWindow", activityHours: 6 },
    hours: 6,
    kind: "activityWindow",
  },
  {
    value: "activityWindow:12",
    selection: { mode: "activityWindow", activityHours: 12 },
    hours: 12,
    kind: "activityWindow",
  },
  {
    value: "activityWindow:24",
    selection: { mode: "activityWindow", activityHours: 24 },
    hours: 24,
    kind: "activityWindow",
  },
];

function isoAt(hoursAgo: number, minutesAgo = 0) {
  return new Date(
    Date.now() - hoursAgo * 3_600_000 - minutesAgo * 60_000,
  ).toISOString();
}

function applyCumulativeTokens<
  TPoint extends { requestTokens: number },
>(points: TPoint[]) {
  let cumulative = 0;
  return points.map((point) => {
    cumulative += point.requestTokens;
    return {
      ...point,
      cumulativeTokens: cumulative,
    };
  });
}

function buildTimelinePoints({
  seed,
  pointCount,
  createdHoursAgo,
  lastActivityHoursAgo,
}: {
  seed: number;
  pointCount: number;
  createdHoursAgo: number;
  lastActivityHoursAgo: number;
}) {
  const span = Math.max(createdHoursAgo - lastActivityHoursAgo, 0.05);

  return applyCumulativeTokens(
    Array.from({ length: pointCount }, (_, index) => {
      const ratio = pointCount === 1 ? 1 : index / (pointCount - 1);
      const shaped = Math.pow(ratio, 0.82);
      const hoursAgo = Math.max(
        lastActivityHoursAgo,
        createdHoursAgo - span * shaped,
      );
      const requestTokens = 520 + seed * 82 + index * 210;
      const hasFailure = pointCount >= 4 && index === pointCount - 2 && seed % 5 === 2;
      return {
        occurredAt: isoAt(hoursAgo, 0),
        status: hasFailure ? "upstream_stream_error" : "completed",
        isSuccess: !hasFailure,
        requestTokens,
      };
    }),
  );
}

function buildDenseConversation(seed: number, variant: "count" | "window") {
  const createdHoursAgo =
    variant === "count" ? 18 + seed * 1.35 : 5.2 - seed * 0.32;
  const lastActivityHoursAgo =
    variant === "count"
      ? Math.max(0.15, 0.45 + (seed % 5) * 0.4)
      : Math.max(0.12, 0.35 + (seed % 4) * 0.32);
  const pointCount = 4 + (seed % 3);
  const points = buildTimelinePoints({
    seed,
    pointCount,
    createdHoursAgo,
    lastActivityHoursAgo,
  });
  const lastPoint = points.at(-1);
  const totalTokens =
    points.reduce((sum, point) => sum + point.requestTokens, 0) +
    seed * 1300 +
    (variant === "count" ? 12000 : 4800);

  return {
    promptCacheKey: `${variant === "count" ? "pck-live-count" : "pck-window"}-${String(
      seed + 1,
    ).padStart(2, "0")}-team-${(seed % 4) + 1}`,
    requestCount: pointCount * 3 + 4 + seed,
    totalTokens,
    totalCost: Number((totalTokens / 42000).toFixed(4)),
    createdAt: isoAt(createdHoursAgo, 0),
    lastActivityAt: lastPoint?.occurredAt ?? isoAt(lastActivityHoursAgo, 0),
    last24hRequests: points,
  } satisfies PromptCacheConversation;
}

function sortConversationsByCreatedAtDesc(
  conversations: PromptCacheConversation[],
) {
  return [...conversations].sort((left, right) => {
    const createdAtDelta =
      Date.parse(right.createdAt) - Date.parse(left.createdAt);
    if (createdAtDelta !== 0) return createdAtDelta;
    return right.promptCacheKey.localeCompare(left.promptCacheKey);
  });
}

const COUNT_MODE_STATS: PromptCacheConversationsResponse = {
  rangeStart: isoAt(24, 0),
  rangeEnd: isoAt(0, 0),
  selectionMode: "count",
  selectedLimit: 20,
  selectedActivityHours: null,
  implicitFilter: {
    kind: "inactiveOutside24h",
    filteredCount: 25,
  },
  conversations: sortConversationsByCreatedAtDesc(
    Array.from({ length: 16 }, (_, index) =>
      buildDenseConversation(index, "count"),
    ),
  ),
};

function buildActivityWindowStats(hours: number): PromptCacheConversationsResponse {
  const spanHours = Math.max(hours, 1);
  const rangeEnd = new Date().toISOString();
  const rangeStart = new Date(Date.now() - spanHours * 3_600_000).toISOString();
  const conversationCount =
    hours >= 24 ? 18 : hours >= 12 ? 14 : hours >= 6 ? 12 : hours >= 3 ? 10 : 8;
  const conversations: PromptCacheConversation[] = Array.from(
    { length: conversationCount },
    (_, index) => {
      const createdAtHours = Math.max(
        hours <= 1
          ? 1.55 - index * 0.07
          : hours <= 3
            ? 4.9 - index * 0.25
            : hours <= 6
              ? 8.2 - index * 0.42
              : hours <= 12
                ? 13.5 - index * 0.6
                : 28 - index * 0.95,
        Math.max(hours * 0.55, 0.9),
      );
      const lastActivityHoursAgo = Math.max(
        0.08,
        Math.min(hours - 0.08, 0.25 + (index % 5) * (hours / 18)),
      );
      const pointCount = 4 + ((index + 1) % 3);
      const withinWindowPoints = buildTimelinePoints({
        seed: index + 7,
        pointCount,
        createdHoursAgo: createdAtHours,
        lastActivityHoursAgo,
      });
      const totalTokens =
        withinWindowPoints.reduce((sum, point) => sum + point.requestTokens, 0) +
        (index + 4) * 1450;

      return {
        promptCacheKey: `pck-window-${String(index + 4).padStart(2, "0")}-team-${
          (index % 4) + 1
        }`,
        requestCount: pointCount * 4 + 3 + index,
        totalTokens,
        totalCost: Number((totalTokens / 42000).toFixed(4)),
        createdAt: isoAt(createdAtHours, 0),
        lastActivityAt: withinWindowPoints.at(-1)?.occurredAt ?? isoAt(0.2, 0),
        last24hRequests: withinWindowPoints,
      };
    },
  );

  return {
    rangeStart,
    rangeEnd,
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: hours,
    implicitFilter:
      hours === 3
        ? { kind: "cappedTo50", filteredCount: 7 }
        : { kind: null, filteredCount: 0 },
    conversations: sortConversationsByCreatedAtDesc(conversations),
  };
}

function resolveStats(selection: PromptCacheConversationSelection) {
  if (selection.mode === "count") {
    return {
      ...COUNT_MODE_STATS,
      selectedLimit: selection.limit,
    };
  }

  return buildActivityWindowStats(selection.activityHours);
}

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <main className="mx-auto w-full max-w-[1200px]">{children}</main>
    </div>
  );
}

function LivePromptCacheSectionStory() {
  const { t } = useTranslation();
  const [selectionValue, setSelectionValue] = useState("activityWindow:24");

  const activeOption = useMemo(
    () =>
      SELECTION_OPTIONS.find((option) => option.value === selectionValue) ??
      SELECTION_OPTIONS[0],
    [selectionValue],
  );
  const stats = useMemo(
    () => resolveStats(activeOption.selection),
    [activeOption.selection],
  );

  return (
    <section className="surface-panel">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t("live.conversations.title")}</h2>
            <p className="section-description">
              {t("live.conversations.description")}
            </p>
          </div>
          <label className="field w-40">
            <span className="field-label">
              {t("live.conversations.selectionLabel")}
            </span>
            <select
              data-testid="live-prompt-cache-selection"
              className="field-select field-select-sm"
              value={selectionValue}
              onChange={(event) => setSelectionValue(event.target.value)}
            >
              {SELECTION_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.kind === "count"
                    ? t("live.conversations.option.count", {
                        count: option.count,
                      })
                    : t("live.conversations.option.activityHours", {
                        hours: option.hours,
                      })}
                </option>
              ))}
            </select>
          </label>
        </div>
        <PromptCacheConversationTable
          stats={stats}
          isLoading={false}
          error={null}
        />
      </div>
    </section>
  );
}

const meta = {
  title: "Monitoring/Live Prompt Cache Section",
  component: LivePromptCacheSectionStory,
  tags: ["autodocs"],
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof LivePromptCacheSectionStory>;

export default meta;

type Story = StoryObj<typeof meta>;

export const InteractiveFilters: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const select = canvas.getByTestId(
      "live-prompt-cache-selection",
    ) as HTMLSelectElement;

    await userEvent.selectOptions(select, "activityWindow:3");
    await expect(select.value).toBe("activityWindow:3");
    await expect(
      canvas.getByText(/有 7 个对话命中活动时间窗/i),
    ).toBeInTheDocument();

    await userEvent.selectOptions(select, "count:20");
    await expect(select.value).toBe("count:20");
    await expect(
      canvas.getByText(/有 25 个更新创建的对话因未在近 24 小时活动而未显示/i),
    ).toBeInTheDocument();
  },
};
