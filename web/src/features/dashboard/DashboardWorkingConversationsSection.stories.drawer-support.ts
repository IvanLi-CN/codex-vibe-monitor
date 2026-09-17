import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type {
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import {
  type DashboardWorkingConversationInvocationSelection,
  formatDashboardWorkingConversationSequenceId,
} from "../../lib/dashboardWorkingConversations";
import {
  DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS,
  jsonResponse,
} from "./DashboardWorkingConversationsSection.stories.support-base";
import {
  type buildCards,
  buildStoryInvocationSummary,
  type buildStoryMockData,
  resolveInitialSelection,
  StoryNoopEventSource,
} from "./DashboardWorkingConversationsSection.stories.support-extra";

export type DrawerPreviewStoryProps = {
  response: PromptCacheConversationsResponse;
  initialSelection?: {
    promptCacheKey: string;
    slotKind: "current" | "previous" | "earlier";
  };
  initialConversationKey?: string;
  initialConversationTab?: "overview" | "calls" | "settings";
  conversationPresentation?: "overlay" | "page";
  historyInvocationsByPromptCacheKey?: Map<string, PromptCacheConversationInvocationPreview[]>;
  upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
  upstreamAccountActivityLoading?: boolean;
  upstreamAccountActivityRefreshing?: boolean;
  upstreamAccountRecentLoading?: boolean;
  upstreamAccountRecentError?: string | null;
  upstreamAccountRecentPreviewLimit?: number;
  recentPreviewLimit?: number;
  theme?: "vibe-light" | "vibe-dark";
};

export type SelectedConversation = {
  conversationSequenceId: string;
  promptCacheKey: string;
  tab: "overview" | "calls" | "settings";
};

export type SelectedAccount = {
  id: number;
  label: string;
  tab: "overview" | "routing" | "healthEvents";
};

type StoryMocks = ReturnType<typeof buildStoryMockData>;
type BindingResponseBuilder = (
  promptCacheKey: string,
  overrides?: Record<string, unknown>,
) => Record<string, unknown>;

export function createPromptCacheBindingResponse(
  promptCacheKey: string,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
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
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    timeoutFieldSources: {
      responsesFirstByteTimeoutSecs: "account",
      compactFirstByteTimeoutSecs: "group",
      responsesStreamTimeoutSecs: "account",
      compactStreamTimeoutSecs: "root",
    },
    allowSwitchUpstream: false,
    fastModeRewriteMode: "inherit",
    imageToolRewriteMode: "inherit",
    availableModels: ["gpt-5.5", "gpt-5.5-mini"],
    forwardProxyKey: null,
    forwardProxyKeys: [],
    policyFieldSources: {
      allowSwitchUpstream: "conversation",
      fastModeRewriteMode: "account",
      imageToolRewriteMode: "group",
      availableModels: "conversation",
      forwardProxyKey: "account",
    },
    updatedAt: "2026-05-12T16:15:57Z",
    ...overrides,
  };
}

export function useDrawerPreviewSelection(
  cards: ReturnType<typeof buildCards>,
  initialSelection: DrawerPreviewStoryProps["initialSelection"],
  initialConversationKey: string | undefined,
  initialConversationTab: NonNullable<DrawerPreviewStoryProps["initialConversationTab"]>,
) {
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(() =>
      resolveInitialSelection(cards, initialSelection),
    );
  const [selectedConversation, setSelectedConversation] = useState<SelectedConversation | null>(
    () => findInitialConversation(cards, initialConversationKey, initialConversationTab),
  );
  const [selectedAccount, setSelectedAccount] = useState<SelectedAccount | null>(null);

  useEffect(() => {
    setSelectedInvocation(resolveInitialSelection(cards, initialSelection));
    setSelectedConversation(
      findInitialConversation(cards, initialConversationKey, initialConversationTab),
    );
    setSelectedAccount(null);
  }, [cards, initialConversationKey, initialConversationTab, initialSelection]);

  return {
    selectedInvocation,
    setSelectedInvocation,
    selectedConversation,
    setSelectedConversation,
    selectedAccount,
    setSelectedAccount,
  };
}

function findInitialConversation(
  cards: ReturnType<typeof buildCards>,
  initialConversationKey: string | undefined,
  initialConversationTab: SelectedConversation["tab"],
): SelectedConversation | null {
  const initialCard = cards.find((card) => card.promptCacheKey === initialConversationKey);
  return initialCard
    ? {
        conversationSequenceId: initialCard.conversationSequenceId,
        promptCacheKey: initialCard.promptCacheKey,
        tab: initialConversationTab,
      }
    : null;
}

