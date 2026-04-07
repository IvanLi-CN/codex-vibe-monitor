import { useMemo, useState, type ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { mapPromptCacheConversationsToDashboardCards } from "../lib/dashboardWorkingConversations";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <div className="app-shell-boundary">{children}</div>
    </div>
  );
}

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    source: overrides.source ?? "pool",
    inputTokens: overrides.inputTokens ?? 148,
    outputTokens: overrides.outputTokens ?? 92,
    cacheInputTokens: overrides.cacheInputTokens ?? 36,
    reasoningTokens: overrides.reasoningTokens ?? 24,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 14,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 136,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 98,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 324,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 601,
  };
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: overrides.requestCount ?? recentInvocations.length,
    totalTokens: overrides.totalTokens ?? recentInvocations.reduce(
      (sum, preview) => sum + Math.max(0, preview.totalTokens),
      0,
    ),
    totalCost: overrides.totalCost ?? Number(
      recentInvocations
        .reduce((sum, preview) => sum + (preview.cost ?? 0), 0)
        .toFixed(4),
    ),
    createdAt: overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt: overrides.lastActivityAt ??
      recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations,
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

function createResponse(
  conversations: PromptCacheConversation[],
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
  };
}

const currentAndPreviousResponse = createResponse([
  createConversation("pck-current-previous", [
    createPreview({
      id: 12,
      invokeId: "invoke-12",
      occurredAt: "2026-04-04T10:04:20Z",
      status: "completed",
      upstreamAccountName: "growth-alpha@example.com",
    }),
    createPreview({
      id: 11,
      invokeId: "invoke-11",
      occurredAt: "2026-04-04T10:01:12Z",
      status: "completed",
      model: "gpt-5.4-mini",
      upstreamAccountName: "backup-alpha@example.com",
      requestedServiceTier: "auto",
      serviceTier: "auto",
    }),
  ]),
]);

const currentOnlyResponse = createResponse([
  createConversation("pck-placeholder-only", [
    createPreview({
      id: 21,
      invokeId: "invoke-21",
      occurredAt: "2026-04-04T10:04:42Z",
      status: "completed",
      upstreamAccountName: "warmup-alpha@example.com",
    }),
  ]),
]);

const runningOnlyResponse = createResponse([
  createConversation("pck-running-only", [
    createPreview({
      id: 31,
      invokeId: "invoke-31",
      occurredAt: "2026-04-04T10:04:58Z",
      status: "running",
      upstreamAccountName: "watch-alpha@example.com",
      tTotalMs: null,
    }),
    createPreview({
      id: 30,
      invokeId: "invoke-30",
      occurredAt: "2026-04-04T09:54:20Z",
      status: "completed",
      upstreamAccountName: "watch-alpha@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const failedClickableResponse = createResponse([
  createConversation("pck-failed-clickable", [
    createPreview({
      id: 41,
      invokeId: "invoke-41",
      occurredAt: "2026-04-04T10:03:40Z",
      status: "http_502",
      failureClass: "service_failure",
      errorMessage: "upstream gateway closed before first byte",
      failureKind: "upstream_timeout",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/chat/completions",
      requestedServiceTier: "auto",
      serviceTier: "auto",
      responseContentEncoding: "identity",
    }),
    createPreview({
      id: 40,
      invokeId: "invoke-40",
      occurredAt: "2026-04-04T10:02:10Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const createdAtDescendingOrderResponse = createResponse([
  createConversation(
    "pck-created-middle",
    [
      createPreview({
        id: 52,
        invokeId: "invoke-created-middle-running",
        occurredAt: "2026-04-04T10:04:58Z",
        status: "running",
        upstreamAccountName: "ordering-middle@example.com",
        tTotalMs: null,
      }),
      createPreview({
        id: 51,
        invokeId: "invoke-created-middle-previous",
        occurredAt: "2026-04-04T10:03:40Z",
        status: "completed",
        upstreamAccountName: "ordering-middle@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:02:00Z",
    },
  ),
  createConversation(
    "pck-created-oldest",
    [
      createPreview({
        id: 61,
        invokeId: "invoke-created-oldest",
        occurredAt: "2026-04-04T10:03:20Z",
        status: "completed",
        upstreamAccountName: "ordering-oldest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T09:58:00Z",
    },
  ),
  createConversation(
    "pck-created-newest",
    [
      createPreview({
        id: 71,
        invokeId: "invoke-created-newest",
        occurredAt: "2026-04-04T10:01:00Z",
        status: "completed",
        upstreamAccountName: "ordering-newest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:03:00Z",
    },
  ),
]);

function buildCards(response: PromptCacheConversationsResponse) {
  return mapPromptCacheConversationsToDashboardCards(response);
}

function InteractiveStory({
  response,
}: {
  response: PromptCacheConversationsResponse;
}) {
  const cards = useMemo(() => buildCards(response), [response]);
  const [lastAccount, setLastAccount] = useState<string>("none");

  return (
    <div className="space-y-4">
      <DashboardWorkingConversationsSection
        cards={cards}
        isLoading={false}
        error={null}
        onOpenUpstreamAccount={(accountId, accountLabel) => {
          setLastAccount(`${accountId}:${accountLabel}`);
        }}
      />
      <div className="rounded-xl border border-base-300/75 bg-base-100/70 px-4 py-3 text-sm text-base-content/75">
        <span className="font-semibold">Last clicked account:</span>{" "}
        <span data-testid="story-click-log" className="font-mono">
          {lastAccount}
        </span>
      </div>
    </div>
  );
}

const meta = {
  title: "Dashboard/WorkingConversationsSection",
  component: DashboardWorkingConversationsSection,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardWorkingConversationsSection>;

export default meta;

type Story = StoryObj<typeof meta>;

export const CurrentAndPrevious: Story = {
  args: {
    cards: buildCards(currentAndPreviousResponse),
    isLoading: false,
    error: null,
  },
};

export const CurrentOnlyPlaceholder: Story = {
  args: {
    cards: buildCards(currentOnlyResponse),
    isLoading: false,
    error: null,
  },
};

export const RunningOnlyConversation: Story = {
  args: {
    cards: buildCards(runningOnlyResponse),
    isLoading: false,
    error: null,
  },
};

export const FailedWithClickableAccount: Story = {
  args: {
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <InteractiveStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountButtons = await canvas.findAllByRole("button", {
      name: /pool-account-77@example.com/i,
    });
    const accountButton = accountButtons[0];
    await userEvent.click(accountButton);
    await expect(canvas.getByTestId("story-click-log")).toHaveTextContent(
      "77:pool-account-77@example.com",
    );
  },
};

export const StateGallery: Story = {
  args: {
    cards: buildCards(
      createResponse([
        ...currentAndPreviousResponse.conversations,
        ...currentOnlyResponse.conversations,
        ...runningOnlyResponse.conversations,
        ...failedClickableResponse.conversations,
      ]),
    ),
    isLoading: false,
    error: null,
  },
};

export const CreatedAtDescendingOrder: Story = {
  args: {
    cards: buildCards(createdAtDescendingOrderResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const cards = await canvas.findAllByTestId("dashboard-working-conversation-card");
    await expect(cards[0]).toHaveTextContent("pck-created-newest");
    await expect(cards[1]).toHaveTextContent("pck-created-middle");
    await expect(cards[2]).toHaveTextContent("pck-created-oldest");
  },
};
