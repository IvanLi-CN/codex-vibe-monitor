import { useMemo, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { useTranslation, I18nProvider } from "../i18n";
import { AppIcon } from "./AppIcon";
import {
  PromptCacheConversationTable,
} from "./PromptCacheConversationTable";
import { Button } from "./ui/button";
import { SelectField } from "./ui/select-field";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationSelection,
  PromptCacheConversationsResponse,
} from "../lib/api";

type StoryPromptCacheConversationPreview =
  PromptCacheConversationInvocationPreview &
    Partial<
      Pick<
        ApiInvocation,
        | "source"
        | "inputTokens"
        | "outputTokens"
        | "cacheInputTokens"
        | "reasoningTokens"
        | "reasoningEffort"
        | "errorMessage"
        | "failureKind"
        | "isActionable"
        | "responseContentEncoding"
        | "requestedServiceTier"
        | "serviceTier"
        | "tReqReadMs"
        | "tReqParseMs"
        | "tUpstreamConnectMs"
        | "tUpstreamTtfbMs"
        | "tUpstreamStreamMs"
        | "tRespParseMs"
        | "tPersistMs"
        | "tTotalMs"
      >
    >;

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

function buildUpstreamAccounts(seed: number, totalTokens: number, lastActivityAt: string) {
  const primaryTokens = Math.max(200, Math.round(totalTokens * 0.46));
  const secondaryTokens = Math.max(120, Math.round(totalTokens * 0.31));
  const tertiaryTokens = Math.max(80, Math.round(totalTokens * 0.17));
  const hiddenTokens = Math.max(40, totalTokens - primaryTokens - secondaryTokens - tertiaryTokens);
  const accountSet = [
    `growth.${(seed % 7) + 2}vv${seed % 10}@relay.example`,
    `backup.f${(seed % 9) + 1}x${(seed % 8) + 2}@ops.example`,
    `audit.q${(seed % 8) + 1}k${(seed % 7) + 3}@ops.example`,
    `burst.f${(seed % 7) + 2}m${(seed % 6) + 4}@watch.example`,
  ];

  return [
    {
      upstreamAccountId: 100 + seed,
      upstreamAccountName: accountSet[0],
      requestCount: 8 + (seed % 5),
      totalTokens: primaryTokens,
      totalCost: Number((primaryTokens / 42000).toFixed(4)),
      lastActivityAt,
    },
    {
      upstreamAccountId: 200 + seed,
      upstreamAccountName: accountSet[1],
      requestCount: 6 + (seed % 4),
      totalTokens: secondaryTokens,
      totalCost: Number((secondaryTokens / 42000).toFixed(4)),
      lastActivityAt: isoAt(0.8 + (seed % 3) * 0.2, 0),
    },
    {
      upstreamAccountId: 300 + seed,
      upstreamAccountName: accountSet[2],
      requestCount: 4 + (seed % 3),
      totalTokens: tertiaryTokens,
      totalCost: Number((tertiaryTokens / 42000).toFixed(4)),
      lastActivityAt: isoAt(1.3 + (seed % 4) * 0.2, 0),
    },
    {
      upstreamAccountId: 400 + seed,
      upstreamAccountName: accountSet[3],
      requestCount: 1,
      totalTokens: hiddenTokens,
      totalCost: Number((hiddenTokens / 42000).toFixed(4)),
      lastActivityAt: isoAt(3.8 + (seed % 4) * 0.3, 0),
    },
  ];
}

