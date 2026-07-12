import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider, useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
  ApiPoolUpstreamRequestAttempt,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
  mapPromptCacheConversationsToDashboardCards,
  type DashboardWorkingConversationInvocationSelection,
} from "../../lib/dashboardWorkingConversations";
import { DashboardInvocationDetailDrawer } from "./DashboardInvocationDetailDrawer";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import { PromptCacheConversationHistoryDrawer } from "../prompt-cache/PromptCacheConversationTable";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import { DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY } from "./dashboardActivityRange";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <div className="app-shell-boundary">{children}</div>
    </div>
  );
}

const DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS = [
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

function ForcedWorkspaceViewStory({
  view,
  children,
}: {
  view: "conversations" | "upstreamAccounts";
  children: ReactNode;
}) {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY, view);
  }
  return <>{children}</>;
}

function useStoryTheme(theme?: "vibe-light" | "vibe-dark") {
  useLayoutEffect(() => {
    if (!theme) return;
    const previousHtmlTheme =
      document.documentElement.getAttribute("data-theme");
    const previousBodyTheme = document.body.getAttribute("data-theme");
    document.documentElement.setAttribute("data-theme", theme);
    document.body.setAttribute("data-theme", theme);
    return () => {
      if (previousHtmlTheme) {
        document.documentElement.setAttribute("data-theme", previousHtmlTheme);
      } else {
        document.documentElement.removeAttribute("data-theme");
      }
      if (previousBodyTheme) {
        document.body.setAttribute("data-theme", previousBodyTheme);
      } else {
        document.body.removeAttribute("data-theme");
      }
    };
  }, [theme]);
}

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "Content-Type": "application/json",
    },
  });
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
    promptCacheKey:
      "promptCacheKey" in overrides ? (overrides.promptCacheKey ?? null) : null,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    livePhase: overrides.livePhase ?? null,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    requestModel:
      "requestModel" in overrides
        ? (overrides.requestModel ?? null)
        : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
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
    transport: overrides.transport,
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

function isInFlightStatus(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase() ?? "";
  return normalized === "running" || normalized === "pending";
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  const lastInFlightPreview = recentInvocations.find((preview) =>
    isInFlightStatus(preview.status),
  );
  const lastTerminalPreview = recentInvocations.find(
    (preview) => !isInFlightStatus(preview.status),
  );
  return {
    promptCacheKey,
    hasEncryptedSessionOwner: overrides.hasEncryptedSessionOwner ?? false,
    encryptedOwnerAccountId: overrides.encryptedOwnerAccountId ?? null,
    encryptedOwnerAccountName: overrides.encryptedOwnerAccountName ?? null,
    encryptedOwnerGroupName: overrides.encryptedOwnerGroupName ?? null,
    requestCount: overrides.requestCount ?? recentInvocations.length,
    totalTokens:
      overrides.totalTokens ??
      recentInvocations.reduce(
        (sum, preview) => sum + Math.max(0, preview.totalTokens),
        0,
      ),
    totalCost:
      overrides.totalCost ??
      Number(
        recentInvocations
          .reduce((sum, preview) => sum + (preview.cost ?? 0), 0)
          .toFixed(4),
      ),
    createdAt:
      overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt:
      overrides.lastActivityAt ??
      recentInvocations[0]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastTerminalAt:
      overrides.lastTerminalAt ?? lastTerminalPreview?.occurredAt ?? null,
    lastInFlightAt:
      overrides.lastInFlightAt ?? lastInFlightPreview?.occurredAt ?? null,
    cursor: overrides.cursor ?? promptCacheKey,
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

function createRelativeStoryIso(offsetMs: number) {
  return new Date(Date.now() + offsetMs).toISOString();
}

function buildRecordFromPreview(
  preview: PromptCacheConversationInvocationPreview,
): ApiInvocation {
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    promptCacheKey: preview.promptCacheKey ?? undefined,
    occurredAt: preview.occurredAt,
    createdAt: preview.occurredAt,
    source: preview.source ?? "pool",
    routeMode: preview.routeMode ?? "pool",
    proxyDisplayName: preview.proxyDisplayName ?? undefined,
    upstreamAccountId: preview.upstreamAccountId ?? null,
    upstreamAccountName: preview.upstreamAccountName ?? undefined,
    endpoint: preview.endpoint ?? undefined,
    compactionRequestKind: preview.compactionRequestKind ?? null,
    compactionResponseKind: preview.compactionResponseKind ?? null,
    imageIntent: preview.imageIntent ?? null,
    transport: preview.transport,
    model: preview.model ?? undefined,
    requestModel: preview.requestModel ?? undefined,
    responseModel: preview.responseModel ?? undefined,
    status: preview.status,
    inputTokens: preview.inputTokens,
    outputTokens: preview.outputTokens,
    cacheInputTokens: preview.cacheInputTokens,
    reasoningTokens: preview.reasoningTokens,
    reasoningEffort: preview.reasoningEffort,
    totalTokens: preview.totalTokens,
    cost: preview.cost ?? undefined,
    errorMessage: preview.errorMessage,
    failureKind: preview.failureKind,
    failureClass: preview.failureClass ?? undefined,
    isActionable: preview.isActionable,
    responseContentEncoding: preview.responseContentEncoding ?? undefined,
    requestedServiceTier: preview.requestedServiceTier ?? undefined,
    serviceTier: preview.serviceTier ?? undefined,
    tReqReadMs: preview.tReqReadMs,
    tReqParseMs: preview.tReqParseMs,
    tUpstreamConnectMs: preview.tUpstreamConnectMs,
    tUpstreamTtfbMs: preview.tUpstreamTtfbMs,
    tUpstreamStreamMs: preview.tUpstreamStreamMs,
    tRespParseMs: preview.tRespParseMs,
    tPersistMs: preview.tPersistMs,
    tTotalMs: preview.tTotalMs,
  };
}

function createUpstreamAccountActivityStoryResponse(
  recentInvocationCount = 4,
  routingRuleOverrides: Partial<
    NonNullable<
      UpstreamAccountActivityResponse["accounts"][number]["effectiveRoutingRule"]
    >
  > = {},
): UpstreamAccountActivityResponse {
  const promptCacheKeys = [
    "tone-seed-4",
    "tone-seed-12",
    "tone-seed-17",
    "tone-seed-25",
    "pck-story-account-retry",
    "pck-story-account-cache",
    "pck-story-account-edition",
    "pck-story-account-proxy",
    "pck-story-account-burst",
    "pck-story-account-latency",
    "pck-story-account-stream",
    "pck-story-account-owner",
    "pck-story-account-route",
    "pck-story-account-budget",
    "pck-story-account-archive",
    "pck-story-account-final",
  ];
  const statuses = [
    "success",
    "success",
    "failed",
    "success",
    "success",
    "success",
    "success",
    "failed",
    "pending",
    "success",
    "success",
    "success",
    "failed",
    "success",
    "success",
    "success",
  ];
  const recentInvocations = Array.from(
    { length: Math.max(0, Math.min(recentInvocationCount, 16)) },
    (_, index) =>
      createPreview({
        id: 9901 + index,
        invokeId: `story-account-${index + 1}`,
        promptCacheKey:
          promptCacheKeys[index] ?? `pck-story-account-${index + 1}`,
        occurredAt: `2026-04-04T10:${String(Math.max(0, 5 - index)).padStart(2, "0")}:00Z`,
        status: statuses[index] ?? "success",
        livePhase:
          statuses[index] === "pending"
            ? "queued"
            : statuses[index] === "running" && index % 2 === 0
              ? "responding"
              : statuses[index] === "running"
                ? "requesting"
                : null,
        upstreamAccountId: 42,
        upstreamAccountName: "Pool Alpha",
        requestModel: index === 0 ? "gpt-5.5-mini" : undefined,
        responseModel: index === 0 ? "gpt-5.5" : undefined,
        model: index === 0 ? "gpt-5.5" : undefined,
        imageIntent: index === 0 ? "yes" : undefined,
        tTotalMs: index === 0 ? 20_000 : undefined,
      }),
  );
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
        enabled: true,
        displayStatus: "upstream_rejected",
        enableStatus: "enabled",
        workStatus: "rate_limited",
        healthStatus: "upstream_rejected",
        syncState: "idle",
        lastError: "upstream rejected the request",
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
            input: 0.12,
            cacheWrite: 0.1,
            cacheRead: 0.04,
            output: 0.21,
            reasoning: 0.05,
            unknown: 0.2,
          },
          models: [
            {
              model: "gpt-5.6",
              reasoningEffort: "high",
              cacheWriteTokens: 1200,
              cacheReadTokens: 600,
              outputTokens: 620,
              costs: { input: 0.12, cacheWrite: 0.1, cacheRead: 0.04, output: 0.21, reasoning: 0.05, unknown: 0 },
            },
            {
              model: "gpt-5.4-mini",
              reasoningEffort: null,
              cacheWriteTokens: 400,
              cacheReadTokens: 200,
              outputTokens: 180,
              costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0.2 },
            },
          ],
        },
        cacheHitRate: 0.25,
        tokensPerMinute: 640,
        spendRate: 0.12,
        firstByteAvgMs: 420,
        firstResponseByteTotalAvgMs: 2_867.5,
        avgTotalMs: 860,
        inProgressInvocationCount: 9,
        inProgressPhaseCounts: {
          queued: 2,
          requesting: 3,
          responding: 4,
        },
        retryInvocationCount: 1,
        effectiveRoutingRule: {
          allowCutOut: true,
          allowCutIn: false,
          priorityTier: "no_new",
          fastModeRewriteMode: "force_add",
          imageToolRewriteMode: "keep_original",
          concurrencyLimit: 3,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 2,
          ...routingRuleOverrides,
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
            upstream429Retry: "group",
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
        recentInvocations,
      },
    ],
  };
}

const LONG_ERROR_SUMMARY =
  '[upstream_http_5xx] pool upstream responded with 502: {"error":{"message":"Upstream request failed","type":"upstream_error"}} event: response.failed data: {"type":"response.failed","response":{"id":"resp_story_error_summary","model":"gpt-5.4","status":"failed"}}';

