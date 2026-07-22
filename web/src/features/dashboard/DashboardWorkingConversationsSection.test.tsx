/** @vitest-environment jsdom */

import { fireEvent, waitFor } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import {
  type DashboardWorkingConversationCardModel,
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
  mapPromptCacheConversationsToDashboardCards,
} from "../../lib/dashboardWorkingConversations";
import { ThemeProvider } from "../../theme";
import {
  type DashboardOpenUpstreamAccountOptions,
  DashboardWorkingConversationsSection,
} from "./DashboardWorkingConversationsSection";
import {
  DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY,
  readPersistedDashboardWorkspaceView,
} from "./dashboardActivityRange";

const LONG_ERROR_SUMMARY =
  '[upstream_http_5xx] pool upstream responded with 502: {"error":{"message":"Upstream request failed","type":"upstream_error"}} event: response.failed data: {"type":"response.failed","response":{"id":"resp_test_error_summary","model":"gpt-5.4","status":"failed"}}';

class MockPointerEvent extends MouseEvent {
  pointerType: string;

  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init);
    this.pointerType = init.pointerType ?? "mouse";
  }
}

const virtualizerMocks = vi.hoisted(() => ({
  rowIndexes: null as number[] | null,
  totalSize: null as number | null,
  customVirtualItems: null as Array<{
    key: number;
    index: number;
    start: number;
    size: number;
    end: number;
    translateStart?: number;
  }> | null,
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
  isRefreshing: false,
  recentLoading: false,
  recentError: null as string | null,
  error: null as string | null,
  resolvedRecentInvocationLimit: null as number | null,
  calls: [] as Array<{
    range: string;
    enabled: boolean;
    recentInvocationLimit?: number;
  }>,
}));

