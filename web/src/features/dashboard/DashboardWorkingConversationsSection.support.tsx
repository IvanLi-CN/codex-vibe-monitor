/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import {
  type DashboardWorkingConversationCardModel,
  mapPromptCacheConversationsToDashboardCards,
} from "../../lib/dashboardWorkingConversations";
import { ThemeProvider } from "../../theme";
import {
  type DashboardOpenUpstreamAccountOptions,
  DashboardWorkingConversationsSection,
} from "./DashboardWorkingConversationsSection";

const LONG_ERROR_SUMMARY =
  '[upstream_http_5xx] pool upstream responded with 502: {"error":{"message":"Upstream request failed","type":"upstream_error"}} event: response.failed data: {"type":"response.failed","response":{"id":"resp_test_error_summary","model":"gpt-5.4","status":"failed"}}';
function requireTestValue<T>(value: T | null | undefined, label: string): T {
  if (value == null) {
    throw new Error(`missing test value: ${label}`);
  }
  return value;
}
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
  }>,
}));
vi.mock("../../hooks/useDashboardUpstreamAccountActivity", () => ({
  resolveUpstreamAccountRecentPreviewLimit: (
    accounts: Array<{ inProgressInvocationCount: number | null }>,
  ) =>
    Math.min(
      16,
      Math.max(
        4,
        Math.max(0, ...accounts.map((account) => account.inProgressInvocationCount ?? 0)),
      ),
    ),
  useDashboardUpstreamAccountActivity: (range: string, enabled: boolean) => {
    upstreamAccountActivityMock.calls.push({ range, enabled });
    return {
      data: upstreamAccountActivityMock.data,
      isLoading: upstreamAccountActivityMock.isLoading,
      isRefreshing: upstreamAccountActivityMock.isRefreshing,
      recentLoading: upstreamAccountActivityMock.recentLoading,
      recentError: upstreamAccountActivityMock.recentError,
      error: upstreamAccountActivityMock.error,
      recentInvocationLimit:
        upstreamAccountActivityMock.resolvedRecentInvocationLimit ??
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
    reasoningEffort:
      "reasoningEffort" in overrides ? (overrides.reasoningEffort ?? undefined) : "high",
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
    tUpstreamTtfbMs: "tUpstreamTtfbMs" in overrides ? (overrides.tUpstreamTtfbMs ?? null) : 70,
    firstTokenMs: "firstTokenMs" in overrides ? (overrides.firstTokenMs ?? null) : undefined,
    tUpstreamStreamMs:
      "tUpstreamStreamMs" in overrides ? (overrides.tUpstreamStreamMs ?? null) : 220,
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
const upstreamAccountActivityResponse: UpstreamAccountActivityResponse = {
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
      firstTokenAvgMs: 2_867.5,
      avgTotalMs: 860,
      currentFirstTokenAvgMs: 2_867.5,
      currentAvgTotalMs: 860,
      currentAvgResponseMs: 860,
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
function createUpstreamAccountActivityResponse(): UpstreamAccountActivityResponse {
  return upstreamAccountActivityResponse;
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
  accounts?: ReadonlyArray<
    Omit<(typeof BULK_BINDING_ACCOUNTS)[number], "groupName"> & { groupName: string }
  >;
}) {
  const failKeys = new Set(options?.failKeys ?? []);
  const accounts = options?.accounts ?? BULK_BINDING_ACCOUNTS;
  const groups = options?.groups ?? [
    { groupName: "CIII", accountCount: 1 },
    { groupName: "Tokyo", accountCount: 1 },
  ];
  return vi.fn((input: RequestInfo | URL, init?: RequestInit) =>
    handleBulkConversationFetch(input, init, {
      accounts,
      groups,
      failKeys,
      onBulkPayload: options?.onBulkPayload,
    }),
  );
}

async function handleBulkConversationFetch(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  options: {
    accounts: ReadonlyArray<unknown>;
    groups: Array<{ groupName: string; accountCount: number }>;
    failKeys: Set<string>;
    onBulkPayload?: (payload: Record<string, unknown>) => void;
  },
) {
  const request =
    typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
  const url = new URL(request, "http://localhost");
  if (url.pathname === "/api/pool/upstream-accounts") {
    return jsonResponse({
      writesEnabled: true,
      items: options.accounts,
      groups: options.groups,
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: options.accounts.length,
      page: 1,
      pageSize: options.accounts.length,
    });
  }
  if (url.pathname !== "/api/stats/prompt-cache-conversation-bindings/bulk-actions") {
    throw new Error(`Unhandled fetch request: ${url.pathname}`);
  }
  const payload = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {};
  options.onBulkPayload?.(payload);
  const keys = Array.isArray(payload.promptCacheKeys)
    ? payload.promptCacheKeys.map((value) => String(value))
    : [];
  const items = keys.map((promptCacheKey) =>
    createBulkBindingItem(promptCacheKey, payload, options.failKeys),
  );
  const succeededCount = items.filter((item) => item.ok).length;
  return jsonResponse({
    action: payload.action ?? "bind",
    totalRequested: items.length,
    totalSucceeded: succeededCount,
    totalFailed: items.length - succeededCount,
    items,
  });
}

function createBulkBindingItem(
  promptCacheKey: string,
  payload: Record<string, unknown>,
  failKeys: Set<string>,
) {
  if (failKeys.has(promptCacheKey)) {
    return { promptCacheKey, ok: false, error: "synthetic failure", binding: null };
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
      upstreamAccountName: payload.bindingKind === "upstreamAccount" ? "Codex Pro - Tokyo" : null,
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
}

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
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
function setHost(nextHost: HTMLDivElement | null) {
  host = nextHost;
}
function setRoot(nextRoot: Root | null) {
  root = nextRoot;
}
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
      slotKind: "current" | "previous" | "earlier";
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
      slotKind: "current" | "previous" | "earlier";
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
      slotKind: "current" | "previous" | "earlier";
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
      slotKind: "current" | "previous" | "earlier";
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

export {
  BULK_BINDING_ACCOUNTS,
  createBulkConversationFetchMock,
  createConversation,
  createPreview,
  createResponse,
  createUpstreamAccountActivityResponse,
  host,
  LONG_ERROR_SUMMARY,
  localStorageMock,
  MockPointerEvent,
  originalResizeObserver,
  renderSection,
  renderSectionWithCards,
  requireTestValue,
  rerenderSection,
  rerenderSectionWithCards,
  root,
  setHost,
  setRoot,
  storage,
  UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS,
  upstreamAccountActivityMock,
  virtualizerMocks,
};