const currentAndPreviousResponse = createResponse([
  createConversation("pck-current-previous", [
    createPreview({
      id: 12,
      invokeId: "invoke-12",
      occurredAt: "2026-04-04T10:04:20Z",
      status: "completed",
      upstreamAccountName: "growth-alpha@example.com",
      upstreamAccountPlanType: "plus",
      reasoningEffort: "medium",
      imageIntent: "yes",
      tTotalMs: 20_000,
    }),
    createPreview({
      id: 11,
      invokeId: "invoke-11",
      occurredAt: "2026-04-04T10:01:12Z",
      status: "completed",
      model: "gpt-5.4-mini",
      upstreamAccountName: "backup-alpha@example.com",
      upstreamAccountPlanType: "free",
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

function createRunningOnlyResponse() {
  return createResponse([
    createConversation("pck-running-only", [
      createPreview({
        id: 31,
        invokeId: "invoke-31",
        occurredAt: createRelativeStoryIso(-2_400),
        status: "running",
        livePhase: "responding",
        upstreamAccountName: "watch-alpha@example.com",
        reasoningEffort: "medium",
        tTotalMs: null,
      }),
      createPreview({
        id: 30,
        invokeId: "invoke-30",
        occurredAt: createRelativeStoryIso(-(11 * 60_000 + 39_000)),
        status: "completed",
        upstreamAccountName: "watch-alpha@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
  ]);
}

function createRequestingOnlyResponse() {
  return createResponse([
    createConversation("pck-requesting-only", [
      createPreview({
        id: 32,
        invokeId: "invoke-32",
        occurredAt: createRelativeStoryIso(-750),
        status: "running",
        livePhase: "requesting",
        upstreamAccountName: "request-alpha@example.com",
        reasoningEffort: "medium",
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: null,
      }),
      createPreview({
        id: 29,
        invokeId: "invoke-29",
        occurredAt: createRelativeStoryIso(-(12 * 60_000 + 18_000)),
        status: "completed",
        upstreamAccountName: "request-alpha@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
  ]);
}

function createPoolRoutingAccountStatesResponse() {
  return createResponse([
    createConversation("pck-routing-account-named", [
      createPreview({
        id: 41,
        invokeId: "invoke-routing-account-named",
        occurredAt: createRelativeStoryIso(-1_600),
        status: "running",
        livePhase: "responding",
        upstreamAccountId: 42,
        upstreamAccountName: "pool-alpha@example.com",
        tTotalMs: null,
      }),
    ]),
    createConversation("pck-routing-account-missing", [
      createPreview({
        id: 42,
        invokeId: "invoke-routing-account-missing",
        occurredAt: createRelativeStoryIso(-3_200),
        status: "pending",
        livePhase: "requesting",
        upstreamAccountId: null,
        upstreamAccountName: null,
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: null,
      }),
    ]),
    createConversation("pck-routing-account-terminal", [
      createPreview({
        id: 43,
        invokeId: "invoke-routing-account-terminal",
        occurredAt: createRelativeStoryIso(-8_000),
        status: "completed",
        upstreamAccountId: 42,
        upstreamAccountName: "pool-alpha@example.com",
      }),
    ]),
  ]);
}

const accountPlanBadgeResponse = createResponse([
  createConversation("pck-plan-enterprise", [
    createPreview({
      id: 221,
      invokeId: "invoke-plan-enterprise",
      occurredAt: "2026-04-04T10:04:58Z",
      status: "running",
      upstreamAccountName:
        "maximiliano.joseph8832.enterprise-routing-lab@example.com",
      upstreamAccountPlanType: "enterprise",
      reasoningEffort: "high",
      tTotalMs: null,
    }),
    createPreview({
      id: 220,
      invokeId: "invoke-plan-team",
      occurredAt: "2026-04-04T10:02:40Z",
      status: "completed",
      upstreamAccountName:
        "maximiliano.joseph8832.enterprise-routing-lab@example.com",
      upstreamAccountPlanType: "team",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-plan-plus-free", [
    createPreview({
      id: 219,
      invokeId: "invoke-plan-plus",
      occurredAt: "2026-04-04T10:03:58Z",
      status: "completed",
      upstreamAccountName: "plus-account-osaka@example.com",
      upstreamAccountPlanType: "plus",
      reasoningEffort: "medium",
    }),
    createPreview({
      id: 218,
      invokeId: "invoke-plan-free",
      occurredAt: "2026-04-04T10:01:20Z",
      status: "completed",
      upstreamAccountName: "free-account-berlin@example.com",
      upstreamAccountPlanType: "free",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const transportBadgeResponse = createResponse([
  createConversation("pck-websocket-mixed", [
    createPreview({
      id: 36,
      invokeId: "invoke-ws-current",
      occurredAt: "2026-04-04T10:04:55Z",
      status: "running",
      transport: "websocket",
      upstreamAccountName: "ws-alpha@example.com",
      reasoningEffort: "medium",
      tTotalMs: null,
    }),
    createPreview({
      id: 35,
      invokeId: "invoke-http-previous",
      occurredAt: "2026-04-04T10:02:28Z",
      status: "completed",
      transport: null,
      upstreamAccountName: "ws-alpha@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-http-control", [
    createPreview({
      id: 34,
      invokeId: "invoke-http-control",
      occurredAt: "2026-04-04T10:03:42Z",
      status: "completed",
      upstreamAccountName: "http-control@example.com",
      model: "gpt-5.4",
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
      reasoningEffort: "medium",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/chat/completions",
      requestedServiceTier: "auto",
      serviceTier: "auto",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 30018,
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

const failedStatusDedupResponse = createResponse([
  createConversation("pck-failed-status-dedup", [
    createPreview({
      id: 43,
      invokeId: "invoke-failed-dedup",
      occurredAt: "2026-04-04T10:03:58Z",
      status: "http_502",
      failureClass: "service_failure",
      errorMessage: "upstream gateway closed before first byte",
      failureKind: "upstream_timeout",
      reasoningEffort: "medium",
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/responses",
      tReqReadMs: 12,
      tReqParseMs: 8,
      tUpstreamConnectMs: 103,
      tUpstreamTtfbMs: 1_640,
      tUpstreamStreamMs: 0,
      tTotalMs: 13_050,
    }),
  ]),
]);

function buildDashboardHistoryEvidenceFixtures() {
  const promptCacheKey = "pck-dashboard-history-realistic";
  const topRecords = [
    createPreview({
      id: 910,
      invokeId: "invoke-history-910",
      occurredAt: "2026-05-12T08:15:57Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 311,
      upstreamAccountName: "pool-ci-311@example.com",
      endpoint: "/v1/responses",
      inputTokens: 164_400,
      cacheInputTokens: 156_032,
      outputTokens: 37,
      reasoningTokens: 0,
      totalTokens: 164_437,
      cost: 0.121,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      requestedServiceTier: "auto",
      serviceTier: "priority",
      tTotalMs: 29_470,
    }),
    createPreview({
      id: 909,
      invokeId: "invoke-history-909",
      occurredAt: "2026-05-12T08:15:37Z",
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "upstream_timeout",
      errorMessage: "[downstream_reset] upstream closed before first byte",
      model: "gpt-5.5",
      upstreamAccountId: 311,
      upstreamAccountName: "pool-ci-311@example.com",
      endpoint: "/v1/responses",
      inputTokens: 163_784,
      cacheInputTokens: 155_520,
      outputTokens: 570,
      reasoningTokens: 137,
      totalTokens: 164_354,
      cost: 0.1362,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 59_000,
    }),
    createPreview({
      id: 908,
      invokeId: "invoke-history-908",
      occurredAt: "2026-05-12T08:15:00Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 312,
      upstreamAccountName: "pool-ci-312@example.com",
      endpoint: "/v1/responses",
      inputTokens: 163_496,
      cacheInputTokens: 155_520,
      outputTokens: 37,
      reasoningTokens: 0,
      totalTokens: 163_533,
      cost: 0.1188,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 16_280,
    }),
    createPreview({
      id: 907,
      invokeId: "invoke-history-907",
      occurredAt: "2026-05-12T08:14:00Z",
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "downstream_reset",
      errorMessage: "[downstream_reset] response stream reset",
      model: "gpt-5.5",
      upstreamAccountId: 311,
      upstreamAccountName: "pool-ci-311@example.com",
      endpoint: "/v1/responses",
      inputTokens: 163_101,
      cacheInputTokens: 155_008,
      outputTokens: 348,
      reasoningTokens: 80,
      totalTokens: 163_449,
      cost: 0.1284,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 50_960,
    }),
    createPreview({
      id: 906,
      invokeId: "invoke-history-906",
      occurredAt: "2026-05-12T08:13:26Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 312,
      upstreamAccountName: "pool-ci-312@example.com",
      endpoint: "/v1/responses",
      inputTokens: 162_990,
      cacheInputTokens: 154_880,
      outputTokens: 42,
      reasoningTokens: 0,
      totalTokens: 163_032,
      cost: 0.1171,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 33_760,
    }),
    createPreview({
      id: 905,
      invokeId: "invoke-history-905",
      occurredAt: "2026-05-12T08:13:10Z",
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "upstream_timeout",
      errorMessage: "[upstream_read_timeout] upstream read timed out",
      model: "gpt-5.5",
      upstreamAccountId: 313,
      upstreamAccountName: "pool-ci-313@example.com",
      endpoint: "/v1/responses",
      inputTokens: 163_496,
      cacheInputTokens: 155_520,
      outputTokens: 37,
      reasoningTokens: 0,
      totalTokens: 163_533,
      cost: 0.1188,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 16_280,
    }),
    createPreview({
      id: 904,
      invokeId: "invoke-history-904",
      occurredAt: "2026-05-12T08:12:18Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 312,
      upstreamAccountName: "pool-ci-312@example.com",
      endpoint: "/v1/responses",
      inputTokens: 162_880,
      cacheInputTokens: 154_752,
      outputTokens: 41,
      reasoningTokens: 0,
      totalTokens: 162_921,
      cost: 0.1167,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 35_520,
    }),
    createPreview({
      id: 903,
      invokeId: "invoke-history-903",
      occurredAt: "2026-05-12T08:11:42Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 314,
      upstreamAccountName: "pool-ci-314@example.com",
      endpoint: "/v1/responses",
      inputTokens: 162_720,
      cacheInputTokens: 154_624,
      outputTokens: 46,
      reasoningTokens: 0,
      totalTokens: 162_766,
      cost: 0.1164,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 35_520,
    }),
    createPreview({
      id: 902,
      invokeId: "invoke-history-902",
      occurredAt: "2026-05-12T08:00:34Z",
      status: "completed",
      model: "gpt-5.5",
      upstreamAccountId: 315,
      upstreamAccountName: "pool-ci-315@example.com",
      endpoint: "/v1/responses",
      inputTokens: 160_104,
      cacheInputTokens: 151_920,
      outputTokens: 37,
      reasoningTokens: 0,
      totalTokens: 160_141,
      cost: 0.121,
      reasoningEffort: "high",
      responseContentEncoding: "identity",
      tTotalMs: 29_470,
    }),
  ];

  const fillerSlots = [
    {
      startAt: "2026-05-12T08:05:30Z",
      count: 100,
      spacingMs: 15_000,
      kind: "recent",
    },
    {
      startAt: "2026-05-12T07:31:20Z",
      count: 90,
      spacingMs: 15_000,
      kind: "recent",
    },
    {
      startAt: "2026-05-12T06:56:10Z",
      count: 70,
      spacingMs: 15_000,
      kind: "recent",
    },
    {
      startAt: "2026-05-12T06:21:00Z",
      count: 46,
      spacingMs: 15_000,
      kind: "recent",
    },
    {
      startAt: "2026-05-11T16:00:00Z",
      count: 1,
      spacingMs: 60_000,
      kind: "first",
    },
  ] as const;

  const fillerRecords: PromptCacheConversationInvocationPreview[] = [];
  let fillerId = 801;
  for (const [slotIndex, slot] of fillerSlots.entries()) {
    const slotStartMs = Date.parse(slot.startAt);
    for (let index = 0; index < slot.count; index += 1) {
      const recordIndex = fillerRecords.length;
      const id = fillerId;
      const occurredAt = new Date(
        slotStartMs - index * slot.spacingMs,
      ).toISOString();
      const cycle = recordIndex % 6;
      const upstreamAccountId = 320 + (recordIndex % 4);
      const baseTokens = 82_000 + (recordIndex % 11) * 3_700;
      const totalTokens = baseTokens + (cycle % 2 === 0 ? 37 : 348);
      const cost = Number((0.062 + (recordIndex % 7) * 0.0037).toFixed(4));
      const durationBase =
        slot.kind === "first" || slotIndex > 0 ? 46_000 : 17_000;

      if (slot.kind === "first") {
        fillerRecords.push(
          createPreview({
            id,
            invokeId: `invoke-history-${id}`,
            occurredAt,
            status: "completed",
            model: "gpt-5.5",
            upstreamAccountId,
            upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
            endpoint: "/v1/responses",
            inputTokens: baseTokens,
            cacheInputTokens: Math.max(0, baseTokens - 8_200),
            outputTokens: 37,
            reasoningTokens: 0,
            totalTokens,
            cost,
            reasoningEffort: "high",
            responseContentEncoding: "identity",
            tTotalMs: durationBase,
          }),
        );
        fillerId -= 1;
        continue;
      }

      if (cycle === 0) {
        fillerRecords.push(
          createPreview({
            id,
            invokeId: `invoke-history-${id}`,
            occurredAt,
            status: "completed",
            model: "gpt-5.5",
            upstreamAccountId,
            upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
            endpoint: "/v1/responses",
            inputTokens: baseTokens,
            cacheInputTokens: Math.max(0, baseTokens - 8_200),
            outputTokens: 37,
            reasoningTokens: 0,
            totalTokens,
            cost,
            reasoningEffort: "high",
            responseContentEncoding: "identity",
            tTotalMs: durationBase + (recordIndex % 5) * 800,
          }),
        );
        fillerId -= 1;
        continue;
      }

      if (cycle === 1 || cycle === 4) {
        fillerRecords.push(
          createPreview({
            id,
            invokeId: `invoke-history-${id}`,
            occurredAt,
            status: "http_502",
            failureClass: "service_failure",
            failureKind: "downstream_reset",
            errorMessage: "[downstream_reset] upstream stream reset mid-flight",
            model: "gpt-5.5",
            upstreamAccountId,
            upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
            endpoint: "/v1/responses",
            inputTokens: baseTokens,
            cacheInputTokens: Math.max(0, baseTokens - 8_000),
            outputTokens: 92 + (recordIndex % 6) * 11,
            reasoningTokens: 48 + (recordIndex % 5) * 9,
            totalTokens,
            cost,
            reasoningEffort: "high",
            responseContentEncoding: "identity",
            tTotalMs: durationBase + 2_400 + (recordIndex % 5) * 900,
          }),
        );
        fillerId -= 1;
        continue;
      }

      if (cycle === 2) {
        fillerRecords.push(
          createPreview({
            id,
            invokeId: `invoke-history-${id}`,
            occurredAt,
            status: "interrupted",
            failureClass: "client_abort",
            failureKind: "proxy_interrupted",
            errorMessage: "proxy request was interrupted before completion",
            model: "gpt-5.5",
            upstreamAccountId,
            upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
            endpoint: "/v1/responses",
            inputTokens: baseTokens,
            cacheInputTokens: Math.max(0, baseTokens - 8_100),
            outputTokens: 0,
            reasoningTokens: 0,
            totalTokens: baseTokens,
            cost: 0,
            reasoningEffort: "high",
            responseContentEncoding: "identity",
            tTotalMs: durationBase - 2_000 + (recordIndex % 4) * 600,
          }),
        );
        fillerId -= 1;
        continue;
      }

      fillerRecords.push(
        createPreview({
          id,
          invokeId: `invoke-history-${id}`,
          occurredAt,
          status: "completed",
          model: "gpt-5.5",
          upstreamAccountId,
          upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
          endpoint: "/v1/responses",
          inputTokens: baseTokens,
          cacheInputTokens: Math.max(0, baseTokens - 7_900),
          outputTokens: 37,
          reasoningTokens: 0,
          totalTokens: baseTokens + 37,
          cost,
          reasoningEffort: "high",
          responseContentEncoding: "identity",
          tTotalMs: durationBase + (recordIndex % 6) * 700,
        }),
      );
      fillerId -= 1;
    }
  }

  const historyInvocations = [...topRecords, ...fillerRecords].map(
    (preview) => ({
      ...preview,
      upstreamAccountId: 311,
      upstreamAccountName: "CIII",
      proxyDisplayName: null,
    }),
  );
  const totalTokens = historyInvocations.reduce(
    (sum, preview) => sum + Math.max(0, preview.totalTokens),
    0,
  );
  const totalCost = Number(
    historyInvocations
      .reduce((sum, preview) => sum + (preview.cost ?? 0), 0)
      .toFixed(4),
  );
  const dashboardPreviewInvocations = historyInvocations.slice(0, 2);
  return {
    dashboardResponse: createResponse([
      createConversation(promptCacheKey, dashboardPreviewInvocations, {
        requestCount: historyInvocations.length,
        totalTokens,
        totalCost,
        createdAt: "2026-05-11T16:00:12Z",
        lastActivityAt: "2026-05-12T08:15:57Z",
        upstreamAccounts: [
          {
            upstreamAccountId: 311,
            upstreamAccountName: "CIII",
            requestCount: 142,
            totalTokens: 1_154_982,
            totalCost: 9.4211,
            lastActivityAt: "2026-05-12T08:15:57Z",
          },
        ],
      }),
    ]),
    historyInvocationsByPromptCacheKey: new Map([
      [promptCacheKey, historyInvocations],
    ]),
  };
}

const interruptedRecoveryResponse = createResponse([
  createConversation("pck-interrupted-recovery", [
    createPreview({
      id: 49,
      invokeId: "invoke-49",
      occurredAt: "2026-04-04T10:03:52Z",
      status: "interrupted",
      failureClass: "service_failure",
      failureKind: "proxy_interrupted",
      errorMessage:
        "proxy request was interrupted before completion and was recovered on startup",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "gzip",
      tUpstreamStreamMs: null,
      tPersistMs: null,
      tTotalMs: null,
    }),
    createPreview({
      id: 48,
      invokeId: "invoke-48",
      occurredAt: "2026-04-04T10:01:20Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const assignedAccountFailureSemanticsResponse = createResponse([
  createConversation("pck-assigned-account-blocked", [
    createPreview({
      id: 53,
      invokeId: "invoke-assigned-account-blocked-current",
      occurredAt: "2026-04-04T10:04:36Z",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_assigned_account_blocked",
      errorMessage:
        '[pool_assigned_account_blocked] upstream account group "sticky-preflight-missing" has no bound forward proxy nodes',
      upstreamAccountId: 52,
      upstreamAccountName: "sticky-account-52@example.com",
      proxyDisplayName: "tokyo-edge-blocked",
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 42,
    }),
    createPreview({
      id: 52,
      invokeId: "invoke-assigned-account-blocked-previous",
      occurredAt: "2026-04-04T10:02:12Z",
      status: "completed",
      upstreamAccountId: 52,
      upstreamAccountName: "sticky-account-52@example.com",
      model: "gpt-5.4-mini",
      requestedServiceTier: "priority",
      serviceTier: "priority",
    }),
  ]),
  createConversation("pck-true-no-account", [
    createPreview({
      id: 63,
      invokeId: "invoke-true-no-account-current",
      occurredAt: "2026-04-04T10:03:44Z",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_no_available_account",
      errorMessage:
        "[pool_no_available_account] no assignable upstream account remains",
      upstreamAccountId: null,
      upstreamAccountName: null,
      proxyDisplayName: null,
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 38,
    }),
    createPreview({
      id: 62,
      invokeId: "invoke-true-no-account-previous",
      occurredAt: "2026-04-04T10:01:08Z",
      status: "completed",
      upstreamAccountId: null,
      upstreamAccountName: null,
      proxyDisplayName: null,
      model: "gpt-5.4-mini",
      requestedServiceTier: "priority",
      serviceTier: "priority",
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

const wideDesktopResponse = createResponse([
  createConversation(
    "pck-wide-running",
    [
      createPreview({
        id: 81,
        invokeId: "invoke-wide-running-current",
        occurredAt: "2026-04-04T10:04:58Z",
        status: "running",
        reasoningEffort: "medium",
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        endpoint: "/v1/responses/compact",
        tTotalMs: null,
      }),
      createPreview({
        id: 80,
        invokeId: "invoke-wide-running-previous",
        occurredAt: "2026-04-04T10:02:44Z",
        status: "completed",
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        endpoint: "/v1/responses/compact",
        model: "gpt-5.4-mini",
      }),
    ],
    {
      requestCount: 245,
      totalTokens: 34089123,
      totalCost: 32.1987,
    },
  ),
  createConversation("pck-wide-failed", [
    createPreview({
      id: 91,
      invokeId: "invoke-wide-failed-current",
      occurredAt: "2026-04-04T10:04:42Z",
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "upstream_timeout",
      errorMessage: "upstream gateway closed before first byte",
      upstreamAccountId: 77,
      upstreamAccountName: "wide-failed@example.com",
      endpoint: "/v1/chat/completions",
      requestedServiceTier: "auto",
      serviceTier: "auto",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 30018,
    }),
    createPreview({
      id: 90,
      invokeId: "invoke-wide-failed-previous",
      occurredAt: "2026-04-04T10:02:10Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "wide-failed@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-placeholder", [
    createPreview({
      id: 101,
      invokeId: "invoke-wide-placeholder-current",
      occurredAt: "2026-04-04T10:04:21Z",
      status: "completed",
      upstreamAccountName: "wide-placeholder@example.com",
    }),
  ]),
  createConversation("pck-wide-success-a", [
    createPreview({
      id: 111,
      invokeId: "invoke-wide-success-a-current",
      occurredAt: "2026-04-04T10:04:10Z",
      status: "completed",
      upstreamAccountName: "wide-success-a@example.com",
      totalTokens: 322,
      cost: 0.0218,
      inputTokens: 186,
      outputTokens: 136,
      cacheInputTokens: 54,
      reasoningTokens: 28,
      tTotalMs: 514,
    }),
    createPreview({
      id: 110,
      invokeId: "invoke-wide-success-a-previous",
      occurredAt: "2026-04-04T10:01:48Z",
      status: "completed",
      upstreamAccountName: "wide-success-a@example.com",
      model: "gpt-5.4-mini",
      totalTokens: 248,
      cost: 0.0164,
    }),
  ]),
  createConversation("pck-wide-pending", [
    createPreview({
      id: 121,
      invokeId: "invoke-wide-pending-current",
      occurredAt: "2026-04-04T10:03:58Z",
      status: "pending",
      upstreamAccountName: "wide-pending@example.com",
      tTotalMs: null,
    }),
    createPreview({
      id: 120,
      invokeId: "invoke-wide-pending-previous",
      occurredAt: "2026-04-04T10:00:58Z",
      status: "completed",
      upstreamAccountName: "wide-pending@example.com",
    }),
  ]),
  createConversation("pck-wide-success-b", [
    createPreview({
      id: 131,
      invokeId: "invoke-wide-success-b-current",
      occurredAt: "2026-04-04T10:03:20Z",
      status: "completed",
      upstreamAccountName: "wide-success-b@example.com",
      totalTokens: 418,
      cost: 0.0276,
      inputTokens: 238,
      outputTokens: 180,
      cacheInputTokens: 76,
      reasoningTokens: 34,
      tTotalMs: 692,
    }),
    createPreview({
      id: 130,
      invokeId: "invoke-wide-success-b-previous",
      occurredAt: "2026-04-04T10:00:20Z",
      status: "completed",
      upstreamAccountName: "wide-success-b@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-running-b", [
    createPreview({
      id: 141,
      invokeId: "invoke-wide-running-b-current",
      occurredAt: "2026-04-04T10:02:44Z",
      status: "running",
      upstreamAccountName: "wide-running-b@example.com",
      tTotalMs: null,
    }),
    createPreview({
      id: 140,
      invokeId: "invoke-wide-running-b-previous",
      occurredAt: "2026-04-04T09:59:12Z",
      status: "completed",
      upstreamAccountName: "wide-running-b@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-warning", [
    createPreview({
      id: 151,
      invokeId: "invoke-wide-warning-current",
      occurredAt: "2026-04-04T10:02:06Z",
      status: "http_429",
      failureClass: "service_failure",
      failureKind: "upstream_rate_limit",
      errorMessage: "upstream rate limit reached for the current account",
      upstreamAccountName: "wide-warning@example.com",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 1820,
    }),
    createPreview({
      id: 150,
      invokeId: "invoke-wide-warning-previous",
      occurredAt: "2026-04-04T09:58:52Z",
      status: "completed",
      upstreamAccountName: "wide-warning@example.com",
    }),
  ]),
]);

function buildVirtualizedLargeResponse(
  prefix: string,
  total: number,
): PromptCacheConversationsResponse {
  const conversations = Array.from({ length: total }, (_, index) => {
    const currentAt = new Date(
      Date.UTC(2026, 3, 4, 10, 59, 0) - index * 70_000,
    ).toISOString();
    const previousAt = new Date(Date.parse(currentAt) - 160_000).toISOString();
    const inFlight =
      index % 7 === 0 ? "running" : index % 5 === 0 ? "pending" : null;
    const currentStatus =
      inFlight ?? (index % 6 === 0 ? "http_429" : "completed");
    return createConversation(
      `${prefix}-${String(index + 1).padStart(3, "0")}`,
      [
        createPreview({
          id: 2_000 + index * 2,
          invokeId: `${prefix}-invoke-${index + 1}-current`,
          occurredAt: currentAt,
          status: currentStatus,
          upstreamAccountName: `${prefix}-account-${(index % 9) + 1}@example.com`,
          reasoningEffort: index % 2 === 0 ? "medium" : "high",
          totalTokens: 280 + index * 7,
          cost: Number((0.014 + index * 0.0006).toFixed(4)),
          tTotalMs: inFlight ? null : 420 + index * 9,
        }),
        createPreview({
          id: 2_001 + index * 2,
          invokeId: `${prefix}-invoke-${index + 1}-previous`,
          occurredAt: previousAt,
          status: "completed",
          model: "gpt-5.4-mini",
          upstreamAccountName: `${prefix}-account-${(index % 9) + 1}@example.com`,
          totalTokens: 180 + index * 5,
          cost: Number((0.009 + index * 0.0004).toFixed(4)),
          tTotalMs: 360 + index * 7,
        }),
      ],
      {
        createdAt: currentAt,
        lastActivityAt: currentAt,
        lastTerminalAt: inFlight ? previousAt : currentAt,
        lastInFlightAt: inFlight ? currentAt : null,
        cursor: `${prefix}-cursor-${index + 1}`,
        requestCount: 12 + index,
        totalTokens: 2_400 + index * 55,
        totalCost: Number((0.12 + index * 0.006).toFixed(4)),
      },
    );
  });

  return createResponse(conversations);
}

function buildCards(response: PromptCacheConversationsResponse) {
  return mapPromptCacheConversationsToDashboardCards(response);
}

const createdAtDescendingOrderCards = buildCards(
  createdAtDescendingOrderResponse,
);
const createdAtDescendingOrderKeys = [
  ...createdAtDescendingOrderResponse.conversations,
]
  .sort(
    (left, right) =>
      right.createdAt.localeCompare(left.createdAt) ||
      right.promptCacheKey.localeCompare(left.promptCacheKey),
  )
  .map((conversation) => conversation.promptCacheKey);

function getStorySequenceIdForPromptCacheKey(promptCacheKey: string) {
  return formatDashboardWorkingConversationSequenceId(
    `WC-${hashDashboardWorkingConversationKey(promptCacheKey).slice(0, 6)}`,
  );
}

const virtualizedLargeDatasetResponse = buildVirtualizedLargeResponse(
  "pck-virtual",
  72,
);
const virtualizedLargeDatasetCards = buildCards(
  virtualizedLargeDatasetResponse,
);
const headInsertBaseResponse = buildVirtualizedLargeResponse("pck-anchor", 56);

function HeadInsertAnchorStory() {
  const baseConversations = useMemo(
    () => headInsertBaseResponse.conversations,
    [],
  );
  const [cards, setCards] = useState(() =>
    buildCards(createResponse(baseConversations)),
  );
  const [status, setStatus] = useState("waiting");

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setCards(
        buildCards(
          createResponse([
            createConversation(
              "pck-anchor-new-head",
              [
                createPreview({
                  id: 9_991,
                  invokeId: "invoke-anchor-new-head-current",
                  occurredAt: "2026-04-04T11:00:12Z",
                  status: "running",
                  upstreamAccountName: "anchor-new-head@example.com",
                  reasoningEffort: "high",
                  tTotalMs: null,
                }),
                createPreview({
                  id: 9_990,
                  invokeId: "invoke-anchor-new-head-previous",
                  occurredAt: "2026-04-04T10:58:02Z",
                  status: "completed",
                  model: "gpt-5.4-mini",
                  upstreamAccountName: "anchor-new-head@example.com",
                  totalTokens: 222,
                  cost: 0.0142,
                }),
              ],
              {
                createdAt: "2026-04-04T11:00:12Z",
                lastActivityAt: "2026-04-04T11:00:12Z",
                lastTerminalAt: "2026-04-04T10:58:02Z",
                lastInFlightAt: "2026-04-04T11:00:12Z",
                cursor: "pck-anchor-new-head",
                requestCount: 27,
                totalTokens: 4_220,
                totalCost: 0.2042,
              },
            ),
            ...baseConversations,
          ]),
        ),
      );
      setStatus("prepended:pck-anchor-new-head");
    }, 450);

    return () => window.clearTimeout(timer);
  }, [baseConversations]);

  return (
    <div data-testid="story-head-insert-anchor" className="space-y-3">
      <div
        data-testid="story-head-insert-status"
        className="rounded-xl border border-base-300/75 bg-base-100/70 px-4 py-3 text-sm text-base-content/75"
      >
        Auto prepend status: <span className="font-mono">{status}</span>
      </div>
      <DashboardWorkingConversationsSection
        activeRange="today"
        cards={cards}
        totalMatched={cards.length}
        isLoading={false}
        error={null}
      />
    </div>
  );
}

function buildStoryMockData(
  response: PromptCacheConversationsResponse,
  historyInvocationsByPromptCacheKey = new Map<
    string,
    PromptCacheConversationInvocationPreview[]
  >(),
) {
  const recordsByInvokeId = new Map<string, ApiInvocation>();
  const recordsByPromptCacheKey = new Map<string, ApiInvocation[]>();
  const detailByRecordId = new Map<number, ApiInvocationRecordDetailResponse>();
  const responseBodyByRecordId = new Map<
    number,
    ApiInvocationResponseBodyResponse
  >();
  const poolAttemptsByInvokeId = new Map<
    string,
    ApiPoolUpstreamRequestAttempt[]
  >();

  const ingestPreview = (
    conversation: PromptCacheConversation,
    preview: PromptCacheConversationInvocationPreview,
  ) => {
    const record = {
      ...buildRecordFromPreview(preview),
      promptCacheKey: conversation.promptCacheKey,
    };
    recordsByInvokeId.set(record.invokeId, record);
    const promptCacheKey = record.promptCacheKey?.trim();
    if (promptCacheKey) {
      recordsByPromptCacheKey.set(promptCacheKey, [
        ...(recordsByPromptCacheKey.get(promptCacheKey) ?? []),
        record,
      ]);
    }

    const normalizedStatus = (record.status ?? "").trim().toLowerCase();
    const isAbnormal =
      record.failureClass === "service_failure" ||
      normalizedStatus === "failed" ||
      normalizedStatus.startsWith("http_");

    if (isAbnormal) {
      detailByRecordId.set(record.id, {
        id: record.id,
        abnormalResponseBody: {
          available: true,
          previewText: JSON.stringify({
            error: {
              message: record.errorMessage ?? "upstream failure",
            },
          }),
          hasMore: false,
        },
      });
      responseBodyByRecordId.set(record.id, {
        available: true,
        bodyText: JSON.stringify({
          error: {
            message: record.errorMessage ?? "upstream failure",
          },
          invokeId: record.invokeId,
        }),
      });
    }

    if (
      (record.routeMode ?? "").trim().toLowerCase() === "pool" &&
      typeof record.upstreamAccountId === "number"
    ) {
      poolAttemptsByInvokeId.set(record.invokeId, [
        {
          id: record.id * 10 + 1,
          invokeId: record.invokeId,
          occurredAt: record.occurredAt,
          endpoint: record.endpoint ?? "/v1/responses",
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          status: isAbnormal ? "failed" : "success",
          httpStatus: normalizedStatus.startsWith("http_")
            ? Number(normalizedStatus.slice("http_".length))
            : 200,
          createdAt: record.createdAt,
          upstreamAccountId: record.upstreamAccountId ?? null,
          upstreamAccountName: record.upstreamAccountName ?? null,
          firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
        },
      ]);
    }
  };

  for (const conversation of response.conversations) {
    const historyInvocations =
      historyInvocationsByPromptCacheKey.get(conversation.promptCacheKey) ??
      conversation.recentInvocations;
    for (const preview of historyInvocations) {
      ingestPreview(conversation, preview);
    }
  }

  return {
    recordsByInvokeId,
    recordsByPromptCacheKey,
    detailByRecordId,
    responseBodyByRecordId,
    poolAttemptsByInvokeId,
  };
}

function buildStoryInvocationSummary(records: ApiInvocation[]) {
  const resolvedFailureClass = (record: ApiInvocation) => {
    const failureClass = (record.failureClass ?? "").trim().toLowerCase();
    if (
      failureClass === "service_failure" ||
      failureClass === "client_failure" ||
      failureClass === "client_abort"
    ) {
      return failureClass;
    }
    return "none";
  };
  const isSuccessRecord = (record: ApiInvocation) => {
    const status = (record.status ?? "").trim().toLowerCase();
    const errorMessage = (record.errorMessage ?? "").trim();
    return (
      resolvedFailureClass(record) === "none" &&
      (status === "success" ||
        status === "completed" ||
        (status === "http_200" && errorMessage === ""))
    );
  };
  const failureRecords = records.filter(
    (record) => resolvedFailureClass(record) !== "none",
  );
  const successRecords = records.filter(isSuccessRecord);
  const totalMsRecords = records.filter(
    (record) =>
      typeof record.tTotalMs === "number" && Number.isFinite(record.tTotalMs),
  );
  const avgTotalMs =
    totalMsRecords.length === 0
      ? null
      : totalMsRecords.reduce(
          (sum, record) => sum + (record.tTotalMs ?? 0),
          0,
        ) / totalMsRecords.length;

  return {
    snapshotId: 1,
    newRecordsCount: 0,
    totalCount: records.length,
    successCount: successRecords.length,
    failureCount: failureRecords.length,
    totalCost: records.reduce((sum, record) => sum + (record.cost ?? 0), 0),
    totalTokens: records.reduce(
      (sum, record) => sum + (record.totalTokens ?? 0),
      0,
    ),
    token: {
      requestCount: records.length,
      totalTokens: records.reduce(
        (sum, record) => sum + (record.totalTokens ?? 0),
        0,
      ),
      avgTokensPerRequest:
        records.length === 0
          ? 0
          : records.reduce(
              (sum, record) => sum + (record.totalTokens ?? 0),
              0,
            ) / records.length,
      cacheInputTokens: records.reduce(
        (sum, record) => sum + (record.cacheInputTokens ?? 0),
        0,
      ),
      totalCost: records.reduce((sum, record) => sum + (record.cost ?? 0), 0),
    },
    network: {
      avgTtfbMs: null,
      p95TtfbMs: null,
      avgTotalMs,
      p95TotalMs: avgTotalMs,
    },
    exception: {
      failureCount: failureRecords.length,
      serviceFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "service_failure",
      ).length,
      clientFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "client_failure",
      ).length,
      clientAbortCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "client_abort",
      ).length,
      actionableFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "service_failure",
      ).length,
    },
  };
}

function resolveInitialSelection(
  cards: ReturnType<typeof buildCards>,
  target?: {
    promptCacheKey: string;
    slotKind: "current" | "previous";
  },
): DashboardWorkingConversationInvocationSelection | null {
  if (!target) return null;
  const card = cards.find(
    (candidate) => candidate.promptCacheKey === target.promptCacheKey,
  );
  if (!card) return null;
  const invocation =
    target.slotKind === "previous"
      ? card.previousInvocation
      : card.currentInvocation;
  if (!invocation) return null;
  return {
    slotKind: target.slotKind,
    conversationSequenceId: card.conversationSequenceId,
    promptCacheKey: card.promptCacheKey,
    invocation,
  };
}

function StoryAccountDrawer({
  account,
  onClose,
}: {
  account: {
    id: number;
    label: string;
    tab: "overview" | "routing" | "healthEvents";
  } | null;
  onClose: () => void;
}) {
  const titleId = useId();

  return (
    <AccountDetailDrawerShell
      open={account != null}
      labelledBy={titleId}
      closeLabel="Close account drawer"
      onClose={onClose}
      header={null}
    >
      {account ? (
        <div
          data-testid="story-account-drawer"
          className="space-y-4 rounded-[1.6rem] border border-base-300/80 bg-base-100/85 p-5"
        >
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/70">
              Shared Account Drawer
            </p>
            <h2
              id={titleId}
              className="text-xl font-semibold text-base-content"
            >
              {account.label}
            </h2>
            <p className="font-mono text-sm text-base-content/60">
              Account ID {account.id}
            </p>
            <p
              data-testid="story-account-drawer-tab"
              className="font-mono text-sm text-base-content/60"
            >
              Tab {account.tab}
            </p>
          </div>
          <p className="text-sm leading-6 text-base-content/70">
            Mock shared account detail drawer used to verify that Dashboard
            account clicks switch away from the invocation drawer without
            opening both drawers at once.
          </p>
        </div>
      ) : null}
    </AccountDetailDrawerShell>
  );
}

class StoryNoopEventSource implements EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readonly url: string;
  readonly withCredentials = false;
  readyState = StoryNoopEventSource.CONNECTING;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null =
    null;
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null;

  private listeners = new Map<
    string,
    Set<EventListenerOrEventListenerObject>
  >();

  constructor(url: string | URL) {
    this.url = typeof url === "string" ? url : url.toString();
    window.setTimeout(() => {
      if (this.readyState === StoryNoopEventSource.CLOSED) return;
      this.readyState = StoryNoopEventSource.OPEN;
      this.dispatchEvent(new Event("open"));
    }, 0);
  }

  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return;
    const bucket =
      this.listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>();
    bucket.add(listener);
    this.listeners.set(type, bucket);
  }

  removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return;
    this.listeners.get(type)?.delete(listener);
  }

  dispatchEvent(event: Event) {
    if (event.type === "open") {
      this.onopen?.call(this as unknown as EventSource, event);
    }
    if (event.type === "message") {
      this.onmessage?.call(
        this as unknown as EventSource,
        event as MessageEvent<string>,
      );
    }
    if (event.type === "error") {
      this.onerror?.call(this as unknown as EventSource, event);
    }
    for (const listener of this.listeners.get(event.type) ?? []) {
      if (typeof listener === "function") {
        listener(event);
      } else {
        listener.handleEvent(event);
      }
    }
    return true;
  }

  close() {
    this.readyState = StoryNoopEventSource.CLOSED;
  }
}

function DrawerPreviewStory({
  response,
  initialSelection,
  initialConversationKey,
  initialConversationTab = "overview",
  conversationPresentation = "overlay",
  historyInvocationsByPromptCacheKey,
  upstreamAccountActivity,
  recentPreviewLimit = 4,
  theme,
}: {
  response: PromptCacheConversationsResponse;
  initialSelection?: {
    promptCacheKey: string;
    slotKind: "current" | "previous";
  };
  initialConversationKey?: string;
  initialConversationTab?: "overview" | "calls" | "settings";
  conversationPresentation?: "overlay" | "page";
  historyInvocationsByPromptCacheKey?: Map<
    string,
    PromptCacheConversationInvocationPreview[]
  >;
  upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
  recentPreviewLimit?: number;
  theme?: "vibe-light" | "vibe-dark";
}) {
  useStoryTheme(theme);
  const { t } = useTranslation();
  const cards = useMemo(() => buildCards(response), [response]);
  const storyMocks = useMemo(
    () => buildStoryMockData(response, historyInvocationsByPromptCacheKey),
    [historyInvocationsByPromptCacheKey, response],
  );
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(() =>
      resolveInitialSelection(cards, initialSelection),
    );
  const [selectedConversation, setSelectedConversation] = useState<{
    conversationSequenceId: string;
    promptCacheKey: string;
  } | null>(() => {
    const initialCard = cards.find(
      (card) => card.promptCacheKey === initialConversationKey,
    );
    return initialCard
      ? {
          conversationSequenceId: initialCard.conversationSequenceId,
          promptCacheKey: initialCard.promptCacheKey,
        }
      : null;
  });
  const [selectedAccount, setSelectedAccount] = useState<{
    id: number;
    label: string;
    tab: "overview" | "routing" | "healthEvents";
  } | null>(null);
  const promptCacheBindingStateRef = useRef<Map<string, Record<string, unknown>>>(
    new Map(),
  );

  const buildPromptCacheBindingResponse = (
    promptCacheKey: string,
    overrides: Record<string, unknown> = {},
  ) => ({
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
  });

  useEffect(() => {
    setSelectedInvocation(resolveInitialSelection(cards, initialSelection));
    const initialCard = cards.find(
      (card) => card.promptCacheKey === initialConversationKey,
    );
    setSelectedConversation(
      initialCard
        ? {
            conversationSequenceId: initialCard.conversationSequenceId,
            promptCacheKey: initialCard.promptCacheKey,
          }
        : null,
    );
    setSelectedAccount(null);
  }, [cards, initialConversationKey, initialSelection]);

  const promptCacheConversationPage = selectedConversation ? (
    <div className="min-h-screen bg-base-200 p-3 text-base-content min-[769px]:p-6">
      <div className="mx-auto w-full max-w-[78rem]">
        <PromptCacheConversationHistoryDrawer
          open
          presentation="page"
          conversationKey={selectedConversation.promptCacheKey}
          conversationLabel={formatDashboardWorkingConversationSequenceId(
            selectedConversation.conversationSequenceId,
          )}
          initialTab={initialConversationTab}
          onClose={() => setSelectedConversation(null)}
          t={t}
          onOpenUpstreamAccount={(
            accountId: number,
            accountLabel: string,
            options?: { tab?: "overview" | "routing" | "healthEvents" },
          ) => {
            setSelectedInvocation(null);
            setSelectedConversation(null);
            setSelectedAccount({
              id: accountId,
              label: accountLabel,
              tab: options?.tab ?? "overview",
            });
          }}
        />
      </div>
    </div>
  ) : null;

  useLayoutEffect(() => {
    originalEventSourceRef.current = window.EventSource;
    window.EventSource =
      StoryNoopEventSource as unknown as typeof window.EventSource;
    return () => {
      if (originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current;
      }
      originalEventSourceRef.current = null;
    };
  }, []);

  useLayoutEffect(() => {
    if (!originalFetchRef.current) {
      originalFetchRef.current = window.fetch.bind(window);
    }
    (
      window as typeof window & { __dashboardStoryFetchLog?: string[] }
    ).__dashboardStoryFetchLog = [];
    (
      window as typeof window & { __dashboardStoryPolicyPatchLog?: string[] }
    ).__dashboardStoryPolicyPatchLog = [];

    window.fetch = async (input, init) => {
      const request =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      const url = new URL(request, window.location.origin);
      (
        window as typeof window & { __dashboardStoryFetchLog?: string[] }
      ).__dashboardStoryFetchLog?.push(
        `${url.pathname}?${url.searchParams.toString()}`,
      );

      const promptCacheBindingMatch = url.pathname.match(
        /^\/api\/stats\/prompt-cache-conversation-bindings\/(.+)$/,
      );
      if (promptCacheBindingMatch) {
        const promptCacheKey = decodeURIComponent(
          promptCacheBindingMatch[1] ?? "",
        );
        if (init?.method === "PATCH") {
          const payload = init?.body ? JSON.parse(String(init.body)) : {};
          const current =
            promptCacheBindingStateRef.current.get(promptCacheKey) ??
            buildPromptCacheBindingResponse(promptCacheKey);
          const next = buildPromptCacheBindingResponse(promptCacheKey, {
            ...current,
            ...("bindingKind" in payload
              ? { bindingKind: payload.bindingKind }
              : null),
            groupName:
              payload.bindingKind === "group"
                ? String(payload.groupName ?? "")
                : payload.bindingKind === "none"
                  ? null
                  : current.groupName,
            upstreamAccountId:
              payload.bindingKind === "upstreamAccount"
                ? Number(payload.upstreamAccountId)
                : payload.bindingKind === "none"
                  ? null
                  : current.upstreamAccountId,
            upstreamAccountName:
              payload.bindingKind === "upstreamAccount"
                ? (DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS.find(
                    (account) =>
                      account.id === Number(payload.upstreamAccountId),
                  )?.displayName ?? null)
                : payload.bindingKind === "none"
                  ? null
                  : current.upstreamAccountName,
            timeouts:
              payload.timeouts != null &&
              typeof payload.timeouts === "object"
                ? {
                    ...(typeof current.timeouts === "object" &&
                    current.timeouts != null
                      ? current.timeouts
                      : {}),
                    ...(payload.timeouts as Record<string, unknown>),
                  }
                : current.timeouts,
            updatedAt: "2026-05-12T16:20:00Z",
          });
          promptCacheBindingStateRef.current.set(promptCacheKey, next);
          return jsonResponse(next);
        }

        const response =
          promptCacheBindingStateRef.current.get(promptCacheKey) ??
          buildPromptCacheBindingResponse(promptCacheKey);
        promptCacheBindingStateRef.current.set(promptCacheKey, response);
        return jsonResponse(response);
      }

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

      const upstreamAccountPatchMatch = url.pathname.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)$/,
      );
      if (upstreamAccountPatchMatch && init?.method === "PATCH") {
        (
          window as typeof window & {
            __dashboardStoryPolicyPatchLog?: string[];
          }
        ).__dashboardStoryPolicyPatchLog = [
          ...((
            window as typeof window & {
              __dashboardStoryPolicyPatchLog?: string[];
            }
          ).__dashboardStoryPolicyPatchLog ?? []),
          typeof init.body === "string" ? init.body : "",
        ];
        return jsonResponse({
          id: Number(upstreamAccountPatchMatch[1]),
          displayName: "Pool Alpha",
          status: "active",
          routingRule: {},
        });
      }

      if (url.pathname === "/api/invocations") {
        const requestId = url.searchParams.get("requestId");
        if (requestId) {
          const record = storyMocks.recordsByInvokeId.get(requestId);
          return jsonResponse({
            snapshotId: 1,
            total: record ? 1 : 0,
            page: 1,
            pageSize: 1,
            records: record ? [record] : [],
          });
        }
        const promptCacheKey = url.searchParams.get("promptCacheKey")?.trim();
        if (promptCacheKey) {
          const page = Math.max(1, Number(url.searchParams.get("page") ?? "1"));
          const pageSize = Math.max(
            1,
            Number(url.searchParams.get("pageSize") ?? "200"),
          );
          const records = (
            storyMocks.recordsByPromptCacheKey.get(promptCacheKey) ?? []
          )
            .slice()
            .sort((left, right) =>
              right.occurredAt.localeCompare(left.occurredAt),
            );
          const start = (page - 1) * pageSize;
          return jsonResponse({
            snapshotId: 1,
            total: records.length,
            page,
            pageSize,
            records: records.slice(start, start + pageSize),
          });
        }
      }

      if (url.pathname === "/api/invocations/summary") {
        const promptCacheKey = url.searchParams.get("promptCacheKey");
        const records =
          promptCacheKey == null
            ? []
            : (storyMocks.recordsByPromptCacheKey.get(promptCacheKey) ?? []);
        return jsonResponse(buildStoryInvocationSummary(records));
      }

      if (url.pathname === "/api/stats/upstream-account-activity") {
        return jsonResponse(
          upstreamAccountActivity ?? {
            range: "today",
            rangeStart: "2026-04-04T10:00:00Z",
            rangeEnd: "2026-04-04T10:05:00Z",
            accounts: [],
          },
        );
      }

      if (url.pathname === "/api/stats/dashboard-activity") {
        const activity = upstreamAccountActivity ?? {
          range: "today",
          rangeStart: "2026-04-04T10:00:00Z",
          rangeEnd: "2026-04-04T10:05:00Z",
          accounts: [],
        };
        const includeAccounts =
          url.searchParams.get("includeAccounts") !== "false";
        return jsonResponse({
          range: activity.range,
          rangeStart: activity.rangeStart,
          rangeEnd: activity.rangeEnd,
          snapshotId: Date.parse(activity.rangeEnd) || 0,
          rateWindow: {
            start: activity.rangeStart,
            end: activity.rangeEnd,
            windowMinutes: 5,
            mode: "story",
          },
          summary: {
            stats: {
              totalCount: activity.accounts.reduce(
                (sum, account) => sum + account.requestCount,
                0,
              ),
              successCount: activity.accounts.reduce(
                (sum, account) => sum + account.successCount,
                0,
              ),
              failureCount: activity.accounts.reduce(
                (sum, account) => sum + account.failureCount,
                0,
              ),
              totalCost: activity.accounts.reduce(
                (sum, account) => sum + account.totalCost,
                0,
              ),
              totalTokens: activity.accounts.reduce(
                (sum, account) => sum + account.totalTokens,
                0,
              ),
              inProgressConversationCount: activity.accounts.reduce(
                (sum, account) =>
                  sum + (account.inProgressInvocationCount ?? 0),
                0,
              ),
            },
            tokensPerMinute: activity.accounts.reduce(
              (sum, account) => sum + (account.tokensPerMinute ?? 0),
              0,
            ),
            spendRate: activity.accounts.reduce(
              (sum, account) => sum + (account.spendRate ?? 0),
              0,
            ),
          },
          accounts: includeAccounts ? activity.accounts : undefined,
        });
      }

      const detailMatch = url.pathname.match(
        /^\/api\/invocations\/(\d+)\/detail$/,
      );
      if (detailMatch) {
        const recordId = Number(detailMatch[1]);
        return jsonResponse(
          storyMocks.detailByRecordId.get(recordId) ?? {
            id: recordId,
            abnormalResponseBody: null,
          },
        );
      }

      const responseBodyMatch = url.pathname.match(
        /^\/api\/invocations\/(\d+)\/response-body$/,
      );
      if (responseBodyMatch) {
        const recordId = Number(responseBodyMatch[1]);
        return jsonResponse(
          storyMocks.responseBodyByRecordId.get(recordId) ?? {
            available: false,
            unavailableReason: "No storybook response body for this record.",
          },
        );
      }

      const attemptsMatch = url.pathname.match(
        /^\/api\/invocations\/([^/]+)\/pool-attempts$/,
      );
      if (attemptsMatch) {
        const invokeId = decodeURIComponent(attemptsMatch[1] ?? "");
        return jsonResponse(
          storyMocks.poolAttemptsByInvokeId.get(invokeId) ?? [],
        );
      }

      if (originalFetchRef.current) {
        return originalFetchRef.current(
          input as Parameters<typeof fetch>[0],
          init,
        );
      }

      throw new Error(`Unhandled Storybook request: ${url.pathname}`);
    };

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
    };
  }, [storyMocks, upstreamAccountActivity]);

  return (
    <>
      {conversationPresentation === "page" && selectedConversation != null ? (
        promptCacheConversationPage
      ) : (
        <DashboardWorkingConversationsSection
          activeRange="today"
          recentPreviewLimit={recentPreviewLimit}
          cards={cards}
          isLoading={false}
          error={null}
          onOpenUpstreamAccount={(
            accountId: number,
            accountLabel: string,
            options?: { tab?: "overview" | "routing" | "healthEvents" },
          ) => {
            setSelectedInvocation(null);
            setSelectedConversation(null);
            setSelectedAccount({
              id: accountId,
              label: accountLabel,
              tab: options?.tab ?? "overview",
            });
          }}
          onOpenConversation={(selection) => {
            setSelectedInvocation(null);
            setSelectedAccount(null);
            setSelectedConversation(selection);
          }}
          onOpenInvocation={(selection) => {
            setSelectedConversation(null);
            setSelectedAccount(null);
            setSelectedInvocation(selection);
          }}
        />
      )}
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={(
          accountId: number,
          accountLabel: string,
          options?: { tab?: "overview" | "routing" | "healthEvents" },
        ) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          setSelectedAccount({
            id: accountId,
            label: accountLabel,
            tab: options?.tab ?? "overview",
          });
        }}
      />
      {conversationPresentation === "page" ? null : (
        <PromptCacheConversationHistoryDrawer
          open={selectedConversation != null}
          presentation={conversationPresentation}
          conversationKey={selectedConversation?.promptCacheKey ?? null}
          conversationLabel={
            selectedConversation
              ? formatDashboardWorkingConversationSequenceId(
                  selectedConversation.conversationSequenceId,
                )
              : null
          }
          initialTab={initialConversationTab}
          onClose={() => setSelectedConversation(null)}
          t={t}
          onOpenUpstreamAccount={(
            accountId: number,
            accountLabel: string,
            options?: { tab?: "overview" | "routing" | "healthEvents" },
          ) => {
            setSelectedInvocation(null);
            setSelectedConversation(null);
            setSelectedAccount({
              id: accountId,
              label: accountLabel,
              tab: options?.tab ?? "overview",
            });
          }}
        />
      )}
      <StoryAccountDrawer
        account={selectedAccount}
        onClose={() => setSelectedAccount(null)}
      />
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
    activeRange: "today",
    cards: buildCards(currentAndPreviousResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    const firstByteLatency = currentSlot.querySelector(
      '[data-testid="dashboard-compact-latency-first-byte"]',
    );
    const responseLatency = currentSlot.querySelector(
      '[data-testid="dashboard-compact-latency-response-time"]',
    );
    if (
      !(firstByteLatency instanceof HTMLElement) ||
      !(responseLatency instanceof HTMLElement)
    ) {
      throw new Error("missing compact latency readings");
    }
    const slotHeader = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    if (!(slotHeader instanceof HTMLElement)) {
      throw new Error("missing slot header");
    }
    await expect(
      slotHeader.querySelector(
        '[data-testid="dashboard-working-conversation-slot-label"]',
      ),
    ).toHaveTextContent(/当前调用|Current invocation/);
    await expect(slotHeader).toContainElement(firstByteLatency);
    await expect(slotHeader).toContainElement(responseLatency);
    await expect(firstByteLatency.className).not.toMatch(/rounded|border|bg-/);
    await expect(responseLatency.className).not.toMatch(/rounded|border|bg-/);
    const imageBadge = currentSlot.querySelector(
      '[data-testid="dashboard-image-tool-icon-badge"]',
    );
    if (!(imageBadge instanceof HTMLElement)) {
      throw new Error("missing image tool icon badge");
    }
    await expect(imageBadge).toHaveAttribute(
      "aria-label",
      expect.stringMatching(/图片工具|Image tool/),
    );
    await expect(imageBadge.className).toMatch(/rounded-full/);
    await expect(imageBadge.className).toMatch(/border/);
    await expect(currentSlot).not.toHaveTextContent(/RQ |UP |ED |TT /);
  },
};

export const CurrentOnlyPlaceholder: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(currentOnlyResponse),
    isLoading: false,
    error: null,
  },
};

export const RunningOnlyConversation: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: (args) => (
    <DashboardWorkingConversationsSection
      {...args}
      cards={buildCards(createRunningOnlyResponse())}
    />
  ),
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    expect(currentSlot).toBeInstanceOf(HTMLElement);
    const currentSlotHeader = currentSlot?.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    expect(currentSlotHeader).toBeInstanceOf(HTMLElement);
    expect(currentSlotHeader?.className).toContain("grid");
    expect(currentSlotHeader?.className).toContain(
      "grid-cols-[auto_minmax(0,1fr)]",
    );
    expect(
      currentSlotHeader?.querySelector(
        '[data-testid="invocation-phase-badge"]',
      ),
    ).toBeInstanceOf(HTMLElement);

    const phaseLabels = Array.from(
      canvasElement.querySelectorAll('[data-testid="invocation-phase-badge"]'),
    );
    expect(phaseLabels.length).toBeGreaterThanOrEqual(2);
    for (const phaseLabel of phaseLabels) {
      const slotHeader = phaseLabel.closest(
        '[data-testid="dashboard-working-conversation-slot-header"]',
      );
      expect(slotHeader).toBeInstanceOf(HTMLElement);
      expect(phaseLabel.className).toContain("inline-flex");
      expect(phaseLabel.className).toMatch(/\brounded-full\b/);
      expect(phaseLabel.getAttribute("data-phase-label-visible")).toBe("false");
      expect(phaseLabel.getAttribute("data-phase-motion")).toBe("dynamic");
      expect(phaseLabel.className).not.toMatch(/\bborder/);
    }
    const respondingBadge = currentSlotHeader?.querySelector(
      '[data-testid="invocation-phase-badge"][data-phase="responding"]',
    );
    expect(respondingBadge).toBeInstanceOf(HTMLElement);
    const respondingIcon = respondingBadge?.querySelector(
      '[data-testid="invocation-phase-icon"]',
    );
    expect(respondingIcon?.className).toContain("animate-spin");
  },
};

export const RequestingConversation: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: (args) => (
    <DashboardWorkingConversationsSection
      {...args}
      cards={buildCards(createRequestingOnlyResponse())}
    />
  ),
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }
    const requestingBadge = currentSlot.querySelector(
      '[data-testid="invocation-phase-badge"][data-phase="requesting"]',
    );
    if (!(requestingBadge instanceof HTMLElement)) {
      throw new Error("missing requesting phase badge");
    }
    await expect(requestingBadge).toHaveAttribute(
      "data-phase-label-visible",
      "false",
    );
    await expect(requestingBadge).toHaveAttribute(
      "data-phase-motion",
      "dynamic",
    );
    const requestingIcon = requestingBadge.querySelector(
      '[data-testid="invocation-phase-icon"]',
    );
    if (!(requestingIcon instanceof HTMLElement)) {
      throw new Error("missing requesting phase icon");
    }
    await expect(requestingIcon.className).toContain(
      "animate-invocation-phase-requesting",
    );
    await expect(currentSlot).not.toHaveTextContent(/请求中|Requesting/);
  },
};

export const PoolRoutingAccountStates: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={createPoolRoutingAccountStatesResponse()} />,
  parameters: {
    docs: {
      description: {
        story:
          "Dashboard working-conversation state gallery for pool routing account attribution: the running concrete upstream account breathes in primary text, the pending no-account slot keeps the neutral pool-routing fallback, and the terminal account stays static.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountButtons = await canvas.findAllByRole("button", {
      name: "pool-alpha@example.com",
    });
    const runningAccount = accountButtons[0]!;
    await expect(runningAccount.className).toContain(
      "invocation-account-routing-in-progress",
    );
    await expect(canvas.getByText(/号池路由中|pool routing/i)).toBeInTheDocument();

    const terminalAccount = accountButtons[accountButtons.length - 1];
    await expect(terminalAccount.className).not.toContain(
      "invocation-account-routing-in-progress",
    );

    await userEvent.click(runningAccount);
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain(
        "Mock shared account detail drawer used to verify",
      );
    });
  },
};

export const FailedStatusIconDedup: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(failedStatusDedupResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
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
    await expect(statusIcon).toHaveAttribute(
      "aria-label",
      expect.stringContaining("失败"),
    );
    await expect(statusIcon).toHaveAttribute(
      "aria-label",
      expect.stringContaining("upstream gateway closed before first byte"),
    );
    expect(
      slotHeader.querySelectorAll(
        '[title*="upstream gateway closed before first byte"]',
      ),
    ).toHaveLength(1);
    await expect(currentSlot).not.toHaveTextContent(/^失败$/);
  },
  parameters: {
    docs: {
      description: {
        story:
          "Failure slot compact status case that keeps exactly one owner-facing failed icon in the header while moving the collapsed error summary onto that single status affordance.",
      },
    },
  },
};

export const AccountPlanBadges: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(accountPlanBadgeResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable account-row polish case with long account names and compact plan badges. Enterprise is abbreviated to `Ent` while the full plan remains available in the badge title.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = Array.from(
      canvasElement.querySelectorAll(
        '[data-testid="dashboard-working-conversation-account-plan"]',
      ),
    );
    expect(badges.map((badge) => badge.textContent)).toEqual(
      expect.arrayContaining(["Ent", "Team", "Plus", "Free"]),
    );
    expect(
      badges
        .find((badge) => badge.textContent === "Ent")
        ?.getAttribute("title"),
    ).toBe("enterprise");
  },
};

export const TransportBadgeMixed: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(transportBadgeResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Mixed transport working-conversation cards. The current WebSocket invocation shows `WS` between the status badge and endpoint pill; the previous HTTP slot stays unbadged.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = canvasElement.querySelectorAll(
      '[data-testid="invocation-transport-badge"]',
    );
    expect(badges.length).toBeGreaterThanOrEqual(1);
    expect(
      Array.from(badges).every(
        (badge) =>
          badge.querySelector('[aria-hidden="true"]')?.textContent === "WS",
      ),
    ).toBe(true);
  },
};

export const ModelRoutingMismatch: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(
      createResponse([
        createConversation("pck-model-routing", [
          createPreview({
            id: 9701,
            invokeId: "inv_dashboard_model_routing",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "success",
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }),
        ]),
      ]),
    ),
    isLoading: false,
    error: null,
  },
};

export const InvocationDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={failedClickableResponse}
      initialSelection={{
        promptCacheKey: "pck-failed-clickable",
        slotKind: "current",
      }}
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Dashboard card section with the new invocation detail drawer opened by default, backed by stable request-id lookups and mock response-body detail data.",
      },
    },
  },
};

export const ModelRoutingDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-model-routing-drawer", [
          createPreview({
            id: 9801,
            invokeId: "inv_dashboard_model_routing_drawer",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "success",
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }),
        ]),
      ])}
      initialSelection={{
        promptCacheKey: "pck-model-routing-drawer",
        slotKind: "current",
      }}
    />
  ),
};

export const InterruptedRecoveryDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={interruptedRecoveryResponse}
      initialSelection={{
        promptCacheKey: "pck-interrupted-recovery",
        slotKind: "current",
      }}
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Recovered interrupted invocation that is immediately queryable from the dashboard drawer and keeps the dedicated interrupted status badge.",
      },
    },
  },
};

export const AssignedAccountFailureSemantics: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(assignedAccountFailureSemanticsResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Current dashboard working-conversation cards proving that assigned-account failures keep the concrete upstream account label, while true no-account failures alone fall back to the unassigned-account label.",
      },
    },
  },
};

export const FailedWithClickableAccount: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountButtons = await canvas.findAllByRole("button", {
      name: /pool-account-77@example.com/i,
    });
    const accountButton = accountButtons[0];

    await userEvent.click(accountButton);

    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain(
        "Mock shared account detail drawer used to verify",
      );
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "account:77:overview",
    );
  },
};