type DrawerFetchContext = {
  originalFetch: typeof window.fetch;
  storyMocks: StoryMocks;
  upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
  bindingState: Map<string, Record<string, unknown>>;
  buildBindingResponse: BindingResponseBuilder;
};

type FetchInput = Parameters<typeof fetch>[0];
type FetchInit = Parameters<typeof fetch>[1];

export function useDrawerPreviewFetch(
  storyMocks: StoryMocks,
  upstreamAccountActivity: UpstreamAccountActivityResponse | null | undefined,
) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const bindingStateRef = useRef<Map<string, Record<string, unknown>>>(new Map());

  useLayoutEffect(() => {
    const originalFetch = originalFetchRef.current ?? window.fetch.bind(window);
    originalFetchRef.current = originalFetch;
    setStoryFetchLogs();
    window.fetch = createDrawerStoryFetchHandler({
      originalFetch,
      storyMocks,
      upstreamAccountActivity,
      bindingState: bindingStateRef.current,
      buildBindingResponse: createPromptCacheBindingResponse,
    });

    return () => {
      window.fetch = originalFetch;
    };
  }, [storyMocks, upstreamAccountActivity]);
}

export function useDrawerPreviewEventSource() {
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);

  useLayoutEffect(() => {
    originalEventSourceRef.current = window.EventSource;
    window.EventSource = StoryNoopEventSource as unknown as typeof window.EventSource;
    return () => {
      if (originalEventSourceRef.current) window.EventSource = originalEventSourceRef.current;
      originalEventSourceRef.current = null;
    };
  }, []);
}

function setStoryFetchLogs() {
  (window as typeof window & { __dashboardStoryFetchLog?: string[] }).__dashboardStoryFetchLog = [];
  (
    window as typeof window & { __dashboardStoryPolicyPatchLog?: string[] }
  ).__dashboardStoryPolicyPatchLog = [];
}

function createDrawerStoryFetchHandler(context: DrawerFetchContext): typeof window.fetch {
  return async (input, init) => {
    const request = getFetchRequest(input);
    const url = new URL(request, window.location.origin);
    recordStoryFetch(url);
    return (
      (await handleBindingRoutes(url, init, context)) ??
      (await handleAccountRoutes(url, init)) ??
      (await handleInvocationRoutes(url, context)) ??
      (await handleActivityRoutes(url, context.upstreamAccountActivity)) ??
      context.originalFetch(input, init)
    );
  };
}

function getFetchRequest(input: FetchInput): string {
  return typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
}

function recordStoryFetch(url: URL) {
  (
    window as typeof window & { __dashboardStoryFetchLog?: string[] }
  ).__dashboardStoryFetchLog?.push(`${url.pathname}?${url.searchParams.toString()}`);
}

async function handleBindingRoutes(
  url: URL,
  init: FetchInit,
  context: DrawerFetchContext,
): Promise<Response | null> {
  if (
    url.pathname === "/api/stats/prompt-cache-conversation-bindings/bulk-actions" &&
    init?.method === "POST"
  ) {
    return handleBulkBindingRequest(init, context);
  }
  const match = url.pathname.match(/^\/api\/stats\/prompt-cache-conversation-bindings\/(.+)$/);
  if (!match) return null;
  return handleSingleBindingRequest(decodeURIComponent(match[1] ?? ""), init, context);
}

async function handleBulkBindingRequest(
  init: RequestInit,
  context: DrawerFetchContext,
): Promise<Response> {
  const payload = (init.body ? JSON.parse(String(init.body)) : {}) as Record<string, unknown>;
  const keys = Array.isArray(payload.promptCacheKeys)
    ? payload.promptCacheKeys.map((key: unknown) => String(key))
    : [];
  const items = keys.map((key: string) => {
    const current = context.bindingState.get(key) ?? context.buildBindingResponse(key);
    const next = buildBulkBindingResult(key, payload, current, context.buildBindingResponse);
    context.bindingState.set(key, next);
    return { promptCacheKey: key, ok: true, error: null, binding: next };
  });
  return jsonResponse({
    action: payload.action ?? "bind",
    totalRequested: keys.length,
    totalSucceeded: keys.length,
    totalFailed: 0,
    items,
  });
}

