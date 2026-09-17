import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  type ComponentProps,
  type Dispatch,
  type SetStateAction,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider, useTranslation } from "../../i18n";
import type { DashboardWorkingConversationInvocationSelection } from "../../lib/dashboardWorkingConversations";
import { PromptCacheConversationHistoryDrawer } from "../prompt-cache/PromptCacheConversationTable";
import { DashboardInvocationDetailDrawer } from "./DashboardInvocationDetailDrawer";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import {
  type DrawerPreviewStoryProps,
  formatConversationLabel,
  type SelectedAccount,
  type SelectedConversation,
  useDrawerPreviewEventSource,
  useDrawerPreviewFetch,
  useDrawerPreviewSelection,
} from "./DashboardWorkingConversationsSection.stories.drawer-support";
import {
  contrastRatio,
  createConversation,
  createPreview,
  createResponse,
  DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS,
  ForcedWorkspaceViewStory,
  imageEditInkChroma,
  parseComputedColorChannels,
  StorySurface,
  useStoryTheme,
} from "./DashboardWorkingConversationsSection.stories.support-base";
import {
  buildCards,
  buildStoryMockData,
  StoryAccountDrawer,
  wideDesktopResponse,
} from "./DashboardWorkingConversationsSection.stories.support-extra";
import {
  DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
  type DashboardBulkRouteBindRecentTarget,
} from "./dashboardBulkRouteBindPreferences";

export function DrawerPreviewStory(props: DrawerPreviewStoryProps) {
  const {
    response,
    historyInvocationsByPromptCacheKey,
    theme,
    initialSelection,
    initialConversationKey,
    initialConversationTab = "overview",
    ...contentProps
  } = props;
  useStoryTheme(theme);
  const { t } = useTranslation();
  const cards = useMemo(() => buildCards(response), [response]);
  const storyMocks = useMemo(
    () => buildStoryMockData(response, historyInvocationsByPromptCacheKey),
    [historyInvocationsByPromptCacheKey, response],
  );
  const selection = useDrawerPreviewSelection(
    cards,
    initialSelection,
    initialConversationKey,
    initialConversationTab,
  );
  useDrawerPreviewEventSource();
  useDrawerPreviewFetch(storyMocks, contentProps.upstreamAccountActivity);

  return (
    <DrawerPreviewStoryContent
      {...contentProps}
      {...selection}
      cards={cards}
      t={t}
      initialConversationTab={initialConversationTab}
      conversationPresentation={props.conversationPresentation ?? "overlay"}
    />
  );
}

type DrawerPreviewStoryContentProps = Omit<
  DrawerPreviewStoryProps,
  | "response"
  | "historyInvocationsByPromptCacheKey"
  | "theme"
  | "initialSelection"
  | "initialConversationKey"
> & {
  cards: ReturnType<typeof buildCards>;
  t: ReturnType<typeof useTranslation>["t"];
  initialConversationTab: NonNullable<DrawerPreviewStoryProps["initialConversationTab"]>;
  conversationPresentation: NonNullable<DrawerPreviewStoryProps["conversationPresentation"]>;
  selectedInvocation: DashboardWorkingConversationInvocationSelection | null;
  setSelectedInvocation: Dispatch<
    SetStateAction<DashboardWorkingConversationInvocationSelection | null>
  >;
  selectedConversation: SelectedConversation | null;
  setSelectedConversation: Dispatch<SetStateAction<SelectedConversation | null>>;
  selectedAccount: SelectedAccount | null;
  setSelectedAccount: Dispatch<SetStateAction<SelectedAccount | null>>;
};