export const SequenceButtonOpensConversationHistory: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={
          fixtures.historyInvocationsByPromptCacheKey
        }
        theme="vibe-dark"
      />
    );
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const sequenceButton = await canvas.findByTestId(
      "dashboard-working-conversation-sequence-button",
    );

    sequenceButton.focus();
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')
          ?.textContent,
      ).toContain("conversation:pck-dashboard-history-realistic");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "conversation:pck-dashboard-history-realistic",
    );
    expect(document.body.textContent ?? "").toContain(
      sequenceButton.textContent ?? "",
    );
    expect(document.body.textContent ?? "").toContain(
      "pck-dashboard-history-realistic",
    );
    await expect(
      within(document.body).getByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(document.body.textContent ?? "").toMatch(
        /共 316 条保留调用记录|316 retained calls/i,
      );
    });
    const dialog = within(document.body).getByRole("dialog");
    expect(within(dialog).queryByRole("button", { name: "今日" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "昨日" })).toBeNull();
    expect(
      within(dialog).queryByRole("button", { name: "24 小时" }),
    ).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "7 日" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "历史" })).toBeNull();
    await waitFor(() => {
      const fetchLog =
        (window as typeof window & { __dashboardStoryFetchLog?: string[] })
          .__dashboardStoryFetchLog ?? [];
      expect(
        fetchLog.some(
          (entry) =>
            entry.startsWith("/api/invocations?") &&
            entry.includes("promptCacheKey=pck-dashboard-history-realistic") &&
            entry.includes("page=2") &&
            entry.includes("snapshotId=1"),
        ),
      ).toBe(true);
    });
  },
  parameters: {
    docs: {
      description: {
        story:
          "Only the compact conversation sequence id is a hot zone for opening the full retained conversation history drawer; invocation slots still open single-call diagnostics.",
      },
    },
  },
};