function buildBulkBindingResult(
  key: string,
  payload: Record<string, unknown>,
  current: Record<string, unknown>,
  buildBindingResponse: BindingResponseBuilder,
) {
  if (payload.action === "bind") {
    const account = DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.find(
      (candidate) => candidate.id === Number(payload.upstreamAccountId),
    );
    return buildBindingResponse(key, {
      ...current,
      bindingKind: payload.bindingKind,
      groupName: payload.bindingKind === "group" ? String(payload.groupName ?? "") : null,
      upstreamAccountId:
        payload.bindingKind === "upstreamAccount" ? Number(payload.upstreamAccountId) : null,
      upstreamAccountName:
        payload.bindingKind === "upstreamAccount" ? (account?.displayName ?? null) : null,
    });
  }
  if (payload.action === "clearAndResetAffinity") {
    return buildBindingResponse(key, {
      ...current,
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
    });
  }
  return buildBindingResponse(key, {
    ...current,
    fastModeRewriteMode: payload.fastModeRewriteMode ?? current.fastModeRewriteMode,
  });
}

async function handleSingleBindingRequest(
  promptCacheKey: string,
  init: FetchInit,
  context: DrawerFetchContext,
): Promise<Response> {
  const current =
    context.bindingState.get(promptCacheKey) ?? context.buildBindingResponse(promptCacheKey);
  if (init?.method === "PATCH") {
    const payload = (init.body ? JSON.parse(String(init.body)) : {}) as Record<string, unknown>;
    const next = buildSingleBindingResult(
      promptCacheKey,
      payload,
      current,
      context.buildBindingResponse,
    );
    context.bindingState.set(promptCacheKey, next);
    return jsonResponse(next);
  }
  context.bindingState.set(promptCacheKey, current);
  return jsonResponse(current);
}

function buildSingleBindingResult(
  key: string,
  payload: Record<string, unknown>,
  current: Record<string, unknown>,
  buildBindingResponse: BindingResponseBuilder,
) {
  const account = DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.find(
    (candidate) => candidate.id === Number(payload.upstreamAccountId),
  );
  return buildBindingResponse(key, {
    ...current,
    ...("bindingKind" in payload ? { bindingKind: payload.bindingKind } : null),
    groupName: resolveBindingValue(
      payload.bindingKind,
      "group",
      payload.groupName,
      current.groupName,
    ),
    upstreamAccountId: resolveBindingValue(
      payload.bindingKind,
      "upstreamAccount",
      Number(payload.upstreamAccountId),
      current.upstreamAccountId,
    ),
    upstreamAccountName: resolveBindingValue(
      payload.bindingKind,
      "upstreamAccount",
      account?.displayName ?? null,
      current.upstreamAccountName,
    ),
    timeouts: mergeTimeouts(payload.timeouts, current.timeouts),
    updatedAt: "2026-05-12T16:20:00Z",
  });
}

function resolveBindingValue(
  bindingKind: unknown,
  targetKind: string,
  targetValue: unknown,
  fallback: unknown,
) {
  return bindingKind === targetKind ? targetValue : bindingKind === "none" ? null : fallback;
}

function mergeTimeouts(payloadTimeouts: unknown, currentTimeouts: unknown) {
  if (payloadTimeouts == null || typeof payloadTimeouts !== "object") return currentTimeouts;
  return {
    ...(typeof currentTimeouts === "object" && currentTimeouts != null ? currentTimeouts : {}),
    ...(payloadTimeouts as Record<string, unknown>),
  };
}

async function handleAccountRoutes(url: URL, init: FetchInit): Promise<Response | null> {
  if (url.pathname === "/api/pool/upstream-accounts") {
    return jsonResponse({
      writesEnabled: true,
      items: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS,
      groups: [
        {
          name: "CIII",
          accountCount: 1,
          oauthAccountCount: 1,
          apiKeyAccountCount: 0,
          enabledAccountCount: 1,
          disabledAccountCount: 0,
          activeConversationCount: 2,
        },
        {
          name: "Tokyo",
          accountCount: 1,
          oauthAccountCount: 1,
          apiKeyAccountCount: 0,
          enabledAccountCount: 1,
          disabledAccountCount: 0,
          activeConversationCount: 1,
        },
      ],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
      page: 1,
      pageSize: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
      metrics: {
        total: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
        oauth: DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.length,
        apiKey: 0,
        attention: 0,
      },
    });
  }
  const match = url.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);
  if (!match || init?.method !== "PATCH" || !init) return null;
  const patchLog =
    (window as typeof window & { __dashboardStoryPolicyPatchLog?: string[] })
      .__dashboardStoryPolicyPatchLog ?? [];
  patchLog.push(typeof init.body === "string" ? init.body : "");
  (
    window as typeof window & { __dashboardStoryPolicyPatchLog?: string[] }
  ).__dashboardStoryPolicyPatchLog = patchLog;
  return jsonResponse({
    id: Number(match[1]),
    displayName: "Pool Alpha",
    status: "active",
    routingRule: {},
  });
}