function DrawerPreviewStoryContent({
  cards,
  t,
  initialConversationTab,
  conversationPresentation,
  selectedInvocation,
  setSelectedInvocation,
  selectedConversation,
  setSelectedConversation,
  selectedAccount,
  setSelectedAccount,
  ...props
}: DrawerPreviewStoryContentProps) {
  const openAccount = (
    accountId: number,
    accountLabel: string,
    options?: { tab?: SelectedAccount["tab"] },
  ) => {
    setSelectedInvocation(null);
    setSelectedConversation(null);
    setSelectedAccount({
      id: accountId,
      label: accountLabel,
      tab: options?.tab ?? "overview",
    });
  };

  return (
    <>
      <DashboardStoryWorkspace
        {...props}
        cards={cards}
        t={t}
        conversationPresentation={conversationPresentation}
        selectedConversation={selectedConversation}
        setSelectedConversation={setSelectedConversation}
        setSelectedInvocation={setSelectedInvocation}
        setSelectedAccount={setSelectedAccount}
        openAccount={openAccount}
      />
      <DashboardStoryDrawers
        {...props}
        t={t}
        initialConversationTab={initialConversationTab}
        conversationPresentation={conversationPresentation}
        selectedInvocation={selectedInvocation}
        setSelectedInvocation={setSelectedInvocation}
        selectedConversation={selectedConversation}
        setSelectedConversation={setSelectedConversation}
        selectedAccount={selectedAccount}
        setSelectedAccount={setSelectedAccount}
        openAccount={openAccount}
      />
    </>
  );
}

type DashboardStoryWorkspaceProps = Pick<
  DrawerPreviewStoryContentProps,
  | "cards"
  | "t"
  | "conversationPresentation"
  | "selectedConversation"
  | "setSelectedConversation"
  | "setSelectedInvocation"
  | "setSelectedAccount"
  | "recentPreviewLimit"
  | "upstreamAccountActivity"
  | "upstreamAccountActivityLoading"
  | "upstreamAccountActivityRefreshing"
  | "upstreamAccountRecentLoading"
  | "upstreamAccountRecentError"
  | "upstreamAccountRecentPreviewLimit"
> & {
  openAccount: (
    accountId: number,
    accountLabel: string,
    options?: { tab?: SelectedAccount["tab"] },
  ) => void;
};

function DashboardStoryWorkspace({
  cards,
  t,
  conversationPresentation,
  selectedConversation,
  setSelectedConversation,
  setSelectedInvocation,
  setSelectedAccount,
  openAccount,
  recentPreviewLimit,
  upstreamAccountActivity,
  upstreamAccountActivityLoading,
  upstreamAccountActivityRefreshing,
  upstreamAccountRecentLoading,
  upstreamAccountRecentError,
  upstreamAccountRecentPreviewLimit,
}: DashboardStoryWorkspaceProps) {
  if (conversationPresentation === "page" && selectedConversation != null) {
    return (
      <div className="min-h-screen bg-base-200 p-3 text-base-content min-[769px]:p-6">
        <div className="mx-auto w-full max-w-[78rem]">
          <PromptCacheConversationHistoryDrawer
            open
            presentation="page"
            conversationKey={selectedConversation.promptCacheKey}
            conversationLabel={formatConversationLabel(selectedConversation)}
            initialTab={selectedConversation.tab}
            onClose={() => setSelectedConversation(null)}
            t={t}
            onOpenUpstreamAccount={openAccount}
          />
        </div>
      </div>
    );
  }

  return (
    <DashboardWorkingConversationsSection
      activeRange="today"
      recentPreviewLimit={recentPreviewLimit}
      cards={cards}
      isLoading={false}
      error={null}
      upstreamAccountActivity={upstreamAccountActivity}
      upstreamAccountActivityLoading={upstreamAccountActivityLoading}
      upstreamAccountActivityRefreshing={upstreamAccountActivityRefreshing}
      upstreamAccountRecentLoading={upstreamAccountRecentLoading}
      upstreamAccountRecentError={upstreamAccountRecentError}
      upstreamAccountRecentPreviewLimit={upstreamAccountRecentPreviewLimit}
      onRetryUpstreamAccountRecent={() => undefined}
      onOpenUpstreamAccount={openAccount}
      onOpenConversation={(selection) => {
        setSelectedInvocation(null);
        setSelectedAccount(null);
        setSelectedConversation({
          conversationSequenceId: selection.conversationSequenceId,
          promptCacheKey: selection.promptCacheKey,
          tab: selection.tab ?? "overview",
        });
      }}
      onOpenInvocation={(selection) => {
        setSelectedConversation(null);
        setSelectedAccount(null);
        setSelectedInvocation(selection);
      }}
    />
  );
}