export const ConversationHistoryDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={
          fixtures.historyInvocationsByPromptCacheKey
        }
        initialConversationKey="pck-dashboard-history-realistic"
        theme="vibe-dark"
      />
    );
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable opened state for the full retained conversation history drawer, including the production-style activity chart and dark floating tooltip surface.",
      },
    },
  },
};

export const ConversationHistoryPageMobile: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={
          fixtures.historyInvocationsByPromptCacheKey
        }
        initialConversationKey="pck-dashboard-history-realistic"
        initialConversationTab="settings"
        conversationPresentation="page"
        theme="vibe-dark"
      />
    );
  },
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "Compact page presentation for the retained conversation history workspace, using the same URL-backed content hierarchy as the desktop drawer.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    await expect(
      canvas.getByRole("tab", { name: /设置|settings/i }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(within(document.body).queryByRole("dialog")).toBeNull();
  },
};

export const UpstreamAccountTab: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account", [
          createPreview({
            id: 9801,
            invokeId: "story-working-invoke",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();
    await expect(canvas.getByText("最近 4 条调用")).toBeInTheDocument();
    await expect(
      canvas.getByTestId("dashboard-upstream-account-header-row"),
    ).not.toHaveTextContent("#42");
    await expect(
      canvasElement.querySelector(
        '[data-testid="dashboard-upstream-account-status"]',
      ),
    ).toBeNull();
    await expect(canvas.getByText("上游拒绝")).toBeInTheDocument();
    await expect(canvas.getByText("限流")).toBeInTheDocument();
    await expect(canvas.getByText("禁新")).toBeInTheDocument();
    await expect(canvas.getByText("强制Fast")).toBeInTheDocument();
    await expect(canvas.getByText("禁入")).toBeInTheDocument();
    await expect(canvas.getByText("进行中")).toBeInTheDocument();
    const recentBreakdown = canvas.getByTestId(
      "dashboard-upstream-account-recent-breakdown",
    );
    await expect(recentBreakdown).toHaveTextContent(/排队中\s*2/);
    await expect(recentBreakdown).toHaveTextContent(/请求中\s*3/);
    await expect(recentBreakdown).toHaveTextContent(/响应中\s*4/);
    await expect(recentBreakdown).toHaveTextContent(/成功\s*6/);
    const phaseSegments = Array.from(
      recentBreakdown.querySelectorAll(
        '[data-testid="invocation-phase-segment"]',
      ),
    );
    expect(phaseSegments).toHaveLength(3);
    for (const phaseSegment of phaseSegments) {
      expect(phaseSegment.getAttribute("data-phase-motion")).toBe("static");
      const icon = phaseSegment.querySelector(
        '[data-testid="invocation-phase-icon"]',
      );
      expect(icon).toBeInstanceOf(HTMLElement);
      expect(icon?.className).not.toContain("animate-invocation-phase-requesting");
      expect(icon?.className).not.toContain("animate-pulse");
      expect(icon?.className).not.toContain("animate-spin");
    }
    await expect(
      canvas.getByTestId("dashboard-upstream-account-policy-badges"),
    ).toHaveTextContent("禁出");
    await expect(canvas.getByText("story-account-1")).toBeInTheDocument();
    await expect(canvas.getByText("gpt-5.5-mini")).toBeInTheDocument();
    await expect(canvas.getByText("gpt-5.5")).toBeInTheDocument();
    const firstRecentRow = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-row",
    )[0];
    if (!(firstRecentRow instanceof HTMLElement)) {
      throw new Error("missing first upstream recent row");
    }
    const firstByteLatency = firstRecentRow.querySelector(
      '[data-testid="dashboard-compact-latency-first-byte"]',
    );
    const responseLatency = firstRecentRow.querySelector(
      '[data-testid="dashboard-compact-latency-response-time"]',
    );
    if (
      !(firstByteLatency instanceof HTMLElement) ||
      !(responseLatency instanceof HTMLElement)
    ) {
      throw new Error("missing upstream compact latency readings");
    }
    await expect(firstByteLatency.className).not.toMatch(/rounded|border|bg-/);
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
    await expect(
      canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
    ).toHaveLength(4);
    const identityChips = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-identity-chip",
    );
    await expect(
      new Set(identityChips.map((chip) => chip.className)).size,
    ).toBe(4);
    await expect(canvas.queryByText("按调用计数，不按对话去重")).toBeNull();
    await expect(canvas.queryByText("仍在重试链路中的调用")).toBeNull();
    await expect(
      canvas.queryByText(
        "最近 4 条调用里仍有活动或异常，优先从下方最近记录继续排查。",
      ),
    ).toBeNull();
    const identityChip = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-identity-chip",
    )[0];
    if (!(identityChip instanceof HTMLButtonElement)) {
      throw new Error("expected upstream identity chip button");
    }
    await expect(identityChip).toHaveAttribute(
      "aria-label",
      expect.stringContaining("打开对话详情"),
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard workspace section switched to the upstream-account tab, showing one enlarged active-account card with account-level KPIs and the dynamic recent invocation window in the selected range, including lightweight short conversation identity chips and request/response model mismatch rows.",
      },
    },
  },
};