vi.mock("../../hooks/useDashboardUpstreamAccountActivity", () => ({
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
      isRefreshing: upstreamAccountActivityMock.isRefreshing,
      recentLoading: upstreamAccountActivityMock.recentLoading,
      recentError: upstreamAccountActivityMock.recentError,
      error: upstreamAccountActivityMock.error,
      recentInvocationLimit:
        upstreamAccountActivityMock.resolvedRecentInvocationLimit ??
        recentInvocationLimit ??
        upstreamAccountActivityMock.data?.accounts[0]?.recentInvocations.length ??
        4,
      hasActivated: enabled,
      reload: vi.fn(),
      retryRecent: vi.fn(),
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
    promptCacheKey: "promptCacheKey" in overrides ? (overrides.promptCacheKey ?? null) : null,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    requestModel: "requestModel" in overrides ? (overrides.requestModel ?? null) : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 200,
    cost: overrides.cost ?? 0.02,
    proxyDisplayName:
      "proxyDisplayName" in overrides ? (overrides.proxyDisplayName ?? null) : "tokyo-edge-01",
    upstreamAccountId:
      "upstreamAccountId" in overrides ? (overrides.upstreamAccountId ?? null) : 42,
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
    blockedBinding: "blockedBinding" in overrides ? (overrides.blockedBinding ?? null) : undefined,
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
    totalTokens: overrides.totalTokens ?? 200,
    totalCost: overrides.totalCost ?? 0.02,
    createdAt:
      overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt:
      overrides.lastActivityAt ?? recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
    lastTerminalAt: overrides.lastTerminalAt ?? null,
    lastInFlightAt: overrides.lastInFlightAt ?? null,
    hasEncryptedSessionOwner: overrides.hasEncryptedSessionOwner ?? false,
    encryptedOwnerAccountId: overrides.encryptedOwnerAccountId ?? null,
    encryptedOwnerAccountName: overrides.encryptedOwnerAccountName ?? null,
    encryptedOwnerGroupName: overrides.encryptedOwnerGroupName ?? null,
    manualBinding: overrides.manualBinding ?? null,
    blockedBinding: "blockedBinding" in overrides ? (overrides.blockedBinding ?? null) : undefined,
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

function createUpstreamAccountActivityResponse(): UpstreamAccountActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    networkLiveBucket: {
      bucketStart: "2026-04-04T10:00:00Z",
      bucketEnd: "2026-04-04T10:05:00Z",
      uploadBytesPerSecond: 1_536,
      downloadBytesPerSecond: 5 * 1024 * 1024,
      uploadBytes: 1_536 * 300,
      downloadBytes: 5 * 1024 * 1024 * 300,
      isLiveBucket: true,
    },
    networkRealtimeRate: {
      sampleStart: "2026-04-04T10:04:59Z",
      sampleEnd: "2026-04-04T10:05:00Z",
      sampleSeconds: 1,
      uploadBytesPerSecond: 1_536,
      downloadBytesPerSecond: 5 * 1024 * 1024,
      uploadBytes: 1_536,
      downloadBytes: 5 * 1024 * 1024,
    },
    accounts: [
      {
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        groupName: "Primary",
        planType: "enterprise",
        enabled: true,
        displayStatus: "upstream_rejected",
        enableStatus: "enabled",
        workStatus: "rate_limited",
        healthStatus: "upstream_rejected",
        syncState: "idle",
        lastError: "upstream rejected",
        lastActionReasonMessage: "上游拒绝最近一次路由请求",
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
        usageBreakdown: {
          cacheWriteTokens: 1600,
          cacheReadTokens: 800,
          outputTokens: 800,
          costs: {
            input: 0.18,
            cacheWrite: 0.14,
            cacheRead: 0.06,
            output: 0.28,
            reasoning: 0.06,
            unknown: 0,
          },
          models: [
            {
              model: "gpt-5.6",
              cacheWriteTokens: 1200,
              cacheReadTokens: 600,
              outputTokens: 620,
              costs: {
                input: 0.12,
                cacheWrite: 0.1,
                cacheRead: 0.04,
                output: 0.21,
                reasoning: 0.05,
                unknown: 0,
              },
            },
            {
              model: "gpt-5.4-mini",
              cacheWriteTokens: 400,
              cacheReadTokens: 200,
              outputTokens: 180,
              costs: {
                input: 0.06,
                cacheWrite: 0.04,
                cacheRead: 0.02,
                output: 0.07,
                reasoning: 0.01,
                unknown: 0,
              },
            },
          ],
        },
        cacheHitRate: 0.25,
        tokensPerMinute: 640,
        spendRate: 0.12,
        firstByteAvgMs: 420,
        firstResponseByteTotalAvgMs: 2_867.5,
        avgTotalMs: 860,
        currentFirstResponseByteTotalAvgMs: 2_867.5,
        currentAvgTotalMs: 860,
        inProgressInvocationCount: 3,
        inProgressPhaseCounts: { queued: 1, requesting: 1, responding: 1 },
        retryInvocationCount: 1,
        uploadBytesPerSecond: 1_536,
        downloadBytesPerSecond: 5 * 1024 * 1024,
        effectiveRoutingRule: {
          allowCutOut: true,
          allowCutIn: false,
          priorityTier: "no_new",
          fastModeRewriteMode: "force_add",
          imageToolRewriteMode: "keep_original",
          concurrencyLimit: 3,
          upstream429RetryEnabled: false,
          upstream429MaxRetries: 0,
          availableModels: [],
          availableModelsDefined: false,
          systemDeniedModels: [],
          sourceTagIds: [],
          sourceTagNames: [],
          fieldSources: {
            allowCutOut: "root",
            allowCutIn: "account",
            priorityTier: "group",
            fastModeRewriteMode: "account",
            imageToolRewriteMode: "root",
            concurrencyLimit: "group",
            upstream429Retry: "root",
            availableModels: "root",
            systemDeniedModels: "root",
          },
          timeouts: {
            responsesFirstByteTimeoutSecs: 120,
            compactFirstByteTimeoutSecs: 120,
            responsesStreamTimeoutSecs: 600,
            compactStreamTimeoutSecs: 600,
          },
          timeoutFieldSources: {
            responsesFirstByteTimeoutSecs: "root",
            compactFirstByteTimeoutSecs: "root",
            responsesStreamTimeoutSecs: "root",
            compactStreamTimeoutSecs: "root",
          },
        },
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

const BULK_BINDING_ACCOUNTS = [
  {
    id: 21,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "growth.6vv4@relay.example",
    groupName: "CIII",
    status: "active",
    displayStatus: "active",
    enabled: true,
  },
  {
    id: 101,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "Codex Pro - Tokyo",
    groupName: "Tokyo",
    status: "active",
    displayStatus: "active",
    enabled: true,
  },
] as const;

function createBulkConversationFetchMock(options?: {
  failKeys?: string[];
  onBulkPayload?: (payload: Record<string, unknown>) => void;
  groups?: Array<{ groupName: string; accountCount: number }>;
  accounts?: ReadonlyArray<(typeof BULK_BINDING_ACCOUNTS)[number]>;
}) {
  const failKeys = new Set(options?.failKeys ?? []);
  const accounts = options?.accounts ?? BULK_BINDING_ACCOUNTS;
  const groups = options?.groups ?? [
    { groupName: "CIII", accountCount: 1 },
    { groupName: "Tokyo", accountCount: 1 },
  ];
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const url = new URL(request, "http://localhost");
    if (url.pathname === "/api/pool/upstream-accounts") {
      return new Response(
        JSON.stringify({
          writesEnabled: true,
          items: accounts,
          groups,
          forwardProxyNodes: [],
          hasUngroupedAccounts: false,
          total: accounts.length,
          page: 1,
          pageSize: accounts.length,
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    }
    if (url.pathname === "/api/stats/prompt-cache-conversation-bindings/bulk-actions") {
      const payload = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {};
      options?.onBulkPayload?.(payload);
      const promptCacheKeys = Array.isArray(payload.promptCacheKeys)
        ? payload.promptCacheKeys.map((value) => String(value))
        : [];
      const items = promptCacheKeys.map((promptCacheKey) => {
        if (failKeys.has(promptCacheKey)) {
          return {
            promptCacheKey,
            ok: false,
            error: "synthetic failure",
            binding: null,
          };
        }
        return {
          promptCacheKey,
          ok: true,
          error: null,
          binding: {
            promptCacheKey,
            bindingKind: payload.action === "bind" ? (payload.bindingKind ?? "group") : "none",
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
            allowSwitchUpstream: null,
            fastModeRewriteMode:
              payload.action === "setFastModeRewriteMode"
                ? (payload.fastModeRewriteMode ?? "keep_original")
                : null,
            imageToolRewriteMode: null,
            availableModels: null,
            forwardProxyKey: null,
            forwardProxyKeys: [],
            timeouts: {
              responsesFirstByteTimeoutSecs: 120,
              compactFirstByteTimeoutSecs: 120,
              imageFirstByteTimeoutSecs: 120,
              responsesStreamTimeoutSecs: 300,
              compactStreamTimeoutSecs: 300,
            },
            timeoutFieldSources: {
              responsesFirstByteTimeoutSecs: "account",
              compactFirstByteTimeoutSecs: "account",
              imageFirstByteTimeoutSecs: "account",
              responsesStreamTimeoutSecs: "account",
              compactStreamTimeoutSecs: "account",
            },
            policyFieldSources: {
              allowSwitchUpstream: "account",
              fastModeRewriteMode: "conversation",
              imageToolRewriteMode: "account",
              availableModels: "account",
              forwardProxyKey: "account",
            },
            updatedAt: "2026-05-12T16:20:00Z",
          },
        };
      });
      const succeededCount = items.filter((item) => item.ok).length;
      return new Response(
        JSON.stringify({
          action: payload.action ?? "bind",
          totalRequested: items.length,
          totalSucceeded: succeededCount,
          totalFailed: items.length - succeededCount,
          items,
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    }
    throw new Error(`Unhandled fetch request: ${url.pathname}`);
  });
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
  Object.defineProperty(window, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(globalThis, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    writable: true,
    value: () => false,
  });
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(window, "scrollBy", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
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
  upstreamAccountActivityMock.isRefreshing = false;
  upstreamAccountActivityMock.recentLoading = false;
  upstreamAccountActivityMock.recentError = null;
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
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: DashboardOpenUpstreamAccountOptions,
    ) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
      tab?: "overview" | "calls" | "settings";
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
    upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
    upstreamAccountActivityLoading?: boolean;
    upstreamAccountActivityRefreshing?: boolean;
    upstreamAccountActivityError?: string | null;
    upstreamAccountRecentLoading?: boolean;
    upstreamAccountRecentError?: string | null;
    upstreamAccountRecentPreviewLimit?: number;
    onConversationsChanged?: () => void;
    activeBlockedBindingFilter?: {
      upstreamAccountId?: number | null;
      constraintSource?: "upstreamAccountBinding" | "encryptedSessionOwner" | null;
    } | null;
    onClearBlockedBindingFilter?: () => void;
  },
) {
  return renderSectionWithCards(mapPromptCacheConversationsToDashboardCards(response), options);
}

function renderSectionWithCards(
  cards: DashboardWorkingConversationCardModel[],
  options?: {
    activeRange?: "today" | "yesterday" | "1d" | "7d" | "usage";
    error?: string | null;
    isLoading?: boolean;
    isLoadingMore?: boolean;
    hasMore?: boolean;
    totalMatched?: number;
    onLoadMore?: () => void;
    setRefreshTargetCount?: (count: number) => void;
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: DashboardOpenUpstreamAccountOptions,
    ) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
      tab?: "overview" | "calls" | "settings";
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
    upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
    upstreamAccountActivityLoading?: boolean;
    upstreamAccountActivityRefreshing?: boolean;
    upstreamAccountActivityError?: string | null;
    upstreamAccountRecentLoading?: boolean;
    upstreamAccountRecentError?: string | null;
    upstreamAccountRecentPreviewLimit?: number;
    activeBlockedBindingFilter?: {
      upstreamAccountId?: number | null;
      constraintSource?: "upstreamAccountBinding" | "encryptedSessionOwner" | null;
    } | null;
    onClearBlockedBindingFilter?: () => void;
  },
) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <ThemeProvider>
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
            upstreamAccountActivity={options?.upstreamAccountActivity}
            upstreamAccountActivityLoading={options?.upstreamAccountActivityLoading}
            upstreamAccountActivityRefreshing={options?.upstreamAccountActivityRefreshing}
            upstreamAccountActivityError={options?.upstreamAccountActivityError}
            upstreamAccountRecentLoading={options?.upstreamAccountRecentLoading}
            upstreamAccountRecentError={options?.upstreamAccountRecentError}
            upstreamAccountRecentPreviewLimit={options?.upstreamAccountRecentPreviewLimit}
            onConversationsChanged={options?.onConversationsChanged}
            activeBlockedBindingFilter={options?.activeBlockedBindingFilter}
            onClearBlockedBindingFilter={options?.onClearBlockedBindingFilter}
          />
        </I18nProvider>
      </ThemeProvider>,
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
      host?.querySelector('[data-testid="dashboard-working-conversation-model-routing-indicator"]'),
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
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: DashboardOpenUpstreamAccountOptions,
    ) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
      tab?: "overview" | "calls" | "settings";
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
    upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
    upstreamAccountActivityLoading?: boolean;
    upstreamAccountActivityRefreshing?: boolean;
    upstreamAccountActivityError?: string | null;
    upstreamAccountRecentLoading?: boolean;
    upstreamAccountRecentError?: string | null;
    upstreamAccountRecentPreviewLimit?: number;
    onConversationsChanged?: () => void;
    activeBlockedBindingFilter?: {
      upstreamAccountId?: number | null;
      constraintSource?: "upstreamAccountBinding" | "encryptedSessionOwner" | null;
    } | null;
    onClearBlockedBindingFilter?: () => void;
  },
) {
  return rerenderSectionWithCards(mapPromptCacheConversationsToDashboardCards(response), options);
}

function rerenderSectionWithCards(
  cards: DashboardWorkingConversationCardModel[],
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
    onOpenUpstreamAccount?: (
      accountId: number,
      accountLabel: string,
      options?: DashboardOpenUpstreamAccountOptions,
    ) => void;
    onOpenConversation?: (selection: {
      conversationSequenceId: string;
      promptCacheKey: string;
      tab?: "overview" | "calls" | "settings";
    }) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
    upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
    upstreamAccountActivityLoading?: boolean;
    upstreamAccountActivityRefreshing?: boolean;
    upstreamAccountActivityError?: string | null;
    upstreamAccountRecentLoading?: boolean;
    upstreamAccountRecentError?: string | null;
    upstreamAccountRecentPreviewLimit?: number;
    onConversationsChanged?: () => void;
    activeBlockedBindingFilter?: {
      upstreamAccountId?: number | null;
      constraintSource?: "upstreamAccountBinding" | "encryptedSessionOwner" | null;
    } | null;
    onClearBlockedBindingFilter?: () => void;
  },
) {
  if (!root) {
    throw new Error("renderSection must run before rerenderSection");
  }
  act(() => {
    root?.render(
      <ThemeProvider>
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
            upstreamAccountActivity={options?.upstreamAccountActivity}
            upstreamAccountActivityLoading={options?.upstreamAccountActivityLoading}
            upstreamAccountActivityRefreshing={options?.upstreamAccountActivityRefreshing}
            upstreamAccountActivityError={options?.upstreamAccountActivityError}
            upstreamAccountRecentLoading={options?.upstreamAccountRecentLoading}
            upstreamAccountRecentError={options?.upstreamAccountRecentError}
            upstreamAccountRecentPreviewLimit={options?.upstreamAccountRecentPreviewLimit}
            onConversationsChanged={options?.onConversationsChanged}
            activeBlockedBindingFilter={options?.activeBlockedBindingFilter}
            onClearBlockedBindingFilter={options?.onClearBlockedBindingFilter}
          />
        </I18nProvider>
      </ThemeProvider>,
    );
  });
  return cards;
}

describe("DashboardWorkingConversationsSection", () => {
  it("shows account skeletons immediately without flashing the empty state", () => {
    upstreamAccountActivityMock.data = null;
    upstreamAccountActivityMock.isLoading = false;

    renderSection(createResponse([]));
    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-grid-skeleton"]'),
    ).not.toBeNull();
    expect(host?.textContent).toContain("账号加载中");
    expect(host?.textContent).not.toContain("当前范围内暂无活动上游账号");
  });

  it("keeps hydrated recent rows visible while a background recent refresh is pending", () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    upstreamAccountActivityMock.recentLoading = true;

    renderSection(createResponse([]));
    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(host?.textContent).toContain("acct-invoke-1");
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-recent-skeleton"]'),
    ).toBeNull();
  });

  it("does not render the header refresh status during short background account refreshes", async () => {
    vi.useFakeTimers();
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    upstreamAccountActivityMock.isRefreshing = true;

    renderSection(createResponse([]));
    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const accountGrid = host?.querySelector('[data-testid="dashboard-upstream-account-grid"]');
    if (!(accountGrid instanceof HTMLElement)) {
      throw new Error("missing account grid");
    }

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-refresh-status"]'),
    ).toBeNull();
    expect(
      Array.from(host?.querySelectorAll('[role="status"]') ?? []).some(
        (element) =>
          element.closest('[data-testid="dashboard-upstream-account-refresh-status"]') == null,
      ),
    ).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(299);
    });

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-refresh-status"]'),
    ).toBeNull();
    expect(
      Array.from(host?.querySelectorAll('[role="status"]') ?? []).some(
        (element) =>
          element.closest('[data-testid="dashboard-upstream-account-refresh-status"]') == null,
      ),
    ).toBe(false);

    vi.useRealTimers();
  });

  it("shows the header refresh status only after the delay and keeps it visible briefly after refresh ends", async () => {
    vi.useFakeTimers();
    const response = createResponse([]);
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    upstreamAccountActivityMock.isRefreshing = true;

    renderSection(response);
    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const accountGrid = host?.querySelector('[data-testid="dashboard-upstream-account-grid"]');
    if (!(accountGrid instanceof HTMLElement)) {
      throw new Error("missing account grid");
    }

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });

    const refreshStatus = host?.querySelector(
      '[data-testid="dashboard-upstream-account-refresh-status"]',
    );
    const refreshText = host?.querySelector(
      '[data-testid="dashboard-upstream-account-refresh-text"]',
    );
    const refreshSpinner = host?.querySelector(
      '[data-testid="dashboard-upstream-account-refresh-spinner"]',
    );
    if (
      !(refreshStatus instanceof HTMLElement) ||
      !(refreshText instanceof HTMLElement) ||
      !(refreshSpinner instanceof HTMLElement)
    ) {
      throw new Error("missing refresh status");
    }

    expect(refreshStatus.getAttribute("aria-label")).toBe("正在更新账号汇总");
    expect(refreshText.textContent).toBe("刷新中");
    expect(refreshText.className).toContain("hidden");
    expect(refreshText.className).toContain("desktop:inline");
    expect(refreshStatus.className).not.toContain("rounded-full");
    expect(refreshStatus.className).not.toContain("border");
    expect(
      Array.from(host?.querySelectorAll('[role="status"]') ?? []).filter(
        (element) =>
          element.closest('[data-testid="dashboard-upstream-account-refresh-status"]') != null,
      ),
    ).toHaveLength(1);

    upstreamAccountActivityMock.isRefreshing = false;
    rerenderSection(response);

    expect(host?.querySelector('[data-testid="dashboard-upstream-account-refresh-status"]')).toBe(
      refreshStatus,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(599);
    });

    expect(host?.querySelector('[data-testid="dashboard-upstream-account-refresh-status"]')).toBe(
      refreshStatus,
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-refresh-status"]'),
    ).toBeNull();

    vi.useRealTimers();
  });

  it("stacks workspace controls below the description on compact screens", () => {
    renderSection(
      createResponse([
        createConversation("pck-compact-header", [
          createPreview({
            id: 70,
            invokeId: "invoke-compact-header",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const controls = host?.querySelector(
      '[data-testid="dashboard-working-conversations-controls"]',
    );
    if (!(controls instanceof HTMLElement)) {
      throw new Error("missing workspace controls");
    }
    expect(controls.className).toContain("flex-col");
    expect(controls.querySelector('[role="tablist"]')?.className).toContain("w-full");
    expect(controls.querySelectorAll('[role="tab"]')[0]?.className).toContain("flex-1");
    expect(controls.querySelectorAll('[role="tab"]')[1]?.className).toContain("flex-1");
  });

  it("keeps workspace tabs before badge and sort controls", () => {
    renderSection(
      createResponse([
        createConversation("pck-header-order", [
          createPreview({
            id: 71,
            invokeId: "invoke-header-order",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const controls = host?.querySelector(
      '[data-testid="dashboard-working-conversations-controls"]',
    );
    if (!(controls instanceof HTMLElement)) {
      throw new Error("missing workspace controls");
    }

    const firstChild = controls.firstElementChild;
    const secondChild = controls.children.item(1);
    expect(firstChild?.getAttribute("role")).toBe("tablist");
    expect(secondChild?.getAttribute("data-testid")).toBe(
      "dashboard-working-conversations-actions",
    );
    expect(
      secondChild?.querySelector('[data-testid="dashboard-workspace-sort-button"]'),
    ).not.toBeNull();
    expect(secondChild?.textContent).toContain("当前对话 1 条");
  });

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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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
    expect(host?.textContent).toContain("最近 4 条调用");
    const totalNetworkSpeed = host?.querySelector(
      '[data-testid="dashboard-upstream-account-total-network-speed"]',
    );
    expect(totalNetworkSpeed).not.toBeNull();
    expect(totalNetworkSpeed?.textContent).toContain("1.5");
    expect(totalNetworkSpeed?.textContent).toContain("KiB/s");
    expect(totalNetworkSpeed?.textContent).toContain("5");
    expect(totalNetworkSpeed?.textContent).toContain("MiB/s");
    expect(host?.textContent).not.toContain("账号状态");
    expect(host?.querySelector('[data-testid="dashboard-upstream-account-status"]')).toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-card"]')?.className,
    ).toContain("desktop1660:min-h-[31.5rem]");
    expect(host?.textContent).not.toContain("繁忙");
    expect(host?.textContent).not.toContain("关注");
    expect(host?.textContent).not.toContain("稳定");
    expect(host?.textContent).not.toContain("渠道 Pool Alpha");
    expect(host?.textContent).not.toContain("Primary");
    expect(host?.textContent).not.toContain("按调用计数，不按对话去重");
    expect(host?.textContent).not.toContain("仍在重试链路中的调用");
    expect(host?.textContent).not.toContain("最近 4 条调用里仍有活动或异常");
    expect(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]').length,
    ).toBe(4);
    expect(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-identity-chip"]')
        .length,
    ).toBe(4);
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-header-row"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-header-row"]')?.textContent,
    ).not.toContain("#42");
    const firstRecentRow = host?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-row"]',
    );
    expect(firstRecentRow?.textContent).toContain("Responses");
    expect(firstRecentRow?.textContent).toContain("Token 200");
    expect(firstRecentRow?.textContent).not.toContain("RQ ");
    expect(firstRecentRow?.textContent).not.toContain("UP ");
    expect(firstRecentRow?.textContent).not.toContain("ED ");
    expect(
      firstRecentRow?.querySelector('[data-testid="dashboard-compact-latency-first-byte"]'),
    ).not.toBeNull();
    expect(
      firstRecentRow?.querySelector('[data-testid="dashboard-compact-latency-response-time"]'),
    ).not.toBeNull();
    expect(
      firstRecentRow?.querySelector('[data-testid="dashboard-compact-latency-first-byte"]')
        ?.className,
    ).not.toMatch(/rounded|border|bg-/);
    expect(
      firstRecentRow?.querySelector('[data-testid="dashboard-compact-latency-response-time"]')
        ?.className,
    ).not.toMatch(/rounded|border|bg-/);
    expect(
      firstRecentRow
        ?.querySelector('[data-testid="dashboard-compact-latency-pills"]')
        ?.getAttribute("aria-label"),
    ).toMatch(/首字用时|Time to first byte/i);

    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-live-call-breakdown"]'),
    ).toBeNull();
    const accountHeader = host?.querySelector(
      '[data-testid="dashboard-upstream-account-header-row"]',
    );
    const accountHeaderText = accountHeader?.textContent;
    expect(accountHeaderText).toContain("3");
    expect(accountHeaderText).not.toContain("并行对话");
    expect(accountHeaderText).not.toContain("重试");
    const headerPlanBadge = accountHeader?.querySelector(
      ".upstream-plan-badge[data-plan='enterprise']",
    );
    expect(headerPlanBadge?.textContent).toBe("Ent");
    expect(accountHeader?.querySelector('[aria-label="进行中 3"]')).not.toBeNull();
    expect(accountHeader?.querySelector('[aria-label="TPM 640"]')).not.toBeNull();
    expect(accountHeader?.querySelector('[aria-label="消费速率 0.12"]')).not.toBeNull();
    expect(
      accountHeader?.querySelector('[data-testid="dashboard-upstream-account-routing-settings"]'),
    ).toBeNull();
    const networkSpeed = accountHeader?.querySelector(
      '[data-testid="dashboard-upstream-account-network-speed"]',
    );
    expect(networkSpeed).toBeNull();
    const inProgressMetric = accountHeader?.querySelector('[aria-label="进行中 3"]');
    expect(inProgressMetric).toBeInstanceOf(HTMLElement);
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
    expect(accountCardText).toContain("0.72");
    expect(accountCardText).not.toContain("$0.72");
    expect(costBreakdown?.textContent).toContain("$0.22");
    expect(costBreakdown?.textContent).toContain("30.6%");
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-cost-icon"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="dashboard-upstream-account-token-icon"]'),
    ).not.toBeNull();

    const tokenBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-token-breakdown"]',
    );
    expect(accountCardText).toContain("3,200");
    expect(tokenBreakdown?.textContent).toContain("25%");
    expect(tokenBreakdown?.textContent).toContain("350");

    const recentBreakdown = host?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-breakdown"]',
    );
    expect(recentBreakdown?.textContent).toContain("排队中");
    expect(recentBreakdown?.textContent).toContain("请求中");
    expect(recentBreakdown?.textContent).toContain("响应中");
    expect(recentBreakdown?.textContent).toContain("失败");
    expect(recentBreakdown?.textContent).toContain("成功");
    expect(recentBreakdown?.textContent).toContain("6");
    expect(recentBreakdown?.textContent).toContain("1");
    const phaseSegments = Array.from(
      recentBreakdown?.querySelectorAll('[data-testid="invocation-phase-segment"]') ?? [],
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
  });

  it("shows aggregated upstream network speed in the header while the conversation tab stays active", () => {
    const upstreamActivity = createUpstreamAccountActivityResponse();
    upstreamActivity.accounts.push({
      ...upstreamActivity.accounts[0],
      accountKey: "upstream:77",
      upstreamAccountId: 77,
      displayName: "Pool Beta",
      requestCount: 3,
      successCount: 3,
      failureCount: 0,
      nonSuccessCount: 0,
      totalTokens: 600,
      successTokens: 600,
      nonSuccessTokens: 0,
      failureTokens: 0,
      failureCost: 0,
      totalCost: 0.18,
      uploadBytesPerSecond: 512,
      downloadBytesPerSecond: 1024 * 1024,
      recentInvocations: [],
    });
    upstreamActivity.networkRealtimeRate = {
      sampleStart: "2026-04-04T10:04:59Z",
      sampleEnd: "2026-04-04T10:05:00Z",
      sampleSeconds: 1,
      uploadBytesPerSecond: 2_048,
      downloadBytesPerSecond: 6 * 1024 * 1024,
      uploadBytes: 2_048,
      downloadBytes: 6 * 1024 * 1024,
    };

    renderSection(
      createResponse([
        createConversation("pck-header-network-speed", [
          createPreview({
            id: 1,
            invokeId: "invoke-header-network-speed",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
      {
        upstreamAccountActivity: upstreamActivity,
      },
    );

    expect(host?.textContent).toContain("当前对话 1 条");
    const totalNetworkSpeed = host?.querySelector(
      '[data-testid="dashboard-upstream-account-total-network-speed"]',
    );
    expect(totalNetworkSpeed).not.toBeNull();
    expect(totalNetworkSpeed?.textContent).toContain("2");
    expect(totalNetworkSpeed?.textContent).toContain("KiB/s");
    expect(totalNetworkSpeed?.textContent).toContain("6");
    expect(totalNetworkSpeed?.textContent).toContain("MiB/s");
  });

  it("renders a fallback for upstream account in-progress invocations when live counts are unavailable", () => {
    const response = createUpstreamAccountActivityResponse();
    upstreamAccountActivityMock.data = {
      ...response,
      accounts: [
        {
          ...response.accounts[0],
          inProgressInvocationCount: null,
          inProgressPhaseCounts: null,
          retryInvocationCount: null,
        },
      ],
    };

    renderSection(
      createResponse([
        createConversation("pck-upstream-yesterday-live-counts", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-yesterday-live-counts",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "success",
          }),
        ]),
      ]),
      { activeRange: "yesterday" },
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const accountHeader = host?.querySelector(
      '[data-testid="dashboard-upstream-account-header-row"]',
    );
    const accountHeaderText = accountHeader?.textContent;
    expect(accountHeader?.querySelector('[aria-label="进行中 —"]')).not.toBeNull();
    expect(accountHeaderText).toContain("—");
    expect(accountHeaderText).not.toContain("并行对话");
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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
      expect(tooltipText).toContain("用量明细");
      expect(tooltipText).toContain("缓存写入");
      expect(tooltipText).toContain("缓存读取");
      expect(tooltipText).toContain("总计");
      expect(tooltipText).toContain("gpt-5.6");
      expect(tooltipText).toContain("$0.34");
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
      expect(tooltipText).toContain("用量明细");
      expect(tooltipText).toContain("3,200");
      expect(tooltipText).toContain("缓存写入");
      expect(tooltipText).toContain("缓存读取");
      expect(tooltipText).toContain("缓存命中率");
      expect(tooltipText).toContain("输出");
      expect(tooltipText).toContain("总计");
      expect(tooltipText).toContain("gpt-5.6");
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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
    expect(costBreakdown?.getAttribute("aria-label")).toContain("$0.00");
    expect(costBreakdown?.getAttribute("aria-label")).toContain("0%");
  });

  it("compacts red-box upstream account metrics under tight widths while preserving full labels", async () => {
    const response = createUpstreamAccountActivityResponse();
    const account = response.accounts[0];
    if (!account) {
      throw new Error("missing upstream activity account");
    }
    account.tokensPerMinute = 1_324_743;
    account.totalTokens = 88_067_672;
    account.totalCost = 39.45;
    account.spendRate = 39.45;
    upstreamAccountActivityMock.data = response;

    const measureWidths = new Map<string, number>([
      ["1,324,743", 112],
      ["1.3247M", 92],
      ["1.325M", 84],
      ["1.32M", 76],
      ["1.3M", 48],
      ["1M", 36],
      ["88,067,672", 126],
      ["88.068M", 102],
      ["88.07M", 96],
      ["88.1M", 56],
      ["88M", 42],
      ["39.45", 58],
      ["39.5", 40],
      ["39", 24],
    ]);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function () {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === "true") {
        return 60;
      }
      return 1700;
    });
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function () {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === "true") {
        const text = (this as HTMLElement).textContent ?? "";
        return measureWidths.get(text) ?? Math.max(28, text.length * 10);
      }
      return 0;
    });

    renderSection(
      createResponse([
        createConversation("pck-upstream-account-adaptive-overflow", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-account-adaptive-overflow",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
      window.dispatchEvent(new Event("resize"));
    });

    await waitFor(() => {
      const tpmValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-tpm-value"]',
      );
      const spendRateValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-spend-rate-value"]',
      );
      const tokenValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-token-value"]',
      );
      const costValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-cost-value"]',
      );

      expect(tpmValue?.getAttribute("data-compact")).toBe("true");
      expect(tpmValue?.getAttribute("data-compact-precision")).toBe("K-0");
      expect(tpmValue?.textContent).toContain("1,325K");
      expect(tpmValue?.getAttribute("title")).toBe("1,324,743");

      expect(spendRateValue?.getAttribute("data-compact")).toBe("false");
      expect(spendRateValue?.getAttribute("data-compact-precision")).toBe("full");
      expect(spendRateValue?.textContent).toBe("39.45");
      expect(spendRateValue?.getAttribute("title")).toBeNull();

      expect(tokenValue?.getAttribute("data-compact")).toBe("true");
      expect(tokenValue?.getAttribute("data-compact-precision")).toBe("1");
      expect(tokenValue?.textContent).toContain("88.1M");
      expect(tokenValue?.getAttribute("title")).toBe("88,067,672");

      expect(costValue?.getAttribute("data-compact")).toBe("false");
      expect(costValue?.getAttribute("data-compact-precision")).toBe("full");
      expect(costValue?.textContent).toBe("39.45");
      expect(costValue?.getAttribute("title")).toBeNull();
    });

    const tpmTrigger = host?.querySelector('[aria-label="TPM 1,324,743"]');
    const spendRateTrigger = host?.querySelector('[aria-label="消费速率 39.45"]');
    const tokenCardTrigger = host?.querySelector('[aria-label="Token 88,067,672"]');
    const costCardTrigger = host?.querySelector('[aria-label="成本 39.45"]');

    expect(tpmTrigger).not.toBeNull();
    expect(spendRateTrigger).not.toBeNull();
    expect(tokenCardTrigger).not.toBeNull();
    expect(costCardTrigger).not.toBeNull();
    expect(host?.textContent).not.toContain("[object Object]");
  });

  it("applies the TPM width budget only to split-header TPM values", async () => {
    const response = createUpstreamAccountActivityResponse();
    const account = response.accounts[0];
    if (!account) {
      throw new Error("missing upstream activity account");
    }
    account.tokensPerMinute = 2_027_266;
    account.spendRate = 0.85;
    account.modelPerformance = {
      available: true,
      total: {
        tokensPerMinute: 2_027_266,
        streamingResponseRate: 19.8,
        avgResponseMs: 860,
        avgFirstResponseByteTotalMs: 2_867.5,
        wallClockUsageDurationMs: 42_000,
        cumulativeUsageDurationMs: 51_000,
        parallelism: 1.2,
      },
      models: [
        {
          model: "gpt-5.6",
          reasoningEffort: "medium",
          tokensPerMinute: 2_027_266,
          streamingResponseRate: 19.8,
          avgResponseMs: 860,
          avgFirstResponseByteTotalMs: 2_867.5,
          wallClockUsageDurationMs: 42_000,
          cumulativeUsageDurationMs: 51_000,
          parallelism: 1.2,
        },
      ],
    };
    upstreamAccountActivityMock.data = response;

    const measureWidths = new Map<string, number>([
      ["2,027,266", 112],
      ["2.03M", 42],
      ["2.0273M", 70],
      ["2.027M", 58],
      ["2.0M", 34],
      ["2M", 24],
      ["0.85", 26],
      ["3", 10],
    ]);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function () {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === "true") {
        if (
          (this as HTMLElement).querySelector(
            '[data-testid="dashboard-upstream-account-inline-tpm-value"]',
          )
        ) {
          return 48;
        }
        return 1700;
      }
      return 1700;
    });
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function () {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === "true") {
        const text = (this as HTMLElement).textContent ?? "";
        return measureWidths.get(text) ?? Math.max(28, text.length * 10);
      }
      return 0;
    });

    renderSection(
      createResponse([
        createConversation("pck-upstream-account-split-tpm-budget", [
          createPreview({
            id: 1,
            invokeId: "invoke-upstream-account-split-tpm-budget",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
      window.dispatchEvent(new Event("resize"));
    });

    await waitFor(() => {
      const accountCard = host?.querySelector('[data-testid="dashboard-upstream-account-card"]');
      const tpmValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-tpm-value"]',
      );
      const spendRateValue = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-spend-rate-value"]',
      );

      expect(accountCard?.getAttribute("data-header-layout")).toBe("split");
      expect(tpmValue?.getAttribute("data-compact")).toBe("true");
      expect(tpmValue?.textContent).toBe("2.03M");
      expect(tpmValue?.getAttribute("title")).toBe("2,027,266");

      expect(spendRateValue?.getAttribute("data-compact")).toBe("false");
      expect(spendRateValue?.textContent).toBe("0.85");
      expect(spendRateValue?.getAttribute("title")).toBeNull();
    });

    const accountHeader = host?.querySelector(
      '[data-testid="dashboard-upstream-account-header-row"]',
    );
    expect(
      accountHeader?.querySelector('[aria-label="TPM 2,027,266 Pool Alpha · 模型性能"]'),
    ).not.toBeNull();
    expect(
      accountHeader?.querySelector('[aria-label="消费速率 0.85 Pool Alpha · 模型性能"]'),
    ).not.toBeNull();
    expect(accountHeader?.querySelector('[aria-label="进行中 3"]')).not.toBeNull();
  });

  it("switches upstream account cards to container-width layouts instead of viewport breakpoints", async () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function () {
      const testId = (this as HTMLElement).getAttribute("data-testid");
      if (
        testId === "dashboard-upstream-account-grid" ||
        testId === "dashboard-upstream-account-card"
      ) {
        return 352;
      }
      return 1700;
    });

    renderSection(
      createResponse([
        createConversation("pck-upstream-account-container-layout", [
          createPreview({
            id: 2,
            invokeId: "invoke-upstream-account-container-layout",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
      window.dispatchEvent(new Event("resize"));
    });

    await waitFor(() => {
      const accountCard = host?.querySelector('[data-testid="dashboard-upstream-account-card"]');
      const inProgressSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-in-progress-slot"]',
      );
      const tpmSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-tpm-slot"]',
      );
      const spendRateSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-spend-rate-slot"]',
      );
      const recentBreakdown = host?.querySelector(
        '[data-testid="dashboard-upstream-account-recent-breakdown"]',
      );
      const phaseSegments = host?.querySelectorAll('[data-testid="invocation-phase-segment"]');
      expect(accountCard?.getAttribute("data-header-layout")).toBe("stacked");
      expect(accountCard?.getAttribute("data-inline-metric-layout")).toBe("three-columns");
      expect(accountCard?.getAttribute("data-metric-columns")).toBe("2");
      expect(accountCard?.getAttribute("data-recent-breakdown-layout")).toBe("stacked");
      expect(inProgressSlot?.classList.contains("w-full")).toBe(true);
      expect(tpmSlot?.classList.contains("w-full")).toBe(true);
      expect(spendRateSlot?.classList.contains("w-full")).toBe(true);
      expect(recentBreakdown?.textContent).not.toContain("排队中");
      expect(recentBreakdown?.textContent).not.toContain("请求中");
      expect(recentBreakdown?.textContent).not.toContain("响应中");
      expect(recentBreakdown?.textContent).not.toContain("失败");
      expect(recentBreakdown?.textContent).not.toContain("成功");
      expect(phaseSegments?.[0]?.getAttribute("data-phase-label-visible")).toBe("false");
    });
  });

  it("keeps split-layout inline metric slots content-sized on wide upstream account cards", async () => {
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function () {
      const testId = (this as HTMLElement).getAttribute("data-testid");
      if (
        testId === "dashboard-upstream-account-grid" ||
        testId === "dashboard-upstream-account-card"
      ) {
        return 920;
      }
      return 1700;
    });

    renderSection(
      createResponse([
        createConversation("pck-upstream-account-split-layout", [
          createPreview({
            id: 3,
            invokeId: "invoke-upstream-account-split-layout",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
      window.dispatchEvent(new Event("resize"));
    });

    await waitFor(() => {
      const accountCard = host?.querySelector('[data-testid="dashboard-upstream-account-card"]');
      const inProgressSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-in-progress-slot"]',
      );
      const tpmSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-tpm-slot"]',
      );
      const spendRateSlot = host?.querySelector(
        '[data-testid="dashboard-upstream-account-inline-spend-rate-slot"]',
      );
      expect(accountCard?.getAttribute("data-header-layout")).toBe("split");
      expect(accountCard?.getAttribute("data-inline-metric-layout")).toBe("inline");
      expect(inProgressSlot?.classList.contains("w-full")).toBe(false);
      expect(tpmSlot?.classList.contains("w-full")).toBe(false);
      expect(spendRateSlot?.classList.contains("w-full")).toBe(false);
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY)).toBe(
      "upstreamAccounts",
    );

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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    rerenderSection(response, { activeRange: "usage" });
    expect(host?.textContent).toContain("当前对话 1 条");
    expect(readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY)).toBe(
      "upstreamAccounts",
    );

    rerenderSection(response, { activeRange: "today" });
    expect(host?.textContent).toContain("当前活动账号 1 个");
    const restoredAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    expect(restoredAccountTab?.getAttribute("aria-selected")).toBe("true");
  });

  it("switches workspace views without rendering the removed description", () => {
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

    expect(host?.textContent).not.toContain("展示最近 5 分钟内有终态调用");

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    expect(host?.textContent).not.toContain("展示当前总览范围内有调用的上游账号");
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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
    expect(onOpenInvocation.mock.calls[0]?.[0]?.promptCacheKey).not.toBe("acct-invoke-1");
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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
    expect(identityChip.getAttribute("aria-label")).toContain("pck-upstream-running");

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
      identityChip.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    expect(onOpenConversation).toHaveBeenCalledTimes(1);
    expect(onOpenInvocation).not.toHaveBeenCalled();

    onOpenConversation.mockClear();

    act(() => {
      identityChip.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true }));
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
          currentFirstResponseByteTotalAvgMs: 420,
          avgTotalMs: 860,
          currentAvgTotalMs: 860,
          inProgressInvocationCount: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length,
          retryInvocationCount: 0,
          recentInvocations: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.map((promptCacheKey, index) =>
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
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

    const renderedShortIds = identityChips.map((chip) => chip.textContent?.trim());
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

    const badges = host?.querySelectorAll('[data-testid="invocation-transport-badge"]');
    expect(badges).toHaveLength(1);
    expect(badges?.[0]?.querySelector('[aria-hidden="true"]')?.textContent).toBe("WS");
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

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
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

    const currentPhaseBadge = card.querySelector(
      '[data-testid="invocation-phase-badge"][data-phase="responding"]',
    );
    expect(currentPhaseBadge).toBeInstanceOf(HTMLElement);
    expect(currentPhaseBadge?.getAttribute("aria-label")).toBe("响应中");
    expect(currentPhaseBadge?.getAttribute("data-phase-label-visible")).toBe("false");
    expect(card.textContent).toContain(expectedSortAnchorLabel);
    expect(card.textContent).toContain("请求");
    expect(card.textContent).toContain("Token");
    expect(card.textContent).toContain("成本");
    expect(card.textContent).not.toContain("累计请求");
    expect(card.textContent).not.toContain("对话 Tokens");
    expect(card.textContent).not.toContain("对话成本");
    expect(card.textContent).toContain(cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "");
    expect(card.textContent).not.toContain("WC-");
    expect(card.textContent).not.toContain("019d68a9-9c32-7482-a353-71e4b6265f09");
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

    expect(host?.querySelector('[data-testid="dashboard-working-conversation-card"]')).toBeTruthy();
    expect(host?.textContent).toContain("load more temporarily unavailable");
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
    const fastIcon = currentSlot.querySelector('[data-testid="invocation-fast-icon"]');
    if (
      !(modelName instanceof HTMLElement) ||
      !(reasoningEffort instanceof HTMLElement) ||
      !(fastIcon instanceof HTMLElement)
    ) {
      throw new Error("missing model/reasoning/service-tier markers");
    }

    expect(reasoningEffort.textContent).toContain("medium");
    expect(
      modelName.compareDocumentPosition(reasoningEffort) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(
      reasoningEffort.compareDocumentPosition(fastIcon) & Node.DOCUMENT_POSITION_FOLLOWING,
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
            upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
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
      accountChip.compareDocumentPosition(accountMeta) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(compactBadge.textContent).toMatch(/远程压缩|Compact/);
    expect(currentSlot.textContent).toContain("Team");
    expect(currentSlot.textContent).not.toContain("RQ ");
    expect(currentSlot.textContent).not.toContain("UP ");
    expect(currentSlot.textContent).not.toContain("ED ");
    expect(currentSlot.textContent).not.toContain("TT ");
    expect(
      currentSlot.querySelector('[data-testid="dashboard-compact-latency-first-byte"]'),
    ).not.toBeNull();
    expect(
      currentSlot.querySelector('[data-testid="dashboard-compact-latency-response-time"]'),
    ).not.toBeNull();
    expect(
      currentSlot.querySelector('[data-testid="dashboard-compact-latency-first-byte"]')?.className,
    ).not.toMatch(/rounded|border|bg-/);
    expect(
      currentSlot.querySelector('[data-testid="dashboard-compact-latency-response-time"]')
        ?.className,
    ).not.toMatch(/rounded|border|bg-/);
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

  it("compresses visible usage into hit token cost while keeping detailed hover metadata", () => {
    renderSection(
      createResponse([
        createConversation("pck-compact-usage-line", [
          createPreview({
            id: 21,
            invokeId: "invoke-compact-usage-current",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "running",
            totalTokens: 74_148,
            inputTokens: 73_951,
            outputTokens: 197,
            cacheInputTokens: 5_632,
            cost: 0.1752,
            reasoningTokens: 62,
          }),
          createPreview({
            id: 20,
            invokeId: "invoke-compact-usage-previous",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
            totalTokens: 13_184,
            inputTokens: 13_184,
            outputTokens: 0,
            cacheInputTokens: 0,
            cost: 0.052,
            reasoningTokens: 0,
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    const previousSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="previous"]',
    );
    if (!(currentSlot instanceof HTMLDivElement) || !(previousSlot instanceof HTMLDivElement)) {
      throw new Error("missing current or previous invocation slot");
    }

    const currentUsageLine = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-line"]',
    );
    const previousUsageLine = previousSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-line"]',
    );
    const currentCostValue = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    const currentHitValue = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const previousHitValue = previousSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const usageMetaLine = currentUsageLine?.parentElement;
    if (
      !(currentUsageLine instanceof HTMLDivElement) ||
      !(previousUsageLine instanceof HTMLDivElement) ||
      !(currentCostValue instanceof HTMLElement) ||
      !(currentHitValue instanceof HTMLElement) ||
      !(previousHitValue instanceof HTMLElement) ||
      !(usageMetaLine instanceof HTMLElement)
    ) {
      throw new Error("missing compact usage line elements");
    }

    expect(currentUsageLine.textContent).toContain("Hit 7.6%");
    expect(currentUsageLine.textContent).toContain("Token 74,148");
    expect(currentUsageLine.textContent).toContain("$0.1752");
    expect(currentUsageLine.textContent).not.toContain("IN ");
    expect(currentUsageLine.textContent).not.toContain("CW ");
    expect(currentUsageLine.textContent).not.toContain(" C ");
    expect(currentUsageLine.textContent).not.toContain("O ");
    expect(previousUsageLine.textContent).toContain("Hit 0%");
    expect(previousUsageLine.textContent).toContain("Token 13,184");
    expect(previousUsageLine.textContent).toContain("$0.0520");
    expect(currentHitValue.dataset.summaryTone).toBe("error");
    expect(previousHitValue.dataset.summaryTone).toBe("error");
    expect(currentCostValue.dataset.summaryTone).toBe("warning");
    expect(usageMetaLine.title).toContain("Cache write: 68,319");
    expect(usageMetaLine.title).toContain("5,632");
    expect(usageMetaLine.title).toContain("74,148");
    expect(usageMetaLine.title).toContain("US$0.1752");
    expect(host?.querySelector('[data-testid="dashboard-upstream-account-recent-row"]')).toBeNull();
  });

  it("applies strict hit and cost threshold tones to conversation and upstream recent summaries", () => {
    upstreamAccountActivityMock.data = {
      ...createUpstreamAccountActivityResponse(),
      accounts: [
        {
          ...createUpstreamAccountActivityResponse().accounts[0],
          recentInvocations: [
            createPreview({
              id: 9101,
              invokeId: "acct-threshold-warning",
              promptCacheKey: "pck-upstream-threshold-warning",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "success",
              totalTokens: 1_000,
              inputTokens: 700,
              outputTokens: 200,
              cacheInputTokens: 899,
              cost: 0.1001,
              reasoningTokens: 20,
            }),
            createPreview({
              id: 9102,
              invokeId: "acct-threshold-error",
              promptCacheKey: "pck-upstream-threshold-error",
              occurredAt: "2026-04-04T10:04:00Z",
              status: "failed",
              totalTokens: 1_000,
              inputTokens: 700,
              outputTokens: 200,
              cacheInputTokens: 499,
              cost: 0.5001,
              reasoningTokens: 20,
            }),
            createPreview({
              id: 9103,
              invokeId: "acct-threshold-default",
              promptCacheKey: "pck-upstream-threshold-default",
              occurredAt: "2026-04-04T10:03:00Z",
              status: "success",
              totalTokens: 1_000,
              inputTokens: 700,
              outputTokens: 200,
              cacheInputTokens: 900,
              cost: 0.1,
              reasoningTokens: 20,
            }),
            createPreview({
              id: 9104,
              invokeId: "acct-threshold-boundary",
              promptCacheKey: "pck-upstream-threshold-boundary",
              occurredAt: "2026-04-04T10:02:00Z",
              status: "success",
              totalTokens: 1_000,
              inputTokens: 700,
              outputTokens: 200,
              cacheInputTokens: 500,
              cost: 0.5,
              reasoningTokens: 20,
            }),
          ],
        },
      ],
    };

    renderSection(
      createResponse([
        createConversation("pck-threshold-default", [
          createPreview({
            id: 31,
            invokeId: "invoke-threshold-default-current",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "running",
            totalTokens: 1_000,
            inputTokens: 700,
            outputTokens: 200,
            cacheInputTokens: 955,
            cost: 0.0586,
            reasoningTokens: 20,
          }),
          createPreview({
            id: 30,
            invokeId: "invoke-threshold-warning-previous",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
            totalTokens: 1_000,
            inputTokens: 700,
            outputTokens: 200,
            cacheInputTokens: 899,
            cost: 0.1001,
            reasoningTokens: 20,
          }),
        ]),
        createConversation("pck-threshold-error", [
          createPreview({
            id: 41,
            invokeId: "invoke-threshold-error-current",
            occurredAt: "2026-04-04T10:04:30Z",
            status: "completed",
            totalTokens: 1_000,
            inputTokens: 700,
            outputTokens: 200,
            cacheInputTokens: 499,
            cost: 0.5001,
            reasoningTokens: 20,
          }),
        ]),
      ]),
    );

    const firstCardCurrentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    const firstCardPreviousSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="previous"]',
    );
    const secondCard = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversation-card"]',
    )[1];
    const secondCardCurrentSlot = secondCard?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (
      !(firstCardCurrentSlot instanceof HTMLElement) ||
      !(firstCardPreviousSlot instanceof HTMLElement) ||
      !(secondCardCurrentSlot instanceof HTMLElement)
    ) {
      throw new Error("missing threshold summary slots");
    }

    const currentHit = firstCardCurrentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const currentCost = firstCardCurrentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    const previousHit = firstCardPreviousSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const previousCost = firstCardPreviousSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    const errorHit = secondCardCurrentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const errorCost = secondCardCurrentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    if (
      !(currentHit instanceof HTMLElement) ||
      !(currentCost instanceof HTMLElement) ||
      !(previousHit instanceof HTMLElement) ||
      !(previousCost instanceof HTMLElement) ||
      !(errorHit instanceof HTMLElement) ||
      !(errorCost instanceof HTMLElement)
    ) {
      throw new Error("missing conversation threshold summary fields");
    }

    expect(currentHit.textContent).toContain("Hit 95.5%");
    expect(currentHit.dataset.summaryTone).toBe("default");
    expect(currentCost.textContent).toContain("$0.0586");
    expect(currentCost.dataset.summaryTone).toBe("default");
    expect(previousHit.textContent).toContain("Hit 89.9%");
    expect(previousHit.dataset.summaryTone).toBe("warning");
    expect(previousCost.textContent).toContain("$0.1001");
    expect(previousCost.dataset.summaryTone).toBe("warning");
    expect(errorHit.textContent).toContain("Hit 49.9%");
    expect(errorHit.dataset.summaryTone).toBe("error");
    expect(errorCost.textContent).toContain("$0.5001");
    expect(errorCost.dataset.summaryTone).toBe("error");

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }
    act(() => {
      fireEvent.click(accountTab);
    });

    const recentRows = Array.from(
      host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]') ?? [],
    );
    expect(recentRows).toHaveLength(4);
    const warningRow = recentRows[0];
    const errorRow = recentRows[1];
    const defaultRow = recentRows[2];
    const boundaryRow = recentRows[3];
    if (
      !(warningRow instanceof HTMLElement) ||
      !(errorRow instanceof HTMLElement) ||
      !(defaultRow instanceof HTMLElement) ||
      !(boundaryRow instanceof HTMLElement)
    ) {
      throw new Error("missing upstream threshold rows");
    }

    const warningSummary = warningRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-line"]',
    );
    const warningMetaLine = warningRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-meta-line"]',
    );
    const warningHit = warningRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-hit"]',
    );
    const warningCost = warningRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
    );
    const errorHitRow = errorRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-hit"]',
    );
    const errorCostRow = errorRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
    );
    const defaultHitRow = defaultRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-hit"]',
    );
    const defaultCostRow = defaultRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
    );
    const boundaryHitRow = boundaryRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-hit"]',
    );
    const boundaryCostRow = boundaryRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
    );
    if (
      !(warningSummary instanceof HTMLElement) ||
      !(warningMetaLine instanceof HTMLElement) ||
      !(warningHit instanceof HTMLElement) ||
      !(warningCost instanceof HTMLElement) ||
      !(errorHitRow instanceof HTMLElement) ||
      !(errorCostRow instanceof HTMLElement) ||
      !(defaultHitRow instanceof HTMLElement) ||
      !(defaultCostRow instanceof HTMLElement) ||
      !(boundaryHitRow instanceof HTMLElement) ||
      !(boundaryCostRow instanceof HTMLElement)
    ) {
      throw new Error("missing upstream threshold summary fields");
    }

    expect(warningRow.textContent).not.toContain("IN ");
    expect(warningRow.textContent).not.toContain("CW ");
    expect(warningRow.textContent).not.toContain(" C ");
    expect(warningRow.textContent).not.toContain("O ");
    expect(warningRow.textContent).not.toContain("T ");
    expect(
      warningSummary.compareDocumentPosition(warningMetaLine) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(warningHit.textContent).toContain("Hit 89.9%");
    expect(warningHit.dataset.summaryTone).toBe("warning");
    expect(warningCost.textContent).toContain("$0.1001");
    expect(warningCost.dataset.summaryTone).toBe("warning");
    expect(errorHitRow.textContent).toContain("Hit 49.9%");
    expect(errorHitRow.dataset.summaryTone).toBe("error");
    expect(errorCostRow.textContent).toContain("$0.5001");
    expect(errorCostRow.dataset.summaryTone).toBe("error");
    expect(defaultHitRow.textContent).toContain("Hit 90%");
    expect(defaultHitRow.dataset.summaryTone).toBe("default");
    expect(defaultCostRow.textContent).toContain("$0.1000");
    expect(defaultCostRow.dataset.summaryTone).toBe("default");
    expect(boundaryHitRow.textContent).toContain("Hit 50%");
    expect(boundaryHitRow.dataset.summaryTone).toBe("warning");
    expect(boundaryCostRow.textContent).toContain("$0.5000");
    expect(boundaryCostRow.dataset.summaryTone).toBe("warning");
    expect(warningSummary.title).toContain("Cache write:");
    expect(warningSummary.title).toContain("缓存输入");
    expect(warningSummary.title).toContain("推理 Tokens");
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

    const badges = host?.querySelectorAll('[data-testid="dashboard-image-tool-icon-badge"]');
    expect(badges?.length ?? 0).toBe(1);
    expect(badges?.[0]?.getAttribute("aria-label")).toMatch(/图片工具|Image tool/);
    expect(badges?.[0]?.textContent).not.toMatch(/图片工具|Image tool/);
    expect(badges?.[0]?.className).toContain("rounded-full");
    expect(badges?.[0]?.className).toContain("border");
  });

  it("renders image endpoint chips and hides the image icon badge on direct image endpoints", () => {
    renderSection(
      createResponse([
        createConversation("pck-image-endpoint", [
          createPreview({
            id: 14,
            invokeId: "invoke-image-endpoint",
            occurredAt: "2026-04-04T10:05:45Z",
            status: "running",
            endpoint: "/v1/images/generations",
            imageIntent: "yes",
            model: "gpt-image-1",
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    const imageEndpointBadge = currentSlot.querySelector(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="image_gen"]',
    );
    if (!(imageEndpointBadge instanceof HTMLElement)) {
      throw new Error("missing image endpoint badge");
    }

    expect(imageEndpointBadge.textContent).toBe("image/gen");
    expect(currentSlot.querySelector('[data-testid="dashboard-image-tool-icon-badge"]')).toBeNull();
    expect(currentSlot.textContent).not.toContain("/v1/images/generations");
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
      '[data-testid="dashboard-image-tool-icon-badge"][data-image-intent-kind="direct_image"]',
    );

    if (!(remoteBadge instanceof HTMLElement) || !(imageBadge instanceof HTMLElement)) {
      throw new Error("missing mixed-signal badges");
    }

    expect(remoteBadge.textContent).toMatch(/远程压缩V2|Remote compaction V2/);
    expect(imageBadge.getAttribute("aria-label")).toMatch(/图片工具|Image tool/);
    expect(imageBadge.textContent).not.toMatch(/图片工具|Image tool/);
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
      host?.querySelectorAll('[data-testid="dashboard-working-conversation-account-plan"]') ?? [],
    );
    const labels = planBadges.map((badge) => badge.textContent);

    expect(labels).toEqual(expect.arrayContaining(["Ent", "Plus", "Free", "Pro"]));
    expect(labels).not.toContain("enterprise");
    expect(labels).not.toContain("local");

    const enterpriseBadge = planBadges.find((badge) => badge.textContent === "Ent");
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

    const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');
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

    const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');
    if (!(grid instanceof HTMLDivElement)) {
      throw new Error("missing working conversations grid");
    }

    const rowGrid = grid.querySelector('[data-testid="dashboard-working-conversations-row"] > div');
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
    vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(1680);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });
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
    vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(640);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });
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
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });
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
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });
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
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });
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

  it("does not load more hidden conversations from the upstream-account tab", () => {
    vi.useFakeTimers();
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return {
          x: 0,
          y: 0,
          top: 0,
          bottom: 1_360,
          left: 0,
          right: 1200,
          width: 1200,
          height: 1_360,
          toJSON: () => ({}),
        } satisfies DOMRect;
      }
      if (this.getAttribute("data-testid") === "dashboard-upstream-account-grid") {
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
    });
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
    const onLoadMore = vi.fn();

    renderSection(
      createResponse(
        Array.from({ length: 4 }, (_, index) =>
          createConversation(`pck-hidden-${index + 1}`, [
            createPreview({
              id: index + 1,
              invokeId: `invoke-hidden-${index + 1}`,
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

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
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

  it("renders warning success status labels in dashboard recent cards via the shared tooltip", async () => {
    renderSection(
      createResponse([
        createConversation("pck-warning-success", [
          createPreview({
            id: 10,
            invokeId: "invoke-warning-success-dashboard",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "warning_success",
            failureClass: "none",
          }),
        ]),
      ]),
    );

    const statusNode = host?.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    ) as HTMLElement | null;
    expect(statusNode?.getAttribute("title")).toBeNull();
    expect(statusNode?.getAttribute("aria-label") ?? "").toContain("警告成功");

    await act(async () => {
      statusNode?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });

    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
        node.textContent?.includes("警告成功"),
      );
      expect(tooltip).toBeInstanceOf(HTMLElement);
    });
  });

  it("keeps warning success recent rows icon-only in upstream-account activity and exposes details through the shared tooltip", async () => {
    const upstreamActivity = createUpstreamAccountActivityResponse();
    upstreamActivity.accounts[0]!.recentInvocations[0] = {
      ...upstreamActivity.accounts[0]!.recentInvocations[0]!,
      status: "warning_success",
      failureKind: "downstream_closed",
      failureClass: "none",
    };
    upstreamAccountActivityMock.data = upstreamActivity;

    renderSection(
      createResponse([
        createConversation("pck-warning-success-account", [
          createPreview({
            id: 20,
            invokeId: "invoke-warning-success-account",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "warning_success",
            failureClass: "none",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const statusNode = host?.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    ) as HTMLElement | null;
    expect(statusNode?.textContent ?? "").not.toContain("警告成功");
    expect(statusNode?.getAttribute("title")).toBeNull();
    expect(statusNode?.getAttribute("aria-label") ?? "").toContain("警告成功");

    await act(async () => {
      statusNode?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });

    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
        node.textContent?.includes("警告成功"),
      );
      expect(tooltip).toBeInstanceOf(HTMLElement);
    });
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

  it("renders interrupted slots with the dedicated interrupted status icon semantics", () => {
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

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing interrupted conversation card");
    }

    const headerStatusIcon = card.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    );
    if (!(headerStatusIcon instanceof HTMLElement)) {
      throw new Error("missing interrupted header status icon");
    }

    expect(headerStatusIcon.getAttribute("aria-label")).toContain("已中断");
    expect(headerStatusIcon.getAttribute("aria-label")).not.toContain("失败");
    expect(card.textContent ?? "").not.toContain("已中断");
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

    const accountButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) => {
      const text = button.textContent ?? "";
      const title = button.getAttribute("title") ?? "";
      return text.includes("pool-account-77") || title.includes("pool-account-77@example.com");
    });
    if (!(accountButton instanceof HTMLButtonElement)) {
      throw new Error("missing account button");
    }

    act(() => {
      accountButton.click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(77, "pool-account-77@example.com");
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("opens health events when clicking upstream account attention badges", async () => {
    const onOpenUpstreamAccount = vi.fn();
    const onOpenInvocation = vi.fn();
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    renderSection(createResponse([]), {
      onOpenUpstreamAccount,
      onOpenInvocation,
    });

    const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
    );
    if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      upstreamAccountTab.click();
    });

    const attentionBadges = host?.querySelector(
      '[data-testid="dashboard-upstream-account-attention-badges"]',
    );
    if (!(attentionBadges instanceof HTMLDivElement)) {
      throw new Error("missing upstream account attention badges");
    }
    expect(attentionBadges.className).not.toContain("rounded-full");
    expect(attentionBadges.className).not.toContain("border-base-300/70");
    expect(attentionBadges.className).not.toContain("bg-base-100/86");
    const attentionBadgeButtons = attentionBadges.querySelectorAll(
      '[data-testid="dashboard-upstream-account-attention-badge"]',
    );
    expect(attentionBadgeButtons).toHaveLength(2);
    expect(attentionBadges.querySelector("button")).toBe(attentionBadgeButtons[0]);

    act(() => {
      (attentionBadgeButtons[0] as HTMLButtonElement).click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(42, "Pool Alpha", {
      tab: "healthEvents",
    });
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("debounces upstream account quick policy writes as account-level overrides", async () => {
    vi.useFakeTimers();
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            id: 42,
            displayName: "Pool Alpha",
            status: "active",
            routingRule: {},
          }),
          {
            status: 200,
            headers: { "Content-Type": "application/json" },
          },
        ),
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    try {
      renderSection(createResponse([]));

      const upstreamAccountTab = Array.from(
        host?.querySelectorAll('button[role="tab"]') ?? [],
      ).find((candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""));
      if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account tab");
      }

      act(() => {
        upstreamAccountTab.click();
      });

      const policyBadge = host?.querySelector(
        '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="priority-new-conversations"]',
      );
      if (!(policyBadge instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account policy badge");
      }

      expect(policyBadge.textContent?.trim()).toBe("禁新");
      expect(policyBadge.dataset.policyTone).toBe("warning");

      act(() => {
        policyBadge.click();
      });
      await act(async () => {});
      expect(policyBadge.textContent?.trim()).toBe("普通");
      expect(policyBadge.dataset.policyTone).toBe("neutral");

      act(() => {
        policyBadge.click();
      });
      expect(policyBadge.textContent?.trim()).toBe("兜底");
      expect(policyBadge.dataset.policyTone).toBe("success");

      expect(fetchMock).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(1000);
      });

      await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
      const [, init] = fetchMock.mock.calls[0]!;
      expect(String(init?.body)).toContain('"priorityTier":"fallback"');
    } finally {
      globalThis.fetch = originalFetch;
      vi.useRealTimers();
    }
  });

  it("cycles upstream account Fast mode as an account-level quick policy", async () => {
    vi.useFakeTimers();
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            id: 42,
            displayName: "Pool Alpha",
            status: "active",
            routingRule: {},
          }),
          {
            status: 200,
            headers: { "Content-Type": "application/json" },
          },
        ),
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    try {
      renderSection(createResponse([]));

      const upstreamAccountTab = Array.from(
        host?.querySelectorAll('button[role="tab"]') ?? [],
      ).find((candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""));
      if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account tab");
      }

      act(() => {
        upstreamAccountTab.click();
      });

      const fastBadge = host?.querySelector(
        '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="fast-mode-rewrite"]',
      );
      if (!(fastBadge instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account Fast policy badge");
      }

      expect(fastBadge.textContent?.trim()).toBe("强制Fast");
      expect(fastBadge.dataset.policyTone).toBe("primary");
      expect(fastBadge.getAttribute("title")).toContain("Fast 改写策略");
      expect(fastBadge.getAttribute("aria-label")).toContain("Fast 改写策略：强制Fast");

      act(() => {
        fastBadge.click();
      });
      expect(fastBadge.textContent?.trim()).toBe("禁Fast");
      expect(fastBadge.dataset.policyTone).toBe("warning");
      expect(fastBadge.disabled).toBe(false);

      act(() => {
        fastBadge.click();
      });
      expect(fastBadge.textContent?.trim()).toBe("不改Fast");
      expect(fastBadge.dataset.policyTone).toBe("neutral");
      expect(fastBadge.getAttribute("aria-label")).toContain("Fast 改写策略：不改Fast");
      expect(fastBadge.disabled).toBe(false);
      expect(fetchMock).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(1000);
      });

      await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
      const [, init] = fetchMock.mock.calls[0]!;
      expect(String(init?.body)).toContain('"routingRule"');
      expect(String(init?.body)).toContain('"fastModeRewriteMode":"keep_original"');
    } finally {
      globalThis.fetch = originalFetch;
      vi.useRealTimers();
    }
  });

  it("flushes a pending upstream account quick policy write on unmount", async () => {
    vi.useFakeTimers();
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            id: 42,
            displayName: "Pool Alpha",
            status: "active",
            routingRule: {},
          }),
          {
            status: 200,
            headers: { "Content-Type": "application/json" },
          },
        ),
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

    try {
      renderSection(createResponse([]));

      const upstreamAccountTab = Array.from(
        host?.querySelectorAll('button[role="tab"]') ?? [],
      ).find((candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""));
      if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account tab");
      }

      act(() => {
        upstreamAccountTab.click();
      });

      const policyBadge = host?.querySelector(
        '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="priority-new-conversations"]',
      );
      if (!(policyBadge instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account policy badge");
      }

      act(() => {
        policyBadge.click();
      });
      expect(fetchMock).not.toHaveBeenCalled();

      act(() => {
        root?.unmount();
      });

      await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
      const [, init] = fetchMock.mock.calls[0]!;
      expect(String(init?.body)).toContain('"routingRule"');
      expect(String(init?.body)).toContain('"priorityTier":"normal"');
    } finally {
      globalThis.fetch = originalFetch;
      vi.useRealTimers();
    }
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

    const accountLabel = host?.querySelector('[title="sticky-account-52@example.com"]');
    expect(accountLabel).not.toBeNull();
    expect(accountLabel?.className).not.toContain("invocation-account-routing-in-progress");
    expect(host?.textContent ?? "").not.toContain("未分配上游账号");
  });

  it("marks the concrete upstream account as routing in progress on running dashboard cards", () => {
    renderSection(
      createResponse([
        createConversation("pck-routing-account", [
          createPreview({
            id: 81,
            invokeId: "invoke-routing-account",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            upstreamAccountId: 52,
            upstreamAccountName: "sticky-account-52@example.com",
          }),
        ]),
      ]),
    );

    const accountLabel = host?.querySelector('[title="sticky-account-52@example.com"]');
    expect(accountLabel).not.toBeNull();
    expect(accountLabel?.className).toContain("invocation-account-routing-in-progress");
    expect(host?.textContent ?? "").not.toContain("号池路由中");
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
            errorMessage: "[pool_no_available_account] no assignable upstream account remains",
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

  it("shows the blocked-binding recovery banner and reuses the clear-affinity confirmation flow", async () => {
    const originalFetch = globalThis.fetch;
    let capturedPayload: Record<string, unknown> | null = null;
    const fetchMock = createBulkConversationFetchMock({
      onBulkPayload: (payload) => {
        capturedPayload = payload;
      },
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const onClearBlockedBindingFilter = vi.fn();
    try {
      renderSection(
        createResponse([
          createConversation("pck-banner-blocked", [
            createPreview({
              id: 91,
              invokeId: "invoke-banner-blocked",
              occurredAt: "2026-04-04T10:04:00Z",
              status: "failed",
              failureClass: "service_failure",
              failureKind: "pool_assigned_account_blocked",
              blockedBinding: {
                constraintSource: "encryptedSessionOwner",
                upstreamAccountId: 2890,
                upstreamAccountLabel: "dzw",
                promptCacheKey: "pck-banner-blocked",
                recoveryAction: "clearAndResetAffinity",
              },
            }),
          ]),
        ]),
        {
          activeBlockedBindingFilter: {
            upstreamAccountId: 2890,
            constraintSource: "encryptedSessionOwner",
          },
          onClearBlockedBindingFilter,
          onConversationsChanged: vi.fn(),
        },
      );

      const upstreamAccountTab = Array.from(
        host?.querySelectorAll('button[role="tab"]') ?? [],
      ).find((node) => node.textContent?.includes("上游账号"));
      if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
        throw new Error("missing upstream account tab");
      }
      expect(upstreamAccountTab.disabled).toBe(true);
      expect(
        host?.querySelector('[data-testid="dashboard-blocked-binding-banner"]')?.textContent ?? "",
      ).toContain("单账号会话约束阻塞");
      expect(host?.textContent ?? "").toContain("dzw");

      const user = userEvent.setup();
      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-blocked-binding-clear-filter-button"]',
        ) as HTMLElement,
      );
      expect(onClearBlockedBindingFilter).toHaveBeenCalledTimes(1);

      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-blocked-binding-clear-and-reselect-button"]',
        ) as HTMLElement,
      );

      const confirmDialog = document.body.querySelector(
        '[data-testid="dashboard-working-conversations-clear-affinity-dialog"]',
      );
      expect(confirmDialog?.textContent).toContain("加密 owner 约束");
      expect(confirmDialog?.textContent).toContain("sticky route");
      expect(confirmDialog?.textContent).toContain("重选");

      const confirmButton = Array.from(confirmDialog?.querySelectorAll("button") ?? []).find(
        (button) => button.textContent?.includes("确认清空并重选"),
      );
      if (!(confirmButton instanceof HTMLButtonElement)) {
        throw new Error("missing clear affinity confirm button");
      }
      await user.click(confirmButton);

      await waitFor(() =>
        expect(capturedPayload).toMatchObject({
          action: "clearAndResetAffinity",
          promptCacheKeys: ["pck-banner-blocked"],
        }),
      );
    } finally {
      globalThis.fetch = originalFetch;
    }
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
    expect(onOpenInvocation.mock.calls[0]?.[0]?.invocation?.record?.invokeId).toBe(
      "invoke-slot-current",
    );

    act(() => {
      currentSlot.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    expect(onOpenInvocation).toHaveBeenCalledTimes(2);
  });

  it("opens conversation detail from the sequence id and invocation detail from the slot", () => {
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
    expect(sequenceButton.getAttribute("aria-label")).not.toContain("invoke-sequence-current");
    expect(sequenceButton.getAttribute("aria-label")).not.toContain("invoke-sequence-previous");
    expect(sequenceButton.getAttribute("aria-label")).toContain("pck-sequence-open");
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

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing dashboard card");
    }

    const currentSlot = card.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }
    const slotHeader = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    if (!(slotHeader instanceof HTMLElement)) {
      throw new Error("missing slot header");
    }
    expect(slotHeader.className).toContain("grid");
    expect(slotHeader.className).toContain("grid-cols-[auto_minmax(0,1fr)]");
    const statusLabel = currentSlot.querySelector('[data-testid="invocation-phase-badge"]');
    if (!(statusLabel instanceof HTMLElement)) {
      throw new Error(`missing phase label in slot: ${currentSlot.textContent ?? ""}`);
    }
    expect(
      slotHeader
        .querySelector('[data-testid="dashboard-working-conversation-slot-label"]')
        ?.textContent?.trim(),
    ).toBe("当前调用");
    expect(slotHeader.querySelector('[data-testid="invocation-phase-badge"]')).toBe(statusLabel);
    expect(statusLabel.getAttribute("data-phase-label-visible")).toBe("false");
    expect(statusLabel.getAttribute("data-phase-motion")).toBe("dynamic");
    expect(
      slotHeader.querySelector('[data-testid="dashboard-compact-latency-first-byte"]'),
    ).toBeInstanceOf(HTMLElement);
    expect(
      slotHeader.querySelector('[data-testid="dashboard-compact-latency-response-time"]'),
    ).toBeInstanceOf(HTMLElement);

    const phaseLabels = Array.from(card.querySelectorAll('[data-testid="invocation-phase-badge"]'));
    expect(phaseLabels.length).toBeGreaterThanOrEqual(2);
    for (const phaseLabel of phaseLabels) {
      expect(phaseLabel.className).toContain("inline-flex");
      expect(phaseLabel.className).toMatch(/\brounded-full\b/);
      expect(phaseLabel.className).toContain("bg-base-100/12");
      expect(phaseLabel.className).not.toMatch(/\bborder/);
      expect(phaseLabel.getAttribute("data-phase-motion")).toBe("dynamic");
      expect(phaseLabel.getAttribute("data-phase-label-visible")).toBe("false");
    }
    const phaseIcons = Array.from(card.querySelectorAll('[data-testid="invocation-phase-icon"]'));
    expect(phaseIcons.some((icon) => icon.className.includes("animate-spin"))).toBe(true);

    const requestMetric = Array.from(card.querySelectorAll("span")).find(
      (node) => node.textContent === "请求",
    );
    if (!(requestMetric instanceof HTMLElement)) {
      throw new Error("missing request metric");
    }

    act(() => {
      statusLabel.click();
    });

    expect(onOpenInvocation).toHaveBeenCalledWith(
      expect.objectContaining({
        conversationSequenceId: cards[0]?.conversationSequenceId,
        promptCacheKey: "pck-sequence-open",
        slotKind: "current",
      }),
    );
    expect(onOpenInvocation).toHaveBeenCalledTimes(1);
    expect(onOpenConversation).not.toHaveBeenCalled();

    onOpenInvocation.mockClear();

    act(() => {
      requestMetric.click();
      currentSlot.click();
    });

    expect(onOpenInvocation).toHaveBeenCalledTimes(1);
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  it("renders a manual binding badge beside the sequence id and opens settings directly", () => {
    const onOpenInvocation = vi.fn();
    const onOpenConversation = vi.fn();
    const response = createResponse([
      createConversation(
        "pck-manual-binding-open",
        [
          createPreview({
            id: 2,
            invokeId: "invoke-manual-binding-current",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
          createPreview({
            id: 1,
            invokeId: "invoke-manual-binding-previous",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
          }),
        ],
        {
          manualBinding: {
            bindingKind: "group",
            groupName: "CIII",
            upstreamAccountId: null,
            upstreamAccountName: null,
          },
        },
      ),
    ]);

    const cards = renderSection(response, {
      onOpenConversation,
      onOpenInvocation,
    });

    const badgeButton = host?.querySelector(
      '[data-testid="dashboard-working-conversation-manual-binding-badge"]',
    );
    if (!(badgeButton instanceof HTMLButtonElement)) {
      throw new Error("missing manual binding badge");
    }

    expect(badgeButton.textContent).toContain("CIII");
    expect(badgeButton.getAttribute("aria-label")).toContain("打开对话设置");
    expect(badgeButton.getAttribute("aria-label")).toContain("当前：分组 CIII");
    expect(badgeButton.className).toContain("max-w-[20rem]");

    act(() => {
      badgeButton.click();
    });

    expect(onOpenConversation).toHaveBeenCalledWith({
      conversationSequenceId: cards[0]?.conversationSequenceId,
      promptCacheKey: "pck-manual-binding-open",
      tab: "settings",
    });
    expect(onOpenConversation).toHaveBeenCalledTimes(1);
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("uses distinct manual binding badge tones and keeps account badges capped at 20rem", () => {
    renderSection(
      createResponse([
        createConversation(
          "pck-manual-binding-group-tone",
          [
            createPreview({
              id: 2,
              invokeId: "invoke-manual-binding-group-tone",
              occurredAt: "2026-04-04T10:04:00Z",
              status: "completed",
            }),
          ],
          {
            manualBinding: {
              bindingKind: "group",
              groupName: "CIII",
              upstreamAccountId: null,
              upstreamAccountName: null,
            },
          },
        ),
        createConversation(
          "pck-manual-binding-account-tone",
          [
            createPreview({
              id: 3,
              invokeId: "invoke-manual-binding-account-tone",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "completed",
            }),
          ],
          {
            manualBinding: {
              bindingKind: "upstreamAccount",
              groupName: null,
              upstreamAccountId: 108,
              upstreamAccountName:
                "paisleeeinar5710 Team sandbox workflow monitor with an intentionally long account label",
            },
          },
        ),
      ]),
    );

    const badges = host?.querySelectorAll(
      '[data-testid="dashboard-working-conversation-manual-binding-badge"]',
    );
    const badgeArray = Array.from(badges ?? []);
    const groupBadge = badgeArray.find((badge) => badge.textContent?.includes("CIII"));
    const accountBadge = badgeArray.find((badge) =>
      badge.textContent?.includes("paisleeeinar5710"),
    );

    expect(groupBadge).toBeInstanceOf(HTMLSpanElement);
    expect(accountBadge).toBeInstanceOf(HTMLSpanElement);
    expect(groupBadge?.className).toContain("text-info");
    expect(accountBadge?.className).toContain("text-secondary");
    expect(groupBadge?.className).toContain("text-[10.5px]");
    expect(accountBadge?.className).toContain("text-[10.5px]");

    const groupBadgeText = groupBadge?.firstElementChild;
    const accountBadgeText = accountBadge?.firstElementChild;

    expect(groupBadgeText?.className).toContain("max-w-[20rem]");
    expect(accountBadgeText?.className).toContain("max-w-[20rem]");
    expect(accountBadgeText?.textContent).toContain("paisleeeinar5710");
  });

  it("keeps the full sequence id visible and only truncates the binding target", () => {
    const cards = mapPromptCacheConversationsToDashboardCards(
      createResponse([
        createConversation(
          "pck-manual-binding-long-sequence",
          [
            createPreview({
              id: 5,
              invokeId: "invoke-manual-binding-long-sequence",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "completed",
            }),
          ],
          {
            manualBinding: {
              bindingKind: "upstreamAccount",
              groupName: null,
              upstreamAccountId: 219,
              upstreamAccountName:
                "paisleeeinar5710 Team sandbox workflow monitor with an intentionally long account label",
            },
          },
        ),
      ]),
    ).map((card) => ({
      ...card,
      conversationSequenceId: "WC-COLLIDE-ABC123",
    }));

    renderSectionWithCards(cards, { onOpenConversation: vi.fn() });

    const sequenceButton = host?.querySelector(
      '[data-testid="dashboard-working-conversation-sequence-button"]',
    );
    const badge = host?.querySelector(
      '[data-testid="dashboard-working-conversation-manual-binding-badge"]',
    );
    const badgeText = badge?.firstElementChild?.firstElementChild;

    expect(sequenceButton).toBeInstanceOf(HTMLButtonElement);
    expect(sequenceButton?.textContent).toBe("COLLIDE-ABC123");
    expect(sequenceButton?.className).toContain("shrink-0");
    expect(sequenceButton?.className).toContain("whitespace-nowrap");
    expect(sequenceButton?.className).not.toContain("min-w-0");
    expect(sequenceButton?.querySelector("span")?.className).not.toContain("truncate");

    expect(badge).toBeInstanceOf(HTMLButtonElement);
    expect(badge?.className).toContain("max-w-[20rem]");
    expect(badgeText?.className).toContain("truncate");
    expect(badgeText?.className).toContain("max-w-[20rem]");
  });

  it("renders the card header status as the same icon-only affordance used by invocation records", () => {
    renderSection(
      createResponse([
        createConversation("pck-card-header-status-icon", [
          createPreview({
            id: 91,
            invokeId: "invoke-card-header-status-icon",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "success",
          }),
        ]),
      ]),
    );

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing working conversation card");
    }

    const headerStatusIcon = card.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    );
    if (!(headerStatusIcon instanceof HTMLElement)) {
      throw new Error("missing card header status icon");
    }

    expect(headerStatusIcon.getAttribute("aria-label")).toContain("成功");
    expect(card.textContent).not.toContain("成功");
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

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    const placeholderLine = host?.querySelector(".working-conversation-placeholder-line");

    expect(card?.className).toContain("working-conversation-card-surface");
    expect(card?.className).not.toContain("bg-[linear-gradient");
    expect(placeholder?.className).toContain("working-conversation-slot-surface");
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

    const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');

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

    const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');
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
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
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
    });

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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-working-conversation-card") {
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
    });

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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-working-conversation-card") {
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
    });

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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-working-conversation-card") {
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
    });

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("preserves the page-scroll anchor when cycling conversation sort reorders the visible cards", () => {
    const conversations = createResponse([
      {
        ...createConversation("sort-anchor-a", [
          createPreview({
            id: 1,
            invokeId: "invoke-sort-anchor-a",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
        createdAt: "2026-04-04T10:03:00Z",
        lastActivityAt: "2026-04-04T10:04:00Z",
      },
      {
        ...createConversation("sort-anchor-b", [
          createPreview({
            id: 2,
            invokeId: "invoke-sort-anchor-b",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "completed",
          }),
        ]),
        createdAt: "2026-04-04T10:02:00Z",
        lastActivityAt: "2026-04-04T10:03:00Z",
      },
      {
        ...createConversation("sort-anchor-c", [
          createPreview({
            id: 3,
            invokeId: "invoke-sort-anchor-c",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
          }),
        ]),
        createdAt: "2026-04-04T10:01:00Z",
        lastActivityAt: "2026-04-04T10:05:00Z",
      },
    ]);

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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-working-conversation-card") {
        const cards = Array.from(
          this.ownerDocument.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-card"]',
          ),
        );
        const cardIndex = cards.indexOf(this);
        return rectFor(-220 + cardIndex * 180);
      }
      return rectFor(0, 0);
    });

    renderSection(conversations);

    const sortButton = host?.querySelector('[data-testid="dashboard-workspace-sort-button"]');
    if (!(sortButton instanceof HTMLButtonElement)) {
      throw new Error("missing workspace sort button");
    }

    const scrollBy = vi.spyOn(window, "scrollBy");

    act(() => {
      fireEvent.click(sortButton);
    });

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("preserves the page-scroll anchor when cycling account sort reorders the visible cards", () => {
    const baseAccount = createUpstreamAccountActivityResponse().accounts[0]!;
    upstreamAccountActivityMock.data = {
      ...createUpstreamAccountActivityResponse(),
      accounts: [
        {
          ...baseAccount,
          upstreamAccountId: 101,
          displayName: "Pool Alpha",
          latestConversationCreatedAt: "2026-04-04T10:03:00Z",
          lastInvocationAt: "2026-04-04T10:04:00Z",
        },
        {
          ...baseAccount,
          upstreamAccountId: 102,
          displayName: "Pool Beta",
          latestConversationCreatedAt: "2026-04-04T10:02:00Z",
          lastInvocationAt: "2026-04-04T10:03:00Z",
        },
        {
          ...baseAccount,
          upstreamAccountId: 103,
          displayName: "Pool Gamma",
          latestConversationCreatedAt: "2026-04-04T10:01:00Z",
          lastInvocationAt: "2026-04-04T10:05:00Z",
        },
      ],
    };

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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-upstream-account-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-upstream-account-card") {
        const cards = Array.from(
          this.ownerDocument.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-upstream-account-card"]',
          ),
        );
        const cardIndex = cards.indexOf(this);
        return rectFor(-220 + cardIndex * 180);
      }
      return rectFor(0, 0);
    });

    renderSection(
      createResponse([
        createConversation("account-sort-anchor-a", [
          createPreview({
            id: 1,
            invokeId: "invoke-account-sort-anchor-a",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const sortButton = host?.querySelector('[data-testid="dashboard-workspace-sort-button"]');
    if (!(sortButton instanceof HTMLButtonElement)) {
      throw new Error("missing workspace sort button");
    }

    const scrollBy = vi.spyOn(window, "scrollBy");

    act(() => {
      fireEvent.click(sortButton);
    });

    expect(scrollBy).toHaveBeenCalledWith(0, 180);
  });

  it("sorts upstream account cards by descending modes and keeps unassigned rows last", () => {
    const baseAccount = createUpstreamAccountActivityResponse().accounts[0]!;
    upstreamAccountActivityMock.data = {
      ...createUpstreamAccountActivityResponse(),
      accounts: [
        {
          ...baseAccount,
          accountKey: "assigned-mid",
          upstreamAccountId: 101,
          isUnassigned: false,
          displayName: "Pool Mid",
          latestConversationCreatedAt: "2026-04-04T10:03:00Z",
          lastInvocationAt: "2026-04-04T10:03:30Z",
          totalCost: 6,
          totalTokens: 600,
        },
        {
          ...baseAccount,
          accountKey: "unassigned",
          upstreamAccountId: null,
          isUnassigned: true,
          displayName: "未分配上游账号",
          latestConversationCreatedAt: "2026-04-04T10:05:00Z",
          lastInvocationAt: "2026-04-04T10:05:00Z",
          totalCost: 999,
          totalTokens: 99_999,
        },
        {
          ...baseAccount,
          accountKey: "assigned-high",
          upstreamAccountId: 102,
          isUnassigned: false,
          displayName: "Pool High",
          latestConversationCreatedAt: "2026-04-04T10:04:00Z",
          lastInvocationAt: "2026-04-04T10:04:30Z",
          totalCost: 9,
          totalTokens: 900,
        },
      ],
    };

    renderSection(
      createResponse([
        createConversation("account-sort-ordering", [
          createPreview({
            id: 1,
            invokeId: "invoke-account-sort-ordering",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const readOrder = () =>
      Array.from(
        host?.querySelectorAll<HTMLElement>('[data-testid="dashboard-upstream-account-card"]') ??
          [],
      ).map((card) => card.getAttribute("data-account-key"));

    expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);

    const sortButton = host?.querySelector('[data-testid="dashboard-workspace-sort-button"]');
    if (!(sortButton instanceof HTMLButtonElement)) {
      throw new Error("missing workspace sort button");
    }

    act(() => {
      fireEvent.click(sortButton);
    });
    expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);

    act(() => {
      fireEvent.click(sortButton);
    });
    expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);

    act(() => {
      fireEvent.click(sortButton);
    });
    expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);
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

    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
        return rectFor(0, 600);
      }
      if (this.getAttribute("data-testid") === "dashboard-working-conversation-card") {
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
    });

    renderSectionWithCards(initialCards);

    const scrollBy = vi.spyOn(window, "scrollBy");

    rerenderSectionWithCards(nextCards);

    expect(scrollBy).not.toHaveBeenCalled();
  });

  it("keeps a single failed status icon in the slot header and attaches the collapsed error summary to it", () => {
    renderSection(
      createResponse([
        createConversation("pck-failed-status-dedup", [
          createPreview({
            id: 81,
            invokeId: "invoke-failed-status-dedup",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "http_502",
            failureClass: "service_failure",
            errorMessage: LONG_ERROR_SUMMARY,
            failureKind: "upstream_timeout",
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing failed current slot");
    }
    const slotHeader = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    if (!(slotHeader instanceof HTMLElement)) {
      throw new Error("missing failed slot header");
    }

    const statusIcon = slotHeader.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    );
    if (!(statusIcon instanceof HTMLElement)) {
      throw new Error("missing compact failed status icon");
    }

    expect(statusIcon.getAttribute("aria-label")).toContain("失败");
    expect(statusIcon.getAttribute("aria-label")).toContain(LONG_ERROR_SUMMARY);
    expect(statusIcon.getAttribute("title")).toBeNull();
    expect(
      slotHeader.querySelectorAll('[data-testid="dashboard-inline-invocation-status"]'),
    ).toHaveLength(1);
  });

  it("renders the slot error summary as a truncated trigger and exposes the full message on hover", async () => {
    renderSection(
      createResponse([
        createConversation("pck-slot-error-tooltip", [
          createPreview({
            id: 82,
            invokeId: "invoke-slot-error-tooltip",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "http_502",
            failureClass: "service_failure",
            failureKind: "upstream_http_5xx",
            errorMessage: LONG_ERROR_SUMMARY,
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing slot for tooltip test");
    }

    const errorSummary = currentSlot.querySelector('[data-testid="invocation-error-summary"]');
    const errorText = currentSlot.querySelector('[data-testid="invocation-error-summary-text"]');
    const errorTrigger = errorSummary?.parentElement;
    if (
      !(errorSummary instanceof HTMLElement) ||
      !(errorTrigger instanceof HTMLElement) ||
      !(errorText instanceof HTMLElement)
    ) {
      throw new Error("missing shared error summary");
    }

    expect(errorTrigger.getAttribute("title")).toBeNull();
    expect(errorText.getAttribute("title")).toBeNull();
    expect(errorText.className).toContain("truncate");
    expect(errorText.className).toContain("whitespace-nowrap");

    expect(errorTrigger.getAttribute("tabindex")).toBe("0");
    expect(errorTrigger.getAttribute("aria-label")).toBe(LONG_ERROR_SUMMARY);

    await act(async () => {
      fireEvent.mouseOver(errorTrigger);
    });

    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
        node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip).toBeInstanceOf(HTMLElement);
    });
  });

  it("keeps completed slot readings on a single no-wrap row", () => {
    renderSection(
      createResponse([
        createConversation("pck-completed-slot-single-line", [
          createPreview({
            id: 91,
            invokeId: "invoke-completed-slot-current",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "http_502",
            failureClass: "service_failure",
            errorMessage: "upstream gateway closed before first byte",
            failureKind: "upstream_timeout",
          }),
          createPreview({
            id: 90,
            invokeId: "invoke-completed-slot-previous",
            occurredAt: "2026-04-04T10:04:10Z",
            status: "success",
            tReqReadMs: 8,
            tReqParseMs: 6,
            tUpstreamConnectMs: 84,
            tUpstreamTtfbMs: 96,
            tUpstreamStreamMs: 240,
            tRespParseMs: 10,
            tPersistMs: 7,
            tTotalMs: 431,
          }),
        ]),
      ]),
    );

    const previousSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="previous"]',
    );
    if (!(previousSlot instanceof HTMLElement)) {
      throw new Error("missing previous slot");
    }

    const slotReadings = previousSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-readings"]',
    );
    if (!(slotReadings instanceof HTMLElement)) {
      throw new Error("missing previous slot readings");
    }

    const latencyPills = slotReadings.querySelector(
      '[data-testid="dashboard-compact-latency-pills"]',
    );
    if (!(latencyPills instanceof HTMLElement)) {
      throw new Error("missing previous slot latency pills");
    }

    expect(slotReadings.className).toContain("flex-nowrap");
    expect(latencyPills.className).toContain("flex-nowrap");
  });

  it("renders the recent-row error summary as a truncated trigger and exposes the full message on focus", async () => {
    const upstreamActivity = createUpstreamAccountActivityResponse();
    upstreamAccountActivityMock.data = {
      ...upstreamActivity,
      accounts: [
        {
          ...upstreamActivity.accounts[0],
          recentInvocations: upstreamActivity.accounts[0].recentInvocations.map(
            (invocation, index) =>
              index === 0
                ? {
                    ...invocation,
                    status: "http_502",
                    failureClass: "service_failure",
                    failureKind: "upstream_http_5xx",
                    errorMessage: LONG_ERROR_SUMMARY,
                    tUpstreamTtfbMs: null,
                    tUpstreamStreamMs: null,
                    tTotalMs: 22_044,
                  }
                : invocation,
          ),
        },
      ],
    };

    renderSection(
      createResponse([
        createConversation("pck-upstream-recent-error-tooltip", [
          createPreview({
            id: 83,
            invokeId: "invoke-upstream-recent-error-tooltip",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
      node.textContent?.includes("上游账号"),
    );
    if (!(accountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      fireEvent.click(accountTab);
    });

    const recentRow = host?.querySelector('[data-testid="dashboard-upstream-account-recent-row"]');
    if (!(recentRow instanceof HTMLElement)) {
      throw new Error("missing recent row for tooltip test");
    }

    const accountGrid = host?.querySelector('[data-testid="dashboard-upstream-account-grid"]');
    const accountCard = recentRow.closest('[data-testid="dashboard-upstream-account-card"]');
    if (!(accountGrid instanceof HTMLElement) || !(accountCard instanceof HTMLElement)) {
      throw new Error("missing account layout shrink chain");
    }

    const errorSummary = recentRow.querySelector('[data-testid="invocation-error-summary"]');
    const errorText = recentRow.querySelector('[data-testid="invocation-error-summary-text"]');
    const errorTrigger = errorSummary?.parentElement;
    if (
      !(errorSummary instanceof HTMLElement) ||
      !(errorTrigger instanceof HTMLElement) ||
      !(errorText instanceof HTMLElement)
    ) {
      throw new Error("missing recent row error summary");
    }

    expect(errorTrigger.getAttribute("title")).toBeNull();
    expect(errorText.getAttribute("title")).toBeNull();
    expect(errorText.className).toContain("truncate");
    expect(errorText.className).toContain("whitespace-nowrap");
    expect(errorTrigger.getAttribute("tabindex")).toBe("0");
    expect(errorTrigger.getAttribute("aria-label")).toBe(LONG_ERROR_SUMMARY);
    expect(accountGrid.className).toContain("desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]");
    expect(accountCard.className).toContain("min-w-0");
    expect(recentRow.className).toContain("min-w-0");
    expect(errorTrigger.className).toContain("w-full");
    expect(errorTrigger.className).toContain("overflow-hidden");

    await act(async () => {
      errorTrigger.focus();
    });

    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
        node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip).toBeInstanceOf(HTMLElement);
    });
  });

  it("formats dashboard latency pills with at most two decimals and without overflowing past four digits", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00.000Z"));

    renderSection(
      createResponse([
        createConversation("pck-latency-compact", [
          createPreview({
            id: 71,
            invokeId: "invoke-latency-current",
            occurredAt: "2026-04-04T10:04:57.744Z",
            status: "running",
            livePhase: "responding",
            tReqReadMs: 400,
            tReqParseMs: 100,
            tUpstreamConnectMs: 700,
            tUpstreamTtfbMs: 1_056,
            tUpstreamStreamMs: null,
            tTotalMs: null,
          }),
          createPreview({
            id: 70,
            invokeId: "invoke-latency-previous",
            occurredAt: "2026-04-04T10:03:20.000Z",
            status: "completed",
            tReqReadMs: 120,
            tReqParseMs: 36,
            tUpstreamConnectMs: 100,
            tUpstreamTtfbMs: 0,
            tUpstreamStreamMs: 0,
            tTotalMs: 8_028_073.3,
          }),
        ]),
      ]),
    );

    const readings = Array.from(
      host?.querySelectorAll('[data-testid="dashboard-working-conversation-slot-readings"]') ?? [],
    )
      .map((element) => element.textContent ?? "")
      .join(" ");

    expect(readings).toContain("2.26 s");
    expect(readings).toContain("8028 s");
    expect(readings).not.toContain("2.256 s");
    expect(readings).not.toContain("8,028");

    vi.useRealTimers();
  });

  it("toggles selection mode on conversation cards and restores navigation after exit", async () => {
    const onOpenConversation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-select-1", [
          createPreview({
            id: 81,
            invokeId: "invoke-select-1",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
        createConversation("pck-select-2", [
          createPreview({
            id: 82,
            invokeId: "invoke-select-2",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
      { onOpenConversation },
    );

    const user = userEvent.setup();
    const selectionModeButton = host?.querySelector(
      '[data-testid="dashboard-working-conversations-selection-mode-button"]',
    );
    const cards = host?.querySelectorAll<HTMLElement>(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    if (!(selectionModeButton instanceof HTMLButtonElement) || !cards || cards.length < 2) {
      throw new Error("missing selection mode controls");
    }

    await user.click(selectionModeButton);
    await user.click(cards[0]!);
    await waitFor(() =>
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]')
          ?.textContent,
      ).toContain("已选 1 个对话"),
    );
    expect(
      host?.querySelector('button[data-testid="dashboard-working-conversation-sequence-button"]'),
    ).toBeNull();

    const refreshedCards = host?.querySelectorAll<HTMLElement>(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const toggledCard = refreshedCards?.[0];
    if (!(toggledCard instanceof HTMLElement)) {
      throw new Error("missing refreshed conversation card");
    }
    toggledCard.focus();
    fireEvent.keyDown(toggledCard, { key: "Enter" });
    await waitFor(() =>
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]'),
      ).toBeNull(),
    );
    await user.click(
      host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
    );
    expect(onOpenConversation).not.toHaveBeenCalled();

    await user.click(selectionModeButton);
    expect(
      document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]'),
    ).toBeNull();

    const sequenceButton = host?.querySelector(
      'button[data-testid="dashboard-working-conversation-sequence-button"]',
    );
    if (!(sequenceButton instanceof HTMLButtonElement)) {
      throw new Error("missing restored conversation button");
    }

    await user.click(sequenceButton);
    expect(onOpenConversation).toHaveBeenCalledWith(
      expect.objectContaining({ promptCacheKey: "pck-select-1" }),
    );
  });

  it("enters selection mode and toggles selection on cmd/ctrl click", async () => {
    const onOpenConversation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-modifier-select-1", [
          createPreview({
            id: 83,
            invokeId: "invoke-modifier-select-1",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
        createConversation("pck-modifier-select-2", [
          createPreview({
            id: 84,
            invokeId: "invoke-modifier-select-2",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
      { onOpenConversation },
    );

    const sequenceButton = host?.querySelector(
      '[data-testid="dashboard-working-conversation-sequence-button"]',
    );
    if (!(sequenceButton instanceof HTMLButtonElement)) {
      throw new Error("missing conversation sequence button");
    }

    act(() => {
      fireEvent.click(sequenceButton, { metaKey: true, button: 0 });
    });

    await waitFor(() =>
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]')
          ?.textContent,
      ).toContain("已选 1 个对话"),
    );
    expect(onOpenConversation).not.toHaveBeenCalled();
    expect(
      host?.querySelector('[data-testid="dashboard-working-conversations-selection-mode-button"]')
        ?.textContent,
    ).toContain("选择模式");

    const cards = host?.querySelectorAll<HTMLElement>(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const secondCard = cards?.[1];
    if (!(secondCard instanceof HTMLElement)) {
      throw new Error("missing second conversation card");
    }

    act(() => {
      fireEvent.click(secondCard, { ctrlKey: true, button: 0 });
    });

    await waitFor(() =>
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]')
          ?.textContent,
      ).toContain("已选 2 个对话"),
    );
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  it("keeps failed bulk route-binding selections while clearing succeeded ones", async () => {
    const originalFetch = globalThis.fetch;
    let capturedPayload: Record<string, unknown> | null = null;
    const fetchMock = createBulkConversationFetchMock({
      failKeys: ["pck-bulk-bind-2"],
      onBulkPayload: (payload) => {
        capturedPayload = payload;
      },
      groups: [],
      accounts: BULK_BINDING_ACCOUNTS.map((account) => ({
        ...account,
        groupName: "",
      })),
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const onConversationsChanged = vi.fn();

    try {
      renderSection(
        createResponse([
          createConversation("pck-bulk-bind-1", [
            createPreview({
              id: 91,
              invokeId: "invoke-bulk-bind-1",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
            }),
          ]),
          createConversation("pck-bulk-bind-2", [
            createPreview({
              id: 92,
              invokeId: "invoke-bulk-bind-2",
              occurredAt: "2026-04-04T10:04:00Z",
              status: "completed",
            }),
          ]),
        ]),
        { onConversationsChanged },
      );

      const user = userEvent.setup();
      const selectionModeButton = host?.querySelector(
        '[data-testid="dashboard-working-conversations-selection-mode-button"]',
      );
      const cards = host?.querySelectorAll<HTMLElement>(
        '[data-testid="dashboard-working-conversation-card"]',
      );
      if (!(selectionModeButton instanceof HTMLButtonElement) || !cards || cards.length < 2) {
        throw new Error("missing bulk binding selection controls");
      }

      await user.click(selectionModeButton);
      await user.click(cards[0]!);
      await user.click(cards[1]!);
      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-button"]',
        ) as HTMLElement,
      );

      await waitFor(() =>
        expect(
          fetchMock.mock.calls.some(([input]) =>
            String(input).includes("/api/pool/upstream-accounts"),
          ),
        ).toBe(true),
      );
      await waitFor(() =>
        expect(
          document.body.querySelector('[role="combobox"][aria-label="批量账号绑定目标"]'),
        ).not.toBeNull(),
      );

      const applyButton = Array.from(
        document.body.querySelectorAll(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"] button',
        ) ?? [],
      ).find((button) => button.textContent?.includes("应用绑定"));
      if (!(applyButton instanceof HTMLButtonElement)) {
        throw new Error("missing route bind apply button");
      }
      await user.click(applyButton);

      await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
      expect(capturedPayload).toMatchObject({
        action: "bind",
        bindingKind: "upstreamAccount",
        upstreamAccountId: 21,
        promptCacheKeys: ["pck-bulk-bind-1", "pck-bulk-bind-2"],
      });
      await waitFor(() =>
        expect(
          document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]')
            ?.textContent,
        ).toContain("已选 1 个对话"),
      );
      expect(host?.textContent).toContain("有 1 个对话批量操作失败");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("confirms clear-and-reselect before submitting the standalone destructive bulk action", async () => {
    const originalFetch = globalThis.fetch;
    let capturedPayload: Record<string, unknown> | null = null;
    const fetchMock = createBulkConversationFetchMock({
      onBulkPayload: (payload) => {
        capturedPayload = payload;
      },
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const onConversationsChanged = vi.fn();

    try {
      renderSection(
        createResponse([
          createConversation("pck-clear-affinity-1", [
            createPreview({
              id: 101,
              invokeId: "invoke-clear-affinity-1",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
            }),
          ]),
        ]),
        { onConversationsChanged },
      );

      const user = userEvent.setup();
      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-working-conversations-selection-mode-button"]',
        ) as HTMLElement,
      );
      await user.click(
        host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
      );
      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-clear-affinity-button"]',
        ) as HTMLElement,
      );

      const confirmDialog = document.body.querySelector(
        '[data-testid="dashboard-working-conversations-clear-affinity-dialog"]',
      );
      expect(confirmDialog?.textContent).toContain("sticky route");
      expect(confirmDialog?.textContent).toContain("owner lock");

      const confirmButton = Array.from(confirmDialog?.querySelectorAll("button") ?? []).find(
        (button) => button.textContent?.includes("确认清空"),
      );
      if (!(confirmButton instanceof HTMLButtonElement)) {
        throw new Error("missing clear affinity confirm button");
      }
      await user.click(confirmButton);

      await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
      expect(capturedPayload).toMatchObject({
        action: "clearAndResetAffinity",
        promptCacheKeys: ["pck-clear-affinity-1"],
      });
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]'),
      ).toBeNull();
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("confirms manual binding clear from the route bind dialog footer", async () => {
    const originalFetch = globalThis.fetch;
    let capturedPayload: Record<string, unknown> | null = null;
    const fetchMock = createBulkConversationFetchMock({
      onBulkPayload: (payload) => {
        capturedPayload = payload;
      },
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const onConversationsChanged = vi.fn();

    try {
      renderSection(
        createResponse([
          createConversation("pck-clear-binding-1", [
            createPreview({
              id: 121,
              invokeId: "invoke-clear-binding-1",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
            }),
          ]),
        ]),
        { onConversationsChanged },
      );

      const user = userEvent.setup();
      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-working-conversations-selection-mode-button"]',
        ) as HTMLElement,
      );
      await user.click(
        host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
      );
      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-button"]',
        ) as HTMLElement,
      );

      await waitFor(() =>
        expect(
          document.body.querySelector(
            '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
          ),
        ).not.toBeNull(),
      );

      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-clear-button"]',
        ) as HTMLElement,
      );

      const confirmDialog = document.body.querySelector(
        '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
      );
      expect(confirmDialog?.textContent).toContain("手工绑定");
      expect(confirmDialog?.textContent).toContain("sticky route");
      expect(confirmDialog?.textContent).toContain("owner lock");
      expect(confirmDialog?.textContent).not.toContain("重选");

      const confirmButton = Array.from(confirmDialog?.querySelectorAll("button") ?? []).find(
        (button) => button.textContent?.includes("确认清空绑定"),
      );
      if (!(confirmButton instanceof HTMLButtonElement)) {
        throw new Error("missing clear binding confirm button");
      }
      await user.click(confirmButton);

      await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
      expect(capturedPayload).toMatchObject({
        action: "bind",
        bindingKind: "none",
        promptCacheKeys: ["pck-clear-binding-1"],
      });
      expect(
        document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]'),
      ).toBeNull();
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("opens the destructive clear flow from the bulk route bind dialog", async () => {
    const originalFetch = globalThis.fetch;
    const fetchMock = createBulkConversationFetchMock();
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    try {
      renderSection(
        createResponse([
          createConversation("pck-bind-clear-shortcut-1", [
            createPreview({
              id: 111,
              invokeId: "invoke-bind-clear-shortcut-1",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
            }),
          ]),
        ]),
      );

      const user = userEvent.setup();
      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-working-conversations-selection-mode-button"]',
        ) as HTMLElement,
      );
      await user.click(
        host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
      );
      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-button"]',
        ) as HTMLElement,
      );

      await waitFor(() =>
        expect(
          document.body.querySelector(
            '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
          ),
        ).not.toBeNull(),
      );

      const clearShortcutButton = document.body.querySelector(
        '[data-testid="dashboard-working-conversations-route-bind-clear-button"]',
      );
      if (!(clearShortcutButton instanceof HTMLButtonElement)) {
        throw new Error("missing route bind clear shortcut button");
      }

      await user.click(clearShortcutButton);

      await waitFor(() => {
        expect(
          document.body.querySelector(
            '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
          ),
        ).toBeNull();
        expect(
          document.body.querySelector(
            '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
          ),
        ).not.toBeNull();
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("submits the selected bulk FAST mode override", async () => {
    const originalFetch = globalThis.fetch;
    let capturedPayload: Record<string, unknown> | null = null;
    const fetchMock = createBulkConversationFetchMock({
      onBulkPayload: (payload) => {
        capturedPayload = payload;
      },
    });
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const onConversationsChanged = vi.fn();

    try {
      renderSection(
        createResponse([
          createConversation("pck-fast-mode-1", [
            createPreview({
              id: 111,
              invokeId: "invoke-fast-mode-1",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
            }),
          ]),
        ]),
        { onConversationsChanged },
      );

      const user = userEvent.setup();
      await user.click(
        host?.querySelector(
          '[data-testid="dashboard-working-conversations-selection-mode-button"]',
        ) as HTMLElement,
      );
      await user.click(
        host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
      );
      await user.click(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-fast-mode-button"]',
        ) as HTMLElement,
      );

      const fastModeOptions = document.body.querySelectorAll(
        '[data-testid="dashboard-working-conversations-fast-mode-option"]',
      );
      expect(fastModeOptions).toHaveLength(4);
      const forceRemoveOption = Array.from(fastModeOptions).find(
        (option) => (option as HTMLElement).dataset.value === "force_remove",
      );
      if (!(forceRemoveOption instanceof HTMLButtonElement)) {
        throw new Error("missing force remove fast mode option");
      }
      await user.click(forceRemoveOption);

      await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
      await waitFor(() =>
        expect(
          document.body.querySelector(
            '[data-testid="dashboard-working-conversations-fast-mode-popover"]',
          ),
        ).toBeNull(),
      );
      expect(capturedPayload).toMatchObject({
        action: "setFastModeRewriteMode",
        fastModeRewriteMode: "force_remove",
        promptCacheKeys: ["pck-fast-mode-1"],
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