function buildRecentInvocations(
  seed: number,
  lastActivityAt: string,
  totalTokens: number,
) : StoryPromptCacheConversationPreview[] {
  return Array.from({ length: 3 }, (_, index) => {
    const offsetMinutes = index * 18;
    const occurredAt = new Date(
      Date.parse(lastActivityAt) - offsetMinutes * 60_000,
    ).toISOString();
    const tokens = Math.max(120, Math.round(totalTokens / (6 + index)));
    const inputTokens = Math.max(80, tokens - (640 + index * 40));
    const outputTokens = Math.max(32, tokens - inputTokens);
    const cacheInputTokens = Math.max(0, inputTokens - (560 + index * 24));
    const isFailure = index === 1 && seed % 4 === 2;
    const requestedPriority = index !== 1;

    return {
      id: seed * 100 + index + 1,
      invokeId: `live-section-${seed + 1}-${index + 1}`,
      occurredAt,
      source: "pool",
      status: isFailure ? "http_502" : "completed",
      failureClass: isFailure ? "service_failure" : "none",
      routeMode: "pool",
      model: index === 2 ? "gpt-5.4-mini" : "gpt-5.4",
      inputTokens,
      outputTokens,
      cacheInputTokens,
      reasoningTokens: isFailure ? undefined : 160 + index * 48,
      reasoningEffort: isFailure ? undefined : index === 0 ? "high" : "medium",
      totalTokens: tokens,
      cost: Number((tokens / 42000).toFixed(4)),
      errorMessage: isFailure
        ? "upstream gateway closed before first byte"
        : undefined,
      failureKind: isFailure ? "upstream_timeout" : undefined,
      isActionable: isFailure ? true : undefined,
      proxyDisplayName: ["tokyo-edge-01", "osaka-edge-02", "singapore-edge-03"][
        (seed + index) % 3
      ],
      upstreamAccountId: 100 + seed,
      upstreamAccountName: `growth.${(seed % 7) + 2}vv${seed % 10}@relay.example`,
      endpoint: index === 1 ? "/v1/chat/completions" : "/v1/responses",
      responseContentEncoding: isFailure ? "identity" : index === 0 ? "gzip, br" : "gzip",
      requestedServiceTier: requestedPriority ? "priority" : "auto",
      serviceTier: isFailure ? "auto" : "priority",
      tReqReadMs: 18 + index * 2,
      tReqParseMs: 5 + index,
      tUpstreamConnectMs: isFailure ? 1200 : 520 + index * 44,
      tUpstreamTtfbMs: isFailure ? null : 120 + index * 12,
      tUpstreamStreamMs: isFailure ? null : 620 + index * 56,
      tRespParseMs: isFailure ? undefined : 10 + index,
      tPersistMs: isFailure ? undefined : 8 + index,
      tTotalMs: isFailure ? 30020 : 1320 + index * 96,
    };
  });
}