export const UpstreamAccountPhaseBreakdownStatic: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-static", [
            createPreview({
              id: 9841,
              invokeId: "story-working-static",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();
    const recentBreakdown = await canvas.findByTestId(
      "dashboard-upstream-account-recent-breakdown",
    );
    const phaseSegments = Array.from(
      recentBreakdown.querySelectorAll(
        '[data-testid="invocation-phase-segment"]',
      ),
    );
    expect(phaseSegments).toHaveLength(3);
    for (const phaseSegment of phaseSegments) {
      expect(phaseSegment.getAttribute("data-phase-motion")).toBe("static");
      const icon = phaseSegment.querySelector(
        '[data-testid="invocation-phase-icon"]',
      );
      expect(icon).toBeInstanceOf(HTMLElement);
      expect(icon?.className).not.toContain("animate-invocation-phase-requesting");
      expect(icon?.className).not.toContain("animate-pulse");
      expect(icon?.className).not.toContain("animate-spin");
    }
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Owner-facing static phase breakdown entry that opens directly on the upstream-account workspace view, so the queued/requesting/responding summary can be reviewed without relying on an interaction step first.",
      },
    },
  },
};

export const UpstreamAccountHeaderActions: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-routing-badges", [
          createPreview({
            id: 9861,
            invokeId: "story-working-routing-badges",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const attentionBadges = await canvas.findByTestId(
      "dashboard-upstream-account-attention-badges",
    );
    await userEvent.click(attentionBadges);
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "account:42:healthEvents",
    );
    await expect(
      within(document.body).getByTestId("story-account-drawer-tab"),
    ).toHaveTextContent("Tab healthEvents");

    const settingsButton = await canvas.findByTestId(
      "dashboard-upstream-account-routing-settings",
    );
    await userEvent.click(settingsButton);
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "account:42:routing",
    );
    await userEvent.click(
      within(document.body).getByRole("button", {
        name: "Close account drawer",
      }),
    );
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "none",
    );

    const policyBadges = await canvas.findAllByTestId(
      "dashboard-upstream-account-policy-badge",
    );
    await userEvent.click(policyBadges[0]!);
    await expect(policyBadges[1]!).toHaveTextContent("强制Fast");
    await expect(policyBadges[1]!).toHaveAttribute(
      "aria-label",
      expect.stringContaining("Fast 改写策略：强制Fast"),
    );
    await userEvent.click(policyBadges[1]!);
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "none",
    );
    await waitFor(
      () => {
        const patchLog = (
          window as typeof window & {
            __dashboardStoryPolicyPatchLog?: string[];
          }
        ).__dashboardStoryPolicyPatchLog;
        expect(patchLog?.[0]).toContain('"priorityTier":"normal"');
        expect(patchLog?.[0]).toContain('"fastModeRewriteMode":"force_remove"');
      },
      { timeout: 1600 },
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard upstream-account card header actions: attention badges open health events, the gear opens routing, and quick policy chips including Fast rewrite labels save account-level overrides with a debounced PATCH.",
      },
    },
  },
};

