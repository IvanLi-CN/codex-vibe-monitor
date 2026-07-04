/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/dom";
import { I18nProvider } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
  mapPromptCacheConversationsToDashboardCards,
  type DashboardWorkingConversationCardModel,
} from "../lib/dashboardWorkingConversations";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import {
  DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY,
  readPersistedDashboardWorkspaceView,
} from "./dashboardActivityRange";

const virtualizerMocks = vi.hoisted(() => ({
  rowIndexes: null as number[] | null,
  totalSize: null as number | null,
  customVirtualItems: null as
    | Array<{
        key: number;
        index: number;
        start: number;
        size: number;
        end: number;
        translateStart?: number;
      }>
    | null,
}));

vi.mock("@tanstack/react-virtual", () => ({
  useWindowVirtualizer: ({ count }: { count: number }) => {
    const rowIndexes =
      virtualizerMocks.rowIndexes ??
      Array.from({ length: Math.min(count, 4) }, (_, index) => index);
    return {
      measureElement: () => undefined,
      getVirtualItems: () =>
        virtualizerMocks.customVirtualItems ??
        rowIndexes
          .filter((index) => index >= 0 && index < count)
          .map((index) => ({
            key: index,
            index,
            start: index * 360,
            size: 360,
            end: index * 360 + 360,
          })),
      getTotalSize: () => virtualizerMocks.totalSize ?? count * 360,
    };
  },
}));

const upstreamAccountActivityMock = vi.hoisted(() => ({
  data: null as UpstreamAccountActivityResponse | null,
  isLoading: false,
  error: null as string | null,
  resolvedRecentInvocationLimit: null as number | null,
  calls: [] as Array<{
    range: string;
    enabled: boolean;
    recentInvocationLimit?: number;
  }>,
}));

vi.mock("../hooks/useDashboardUpstreamAccountActivity", () => ({
  useDashboardUpstreamAccountActivity: (
    range: string,
    enabled: boolean,
    recentInvocationLimit?: number,
  ) => {
    upstreamAccountActivityMock.calls.push({
      range,
      enabled,
      recentInvocationLimit,
    });
    return {
      data: upstreamAccountActivityMock.data,
      isLoading: upstreamAccountActivityMock.isLoading,
      error: upstreamAccountActivityMock.error,
      recentInvocationLimit:
        upstreamAccountActivityMock.resolvedRecentInvocationLimit ??
        recentInvocationLimit ??
        upstreamAccountActivityMock.data?.accounts[0]?.recentInvocations.length ??
        4,
      hasActivated: enabled,
      reload: vi.fn(),
    };
  },
}));

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
    promptCacheKey:
      "promptCacheKey" in overrides ? (overrides.promptCacheKey ?? null) : null,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    requestModel:
      "requestModel" in overrides ? (overrides.requestModel ?? null) : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 200,
    cost: overrides.cost ?? 0.02,
    proxyDisplayName:
      "proxyDisplayName" in overrides
        ? (overrides.proxyDisplayName ?? null)
        : "tokyo-edge-01",
    upstreamAccountId:
      "upstreamAccountId" in overrides
        ? (overrides.upstreamAccountId ?? null)
        : 42,
    upstreamAccountName:
      "upstreamAccountName" in overrides
        ? (overrides.upstreamAccountName ?? null)
        : "pool-alpha@example.com",
    upstreamAccountPlanType:
      "upstreamAccountPlanType" in overrides
        ? (overrides.upstreamAccountPlanType ?? null)
        : undefined,
    endpoint: overrides.endpoint ?? "/v1/responses",
    compactionRequestKind: overrides.compactionRequestKind ?? null,
    compactionResponseKind: overrides.compactionResponseKind ?? null,
    imageIntent: overrides.imageIntent ?? null,
    inputTokens: overrides.inputTokens ?? 120,
    outputTokens: overrides.outputTokens ?? 80,
    cacheInputTokens: overrides.cacheInputTokens ?? 30,
    reasoningTokens: overrides.reasoningTokens ?? 14,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    errorMessage: overrides.errorMessage,
    downstreamStatusCode: overrides.downstreamStatusCode,
    downstreamErrorMessage: overrides.downstreamErrorMessage,
    failureKind: overrides.failureKind,
    transport: overrides.transport,
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 10,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 90,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 70,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 220,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 418,
  };
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: 200,
    totalCost: 0.02,
    createdAt:
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt: recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: [],
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

function createUpstreamAccountActivityResponse(): UpstreamAccountActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    accounts: [
      {
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        groupName: "Primary",
        planType: "enterprise",
        requestCount: 8,
        successCount: 6,
        failureCount: 2,
        nonSuccessCount: 2,
        totalTokens: 3200,
        successTokens: 2800,
        nonSuccessTokens: 400,
        failureTokens: 350,
        failureCost: 0.22,
        totalCost: 0.72,
        cacheHitRate: 0.25,
        tokensPerMinute: 640,
        spendRate: 0.12,
        firstByteAvgMs: 420,
        firstResponseByteTotalAvgMs: 2_867.5,
        avgTotalMs: 860,
        inProgressInvocationCount: 3,
        retryInvocationCount: 1,
        recentInvocations: [
          createPreview({
            id: 9001,
            invokeId: "acct-invoke-1",
            promptCacheKey: "pck-upstream-running",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountName: "Pool Alpha",
            requestModel: "gpt-5.5-mini",
            responseModel: "gpt-5.5",
            model: "gpt-5.5",
          }),
          createPreview({
            id: 9002,
            invokeId: "acct-invoke-2",
            promptCacheKey: "pck-upstream-failed",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "failed",
            upstreamAccountName: "Pool Alpha",
          }),
          createPreview({
            id: 9003,
            invokeId: "acct-invoke-3",
            promptCacheKey: "pck-upstream-success",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "success",
            upstreamAccountName: "Pool Alpha",
          }),
          createPreview({
            id: 9004,
            invokeId: "acct-invoke-4",
            promptCacheKey: "pck-upstream-pending",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "pending",
            upstreamAccountName: "Pool Alpha",
          }),
        ],
      },
    ],
  };
}

const UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS = [
  "tone-seed-4",
  "tone-seed-12",
  "tone-seed-17",
  "tone-seed-25",
  "tone-seed-31",
] as const;

let host: HTMLDivElement | null = null;
let root: Root | null = null;
const originalResizeObserver = globalThis.ResizeObserver;
const storage = new Map<string, string>();
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => {
    storage.set(key, value);
  },
  removeItem: (key: string) => {
    storage.delete(key);
  },
  clear: () => {
    storage.clear();
  },
};

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "scrollBy", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
});

beforeEach(() => {
  window.scrollBy = vi.fn();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  virtualizerMocks.rowIndexes = null;
  virtualizerMocks.totalSize = null;
  virtualizerMocks.customVirtualItems = null;
  upstreamAccountActivityMock.data = null;
  upstreamAccountActivityMock.isLoading = false;
  upstreamAccountActivityMock.error = null;
  upstreamAccountActivityMock.resolvedRecentInvocationLimit = null;
  upstreamAccountActivityMock.calls = [];
  window.localStorage.clear();
  globalThis.ResizeObserver = originalResizeObserver;
  vi.restoreAllMocks();
});

function renderSection(
  response: PromptCacheConversationsResponse,
  options?: {
    activeRange?: "today" | "yesterday" | "1d" | "7d" | "usage";
    error?: string | null;
    isLoading?: boolean;
    isLoadingMore?: boolean;
    hasMore?: boolean;
    totalMatched?: number;
    recentPreviewLimit?: number;
    onLoadMore?: () => void;
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  },
) {
  return renderSectionWithCards(
    mapPromptCacheConversationsToDashboardCards(response),
    options,
  );
}

function renderSectionWithCards(
  cards: DashboardWorkingConversationCardModel[],
  options?: {
    error?: string | null;
    isLoading?: boolean;
    isLoadingMore?: boolean;
    hasMore?: boolean;
    totalMatched?: number;
    onLoadMore?: () => void;
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  },
) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <DashboardWorkingConversationsSection
          activeRange={options?.activeRange ?? "today"}
          recentPreviewLimit={options?.recentPreviewLimit}
          cards={cards}
          totalMatched={options?.totalMatched}
          hasMore={options?.hasMore}
          isLoading={options?.isLoading ?? false}
          isLoadingMore={options?.isLoadingMore}
          error={options?.error ?? null}
          onLoadMore={options?.onLoadMore}
          setRefreshTargetCount={options?.setRefreshTargetCount}
          onOpenUpstreamAccount={options?.onOpenUpstreamAccount}
          onOpenConversation={options?.onOpenConversation}
          onOpenInvocation={options?.onOpenInvocation}
        />
      </I18nProvider>,
    );
  });
  return cards;
}