function DashboardStoryDrawers({
  t,
  initialConversationTab,
  conversationPresentation,
  selectedInvocation,
  setSelectedInvocation,
  selectedConversation,
  setSelectedConversation,
  selectedAccount,
  setSelectedAccount,
  openAccount,
}: Pick<
  DrawerPreviewStoryContentProps,
  | "t"
  | "initialConversationTab"
  | "conversationPresentation"
  | "selectedInvocation"
  | "setSelectedInvocation"
  | "selectedConversation"
  | "setSelectedConversation"
  | "selectedAccount"
  | "setSelectedAccount"
> & {
  openAccount: (
    accountId: number,
    accountLabel: string,
    options?: { tab?: SelectedAccount["tab"] },
  ) => void;
}) {
  return (
    <>
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={openAccount}
      />
      {conversationPresentation === "page" ? null : (
        <PromptCacheConversationHistoryDrawer
          open={selectedConversation != null}
          presentation={conversationPresentation}
          conversationKey={selectedConversation?.promptCacheKey ?? null}
          conversationLabel={formatConversationLabel(selectedConversation)}
          initialTab={selectedConversation?.tab ?? initialConversationTab}
          onClose={() => setSelectedConversation(null)}
          t={t}
          onOpenUpstreamAccount={openAccount}
        />
      )}
      <StoryAccountDrawer account={selectedAccount} onClose={() => setSelectedAccount(null)} />
      {conversationPresentation === "page" && selectedConversation != null ? null : (
        <div className="rounded-xl border border-base-300/75 bg-base-100/70 px-4 py-3 text-sm text-base-content/75">
          <span className="font-semibold">Drawer state:</span>{" "}
          <span data-testid="story-drawer-state" className="font-mono">
            {selectedInvocation
              ? `invocation:${selectedInvocation.invocation.record.invokeId}`
              : selectedConversation
                ? `conversation:${selectedConversation.promptCacheKey}`
                : selectedAccount
                  ? `account:${selectedAccount.id}:${selectedAccount.tab}`
                  : "none"}
          </span>
        </div>
      )}
    </>
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
          <ForcedWorkspaceViewStory view="conversations">
            <Story />
          </ForcedWorkspaceViewStory>
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardWorkingConversationsSection>;

type Story = StoryObj<typeof meta>;

const gpt56ModelContextWithoutReasoningResponse = createResponse([
  createConversation("story-gpt56-model-context-missing-reasoning", [
    createPreview({
      id: 9_561,
      invokeId: "story-gpt56-model-context-missing-reasoning-invoke",
      occurredAt: "2026-04-04T10:04:00Z",
      status: "completed",
      model: "gpt-5.6-sol",
      requestModel: "gpt-5.6-sol",
      responseModel: "gpt-5.6-sol",
      reasoningEffort: "—",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      transport: "websocket",
      totalTokens: 12_520,
      cost: 0.014,
    }),
  ]),
]);

async function assertImageEditEndpointRecentRow(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const recentRow = await canvas.findByTestId("dashboard-upstream-account-recent-row");
  await expect(recentRow).toBeVisible();
  expect(recentRow.getBoundingClientRect().width).toBeGreaterThan(0);
  expect(recentRow.getBoundingClientRect().height).toBeGreaterThan(0);
  const imageEditBadge = recentRow.querySelector(
    '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="image_edit"]',
  );
  if (!(imageEditBadge instanceof HTMLElement)) {
    throw new Error("missing image edit endpoint badge");
  }

  await expect(imageEditBadge).toHaveTextContent("image/edit");
  await expect(
    within(recentRow).queryByTestId("dashboard-image-tool-icon-badge"),
  ).not.toBeInTheDocument();

  const styles = getComputedStyle(imageEditBadge);
  const ratio = contrastRatio(styles.color, styles.backgroundColor);
  const colorChannels = parseComputedColorChannels(styles.color);
  expect(
    ratio,
    `expected ${styles.color} on ${styles.backgroundColor} to meet the 4.5:1 contrast threshold`,
  ).toBeGreaterThanOrEqual(4.5);
  const lightness = colorChannels[0] ?? Number.NaN;
  expect(lightness, `expected ${styles.color} to avoid black ink`).toBeGreaterThan(0.2);
  expect(lightness, `expected ${styles.color} to avoid white ink`).toBeLessThan(0.95);
  expect(
    imageEditInkChroma(styles.color),
    `expected ${styles.color} to retain amber chroma`,
  ).toBeGreaterThanOrEqual(0.1);
  expect(styles.borderColor).not.toBe(styles.backgroundColor);
}

async function assertQuickPolicyTonePalette(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
  await userEvent.click(accountTab);

  const policyBadges = await canvas.findAllByTestId("dashboard-upstream-account-policy-badge");
  await expect(policyBadges.map((badge) => badge.textContent?.trim())).toEqual([
    "兜底",
    "Fast",
    "禁出",
    "禁入",
  ]);
  await expect(policyBadges.map((badge) => badge.getAttribute("data-policy-tone"))).toEqual([
    "success",
    "primary",
    "warning",
    "neutral",
  ]);
}
export async function assertUpstreamAccountTabStory(canvasElement: HTMLElement) {
  await assertUpstreamAccountTabSummary(canvasElement);
  await assertUpstreamAccountTabRows(canvasElement);
}

async function assertUpstreamAccountTabSummary(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
  await userEvent.click(accountTab);
  const sortButton = canvas.getByTestId("dashboard-workspace-sort-button");
  await expect(sortButton).toHaveTextContent(/对话创建|Conversation created/);
  await userEvent.click(sortButton);
  await expect(sortButton).toHaveTextContent(/最新调用|Latest invocation/);
  await userEvent.click(sortButton);
  await expect(sortButton).toHaveTextContent(/成本|Cost/);
  await userEvent.click(sortButton);
  await expect(sortButton).toHaveTextContent(/Token|Tokens/);
  await userEvent.click(sortButton);
  await expect(sortButton).toHaveTextContent(/对话创建|Conversation created/);
  await expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();
  const totalNetworkSpeed = await canvas.findByTestId(
    "dashboard-upstream-account-total-network-speed",
  );
  await expect(totalNetworkSpeed).toHaveTextContent("46");
  await expect(totalNetworkSpeed).toHaveTextContent("KiB/s");
  await expect(totalNetworkSpeed).toHaveTextContent("214");
  await expect(canvas.getByText("最近 4 条调用")).toBeInTheDocument();
  await expect(canvas.getByTestId("dashboard-upstream-account-header-row")).not.toHaveTextContent(
    "#42",
  );
  await expect(canvas.getByTestId("dashboard-upstream-account-network-speed")).toHaveTextContent(
    "46",
  );
  const attentionBadges = canvas.getByTestId("dashboard-upstream-account-attention-badges");
  await expect(attentionBadges).not.toHaveClass("rounded-full");
  await expect(attentionBadges).not.toHaveClass("border-base-300/70");
  await expect(attentionBadges).not.toHaveClass("bg-base-100/86");
  const attentionBadgeButtons = await canvas.findAllByTestId(
    "dashboard-upstream-account-attention-badge",
  );
  await expect(attentionBadgeButtons).toHaveLength(2);
  expect(attentionBadges.querySelector("button")).toBe(attentionBadgeButtons[0]);
  await expect(canvas.queryByTestId("dashboard-upstream-account-routing-settings")).toBeNull();
  await expect(
    canvasElement.querySelector('[data-testid="dashboard-upstream-account-status"]'),
  ).toBeNull();
  await expect(canvas.getByText("上游拒绝")).toBeInTheDocument();
  await expect(canvas.getByText("限流")).toBeInTheDocument();
  await expect(canvas.getByText("禁新")).toBeInTheDocument();
  await expect(canvas.getByText("强制Fast")).toBeInTheDocument();
  await expect(canvas.getByText("禁入")).toBeInTheDocument();
  await expect(canvas.getByText("进行中")).toBeInTheDocument();
}

async function assertUpstreamAccountIdentityChips(canvas: ReturnType<typeof within>) {
  await expect(
    canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
  ).toHaveLength(4);
  const identityChips = canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip");
  await expect(
    new Set(identityChips.map((chip: Element) => (chip as HTMLElement).className)).size,
  ).toBe(4);
  await expect(canvas.queryByText("按调用计数，不按对话去重")).toBeNull();
  await expect(canvas.queryByText("仍在重试链路中的调用")).toBeNull();
  await expect(
    canvas.queryByText("最近 4 条调用里仍有活动或异常，优先从下方最近记录继续排查。"),
  ).toBeNull();
  const identityChip = canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip")[0];
  if (!(identityChip instanceof HTMLButtonElement)) {
    throw new Error("expected upstream identity chip button");
  }
  await expect(identityChip).toHaveAttribute("aria-label", expect.stringContaining("打开对话详情"));
}

async function assertUpstreamAccountTabRows(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const recentBreakdown = canvas.getByTestId("dashboard-upstream-account-recent-breakdown");
  await expect(recentBreakdown).toHaveTextContent(/排队中\s*1/);
  await expect(recentBreakdown).toHaveTextContent(/请求中\s*1/);
  await expect(recentBreakdown).toHaveTextContent(/响应中\s*1/);
  await expect(recentBreakdown).toHaveTextContent(/成功\s*24/);
  const phaseSegments = Array.from(
    recentBreakdown.querySelectorAll('[data-testid="invocation-phase-segment"]'),
  );
  expect(phaseSegments).toHaveLength(3);
  for (const phaseSegment of phaseSegments) {
    expect(phaseSegment.getAttribute("data-phase-motion")).toBe("static");
    const icon = phaseSegment.querySelector('[data-testid="invocation-phase-icon"]');
    expect(icon).toBeInstanceOf(HTMLElement);
    expect(icon?.className).not.toContain("animate-invocation-phase-requesting");
    expect(icon?.className).not.toContain("animate-pulse");
    expect(icon?.className).not.toContain("animate-spin");
  }
  await expect(canvas.getByTestId("dashboard-upstream-account-policy-badges")).toHaveTextContent(
    "禁出",
  );
  await expect(canvas.getByText("story-account-1")).toBeInTheDocument();
  await expect(canvas.getByText("gpt-5.5-mini")).toBeInTheDocument();
  await expect(canvas.getByText("gpt-5.5")).toBeInTheDocument();
  const firstRecentRow = canvas.getAllByTestId("dashboard-upstream-account-recent-row")[0];
  if (!(firstRecentRow instanceof HTMLElement)) {
    throw new Error("missing first upstream recent row");
  }
  const ttftLatency = firstRecentRow.querySelector(
    '[data-testid="dashboard-compact-latency-ttft"]',
  );
  const responseLatency = firstRecentRow.querySelector(
    '[data-testid="dashboard-compact-latency-response"]',
  );
  if (!(ttftLatency instanceof HTMLElement) || !(responseLatency instanceof HTMLElement)) {
    throw new Error("missing upstream compact latency readings");
  }
  await expect(ttftLatency.className).not.toMatch(/rounded|border|bg-/);
  await expect(responseLatency.className).not.toMatch(/rounded|border|bg-/);
  const imageBadge = firstRecentRow.querySelector(
    '[data-testid="dashboard-image-tool-icon-badge"]',
  );
  if (!(imageBadge instanceof HTMLElement)) {
    throw new Error("missing upstream image tool icon badge");
  }
  await expect(imageBadge).toHaveAttribute(
    "aria-label",
    expect.stringMatching(/图片工具|Image tool/),
  );
  await expect(imageBadge.className).toMatch(/rounded-full/);
  await expect(imageBadge.className).toMatch(/border/);
  await expect(firstRecentRow).not.toHaveTextContent(/RQ |UP |ED |TT /);
  await expect(firstRecentRow).not.toHaveTextContent(/\bIN\b|\bCW\b|\bO\b|\bT\b/);
  const recentSummaryLine = firstRecentRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-summary-line"]',
  );
  const recentMetaLine = firstRecentRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-meta-line"]',
  );
  const recentSummaryHit = firstRecentRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-summary-hit"]',
  );
  const recentSummaryCost = firstRecentRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
  );
  if (
    !(recentSummaryLine instanceof HTMLElement) ||
    !(recentMetaLine instanceof HTMLElement) ||
    !(recentSummaryHit instanceof HTMLElement) ||
    !(recentSummaryCost instanceof HTMLElement)
  ) {
    throw new Error("missing upstream account summary line");
  }
  await expect(recentSummaryLine).toHaveTextContent(/Hit .*Token .*\$/);
  expect(
    recentSummaryLine.compareDocumentPosition(recentMetaLine) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).not.toBe(0);
  await expect(recentSummaryHit).toHaveAttribute("data-summary-tone", "error");
  await expect(recentSummaryCost).toHaveAttribute("data-summary-tone", "warning");
  await expect(recentSummaryLine).toHaveAttribute("title", expect.stringContaining("Cache write:"));
  await assertUpstreamAccountIdentityChips(canvas);
}