async function assertQuickPolicyTonePalette(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
  await userEvent.click(accountTab);

  const policyBadges = await canvas.findAllByTestId(
    "dashboard-upstream-account-policy-badge",
  );
  await expect(policyBadges.map((badge) => badge.textContent?.trim())).toEqual(
    ["兜底", "Fast", "禁出", "禁入"],
  );
  await expect(
    policyBadges.map((badge) => badge.getAttribute("data-policy-tone")),
  ).toEqual(["success", "primary", "warning", "neutral"]);
}

export const UpstreamAccountQuickPolicyTonePalette: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-policy-tones", [
          createPreview({
            id: 9871,
            invokeId: "story-working-policy-tones",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(4, {
        allowCutOut: false,
        allowCutIn: true,
        priorityTier: "fallback",
        fastModeRewriteMode: "force_add",
      })}
    />
  ),
  play: async ({ canvasElement }) =>
    assertQuickPolicyTonePalette(canvasElement),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard upstream-account quick policy chips shown as a semantic tone palette: fallback uses success, force Fast uses primary, active cut-out block uses warning, and inactive cut-in remains neutral.",
      },
    },
  },
};

export const UpstreamAccountQuickPolicyTonePaletteDark: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-policy-tones-dark", [
          createPreview({
            id: 9872,
            invokeId: "story-working-policy-tones-dark",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(4, {
        allowCutOut: false,
        allowCutIn: true,
        priorityTier: "fallback",
        fastModeRewriteMode: "force_add",
      })}
      theme="vibe-dark"
    />
  ),
  play: async ({ canvasElement }) =>
    assertQuickPolicyTonePalette(canvasElement),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dark theme checkpoint for the dashboard upstream-account quick policy tone palette.",
      },
    },
  },
};