describe("DashboardWorkingConversationsSection model routing", () => {
  it("shows response model as primary text and renders routing indicator on mismatch", () => {
    renderSection(
      createResponse([
        createConversation("pck-mismatch", [
          createPreview({
            id: 1,
            invokeId: "invoke-mismatch",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "success",
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }),
        ]),
      ]),
    );

    expect(host?.textContent).toContain("gpt-5.5");
    expect(
      host?.querySelector(
        '[data-testid="dashboard-working-conversation-model-routing-indicator"]',
      ),
    ).not.toBeNull();
  });
});

function rerenderSection(
  response: PromptCacheConversationsResponse,
  options?: {
    activeRange?: "today" | "yesterday" | "1d" | "7d" | "usage";
    error?: string | null;
    isLoading?: boolean;
    isLoadingMore?: boolean;
    hasMore?: boolean;
    totalMatched?: number;
    recentPreviewLimit?: number;
    onLoadMore?: () => void;
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  },
) {
  return rerenderSectionWithCards(
    mapPromptCacheConversationsToDashboardCards(response),
    options,
  );
}

function rerenderSectionWithCards(
  cards: DashboardWorkingConversationCardModel[],
  options?: {
    error?: string | null;
    isLoading?: boolean;
    isLoadingMore?: boolean;
    hasMore?: boolean;
    totalMatched?: number;
    recentPreviewLimit?: number;
    onLoadMore?: () => void;
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  },
) {
  if (!root) {
    throw new Error("renderSection must run before rerenderSection");
  }
  act(() => {
    root?.render(
      <I18nProvider>
        <DashboardWorkingConversationsSection
          activeRange={options?.activeRange ?? "today"}
          recentPreviewLimit={options?.recentPreviewLimit}
          cards={cards}
          totalMatched={options?.totalMatched}
          hasMore={options?.hasMore}
          isLoading={options?.isLoading ?? false}
          isLoadingMore={options?.isLoadingMore}
          error={options?.error ?? null}
          onLoadMore={options?.onLoadMore}
          setRefreshTargetCount={options?.setRefreshTargetCount}
          onOpenUpstreamAccount={options?.onOpenUpstreamAccount}
          onOpenConversation={options?.onOpenConversation}
          onOpenInvocation={options?.onOpenInvocation}
        />
      </I18nProvider>,
    );
  });
  return cards;
}