function buildBulkSelectionStoryBindingResponse(
  promptCacheKey: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    promptCacheKey,
    bindingKind: "none",
    groupName: null,
    upstreamAccountId: null,
    upstreamAccountName: null,
    hasEncryptedSessionOwner: true,
    encryptedOwnerAccountId: 21,
    encryptedOwnerAccountName: "growth.6vv4@relay.example",
    encryptedOwnerGroupName: "CIII",
    timeouts: {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 300,
      imageFirstByteTimeoutSecs: 120,
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    timeoutFieldSources: {
      responsesFirstByteTimeoutSecs: "account",
      compactFirstByteTimeoutSecs: "group",
      imageFirstByteTimeoutSecs: "account",
      responsesStreamTimeoutSecs: "account",
      compactStreamTimeoutSecs: "root",
    },
    allowSwitchUpstream: false,
    fastModeRewriteMode: "keep_original",
    imageToolRewriteMode: "inherit",
    availableModels: ["gpt-5.5", "gpt-5.5-mini"],
    forwardProxyKey: null,
    forwardProxyKeys: [],
    policyFieldSources: {
      allowSwitchUpstream: "conversation",
      fastModeRewriteMode: "conversation",
      imageToolRewriteMode: "group",
      availableModels: "conversation",
      forwardProxyKey: "account",
    },
    updatedAt: "2026-05-12T16:15:57Z",
    ...overrides,
  };
}