export const UpstreamAccountMetricTooltips: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account-tooltips", [
          createPreview({
            id: 9831,
            invokeId: "story-working-tooltips",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const triggers = await canvas.findAllByTestId(
      "dashboard-upstream-account-metric-card",
    );
    await expect(triggers).toHaveLength(4);

    const tpmInlineMetric = canvas.getByLabelText("TPM 640");
    await userEvent.click(tpmInlineMetric);
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("TPM 640");
    });
    await userEvent.click(tpmInlineMetric);

    const assertMetricTooltip = async (
      metric: string,
      expectedTexts: string[],
    ) => {
      const trigger = canvasElement.querySelector(
        `[data-testid="dashboard-upstream-account-metric-card"][data-metric="${metric}"]`,
      );
      if (!(trigger instanceof HTMLElement)) {
        throw new Error(`missing ${metric} metric trigger`);
      }
      await userEvent.click(trigger);
      await waitFor(() => {
        const tooltipText = document.body.textContent ?? "";
        for (const text of expectedTexts) {
          expect(tooltipText).toContain(text);
        }
      });
      await userEvent.click(trigger);
      await userEvent.unhover(trigger);
    };

    await assertMetricTooltip("latency", [
      "首字用时",
      "2.87 s",
      "响应时间",
      "阶段首字节",
    ]);
    await assertMetricTooltip("requests", [
      "请求数",
      "成功率",
      "75%",
      "非成功率",
    ]);
    await assertMetricTooltip("cost", [
      "成本",
      "0.72",
      "缓存写入",
      "缓存读取",
      "推理",
      "未知",
      "gpt-5.6",
    ]);
    await assertMetricTooltip("token", [
      "Token",
      "缓存写入",
      "缓存读取 Token",
      "输出",
      "gpt-5.6",
    ]);

    const finalTrigger = canvasElement.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="cost"]',
    );
    if (finalTrigger instanceof HTMLElement)
      await userEvent.click(finalTrigger);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Stable interaction coverage for the four upstream-account metric cards. Each whole metric card opens a structured tooltip with explicit field labels, values, and related computed data while the card surface stays compact.",
      },
    },
  },
};