describe("DashboardWorkingConversationsSection", () => {
  it("lazy-loads upstream account activity only after the account tab is opened", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    renderSection(
      createResponse([
        createConversation("pck-lazy-load", [
          createPreview({
            id: 1,
            invokeId: "invoke-lazy-load",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    expect(upstreamAccountActivityMock.calls[0]).toEqual({
      range: "today",
      enabled: false,
      recentInvocationLimit: 4,
    });
    expect(host?.textContent).toContain("当前对话 1 条");

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(upstreamAccountActivityMock.calls.at(-1)).toEqual({
      range: "today",
      enabled: true,
      recentInvocationLimit: 4,
    });
    expect(host?.textContent).toContain("当前活动账号 1 个");
    expect(host?.textContent).toContain(
      "展示当前总览范围内有调用的上游账号，以及每个账号的动态最近调用窗口。",
    );
    expect(host?.textContent).toContain("最近 4 条调用");
    expect(host?.textContent).not.toContain("账号状态");
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-card"]')?.getAttribute(
        "data-account-status",
      ),
    ).toBe("busy");
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-card"]')?.className,
    ).toContain("desktop1660:min-h-[31.5rem]");
    expect(host?.textContent).toContain("繁忙");
    expect(host?.textContent).not.toContain("渠道 Pool Alpha");
    expect(host?.textContent).not.toContain("Primary");
    expect(host?.textContent).not.toContain("按调用计数，不按对话去重");
    expect(host?.textContent).not.toContain("仍在重试链路中的调用");
    expect(host?.textContent).not.toContain("最近 4 条调用里仍有活动或异常");
    expect(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]')
        .length,
    ).toBe(4);
    expect(
      host?.querySelectorAll(
        '[data-testid="dashboard-upstream-account-recent-identity-chip"]',
      ).length,
    ).toBe(4);
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-header-row"]'),
    ).not.toBeNull();
    const firstRecentRow = host?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-row"]',
    );
    expect(firstRecentRow?.textContent).toContain("Responses");
    expect(firstRecentRow?.textContent).toContain("T 200");
    expect(firstRecentRow?.textContent).toContain("RQ 10/7");
    expect(firstRecentRow?.textContent).toContain("UP 90/70/220");
    expect(firstRecentRow?.textContent).toContain("ED 12/9");

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-live-call-breakdown"]'),
    ).toBeNull();
    const accountHeaderText = host?.querySelector(
      '[data-testid="dashboard-upstream-account-header-row"]',
    )?.textContent;
    expect(accountHeaderText).toContain("TPM");
    expect(accountHeaderText).toContain("消费速率");
    expect(accountHeaderText).not.toContain("调用");
    expect(accountHeaderText).not.toContain("重试");

    const latencyBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-latency-breakdown"]',
    );
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-card"]')?.textContent,
    ).toContain("2.87 s");
    expect(latencyBreakdown?.textContent).toContain("860");

    const requestBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-request-breakdown"]',
    );
    expect(requestBreakdown?.textContent).toContain("6");
    expect(requestBreakdown?.textContent).toContain("2");
    expect(requestBreakdown?.textContent).not.toContain("成");
    expect(requestBreakdown?.textContent).not.toContain("失");
    expect(requestBreakdown?.textContent).not.toContain("非");

    const requestSegments = Array.from(
      requestBreakdown?.querySelectorAll('[data-testid="dashboard-upstream-account-segment"]') ??
        [],
    );
    expect(requestSegments).toHaveLength(3);
    expect(requestSegments[0]?.textContent).toContain("6");
    expect(requestSegments[1]?.textContent).toContain("2");
    expect(requestSegments[2]?.textContent).toContain("0");
    expect(requestSegments[0]?.parentElement?.getAttribute("aria-label")).toContain("成功 6");
    expect(requestSegments[1]?.parentElement?.getAttribute("aria-label")).toContain("失败 2");
    expect(requestSegments[2]?.parentElement?.getAttribute("aria-label")).toContain("其他 0");

    const costBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-cost-breakdown"]',
    );
    const accountCardText = host?.querySelector(
      '[data-testid="dashboard-upstream-account-card"]',
    )?.textContent;
    expect(accountCardText).toContain("$0.72");
    expect(costBreakdown?.textContent).toContain("$0.22");
    expect(costBreakdown?.textContent).toContain("30.6%");

    const tokenBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-token-breakdown"]',
    );
    expect(accountCardText).toContain("3,200");
    expect(tokenBreakdown?.textContent).toContain("25%");
    expect(tokenBreakdown?.textContent).toContain("350");

    const recentBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-breakdown"]',
    );
    expect(recentBreakdown?.textContent).toContain("进行中");
    expect(recentBreakdown?.textContent).toContain("失败");
    expect(recentBreakdown?.textContent).toContain("成功");
    expect(recentBreakdown?.textContent).toContain("2");
    expect(recentBreakdown?.textContent).toContain("1");
  });

  it("opens detailed metric tooltips from the whole upstream account metric cards", async () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    renderSection(
      createResponse([
        createConversation("pck-upstream-metric-tooltips", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-metric-tooltips",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-metric-card"]'),
    ).toHaveLength(4);

    const costTrigger = host?.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="cost"]',
    );
    if (!(costTrigger instanceof HTMLElement)) {
      throw new Error("missing cost metric trigger");
    }

    act(() => {
      fireEvent.click(costTrigger);
    });

    await waitFor(() => {
      const tooltipText = document.body.textContent ?? "";
      expect(tooltipText).toContain("成本");
      expect(tooltipText).toContain("$0.72");
      expect(tooltipText).toContain("失败成本");
      expect(tooltipText).toContain("$0.22");
      expect(tooltipText).toContain("失败成本比率");
      expect(tooltipText).toContain("30.6%");
      expect(tooltipText).toContain("成功/其他成本");
      expect(tooltipText).toContain("$0.50");
      expect(tooltipText).toContain("单次均价");
    });

    const tokenTrigger = host?.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="token"]',
    );
    if (!(tokenTrigger instanceof HTMLElement)) {
      throw new Error("missing token metric trigger");
    }

    act(() => {
      fireEvent.click(tokenTrigger);
    });

    await waitFor(() => {
      const tooltipText = document.body.textContent ?? "";
      expect(tooltipText).toContain("Token");
      expect(tooltipText).toContain("3,200");
      expect(tooltipText).toContain("缓存命中率");
      expect(tooltipText).toContain("25%");
      expect(tooltipText).toContain("失败 Token");
      expect(tooltipText).toContain("350");
      expect(tooltipText).toContain("成功 Token");
      expect(tooltipText).toContain("2,800");
      expect(tooltipText).toContain("单请求 Token");
      expect(tooltipText).toContain("400");
    });

    const requestTrigger = host?.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="requests"]',
    );
    if (!(requestTrigger instanceof HTMLElement)) {
      throw new Error("missing request metric trigger");
    }

    act(() => {
      fireEvent.click(requestTrigger);
    });

    await waitFor(() => {
      const tooltipText = document.body.textContent ?? "";
      expect(tooltipText).toContain("请求数");
      expect(tooltipText).toContain("8");
      expect(tooltipText).toContain("成功率");
      expect(tooltipText).toContain("75%");
      expect(tooltipText).toContain("非成功率");
      expect(tooltipText).toContain("25%");
    });

    const latencyTrigger = host?.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="latency"]',
    );
    if (!(latencyTrigger instanceof HTMLElement)) {
      throw new Error("missing latency metric trigger");
    }

    act(() => {
      fireEvent.click(latencyTrigger);
    });

    await waitFor(() => {
      const tooltipText = document.body.textContent ?? "";
      expect(tooltipText).toContain("首字用时");
      expect(tooltipText).toContain("2.87 s");
      expect(tooltipText).toContain("响应时间");
      expect(tooltipText).toContain("860");
      expect(tooltipText).toContain("阶段首字节");
      expect(tooltipText).toContain("420");
    });
  });

  it("uses failure cost share for the upstream account cost failure ratio", async () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    const account = upstreamAccountActivityMock.data.accounts[0];
    account.failureCount = 2;
    account.nonSuccessCount = 2;
    account.failureCost = 0;
    account.totalCost = 0.72;

    renderSection(
      createResponse([
        createConversation("pck-upstream-cost-ratio", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-cost-ratio",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const costBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-cost-breakdown"]',
    );
    expect(costBreakdown?.textContent).toContain("$0.00");
    expect(costBreakdown?.textContent).toContain("0%");

    const costTrigger = host?.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="cost"]',
    );
    if (!(costTrigger instanceof HTMLElement)) {
      throw new Error("missing cost metric trigger");
    }

    act(() => {
      fireEvent.click(costTrigger);
    });

    await waitFor(() => {
      const tooltip = Array.from(
        document.body.querySelectorAll('[data-testid="dashboard-upstream-account-metric-tooltip"]'),
      ).find((node) => node.textContent?.includes("失败成本"));
      const tooltipText = tooltip?.textContent ?? "";
      expect(tooltipText).toContain("失败成本");
      expect(tooltipText).toContain("$0.00");
      expect(tooltipText).toContain("失败成本比率");
      expect(tooltipText).toContain("0%");
      expect(tooltipText).not.toContain("25%");
    });
  });

  it("passes the dynamic recent preview limit into upstream account activity", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    renderSection(
      createResponse([
        createConversation("pck-upstream-dynamic-limit", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-dynamic-limit",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      { recentPreviewLimit: 7 },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(upstreamAccountActivityMock.calls.at(-1)).toEqual({
      range: "today",
      enabled: true,
      recentInvocationLimit: 7,
    });
    expect(host?.textContent).toContain("最近 7 条调用");
  });

  it("renders upstream account cards with their own resolved recent preview limit", () => {
    upstreamAccountActivityMock.data = {
      ...createUpstreamAccountActivityResponse(),
      accounts: [
        {
          ...createUpstreamAccountActivityResponse().accounts[0]!,
          inProgressInvocationCount: 9,
          recentInvocations: Array.from({ length: 9 }, (_, index) =>
            createPreview({
              id: 9800 + index,
              invokeId: `acct-expanded-${index + 1}`,
              promptCacheKey: `pck-upstream-expanded-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: index < 7 ? "running" : "success",
              upstreamAccountName: "Pool Alpha",
            }),
          ),
        },
      ],
    };
    upstreamAccountActivityMock.resolvedRecentInvocationLimit = 9;

    renderSection(
      createResponse([
        createConversation("pck-upstream-account-limit", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-account-limit",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      { recentPreviewLimit: 4 },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(host?.textContent).toContain("最近 9 条调用");
    expect(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]'),
    ).toHaveLength(9);
  });

  it("disables and falls back from the upstream account tab for usage range", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    const response = createResponse([
      createConversation("pck-usage-fallback", [
        createPreview({
          id: 1,
          invokeId: "invoke-usage-fallback",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]);

    renderSection(response);

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });
    expect(host?.textContent).toContain("当前活动账号 1 个");

    act(() => {
      rerenderSection(response, { activeRange: "usage" });
    });

    expect(host?.textContent).toContain("当前对话 1 条");
    const accountTabAfter = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    expect(accountTabAfter?.disabled).toBe(true);
  });

  it("persists the preferred workspace view and restores it on remount", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    const response = createResponse([
      createConversation("pck-view-persist", [
        createPreview({
          id: 1,
          invokeId: "invoke-view-persist",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]);

    renderSection(response);

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(
      readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY),
    ).toBe("upstreamAccounts");

    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;

    renderSection(response);

    expect(host?.textContent).toContain("当前活动账号 1 个");
    expect(accountTab.getAttribute("aria-selected")).toBe("true");
  });

  it("preserves the upstream-account preference when usage temporarily forces conversations", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    const response = createResponse([
      createConversation("pck-usage-restore", [
        createPreview({
          id: 1,
          invokeId: "invoke-usage-restore",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]);

    renderSection(response);

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    rerenderSection(response, { activeRange: "usage" });
    expect(host?.textContent).toContain("当前对话 1 条");
    expect(
      readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY),
    ).toBe("upstreamAccounts");

    rerenderSection(response, { activeRange: "today" });
    expect(host?.textContent).toContain("当前活动账号 1 个");
    const restoredAccountTab = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((node) => node.textContent?.includes("上游账号"));
    expect(restoredAccountTab?.getAttribute("aria-selected")).toBe("true");
  });

  it("switches the section subtitle when the upstream account tab is active", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    renderSection(
      createResponse([
        createConversation("pck-upstream-subtitle", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-subtitle",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    expect(host?.textContent).toContain(
      "展示最近 5 分钟内有终态调用，或当前仍处于运行中 / 排队中的对话。",
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(host?.textContent).toContain(
      "展示当前总览范围内有调用的上游账号，以及每个账号的动态最近调用窗口。",
    );
    expect(host?.textContent).not.toContain(
      "展示最近 5 分钟内有终态调用，或当前仍处于运行中 / 排队中的对话。",
    );
  });

  it("shows conversation short id, full request id, mismatch models, and real prompt cache key in upstream recent rows", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    const onOpenInvocation = vi.fn();
    const onOpenConversation = vi.fn();

    renderSection(
      createResponse([
        createConversation("pck-upstream-anchor", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-anchor",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      { onOpenConversation, onOpenInvocation },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const rows = Array.from(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]') ?? [],
    );
    const firstRow = rows[0];
    if (!(firstRow instanceof HTMLButtonElement)) {
      throw new Error("missing first upstream recent row");
    }

    const expectedConversationId = formatDashboardWorkingConversationSequenceId(
      `WC-${hashDashboardWorkingConversationKey("pck-upstream-running").slice(0, 6)}`,
    );
    const displayConversationId = expectedConversationId.replace(/^WC-/, "");

    const identity = firstRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-identity"]',
    );
    const identityChip = firstRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-identity-chip"]',
    );
    expect(identityChip).not.toBeNull();
    expect(identityChip?.className).toContain("rounded-full");
    expect(identityChip?.className).toContain("font-mono");
    expect(identity?.textContent).toContain(displayConversationId);
    expect(identity?.textContent).toContain("acct-invoke-1");
    expect(identity?.textContent).not.toContain("WC-");
    expect(firstRow.textContent).not.toContain("Pool Alpha");
    expect(firstRow.textContent).toContain("gpt-5.5-mini");
    expect(firstRow.textContent).toContain("gpt-5.5");
    expect(
      firstRow.querySelector(
        '[data-testid="dashboard-upstream-account-recent-model-routing-indicator"]',
      ),
    ).not.toBeNull();

    const reasoningBadge = firstRow.querySelector(
      '[data-testid="dashboard-working-conversation-reasoning-effort"]',
    );
    const endpointBadge = Array.from(firstRow.querySelectorAll("span")).find(
      (element) => element.textContent?.trim() === "Responses",
    )?.parentElement;
    expect(reasoningBadge?.className).toContain("min-h-5");
    expect(endpointBadge?.className).toContain("min-h-5");

    act(() => {
      firstRow.click();
    });

    expect(onOpenInvocation).toHaveBeenCalledWith(
      expect.objectContaining({
        promptCacheKey: "pck-upstream-running",
      }),
    );
    expect(onOpenInvocation.mock.calls[0]?.[0]?.promptCacheKey).not.toBe(
      "acct-invoke-1",
    );
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  it("opens conversation detail from the upstream recent identity chip only", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    const onOpenInvocation = vi.fn();
    const onOpenConversation = vi.fn();

    renderSection(
      createResponse([
        createConversation("pck-upstream-anchor", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-anchor",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      { onOpenConversation, onOpenInvocation },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const identityChip = host?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-identity-chip"]',
    );
    if (!(identityChip instanceof HTMLButtonElement)) {
      throw new Error("missing upstream recent identity chip");
    }

    expect(identityChip.getAttribute("aria-label")).toContain("打开对话详情");
    expect(identityChip.getAttribute("aria-label")).toContain(
      "pck-upstream-running",
    );

    act(() => {
      identityChip.click();
    });

    expect(onOpenConversation).toHaveBeenCalledWith({
      conversationSequenceId: `WC-${hashDashboardWorkingConversationKey("pck-upstream-running").slice(0, 6)}`,
      promptCacheKey: "pck-upstream-running",
    });
    expect(onOpenInvocation).not.toHaveBeenCalled();

    onOpenConversation.mockClear();
    onOpenInvocation.mockClear();

    act(() => {
      identityChip.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });

    expect(onOpenConversation).toHaveBeenCalledTimes(1);
    expect(onOpenInvocation).not.toHaveBeenCalled();

    onOpenConversation.mockClear();

    act(() => {
      identityChip.dispatchEvent(
        new KeyboardEvent("keydown", { key: " ", bubbles: true }),
      );
    });

    expect(onOpenConversation).toHaveBeenCalledTimes(1);
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("spreads identity chip tones for prompt cache keys that used to collide on the same low-bit slot", () => {
    upstreamAccountActivityMock.data = {
      range: "today",
      rangeStart: "2026-04-04T10:00:00Z",
      rangeEnd: "2026-04-04T10:05:00Z",
      accounts: [
        {
          upstreamAccountId: 42,
          displayName: "Pool Alpha",
          groupName: "Primary",
          planType: "enterprise",
          requestCount: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length,
          successCount: 0,
          failureCount: 0,
          nonSuccessCount: 0,
          totalTokens: 1600,
          successTokens: 0,
          nonSuccessTokens: 0,
          failureTokens: 0,
          failureCost: 0,
          totalCost: 0.12,
          cacheHitRate: 0.25,
          tokensPerMinute: 640,
          spendRate: 0.12,
          firstByteAvgMs: 420,
          avgTotalMs: 860,
          inProgressInvocationCount: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length,
          retryInvocationCount: 0,
          recentInvocations: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.map(
            (promptCacheKey, index) =>
              createPreview({
                id: 9100 + index,
                invokeId: `acct-tone-${index + 1}`,
                promptCacheKey,
                occurredAt: `2026-04-04T10:0${5 - index}:00Z`,
                status: "running",
                upstreamAccountName: "Pool Alpha",
              }),
          ),
        },
      ],
    };

    renderSection(
      createResponse([
        createConversation("pck-upstream-tone-anchor", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-tone-anchor",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      { recentPreviewLimit: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const identityChips = Array.from(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-identity-chip"]') ??
        [],
    );
    expect(identityChips).toHaveLength(UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length);

    const toneClassNames = identityChips.map((chip) => chip.className);
    expect(new Set(toneClassNames).size).toBeGreaterThanOrEqual(4);

    const renderedShortIds = identityChips.map((chip) =>
      chip.textContent?.trim(),
    );
    for (const promptCacheKey of UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS) {
      const expectedShortId = formatDashboardWorkingConversationSequenceId(
        `WC-${hashDashboardWorkingConversationKey(promptCacheKey).slice(0, 6)}`,
      ).replace(/^WC-/, "");
      expect(renderedShortIds).toContain(expectedShortId);
    }
  });

  it("renders the WS transport badge only in websocket invocation slots", () => {
    renderSection(
      createResponse([
        createConversation("pck-ws-transport", [
          createPreview({
            id: 1,
            invokeId: "invoke-current-ws",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            transport: "websocket",
          }),
          createPreview({
            id: 2,
            invokeId: "invoke-previous-http",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "failed",
            transport: "http",
          }),
        ]),
      ]),
    );

    const badges = host?.querySelectorAll(
      '[data-testid="invocation-transport-badge"]',
    );
    expect(badges).toHaveLength(1);
    expect(
      badges?.[0]?.querySelector('[aria-hidden="true"]')?.textContent,
    ).toBe("WS");
    expect(badges?.[0]?.textContent).toContain("WebSocket transport");
    expect(badges?.[0]?.getAttribute("title")).toBe("WebSocket");
  });

  it("shows a bare hash in the card header while keeping the raw prompt cache key non-visible", () => {
    const cards = renderSection(
      createResponse([
        createConversation("019d68a9-9c32-7482-a353-71e4b6265f09", [
          createPreview({
            id: 1,
            invokeId: "invoke-header",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const card = host?.querySelector(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing working conversation card");
    }

    const expectedSortAnchorLabel = new Intl.DateTimeFormat("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(new Date("2026-04-04T10:04:00Z"));

    expect(card.textContent).toContain("运行中");
    expect(card.textContent).toContain(expectedSortAnchorLabel);
    expect(card.textContent).toContain("请求");
    expect(card.textContent).toContain("Token");
    expect(card.textContent).toContain("成本");
    expect(card.textContent).not.toContain("累计请求");
    expect(card.textContent).not.toContain("对话 Tokens");
    expect(card.textContent).not.toContain("对话成本");
    expect(card.textContent).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(card.textContent).not.toContain("WC-");
    expect(card.textContent).not.toContain(
      "019d68a9-9c32-7482-a353-71e4b6265f09",
    );
    expect(card.getAttribute("data-prompt-cache-key")).toBeNull();
    expect(card.getAttribute("data-anchor-prompt-cache-key")).toBeNull();
    expect(card.getAttribute("data-conversation-sequence-id")).toBe(
      cards[0]?.conversationSequenceId.replace(/^WC-/, ""),
    );
  });

  it("keeps rendered cards visible while surfacing a non-blocking error banner", () => {
    renderSection(
      createResponse([
        createConversation("pck-inline-error", [
          createPreview({
            id: 1,
            invokeId: "invoke-inline-error",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      {
        error: "load more temporarily unavailable",
      },
    );

    expect(
      host?.querySelector(
        '[data-testid="dashboard-working-conversation-card"]',
      ),
    ).toBeTruthy();
    expect(host?.textContent).toContain("load more temporarily unavailable");
    expect(host?.textContent).toContain("运行中");
  });

  it("places reasoning effort between the model name and service-tier indicator", () => {
    renderSection(
      createResponse([
        createConversation("pck-reasoning-layout", [
          createPreview({
            id: 1,
            invokeId: "invoke-reasoning-layout",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
            reasoningEffort: "medium",
            requestedServiceTier: "priority",
            serviceTier: "priority",
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLDivElement)) {
      throw new Error("missing current invocation slot");
    }

    const modelName = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-model-name"]',
    );
    const reasoningEffort = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-reasoning-effort"]',
    );
    const fastIcon = currentSlot.querySelector(
      '[data-testid="invocation-fast-icon"]',
    );
    if (
      !(modelName instanceof HTMLElement) ||
      !(reasoningEffort instanceof HTMLElement) ||
      !(fastIcon instanceof HTMLElement)
    ) {
      throw new Error("missing model/reasoning/service-tier markers");
    }

    expect(reasoningEffort.textContent).toContain("medium");
    expect(
      modelName.compareDocumentPosition(reasoningEffort) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(
      reasoningEffort.compareDocumentPosition(fastIcon) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
  });

  it("keeps the account chip inline with model metadata and surfaces compact endpoint badges", () => {
    renderSection(
      createResponse([
        createConversation("pck-account-inline-compact", [
          createPreview({
            id: 1,
            invokeId: "invoke-account-inline-compact",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            upstreamAccountName:
              "paisleeeinar5710 Team sandbox workflow monitor",
            upstreamAccountPlanType: "team",
            endpoint: "/v1/responses/compact",
            reasoningEffort: "medium",
            tTotalMs: null,
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLDivElement)) {
      throw new Error("missing current invocation slot");
    }

    const accountLine = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-account-line"]',
    );
    const accountChip = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-account-chip"]',
    );
    const accountName = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-account-name"]',
    );
    const accountMeta = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-account-meta"]',
    );
    const compactBadge = currentSlot.querySelector(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
    );

    if (
      !(accountLine instanceof HTMLDivElement) ||
      !(accountChip instanceof HTMLElement) ||
      !(accountName instanceof HTMLElement) ||
      !(accountMeta instanceof HTMLDivElement) ||
      !(compactBadge instanceof HTMLElement)
    ) {
      throw new Error("missing account row or compact endpoint markers");
    }

    expect(accountLine.className).toContain("sm:flex-nowrap");
    expect(accountChip.className).not.toContain("bg-base-100");
    expect(accountChip.className).not.toContain("px-1.5");
    expect(accountName.className).toContain("truncate");
    expect(accountName.className).toContain("whitespace-nowrap");
    expect(accountName.className).not.toContain("line-clamp-2");
    expect(accountName.className).not.toContain("break-all");
    expect(
      accountChip.compareDocumentPosition(accountMeta) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(compactBadge.textContent).toMatch(/远程压缩|Compact/);
    expect(currentSlot.textContent).toContain("Team");
  });

  it("shows the remote compaction v2 badge for running responses previews", () => {
    renderSection(
      createResponse([
        createConversation("pck-remote-v2", [
          createPreview({
            id: 11,
            invokeId: "invoke-remote-v2",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "running",
            endpoint: "/v1/responses",
            compactionRequestKind: "remote_v2",
          }),
        ]),
      ]),
    );

    const badge = host?.querySelector(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="remote_v2"]',
    );
    if (!(badge instanceof HTMLElement)) {
      throw new Error("missing remote v2 badge");
    }

    expect(badge.textContent).toMatch(/远程压缩V2|Remote compaction V2/);
  });

  it("shows the image-tool badge only for image-capable previews", () => {
    renderSection(
      createResponse([
        createConversation("pck-image-yes", [
          createPreview({
            id: 12,
            invokeId: "invoke-image-yes",
            occurredAt: "2026-04-04T10:05:30Z",
            status: "running",
            endpoint: "/v1/responses",
            imageIntent: "yes",
          }),
        ]),
        createConversation("pck-image-no", [
          createPreview({
            id: 13,
            invokeId: "invoke-image-no",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
            endpoint: "/v1/responses",
            imageIntent: "no",
          }),
        ]),
      ]),
    );

    const badges = host?.querySelectorAll(
      '[data-testid="invocation-image-tool-badge"]',
    );
    expect(badges?.length ?? 0).toBe(1);
    expect(host?.textContent).toMatch(/图片工具|Image tool/);
  });

  it("keeps image and remote_v2 badges visible together for mixed-signal previews", () => {
    renderSection(
      createResponse([
        createConversation("pck-mixed-signal", [
          createPreview({
            id: 14,
            invokeId: "invoke-mixed-signal",
            occurredAt: "2026-04-04T10:06:30Z",
            status: "running",
            endpoint: "/v1/responses",
            compactionRequestKind: "remote_v2",
            imageIntent: "direct_image",
          }),
        ]),
      ]),
    );

    const remoteBadge = host?.querySelector(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="remote_v2"]',
    );
    const imageBadge = host?.querySelector(
      '[data-testid="invocation-image-tool-badge"][data-image-intent-kind="direct_image"]',
    );

    if (!(remoteBadge instanceof HTMLElement) || !(imageBadge instanceof HTMLElement)) {
      throw new Error("missing mixed-signal badges");
    }

    expect(remoteBadge.textContent).toMatch(/远程压缩V2|Remote compaction V2/);
    expect(imageBadge.textContent).toMatch(/图片工具|Image tool/);
  });

  it("renders compact account plan badges and hides local or missing plans", () => {
    renderSection(
      createResponse([
        createConversation("pck-enterprise-plan", [
          createPreview({
            id: 1,
            invokeId: "invoke-enterprise-plan",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            upstreamAccountName: "enterprise-account@example.com",
            upstreamAccountPlanType: "enterprise",
          }),
        ]),
        createConversation("pck-plus-plan", [
          createPreview({
            id: 2,
            invokeId: "invoke-plus-plan",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "running",
            upstreamAccountName: "plus-account@example.com",
            upstreamAccountPlanType: "plus",
          }),
        ]),
        createConversation("pck-free-plan", [
          createPreview({
            id: 3,
            invokeId: "invoke-free-plan",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "running",
            upstreamAccountName: "free-account@example.com",
            upstreamAccountPlanType: "free",
          }),
        ]),
        createConversation("pck-pro-plan", [
          createPreview({
            id: 4,
            invokeId: "invoke-pro-plan",
            occurredAt: "2026-04-04T10:01:00Z",
            status: "running",
            upstreamAccountName: "pro-account@example.com",
            upstreamAccountPlanType: "pro",
          }),
        ]),
        createConversation("pck-local-plan", [
          createPreview({
            id: 5,
            invokeId: "invoke-local-plan",
            occurredAt: "2026-04-04T10:00:00Z",
            status: "running",
            upstreamAccountName: "local-account@example.com",
            upstreamAccountPlanType: "local",
          }),
        ]),
      ]),
    );

    const planBadges = Array.from(
      host?.querySelectorAll(
        '[data-testid="dashboard-working-conversation-account-plan"]',
      ) ?? [],
    );
    const labels = planBadges.map((badge) => badge.textContent);

    expect(labels).toEqual(expect.arrayContaining(["Ent", "Plus", "Free", "Pro"]));
    expect(labels).not.toContain("enterprise");
    expect(labels).not.toContain("local");

    const enterpriseBadge = planBadges.find(
      (badge) => badge.textContent === "Ent",
    );
    expect(enterpriseBadge?.getAttribute("title")).toBe("enterprise");
    expect(enterpriseBadge?.getAttribute("data-plan")).toBe("enterprise");
  });

  it("keeps the virtualized viewport spanning the full responsive grid width", () => {
    renderSection(
      createResponse([
        createConversation("pck-layout", [
          createPreview({
            id: 1,
            invokeId: "invoke-layout",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const grid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-grid"]',
    );
    if (!(grid instanceof HTMLDivElement)) {
      throw new Error("missing working conversations grid");
    }
    const viewport = grid.firstElementChild;
    if (!(viewport instanceof HTMLDivElement)) {
      throw new Error("missing virtualized viewport");
    }

    expect(viewport.className).toContain("col-span-full");
  });

  it("rebinds width observation after the grid first mounts so wide layouts keep multi-column rendering", () => {
    const observe = vi.fn();
    const disconnect = vi.fn();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    globalThis.ResizeObserver = class {
      observe = observe;
      disconnect = disconnect;
    } as unknown as typeof ResizeObserver;

    renderSection(createResponse([]));
    rerenderSection(
      createResponse([
        createConversation("pck-wide-mounted", [
          createPreview({
            id: 1,
            invokeId: "invoke-wide-mounted",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const grid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-grid"]',
    );
    if (!(grid instanceof HTMLDivElement)) {
      throw new Error("missing working conversations grid");
    }

    const rowGrid = grid.querySelector(
      '[data-testid="dashboard-working-conversations-row"] > div',
    );
    if (!(rowGrid instanceof HTMLDivElement)) {
      throw new Error("missing row grid");
    }

    expect(observe).toHaveBeenCalledWith(grid);
    expect(rowGrid.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 1fr))");
    expect(disconnect).not.toHaveBeenCalled();
  });

  it("prefers the resolved CSS grid track count over the narrower container width fallback", () => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1570);
    const originalGetComputedStyle = window.getComputedStyle.bind(window);
    vi.spyOn(window, "getComputedStyle").mockImplementation((element) => {
      const styles = originalGetComputedStyle(element);
      if (
        element instanceof HTMLElement &&
        element.dataset.testid === "dashboard-working-conversations-grid"
      ) {
        const mockedStyles = Object.create(styles) as CSSStyleDeclaration;
        Object.defineProperty(mockedStyles, "gridTemplateColumns", {
          configurable: true,
          value: "1fr 1fr 1fr 1fr",
        });
        return mockedStyles;
      }
      return styles;
    });

    renderSection(
      createResponse([
        createConversation("pck-css-track-count", [
          createPreview({
            id: 1,
            invokeId: "invoke-css-track-count",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
          createPreview({
            id: 2,
            invokeId: "invoke-css-track-count-prev",
            occurredAt: "2026-04-04T10:03:30Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const rowGrid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-row"] > div',
    );
    if (!(rowGrid instanceof HTMLDivElement)) {
      throw new Error("missing row grid");
    }

    expect(rowGrid.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 1fr))");
  });

  it("keeps a vertical gutter between virtualized rows", () => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);

    renderSection(
      createResponse(
        Array.from({ length: 8 }, (_, index) =>
          createConversation(`pck-vertical-gap-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-vertical-gap-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
    );

    const rows = host?.querySelectorAll<HTMLElement>(
      '[data-testid="dashboard-working-conversations-row"]',
    );
    if (!rows || rows.length < 2) {
      throw new Error("expected at least two virtualized rows");
    }

    expect(rows[0]?.style.paddingBottom).toBe("16px");
    expect(rows[1]?.style.paddingBottom).toBe("0px");
  });

  it("does not auto-load another page on first paint when the initial grid already overflows", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(700);
    vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(
      1680,
    );
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: 0,
            top: 0,
            bottom: 1480,
            left: 0,
            right: 1200,
            width: 1200,
            height: 1480,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 20 }, (_, index) =>
          createConversation(`pck-overflow-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-overflow-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
      {
        hasMore: true,
        onLoadMore,
      },
    );

    act(() => {
      vi.runAllTimers();
    });

    expect(onLoadMore).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("backfills immediately on first paint when the initial grid is not scrollable yet", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
    vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(
      640,
    );
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: 0,
            top: 0,
            bottom: 640,
            left: 0,
            right: 1200,
            width: 1200,
            height: 640,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 4 }, (_, index) =>
          createConversation(`pck-underflow-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-underflow-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
      {
        hasMore: true,
        onLoadMore,
      },
    );

    act(() => {
      vi.runAllTimers();
    });

    expect(onLoadMore).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("does not eagerly prefetch on mount when the section starts below the fold", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: 1_120,
            top: 1_120,
            bottom: 1_760,
            left: 0,
            right: 1200,
            width: 1200,
            height: 640,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 4 }, (_, index) =>
          createConversation(`pck-below-fold-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-below-fold-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
      {
        hasMore: true,
        onLoadMore,
      },
    );

    act(() => {
      vi.runAllTimers();
    });

    expect(onLoadMore).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("continues initial load-more on mount when the page restores near the visible section bottom", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: -260,
            top: -260,
            bottom: 1_160,
            left: 0,
            right: 1200,
            width: 1200,
            height: 1_420,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 4 }, (_, index) =>
          createConversation(`pck-restored-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-restored-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
      {
        hasMore: true,
        onLoadMore,
      },
    );

    act(() => {
      vi.runAllTimers();
    });

    expect(onLoadMore).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("does not keep loading more after the section has scrolled above the viewport", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: -1_320,
            top: -1_320,
            bottom: -40,
            left: 0,
            right: 1200,
            width: 1200,
            height: 1_280,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 4 }, (_, index) =>
          createConversation(`pck-above-viewport-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-above-viewport-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
      {
        hasMore: true,
        onLoadMore,
      },
    );

    act(() => {
      vi.runAllTimers();
      window.dispatchEvent(new Event("scroll"));
    });

    expect(onLoadMore).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("falls back to downstream-facing diagnostics in the dashboard card summary", () => {
    renderSection(
      createResponse([
        createConversation("pck-downstream-dashboard", [
          createPreview({
            id: 9,
            invokeId: "invoke-downstream-dashboard",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "failed",
            failureClass: "client_abort",
            failureKind: "downstream_closed",
            downstreamStatusCode: 200,
            downstreamErrorMessage:
              "[downstream_closed] downstream closed while streaming upstream response",
          }),
        ]),
      ]),
    );

    const text = host?.textContent ?? "";
    expect(text).toContain(
      "[downstream_closed] downstream closed while streaming upstream response",
    );
  });

  it("renders a fixed previous-invocation placeholder when a conversation has only one call", () => {
    renderSection(
      createResponse([
        createConversation("pck-single", [
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );

    expect(placeholder).not.toBeNull();
    expect(placeholder?.textContent).toContain("上一条调用");
    expect(placeholder?.textContent).toContain("等高占位");
  });

  it("keeps the placeholder slot non-interactive when there is no previous invocation", () => {
    const onOpenInvocation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-single", [
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
      { onOpenInvocation },
    );

    const previousSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="previous"]',
    );
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    if (!(placeholder instanceof HTMLDivElement)) {
      throw new Error("missing placeholder");
    }

    act(() => {
      placeholder.click();
    });

    expect(previousSlot).toBeNull();
    expect(placeholder.getAttribute("role")).toBeNull();
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("renders interrupted slots with the dedicated interrupted label", () => {
    renderSection(
      createResponse([
        createConversation("pck-interrupted", [
          createPreview({
            id: 2,
            invokeId: "invoke-interrupted",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "interrupted",
            failureClass: "service_failure",
            failureKind: "proxy_interrupted",
            errorMessage:
              "proxy request was interrupted before completion and was recovered on startup",
          }),
          createPreview({
            id: 1,
            invokeId: "invoke-interrupted-old",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("已中断");
    expect(text).not.toContain("失败");
  });

  it("keeps upstream account buttons interactive so the shared drawer can open", () => {
    const onOpenUpstreamAccount = vi.fn();
    const onOpenInvocation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-account", [
          createPreview({
            id: 2,
            invokeId: "invoke-2",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            upstreamAccountId: 77,
            upstreamAccountName: "pool-account-77@example.com",
          }),
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
            upstreamAccountId: 77,
            upstreamAccountName: "pool-account-77@example.com",
          }),
        ]),
      ]),
      {
        onOpenUpstreamAccount,
        onOpenInvocation,
      },
    );

    const accountButton = Array.from(
      host?.querySelectorAll("button") ?? [],
    ).find((button) => {
      const text = button.textContent ?? "";
      const title = button.getAttribute("title") ?? "";
      return (
        text.includes("pool-account-77") ||
        title.includes("pool-account-77@example.com")
      );
    });
    if (!(accountButton instanceof HTMLButtonElement)) {
      throw new Error("missing account button");
    }

    act(() => {
      accountButton.click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(
      77,
      "pool-account-77@example.com",
    );
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("keeps the concrete upstream account label on assigned-account blocked dashboard cards", () => {
    renderSection(
      createResponse([
        createConversation("pck-assigned-account-blocked", [
          createPreview({
            id: 61,
            invokeId: "invoke-assigned-account-blocked",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "failed",
            failureClass: "service_failure",
            failureKind: "pool_assigned_account_blocked",
            errorMessage:
              '[pool_assigned_account_blocked] upstream account group "sticky-preflight-missing" has no bound forward proxy nodes',
            upstreamAccountId: 52,
            upstreamAccountName: "sticky-account-52@example.com",
            proxyDisplayName: "tokyo-edge-blocked",
            tUpstreamTtfbMs: null,
            tUpstreamStreamMs: null,
            tTotalMs: 42,
          }),
          createPreview({
            id: 60,
            invokeId: "invoke-assigned-account-blocked-previous",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
            upstreamAccountId: 52,
            upstreamAccountName: "sticky-account-52@example.com",
          }),
        ]),
      ]),
    );

    const accountLabel = host?.querySelector(
      '[title="sticky-account-52@example.com"]',
    );
    expect(accountLabel).not.toBeNull();
    expect(host?.textContent ?? "").not.toContain("未分配上游账号");
  });

  it("uses the unassigned-account fallback only for true no-account dashboard cards", () => {
    renderSection(
      createResponse([
        createConversation("pck-true-no-account", [
          createPreview({
            id: 71,
            invokeId: "invoke-true-no-account",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "failed",
            failureClass: "service_failure",
            failureKind: "pool_no_available_account",
            errorMessage:
              "[pool_no_available_account] no assignable upstream account remains",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: null,
            tUpstreamTtfbMs: null,
            tUpstreamStreamMs: null,
            tTotalMs: 38,
          }),
          createPreview({
            id: 70,
            invokeId: "invoke-true-no-account-previous",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: null,
          }),
        ]),
      ]),
    );

    const accountLabel = host?.querySelector('[title="未分配上游账号"]');
    expect(accountLabel).not.toBeNull();
  });

  it("opens invocation details from the slot container by click and keyboard", () => {
    const onOpenInvocation = vi.fn();
    const response = createResponse([
      createConversation("pck-slot-open", [
        createPreview({
          id: 2,
          invokeId: "invoke-slot-current",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
        createPreview({
          id: 1,
          invokeId: "invoke-slot-previous",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "completed",
        }),
      ]),
    ]);
    const cards = renderSection(response, { onOpenInvocation });

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLDivElement)) {
      throw new Error("missing current invocation slot");
    }

    expect(currentSlot.getAttribute("aria-label")).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(currentSlot.getAttribute("aria-label")).not.toContain("WC-");

    act(() => {
      currentSlot.click();
    });

    expect(onOpenInvocation).toHaveBeenCalledWith(
      expect.objectContaining({
        slotKind: "current",
        conversationSequenceId: cards[0]?.conversationSequenceId,
        promptCacheKey: "pck-slot-open",
      }),
    );
    expect(
      onOpenInvocation.mock.calls[0]?.[0]?.invocation?.record?.invokeId,
    ).toBe("invoke-slot-current");

    act(() => {
      currentSlot.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });

    expect(onOpenInvocation).toHaveBeenCalledTimes(2);
  });

  it("opens the conversation detail from the sequence id button only", () => {
    const onOpenInvocation = vi.fn();
    const onOpenConversation = vi.fn();
    const response = createResponse([
      createConversation("pck-sequence-open", [
        createPreview({
          id: 2,
          invokeId: "invoke-sequence-current",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
        createPreview({
          id: 1,
          invokeId: "invoke-sequence-previous",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "completed",
        }),
      ]),
    ]);
    const cards = renderSection(response, {
      onOpenConversation,
      onOpenInvocation,
    });

    const sequenceButton = host?.querySelector(
      '[data-testid="dashboard-working-conversation-sequence-button"]',
    );
    if (!(sequenceButton instanceof HTMLButtonElement)) {
      throw new Error("missing sequence button");
    }

    expect(sequenceButton.textContent).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(sequenceButton.getAttribute("aria-label")).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(sequenceButton.getAttribute("aria-label")).not.toContain(
      "invoke-sequence-current",
    );
    expect(sequenceButton.getAttribute("aria-label")).not.toContain(
      "invoke-sequence-previous",
    );
    expect(sequenceButton.getAttribute("aria-label")).toContain(
      "pck-sequence-open",
    );
    expect(sequenceButton.type).toBe("button");

    act(() => {
      sequenceButton.click();
    });

    expect(onOpenConversation).toHaveBeenCalledWith(
      expect.objectContaining({
        conversationSequenceId: cards[0]?.conversationSequenceId,
        promptCacheKey: "pck-sequence-open",
      }),
    );
    expect(onOpenConversation).toHaveBeenCalledTimes(1);
    expect(onOpenInvocation).not.toHaveBeenCalled();

    onOpenConversation.mockClear();
    onOpenInvocation.mockClear();

    const card = host?.querySelector(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing dashboard card");
    }

    const statusLabel = Array.from(card.querySelectorAll("span")).find(
      (node) => node.textContent === "运行中",
    );
    if (!(statusLabel instanceof HTMLElement)) {
      throw new Error("missing status label");
    }

    const requestMetric = Array.from(card.querySelectorAll("span")).find(
      (node) => node.textContent === "请求",
    );
    if (!(requestMetric instanceof HTMLElement)) {
      throw new Error("missing request metric");
    }

    act(() => {
      statusLabel.click();
      requestMetric.click();
      card.click();
    });

    expect(onOpenInvocation).not.toHaveBeenCalled();
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  it("uses theme-aware surface classes instead of a hardcoded dark canvas surface", () => {
    renderSection(
      createResponse([
        createConversation("pck-theme-aware", [
          createPreview({
            id: 1,
            invokeId: "invoke-theme-aware",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const card = host?.querySelector(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    const placeholderLine = host?.querySelector(
      ".working-conversation-placeholder-line",
    );

    expect(card?.className).toContain("working-conversation-card-surface");
    expect(card?.className).not.toContain("bg-[linear-gradient");
    expect(placeholder?.className).toContain(
      "working-conversation-slot-surface",
    );
    expect(placeholderLine).not.toBeNull();
  });

  it("keeps the wide-screen grid contract on the conversations section", () => {
    renderSection(
      createResponse([
        createConversation("pck-grid-one", [
          createPreview({
            id: 1,
            invokeId: "invoke-grid-one",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const grid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-grid"]',
    );

    expect(grid).not.toBeNull();
    expect(grid?.className).toContain("xl:grid-cols-2");
    expect(grid?.className).toContain("2xl:grid-cols-3");
    expect(grid?.className).toContain("desktop1660:grid-cols-4");
  });

  it("does not turn the conversations grid into an inner scrolling container", () => {
    renderSection(
      createResponse([
        createConversation("pck-page-scroll", [
          createPreview({
            id: 1,
            invokeId: "invoke-page-scroll",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const grid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-grid"]',
    );
    if (!(grid instanceof HTMLDivElement)) {
      throw new Error("missing working conversations grid");
    }

    expect(grid.className).not.toContain("overflow-auto");
    expect(grid.className).not.toContain("max-h-[72vh]");
  });

  it("renders only the virtualized rows instead of keeping every card mounted in the DOM", () => {
    virtualizerMocks.rowIndexes = [0, 1, 2, 3];
    virtualizerMocks.totalSize = 30 * 360;

    renderSection(
      createResponse(
        Array.from({ length: 30 }, (_, index) =>
          createConversation(`pck-virtual-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-virtual-${index + 1}`,
              occurredAt: `2026-04-04T10:${String((59 - index) % 60).padStart(2, "0")}:00Z`,
              status: index % 3 === 0 ? "running" : "completed",
            }),
          ]),
        ),
      ),
    );

    const renderedCards = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const renderedRows = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversations-row"]',
    );

    expect(renderedRows?.length).toBe(4);
    expect(renderedCards?.length).toBe(4);
    expect(renderedCards?.length).toBeLessThan(30);
  });

  it("subtracts scrollMargin when virtual rows expose document-based translateStart", () => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    virtualizerMocks.customVirtualItems = [
      {
        key: 1,
        index: 1,
        start: 600,
        size: 360,
        end: 960,
        translateStart: 600,
      },
    ];
    virtualizerMocks.totalSize = 960;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return {
            x: 0,
            y: 240,
            top: 240,
            bottom: 840,
            left: 0,
            right: 1200,
            width: 1200,
            height: 600,
            toJSON: () => ({}),
          } satisfies DOMRect;
        }
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
          width: 0,
          height: 0,
          toJSON: () => ({}),
        } satisfies DOMRect;
      },
    );

    renderSection(
      createResponse(
        Array.from({ length: 8 }, (_, index) =>
          createConversation(`pck-scroll-margin-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-scroll-margin-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "completed",
            }),
          ]),
        ),
      ),
    );

    const row = host?.querySelector(
      '[data-testid="dashboard-working-conversations-row"][data-row-index="1"]',
    );
    if (!(row instanceof HTMLElement)) {
      throw new Error("missing virtualized row");
    }

    expect(row.style.transform).toBe("translateY(360px)");
  });

  it("keeps the pre-measure fallback bounded before virtual rows are available", () => {
    virtualizerMocks.rowIndexes = [];
    virtualizerMocks.totalSize = 30 * 360;

    renderSection(
      createResponse(
        Array.from({ length: 30 }, (_, index) =>
          createConversation(`pck-fallback-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-fallback-${index + 1}`,
              occurredAt: `2026-04-04T10:${String((59 - index) % 60).padStart(2, "0")}:00Z`,
              status: index % 2 === 0 ? "running" : "completed",
            }),
          ]),
        ),
      ),
    );

    const renderedCards = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const renderedRows = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversations-row"]',
    );

    expect(renderedRows?.length).toBeLessThan(30);
    expect(renderedCards?.length).toBeLessThan(30);
  });

  it("reports the current virtualized depth instead of pinning refreshes to historical rows", () => {
    const setRefreshTargetCount = vi.fn();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    virtualizerMocks.rowIndexes = [0, 1, 2, 3, 4, 5];

    const response = createResponse(
      Array.from({ length: 32 }, (_, index) =>
        createConversation(`pck-depth-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-depth-${index + 1}`,
            occurredAt: `2026-04-04T10:${String((59 - index) % 60).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    );

    renderSection(response, { setRefreshTargetCount });
    expect(setRefreshTargetCount).toHaveBeenLastCalledWith(24);

    virtualizerMocks.rowIndexes = [0, 1, 2];
    rerenderSection(response, { setRefreshTargetCount });
    expect(setRefreshTargetCount).toHaveBeenLastCalledWith(20);
  });

  it("keeps page-scroll anchor compensation stable when display ids are renumbered by a new collision", () => {
    const baseCards = mapPromptCacheConversationsToDashboardCards(
      createResponse([
        createConversation("hidden-before", [
          createPreview({
            id: 1,
            invokeId: "invoke-hidden-before",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
          }),
        ]),
        createConversation("stable-anchor", [
          createPreview({
            id: 2,
            invokeId: "invoke-stable-anchor",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
        createConversation("new-head", [
          createPreview({
            id: 3,
            invokeId: "invoke-new-head",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const initialCards = [
      {
        ...baseCards[0]!,
        promptCacheKey: "hidden-before",
        normalizedPromptCacheKey: "hidden-before",
        conversationSequenceId: "WC-COLLIDE-A",
      },
      {
        ...baseCards[1]!,
        promptCacheKey: "stable-anchor",
        normalizedPromptCacheKey: "stable-anchor",
        conversationSequenceId: "WC-COLLIDE-B",
      },
    ] satisfies DashboardWorkingConversationCardModel[];

    const nextCards = [
      {
        ...baseCards[2]!,
        promptCacheKey: "new-head",
        normalizedPromptCacheKey: "new-head",
        conversationSequenceId: "WC-COLLIDE-A",
      },
      {
        ...baseCards[0]!,
        promptCacheKey: "hidden-before",
        normalizedPromptCacheKey: "hidden-before",
        conversationSequenceId: "WC-COLLIDE-A-1",
      },
      {
        ...baseCards[1]!,
        promptCacheKey: "stable-anchor",
        normalizedPromptCacheKey: "stable-anchor",
        conversationSequenceId: "WC-COLLIDE-B-1",
      },
    ] satisfies DashboardWorkingConversationCardModel[];

    const rectFor = (top: number, height = 160) =>
      ({
        x: 0,
        y: top,
        top,
        bottom: top + height,
        left: 0,
        right: 1200,
        width: 1200,
        height,
        toJSON: () => ({}),
      }) satisfies DOMRect;

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return rectFor(0, 600);
        }
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversation-card"
        ) {
          switch (this.getAttribute("data-conversation-sequence-id")) {
            case "COLLIDE-A":
              return rectFor(-220);
            case "COLLIDE-B":
              return rectFor(40);
            case "COLLIDE-A-1":
              return rectFor(-20);
            case "COLLIDE-B-1":
              return rectFor(220);
            default:
              return rectFor(720);
          }
        }
        return rectFor(0, 0);
      },
    );

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("preserves the page-scroll anchor even when virtualization has pruned rows above the viewport", () => {
    virtualizerMocks.rowIndexes = [2, 3, 4, 5];
    virtualizerMocks.totalSize = 12 * 360;

    const baseCards = mapPromptCacheConversationsToDashboardCards(
      createResponse(
        Array.from({ length: 8 }, (_, index) =>
          createConversation(`pruned-row-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-pruned-row-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "running",
            }),
          ]),
        ),
      ),
    );

    const initialCards = baseCards.map((card, index) => ({
      ...card,
      promptCacheKey: `pruned-row-${index + 1}`,
      normalizedPromptCacheKey: `pruned-row-${index + 1}`,
      conversationSequenceId: `WC-PRUNED-${index + 1}`,
    })) satisfies DashboardWorkingConversationCardModel[];

    const insertedHead = {
      ...baseCards[0]!,
      promptCacheKey: "pruned-new-head",
      normalizedPromptCacheKey: "pruned-new-head",
      conversationSequenceId: "WC-PRUNED-NEW",
    } satisfies DashboardWorkingConversationCardModel;

    const nextCards = [
      insertedHead,
      ...initialCards.map((card, index) => ({
        ...card,
        conversationSequenceId: `WC-PRUNED-NEXT-${index + 1}`,
      })),
    ] satisfies DashboardWorkingConversationCardModel[];

    const rectFor = (top: number, height = 160) =>
      ({
        x: 0,
        y: top,
        top,
        bottom: top + height,
        left: 0,
        right: 1200,
        width: 1200,
        height,
        toJSON: () => ({}),
      }) satisfies DOMRect;

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return rectFor(0, 600);
        }
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversation-card"
        ) {
          switch (this.getAttribute("data-conversation-sequence-id")) {
            case "PRUNED-3":
              return rectFor(40);
            case "PRUNED-4":
              return rectFor(220);
            case "PRUNED-5":
              return rectFor(400);
            case "PRUNED-6":
              return rectFor(580);
            case "PRUNED-NEXT-3":
              return rectFor(220);
            case "PRUNED-NEXT-4":
              return rectFor(400);
            case "PRUNED-NEXT-5":
              return rectFor(580);
            case "PRUNED-NEXT-6":
              return rectFor(760);
            default:
              return rectFor(920);
          }
        }
        return rectFor(0, 0);
      },
    );

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("preserves the page-scroll anchor when the first visible card is partially clipped", () => {
    const baseCards = mapPromptCacheConversationsToDashboardCards(
      createResponse(
        Array.from({ length: 3 }, (_, index) =>
          createConversation(`partial-anchor-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-partial-anchor-${index + 1}`,
              occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
              status: "running",
            }),
          ]),
        ),
      ),
    );

    const initialCards = baseCards.map((card, index) => ({
      ...card,
      promptCacheKey: `partial-anchor-${index + 1}`,
      normalizedPromptCacheKey: `partial-anchor-${index + 1}`,
      conversationSequenceId: `WC-PARTIAL-${index + 1}`,
    })) satisfies DashboardWorkingConversationCardModel[];

    const insertedHead = {
      ...baseCards[0]!,
      promptCacheKey: "partial-anchor-new-head",
      normalizedPromptCacheKey: "partial-anchor-new-head",
      conversationSequenceId: "WC-PARTIAL-NEW",
    } satisfies DashboardWorkingConversationCardModel;

    const nextCards = [
      insertedHead,
      ...initialCards.map((card, index) => ({
        ...card,
        conversationSequenceId: `WC-PARTIAL-NEXT-${index + 1}`,
      })),
    ] satisfies DashboardWorkingConversationCardModel[];

    const rectFor = (top: number, height = 160) =>
      ({
        x: 0,
        y: top,
        top,
        bottom: top + height,
        left: 0,
        right: 1200,
        width: 1200,
        height,
        toJSON: () => ({}),
      }) satisfies DOMRect;

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return rectFor(0, 600);
        }
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversation-card"
        ) {
          switch (this.getAttribute("data-conversation-sequence-id")) {
            case "PARTIAL-1":
              return rectFor(-24);
            case "PARTIAL-2":
              return rectFor(180);
            case "PARTIAL-3":
              return rectFor(360);
            case "PARTIAL-NEXT-1":
              return rectFor(156);
            case "PARTIAL-NEXT-2":
              return rectFor(360);
            case "PARTIAL-NEXT-3":
              return rectFor(540);
            default:
              return rectFor(720);
          }
        }
        return rectFor(0, 0);
      },
    );

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("does not preserve an anchor when new cards arrive while the list top is still visible", () => {
    const baseCards = mapPromptCacheConversationsToDashboardCards(
      createResponse([
        createConversation("visible-top", [
          createPreview({
            id: 1,
            invokeId: "invoke-visible-top",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
        createConversation("visible-second", [
          createPreview({
            id: 2,
            invokeId: "invoke-visible-second",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "running",
          }),
        ]),
        createConversation("visible-new-head", [
          createPreview({
            id: 3,
            invokeId: "invoke-visible-new-head",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const initialCards = [
      {
        ...baseCards[0]!,
        promptCacheKey: "visible-top",
        normalizedPromptCacheKey: "visible-top",
        conversationSequenceId: "WC-VISIBLE-A",
      },
      {
        ...baseCards[1]!,
        promptCacheKey: "visible-second",
        normalizedPromptCacheKey: "visible-second",
        conversationSequenceId: "WC-VISIBLE-B",
      },
    ] satisfies DashboardWorkingConversationCardModel[];

    const nextCards = [
      {
        ...baseCards[2]!,
        promptCacheKey: "visible-new-head",
        normalizedPromptCacheKey: "visible-new-head",
        conversationSequenceId: "WC-VISIBLE-C",
      },
      {
        ...baseCards[0]!,
        promptCacheKey: "visible-top",
        normalizedPromptCacheKey: "visible-top",
        conversationSequenceId: "WC-VISIBLE-A-1",
      },
      {
        ...baseCards[1]!,
        promptCacheKey: "visible-second",
        normalizedPromptCacheKey: "visible-second",
        conversationSequenceId: "WC-VISIBLE-B-1",
      },
    ] satisfies DashboardWorkingConversationCardModel[];

    const rectFor = (top: number, height = 160) =>
      ({
        x: 0,
        y: top,
        top,
        bottom: top + height,
        left: 0,
        right: 1200,
        width: 1200,
        height,
        toJSON: () => ({}),
      }) satisfies DOMRect;

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversations-grid"
        ) {
          return rectFor(0, 600);
        }
        if (
          this.getAttribute("data-testid") ===
          "dashboard-working-conversation-card"
        ) {
          switch (this.getAttribute("data-conversation-sequence-id")) {
            case "VISIBLE-A":
              return rectFor(40);
            case "VISIBLE-B":
              return rectFor(220);
            case "VISIBLE-C":
              return rectFor(40);
            case "VISIBLE-A-1":
              return rectFor(220);
            case "VISIBLE-B-1":
              return rectFor(400);
            default:
              return rectFor(720);
          }
        }
        return rectFor(0, 0);
      },
    );

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).not.toHaveBeenCalled();
  });
});