function BulkSelectionStorySurface({
  theme,
  recentTargets,
  ...props
}: ComponentProps<typeof DashboardWorkingConversationsSection> & {
  theme?: "vibe-light" | "vibe-dark";
  recentTargets?: DashboardBulkRouteBindRecentTarget[];
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const [recentTargetsReady, setRecentTargetsReady] = useState(recentTargets == null);
  useStoryTheme(theme);

  useLayoutEffect(() => {
    const previousValue = window.localStorage.getItem(
      DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
    );
    if (recentTargets && recentTargets.length > 0) {
      window.localStorage.setItem(
        DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
        JSON.stringify(recentTargets),
      );
    } else {
      window.localStorage.removeItem(DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY);
    }
    setRecentTargetsReady(true);

    return () => {
      setRecentTargetsReady(recentTargets == null);
      if (previousValue == null) {
        window.localStorage.removeItem(DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY);
      } else {
        window.localStorage.setItem(
          DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
          previousValue,
        );
      }
    };
  }, [recentTargets]);

  useLayoutEffect(() => {
    if (!originalFetchRef.current) {
      originalFetchRef.current = window.fetch.bind(window);
    }

    window.fetch = createBulkSelectionStoryFetchHandler(
      originalFetchRef.current as typeof window.fetch,
    );

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
      originalFetchRef.current = null;
    };
  }, []);

  if (!recentTargetsReady) {
    return null;
  }

  return (
    <ForcedWorkspaceViewStory view="conversations">
      <DashboardWorkingConversationsSection {...props} />
    </ForcedWorkspaceViewStory>
  );
}