function buildPromptCacheKey(seed: number, variant: "count" | "window") {
  const prefix = variant === "count" ? "019d2c" : "019d2d";
  const groupTwo = (0x8100 + seed).toString(16).padStart(4, "0");
  const groupThree = (0x7100 + ((seed * 13) % 256)).toString(16).padStart(4, "0");
  const groupFour = (0xb100 + ((seed * 17) % 256)).toString(16).padStart(4, "0");
  const tail = `${(seed * 97 + 0x9f0000).toString(16).slice(-6)}a${(
    seed * 29 + 0x4f000
  )
    .toString(16)
    .slice(-5)}`;
  return `${prefix}-${groupTwo}-${groupThree}-${groupFour}-${tail}`;
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
        promptCacheKey: buildPromptCacheKey(seed + (variant === "count" ? 20 : 40), variant),
    requestCount: pointCount * 3 + 4 + seed,
    totalTokens,
    totalCost: Number((totalTokens / 42000).toFixed(4)),
    createdAt: isoAt(createdHoursAgo, 0),
    lastActivityAt: lastPoint?.occurredAt ?? isoAt(lastActivityHoursAgo, 0),
    upstreamAccounts: buildUpstreamAccounts(
      seed,
      totalTokens,
      lastPoint?.occurredAt ?? isoAt(lastActivityHoursAgo, 0),
    ),
    recentInvocations: buildRecentInvocations(
      seed,
      lastPoint?.occurredAt ?? isoAt(lastActivityHoursAgo, 0),
      totalTokens,
    ),
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
        promptCacheKey: buildPromptCacheKey(index + 7, "window"),
        requestCount: pointCount * 4 + 3 + index,
        totalTokens,
        totalCost: Number((totalTokens / 42000).toFixed(4)),
        createdAt: isoAt(createdAtHours, 0),
        lastActivityAt: withinWindowPoints.at(-1)?.occurredAt ?? isoAt(0.2, 0),
        upstreamAccounts: buildUpstreamAccounts(
          index + 7,
          totalTokens,
          withinWindowPoints.at(-1)?.occurredAt ?? isoAt(0.2, 0),
        ),
        recentInvocations: buildRecentInvocations(
          index + 7,
          withinWindowPoints.at(-1)?.occurredAt ?? isoAt(0.2, 0),
          totalTokens,
        ),
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

  if ("activityMinutes" in selection) {
    return buildActivityWindowStats(Math.max(1, selection.activityMinutes / 60));
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
  const [expandedPromptCacheKeys, setExpandedPromptCacheKeys] = useState<
    string[]
  >([]);

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
  const visiblePromptCacheKeys = useMemo(
    () => stats.conversations.map((conversation) => conversation.promptCacheKey),
    [stats],
  );
  const allExpanded =
    visiblePromptCacheKeys.length > 0 &&
    visiblePromptCacheKeys.every((promptCacheKey) =>
      expandedPromptCacheKeys.includes(promptCacheKey),
    );

  const toggleAllVisible = () => {
    setExpandedPromptCacheKeys((current) => {
      if (
        visiblePromptCacheKeys.length > 0 &&
        visiblePromptCacheKeys.every((promptCacheKey) =>
          current.includes(promptCacheKey),
        )
      ) {
        return current.filter(
          (promptCacheKey) => !visiblePromptCacheKeys.includes(promptCacheKey),
        );
      }
      return visiblePromptCacheKeys;
    });
  };

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
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="gap-2"
              data-testid="live-prompt-cache-expand-all"
              onClick={toggleAllVisible}
            >
              <AppIcon
                name={allExpanded ? "chevron-up" : "chevron-down"}
                className="h-4 w-4"
                data-testid="live-prompt-cache-expand-all-icon"
                data-icon-name={allExpanded ? "chevron-up" : "chevron-down"}
                aria-hidden
              />
              {allExpanded
                ? t("live.conversations.actions.collapseAllRecords")
                : t("live.conversations.actions.expandAllRecords")}
            </Button>
            <SelectField
              className="w-40"
              label={t("live.conversations.selectionLabel")}
              name="livePromptCacheSelection"
              size="sm"
              data-testid="live-prompt-cache-selection"
              value={selectionValue}
              onValueChange={setSelectionValue}
              options={SELECTION_OPTIONS.map((option) => ({
                value: option.value,
                label:
                  option.kind === "count"
                    ? t("live.conversations.option.count", {
                        count: option.count,
                      })
                    : t("live.conversations.option.activityHours", {
                        hours: option.hours,
                      }),
              }))}
            />
          </div>
        </div>
        <PromptCacheConversationTable
          stats={stats}
          isLoading={false}
          error={null}
          expandedPromptCacheKeys={expandedPromptCacheKeys}
          onToggleExpandedPromptCacheKey={(promptCacheKey) => {
            setExpandedPromptCacheKeys((current) =>
              current.includes(promptCacheKey)
                ? current.filter((value) => value !== promptCacheKey)
                : [...current, promptCacheKey],
            );
          }}
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
    const trigger = canvas.getByTestId("live-prompt-cache-selection");
    const expandAllButton = canvas.getByTestId("live-prompt-cache-expand-all");
    const expandAllIcon = canvas.getByTestId("live-prompt-cache-expand-all-icon");
    const documentScope = within(canvasElement.ownerDocument.body);

    await expect(canvas.getByRole("heading", { name: "对话" })).toBeInTheDocument();
    await expect(expandAllIcon).toHaveAttribute("data-icon-name", "chevron-down");

    await userEvent.click(trigger);
    await userEvent.click(
      await documentScope.findByRole("option", { name: /近 3 小时活动/i }),
    );
    await expect(trigger.textContent ?? "").toContain("近 3 小时活动");
    await expect(
      canvas.getByText(/有 7 个对话命中活动时间窗/i),
    ).toBeInTheDocument();

    await userEvent.click(trigger);
    await userEvent.click(
      await documentScope.findByRole("option", { name: /20 个对话/i }),
    );
    await expect(trigger.textContent ?? "").toContain("20 个对话");
    await expect(
      canvas.getByText(/有 25 个更新创建的对话因未在近 24 小时活动而未显示/i),
    ).toBeInTheDocument();

    await userEvent.click(expandAllButton);
    await expect(expandAllIcon).toHaveAttribute("data-icon-name", "chevron-up");
    await expect(canvas.getAllByTestId("invocation-table-scroll").length).toBeGreaterThan(0);
    await expect(canvas.queryByText(/输入 \/ 缓存/i)).not.toBeInTheDocument();
    await expect(canvas.getAllByText(/总时延/i).length).toBeGreaterThan(0);
  },
};