async function handleInvocationRoutes(
  url: URL,
  context: DrawerFetchContext,
): Promise<Response | null> {
  if (url.pathname === "/api/invocations") return handleInvocationList(url, context.storyMocks);
  const summaryKey = url.pathname === "/api/invocations/summary";
  if (summaryKey) {
    const key = url.searchParams.get("promptCacheKey");
    return jsonResponse(
      buildStoryInvocationSummary(
        key == null ? [] : (context.storyMocks.recordsByPromptCacheKey.get(key) ?? []),
      ),
    );
  }
  const detail = url.pathname.match(/^\/api\/invocations\/(\d+)\/(detail|response-body)$/);
  if (detail) return handleInvocationDetail(detail[1] ?? "", detail[2], context.storyMocks);
  const attempts = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/);
  return attempts
    ? jsonResponse(
        context.storyMocks.poolAttemptsByInvokeId.get(decodeURIComponent(attempts[1] ?? "")) ?? [],
      )
    : null;
}

function handleInvocationList(url: URL, storyMocks: StoryMocks): Response {
  const invokeId = url.searchParams.get("invokeId") ?? url.searchParams.get("requestId");
  if (invokeId) {
    const record = storyMocks.recordsByInvokeId.get(invokeId);
    return jsonResponse({
      snapshotId: 1,
      total: record ? 1 : 0,
      page: 1,
      pageSize: 1,
      records: record ? [record] : [],
    });
  }
  const key = url.searchParams.get("promptCacheKey")?.trim();
  const records =
    key == null
      ? []
      : (storyMocks.recordsByPromptCacheKey.get(key) ?? [])
          .slice()
          .sort((left, right) => right.occurredAt.localeCompare(left.occurredAt));
  const page = Math.max(1, Number(url.searchParams.get("page") ?? "1"));
  const pageSize = Math.max(1, Number(url.searchParams.get("pageSize") ?? "200"));
  const start = (page - 1) * pageSize;
  return jsonResponse({
    snapshotId: 1,
    total: records.length,
    page,
    pageSize,
    records: records.slice(start, start + pageSize),
  });
}

function handleInvocationDetail(recordId: string, kind: string, storyMocks: StoryMocks): Response {
  const id = Number(recordId);
  if (kind === "detail") {
    return jsonResponse(storyMocks.detailByRecordId.get(id) ?? { id, abnormalResponseBody: null });
  }
  return jsonResponse(
    storyMocks.responseBodyByRecordId.get(id) ?? {
      available: false,
      unavailableReason: "No storybook response body for this record.",
    },
  );
}

async function handleActivityRoutes(
  url: URL,
  activity: UpstreamAccountActivityResponse | null | undefined,
): Promise<Response | null> {
  if (url.pathname === "/api/stats/upstream-account-activity")
    return jsonResponse(activity ?? emptyActivity());
  if (url.pathname !== "/api/stats/dashboard-activity") return null;
  const current = activity ?? emptyActivity();
  const stats = current.accounts.reduce(
    (summary, account) => ({
      totalCount: summary.totalCount + account.requestCount,
      successCount: summary.successCount + account.successCount,
      failureCount: summary.failureCount + account.failureCount,
      totalCost: summary.totalCost + account.totalCost,
      totalTokens: summary.totalTokens + account.totalTokens,
      inProgressConversationCount:
        summary.inProgressConversationCount + (account.inProgressInvocationCount ?? 0),
    }),
    {
      totalCount: 0,
      successCount: 0,
      failureCount: 0,
      totalCost: 0,
      totalTokens: 0,
      inProgressConversationCount: 0,
    },
  );
  return jsonResponse({
    range: current.range,
    rangeStart: current.rangeStart,
    rangeEnd: current.rangeEnd,
    snapshotId: Date.parse(current.rangeEnd) || 0,
    rateWindow: {
      start: current.rangeStart,
      end: current.rangeEnd,
      windowMinutes: 1,
      mode: "rolling_60s_live_mean",
    },
    summary: {
      stats,
      tokensPerMinute: current.accounts.reduce(
        (sum, account) => sum + (account.tokensPerMinute ?? 0),
        0,
      ),
      spendRate: current.accounts.reduce((sum, account) => sum + (account.spendRate ?? 0), 0),
    },
    accounts: url.searchParams.get("includeAccounts") === "false" ? undefined : current.accounts,
  });
}

function emptyActivity(): UpstreamAccountActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    accounts: [],
  };
}

export function formatConversationLabel(conversation: SelectedConversation | null) {
  return conversation
    ? formatDashboardWorkingConversationSequenceId(conversation.conversationSequenceId)
    : null;
}