const bulkSelectionStoryArgs = {
  activeRange: "today" as const,
  cards: buildCards(wideDesktopResponse),
  totalMatched: wideDesktopResponse.conversations.length,
  isLoading: false,
  error: null,
};

const bulkSelectionStoryRecentTargets: DashboardBulkRouteBindRecentTarget[] = [
  {
    kind: "upstreamAccount",
    upstreamAccountId: 21,
    usedAt: Date.parse("2026-07-20T11:00:00Z"),
  },
  {
    kind: "group",
    groupName: "Tokyo",
    usedAt: Date.parse("2026-07-20T10:00:00Z"),
  },
  {
    kind: "group",
    groupName: "CIII",
    usedAt: Date.parse("2026-07-20T09:00:00Z"),
  },
  {
    kind: "upstreamAccount",
    upstreamAccountId: 101,
    usedAt: Date.parse("2026-07-20T08:00:00Z"),
  },
  {
    kind: "group",
    groupName: "Relay-Blue",
    usedAt: Date.parse("2026-07-20T07:00:00Z"),
  },
];

export type { Story };
export {
  assertImageEditEndpointRecentRow,
  assertQuickPolicyTonePalette,
  BulkSelectionStorySurface,
  buildBulkSelectionStoryBindingResponse,
  bulkSelectionStoryArgs,
  bulkSelectionStoryRecentTargets,
  gpt56ModelContextWithoutReasoningResponse,
  meta,
};