export const ErrorSummaryTooltips: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(
      createResponse([
        createConversation("pck-story-error-summary-tooltips", [
          createPreview({
            id: 9941,
            invokeId: "story-error-summary-current",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "http_502",
            failureClass: "service_failure",
            failureKind: "upstream_http_5xx",
            errorMessage: LONG_ERROR_SUMMARY,
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
            tUpstreamTtfbMs: null,
            tUpstreamStreamMs: null,
            tTotalMs: 18_420,
          }),
          createPreview({
            id: 9940,
            invokeId: "story-error-summary-previous",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "success",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ]),
    ),
    isLoading: false,
    error: null,
  },
  render: () => {
    const upstreamAccountActivity = createUpstreamAccountActivityStoryResponse();
    upstreamAccountActivity.accounts[0] = {
      ...upstreamAccountActivity.accounts[0],
      recentInvocations: upstreamAccountActivity.accounts[0].recentInvocations.map(
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
                tTotalMs: 21_006,
              }
            : invocation,
      ),
    };

    return (
      <ForcedWorkspaceViewStory view="conversations">
        <DrawerPreviewStory
          response={createResponse([
            createConversation("pck-story-error-summary-tooltips", [
              createPreview({
                id: 9941,
                invokeId: "story-error-summary-current",
                occurredAt: "2026-04-04T10:05:00Z",
                status: "http_502",
                failureClass: "service_failure",
                failureKind: "upstream_http_5xx",
                errorMessage: LONG_ERROR_SUMMARY,
                upstreamAccountId: 42,
                upstreamAccountName: "Pool Alpha",
                tUpstreamTtfbMs: null,
                tUpstreamStreamMs: null,
                tTotalMs: 18_420,
              }),
              createPreview({
                id: 9940,
                invokeId: "story-error-summary-previous",
                occurredAt: "2026-04-04T10:03:00Z",
                status: "success",
                upstreamAccountId: 42,
                upstreamAccountName: "Pool Alpha",
              }),
            ]),
          ])}
          upstreamAccountActivity={upstreamAccountActivity}
        />
      </ForcedWorkspaceViewStory>
    );
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing failed current slot");
    }

    const slotErrorSummary = currentSlot.querySelector(
      '[data-testid="invocation-error-summary"]',
    );
    const slotErrorTrigger = slotErrorSummary?.parentElement;
    if (!(slotErrorSummary instanceof HTMLElement) || !(slotErrorTrigger instanceof HTMLElement)) {
      throw new Error("missing current slot error summary trigger");
    }

    await userEvent.hover(slotErrorTrigger);
    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll("[data-side]")).find(
        (node) => node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip?.textContent).toContain(LONG_ERROR_SUMMARY);
      expect(tooltip?.getAttribute("data-side")).toBe("bottom");
    });
    await userEvent.unhover(slotErrorTrigger);

    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const recentRow = await canvas.findByTestId(
      "dashboard-upstream-account-recent-row",
    );
    const recentErrorSummary = recentRow.querySelector(
      '[data-testid="invocation-error-summary"]',
    );
    const recentErrorTrigger = recentErrorSummary?.parentElement;
    if (!(recentErrorSummary instanceof HTMLElement) || !(recentErrorTrigger instanceof HTMLElement)) {
      throw new Error("missing recent row error summary trigger");
    }

    const accountGrid = canvasElement.querySelector(
      '[data-testid="dashboard-upstream-account-grid"]',
    );
    const accountCard = recentRow.closest(
      '[data-testid="dashboard-upstream-account-card"]',
    );
    if (!(accountGrid instanceof HTMLElement) || !(accountCard instanceof HTMLElement)) {
      throw new Error("missing upstream account layout shrink chain");
    }

    expect(accountGrid.className).toContain(
      "desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]",
    );
    expect(accountCard.className).toContain("min-w-0");
    expect(recentRow.className).toContain("min-w-0");
    expect(recentErrorTrigger.className).toContain("w-full");
    expect(recentErrorTrigger.className).toContain("overflow-hidden");

    await userEvent.hover(recentErrorTrigger);
    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll("[data-side]")).find(
        (node) => node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip?.textContent).toContain(LONG_ERROR_SUMMARY);
      expect(tooltip?.getAttribute("data-side")).toBe("bottom");
    });
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Long failed invocation summaries stay single-line and truncated inside both the current slot and upstream-account recent rows, while hover opens the shared tooltip below the trigger with the full upstream error payload.",
      },
    },
  },
};

export const UpstreamAccountRecentIdentityChipOpensConversation: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account", [
          createPreview({
            id: 9801,
            invokeId: "story-working-invoke",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const identityChip = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-identity-chip",
    )[0];
    if (!(identityChip instanceof HTMLButtonElement)) {
      throw new Error("expected upstream identity chip button");
    }

    await userEvent.click(identityChip);
    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')
          ?.textContent,
      ).toContain("conversation:pck-upstream-running");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "conversation:pck-upstream-running",
    );

    const firstRow = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-row",
    )[0];
    if (!(firstRow instanceof HTMLButtonElement)) {
      throw new Error("expected upstream recent row button");
    }

    await userEvent.click(firstRow);
    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')
          ?.textContent,
      ).toContain("invocation:acct-invoke-1");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "invocation:acct-invoke-1",
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Proves the upstream-account recent identity chip opens the conversation drawer while the surrounding recent row still opens the invocation drawer.",
      },
    },
  },
};

export const UpstreamAccountTabDynamicSeven: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account-seven", [
          createPreview({
            id: 9811,
            invokeId: "story-working-seven",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(7)}
      recentPreviewLimit={7}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(canvas.getByText("最近 7 条调用")).toBeInTheDocument();
    await expect(canvas.getByText("story-account-7")).toBeInTheDocument();
    await expect(
      canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
    ).toHaveLength(7);
    const identityChips = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-identity-chip",
    );
    await expect(
      new Set(identityChips.map((chip) => chip.className)).size,
    ).toBeGreaterThan(3);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Medium dynamic recent invocation window showing seven account rows and stable short conversation identity chips with discrete helper tones.",
      },
    },
  },
};

export const UpstreamAccountTabMaxSixteen: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account-sixteen", [
          createPreview({
            id: 9821,
            invokeId: "story-working-sixteen",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(16)}
      recentPreviewLimit={16}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(canvas.getByText("最近 16 条调用")).toBeInTheDocument();
    await expect(canvas.getByText("story-account-16")).toBeInTheDocument();
    await expect(
      canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
    ).toHaveLength(16);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Upper clamp state for the upstream-account recent invocation list, keeping the dense account card scannable at sixteen rows.",
      },
    },
  },
};

export const DrawerInteractionFlow: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    await userEvent.click(currentSlot);

    await waitFor(() => {
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-invocation-detail-drawer"]',
        ),
      ).not.toBeNull();
    });

    const drawerAccountButton = document.body.querySelector(
      '[data-testid="dashboard-invocation-detail-drawer"] button[title="pool-account-77@example.com"]',
    );
    if (!(drawerAccountButton instanceof HTMLButtonElement)) {
      throw new Error("missing drawer account button");
    }

    await userEvent.click(drawerAccountButton);

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-account-drawer"]'),
      ).not.toBeNull();
    });
  },
};

export const StateGallery: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    isLoading: false,
    error: null,
  },
};

export const LoadingState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: true,
    error: null,
  },
};

export const EmptyState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: false,
    error: null,
  },
};

export const ErrorState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: false,
    error: "Request failed: 503 working conversations snapshot unavailable",
  },
};

export const Mobile390: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    totalMatched: wideDesktopResponse.conversations.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "Mobile viewport keeps the working-conversations section in a single column while preserving the compact header and dual-slot summary hierarchy.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const controls = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversations-controls"]',
    );
    if (!(controls instanceof HTMLElement)) {
      throw new Error("missing workspace controls");
    }
    await expect(controls.className).toContain("flex-col");
    await expect(controls.querySelector('[role="tablist"]')?.className).toContain("w-full");
  },
};

export const WideDesktop1660: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    totalMatched: wideDesktopResponse.conversations.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Wide desktop state gallery proving the 1660px shell now renders the working conversations section in four columns without horizontal overflow.",
      },
    },
  },
};

export const VirtualizedLargeDataset: Story = {
  args: {
    activeRange: "today",
    cards: virtualizedLargeDatasetCards,
    totalMatched: virtualizedLargeDatasetCards.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Large loaded working set proving the section keeps the DOM virtualized instead of mounting every card at once.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const container = await canvas.findByTestId(
      "dashboard-working-conversations-grid",
    );
    const storyWindow = canvasElement.ownerDocument.defaultView;
    if (!storyWindow) {
      throw new Error("missing story window");
    }

    const scrollTarget =
      container.getBoundingClientRect().top + storyWindow.scrollY + 1_600;
    storyWindow.scrollTo({ top: scrollTarget });

    await waitFor(() => {
      const renderedCards = container.querySelectorAll(
        '[data-testid="dashboard-working-conversation-card"]',
      ).length;
      expect(renderedCards).toBeGreaterThan(0);
      expect(renderedCards).toBeLessThan(virtualizedLargeDatasetCards.length);
    });

    expect(container.className).not.toContain("overflow-auto");
    expect(container.className).not.toContain("max-h-[72vh]");
  },
};

export const HeadInsertAnchorCompensation: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <HeadInsertAnchorStory />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Auto-prepends a fresh head card after the list has been scrolled, and the existing viewport anchor should stay visually pinned instead of jumping downward.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const container = await canvas.findByTestId(
      "dashboard-working-conversations-grid",
    );
    const storyWindow = canvasElement.ownerDocument.defaultView;
    if (!storyWindow) {
      throw new Error("missing story window");
    }

    const scrollTarget =
      container.getBoundingClientRect().top + storyWindow.scrollY + 1_600;
    storyWindow.scrollTo({ top: scrollTarget });
    storyWindow.dispatchEvent(new Event("scroll"));

    let anchorCard: HTMLElement | undefined;
    await waitFor(() => {
      anchorCard = Array.from(
        container.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      ).find((candidate) => candidate.getBoundingClientRect().height > 0);
      expect(anchorCard).toBeDefined();
    });

    const anchorSequenceId = anchorCard?.dataset.conversationSequenceId ?? "";
    const containerTopBoundary = Math.max(
      0,
      container.getBoundingClientRect().top,
    );
    const anchorTop =
      (anchorCard?.getBoundingClientRect().top ?? 0) - containerTopBoundary;

    await waitFor(() => {
      expect(canvas.getByTestId("story-head-insert-status")).toHaveTextContent(
        "prepended:pck-anchor-new-head",
      );
    });

    await waitFor(() => {
      const nextAnchor = Array.from(
        container.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      ).find(
        (candidate) =>
          candidate.dataset.conversationSequenceId === anchorSequenceId,
      );
      expect(nextAnchor).toBeDefined();
      const nextTop =
        (nextAnchor?.getBoundingClientRect().top ?? 0) - containerTopBoundary;
      expect(Math.abs(nextTop - anchorTop)).toBeLessThanOrEqual(12);
    });
  },
};

export const CreatedAtDescendingOrder: Story = {
  args: {
    activeRange: "today",
    cards: createdAtDescendingOrderCards,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const cards = await canvas.findAllByTestId(
      "dashboard-working-conversation-card",
    );
    expect(
      cards.map((card) => card.getAttribute("data-conversation-sequence-id")),
    ).toEqual(
      createdAtDescendingOrderKeys.map(getStorySequenceIdForPromptCacheKey),
    );
  },
};