function createBulkSelectionStoryFetchHandler(
  originalFetch: typeof window.fetch,
): typeof window.fetch {
  return async (input, init) => {
    const request =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const url = new URL(request, window.location.origin);

    if (url.pathname === "/api/pool/upstream-accounts") {
      return new Response(
        JSON.stringify({
          writesEnabled: true,
          items: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS,
          groups: [
            { groupName: "CIII", accountCount: 1 },
            { groupName: "Tokyo", accountCount: 1 },
            { groupName: "Relay-Blue", accountCount: 1 },
          ],
          forwardProxyNodes: [],
          hasUngroupedAccounts: false,
          total: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
          page: 1,
          pageSize: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    }

    if (
      url.pathname === "/api/stats/prompt-cache-conversation-bindings/bulk-actions" &&
      init?.method === "POST"
    ) {
      const payload = init.body ? JSON.parse(String(init.body)) : {};
      const promptCacheKeys = Array.isArray(payload.promptCacheKeys)
        ? payload.promptCacheKeys.map((value: unknown) => String(value))
        : [];
      const items = promptCacheKeys.map((promptCacheKey: string) => ({
        promptCacheKey,
        ok: true,
        error: null,
        binding: buildBulkSelectionStoryBindingResponse(promptCacheKey, {
          bindingKind:
            payload.action === "bind"
              ? payload.bindingKind === "none"
                ? "none"
                : payload.bindingKind === "upstreamAccount"
                  ? "upstreamAccount"
                  : "group"
              : "none",
          groupName: payload.bindingKind === "group" ? (payload.groupName ?? "CIII") : null,
          upstreamAccountId:
            payload.bindingKind === "upstreamAccount" ? (payload.upstreamAccountId ?? 101) : null,
          upstreamAccountName:
            payload.bindingKind === "upstreamAccount" ? "Codex Pro - Tokyo" : null,
          hasEncryptedSessionOwner: payload.action !== "clearAndResetAffinity",
          encryptedOwnerAccountId: payload.action === "clearAndResetAffinity" ? null : 21,
          encryptedOwnerAccountName:
            payload.action === "clearAndResetAffinity" ? null : "growth.6vv4@relay.example",
          encryptedOwnerGroupName: payload.action === "clearAndResetAffinity" ? null : "CIII",
          fastModeRewriteMode:
            payload.action === "setFastModeRewriteMode"
              ? (payload.fastModeRewriteMode ?? "keep_original")
              : "keep_original",
        }),
      }));
      return new Response(
        JSON.stringify({
          action: payload.action ?? "bind",
          totalRequested: promptCacheKeys.length,
          totalSucceeded: promptCacheKeys.length,
          totalFailed: 0,
          items,
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    }

    return originalFetch(input, init);
  };
}
