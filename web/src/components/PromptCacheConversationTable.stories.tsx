import { useEffect, useRef, type ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { MemoryRouter } from "react-router-dom";
import { I18nProvider } from "../i18n";
import type {
  ApiInvocation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

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

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "Content-Type": "application/json",
    },
  });
}

function buildAccountDetail(
  id: number,
  displayName: string,
  overrides?: Partial<UpstreamAccountDetail>,
): UpstreamAccountDetail {
  const normalizedEmail = displayName.includes("@")
    ? displayName
    : `${displayName.toLowerCase().replace(/\s+/g, "-")}@example.com`;
  return {
    id,
    kind: "oauth_codex",
    provider: "openai",
    displayName,
    groupName: "storybook-group",
    isMother: false,
    status: "active",
    enabled: true,
    email: normalizedEmail,
    chatgptAccountId: `org_${id}`,
    chatgptUserId: `user_${id}`,
    planType: "team",
    maskedApiKey: null,
    lastSyncedAt: "2026-03-03T12:40:00.000Z",
    lastSuccessfulSyncAt: "2026-03-03T12:38:00.000Z",
    lastActivityAt: "2026-03-03T12:44:10.000Z",
    lastError: null,
    lastErrorAt: null,
    tokenExpiresAt: "2026-03-03T18:00:00.000Z",
    lastRefreshedAt: "2026-03-03T12:39:00.000Z",
    primaryWindow: {
      usedPercent: 22,
      usedText: "22 / 100",
      limitText: "100 requests",
      resetsAt: "2026-03-03T18:00:00.000Z",
      windowDurationMins: 300,
    },
    secondaryWindow: {
      usedPercent: 38,
      usedText: "38 / 100",
      limitText: "100 requests",
      resetsAt: "2026-03-10T00:00:00.000Z",
      windowDurationMins: 10080,
    },
    credits: null,
    localLimits: null,
    duplicateInfo: null,
    tags: [],
    effectiveRoutingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    },
    note: null,
    upstreamBaseUrl: null,
    history: [],
    ...overrides,
  };
}

const accountDetails = new Map<number, UpstreamAccountDetail>([
  [
    11,
    buildAccountDetail(11, "growth.6vv4@relay.example", {
      isMother: true,
      note: "Primary prompt-cache routing account",
    }),
  ],
  [
    12,
    buildAccountDetail(12, "backup.f3x2@ops.example", {
      note: "Fallback for burst traffic",
    }),
  ],
  [
    13,
    buildAccountDetail(13, "audit.q9k8@ops.example", {
      note: "Shared overflow path for recovery retries",
    }),
  ],
  [
    21,
    buildAccountDetail(21, "growth.6vv4@relay.example", {
      note: "Shared growth workspace account",
    }),
  ],
  [
    22,
    buildAccountDetail(22, "mia.7rmmq@support.example", {
      note: "Secondary escalation workspace account",
    }),
  ],
  [31, buildAccountDetail(31, "sweep.q1h2@watch.example")],
  [41, buildAccountDetail(41, "burst.f9m4@watch.example")],
]);

function buildInvocationRecord(
  overrides: Partial<ApiInvocation> & {
    id: number;
    invokeId: string;
    occurredAt: string;
  },
): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    createdAt: overrides.createdAt ?? overrides.occurredAt,
    source: overrides.source ?? "pool",
    routeMode: overrides.routeMode ?? "pool",
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    upstreamAccountName: overrides.upstreamAccountName ?? undefined,
    endpoint: overrides.endpoint ?? "/v1/responses",
    model: overrides.model ?? "gpt-5.4",
    status: overrides.status ?? "completed",
    inputTokens: overrides.inputTokens ?? 0,
    outputTokens: overrides.outputTokens ?? 0,
    cacheInputTokens: overrides.cacheInputTokens ?? 0,
    reasoningTokens: overrides.reasoningTokens,
    reasoningEffort: overrides.reasoningEffort,
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    failureClass: overrides.failureClass ?? undefined,
    isActionable: overrides.isActionable,
    promptCacheKey: overrides.promptCacheKey,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 24,
    tReqParseMs: overrides.tReqParseMs ?? 6,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 480,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 120,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 640,
    tRespParseMs: overrides.tRespParseMs ?? 10,
    tPersistMs: overrides.tPersistMs ?? 8,
    tTotalMs: overrides.tTotalMs ?? 1280,
  };
}

function buildPreviewFromRecord(
  record: ApiInvocation,
) : StoryPromptCacheConversationPreview {
  return {
    id: record.id,
    invokeId: record.invokeId,
    occurredAt: record.occurredAt,
    source: record.source,
    status: record.status ?? "unknown",
    failureClass: record.failureClass ?? null,
    routeMode: record.routeMode ?? null,
    model: record.model ?? null,
    inputTokens: record.inputTokens,
    outputTokens: record.outputTokens,
    cacheInputTokens: record.cacheInputTokens,
    reasoningTokens: record.reasoningTokens,
    reasoningEffort: record.reasoningEffort,
    totalTokens: record.totalTokens ?? 0,
    cost: record.cost ?? null,
    errorMessage: record.errorMessage,
    failureKind: record.failureKind,
    isActionable: record.isActionable,
    proxyDisplayName: record.proxyDisplayName ?? null,
    upstreamAccountId: record.upstreamAccountId ?? null,
    upstreamAccountName: record.upstreamAccountName ?? null,
    endpoint: record.endpoint ?? null,
    responseContentEncoding: record.responseContentEncoding,
    requestedServiceTier: record.requestedServiceTier,
    serviceTier: record.serviceTier,
    tReqReadMs: record.tReqReadMs,
    tReqParseMs: record.tReqParseMs,
    tUpstreamConnectMs: record.tUpstreamConnectMs,
    tUpstreamTtfbMs: record.tUpstreamTtfbMs,
    tUpstreamStreamMs: record.tUpstreamStreamMs,
    tRespParseMs: record.tRespParseMs,
    tPersistMs: record.tPersistMs,
    tTotalMs: record.tTotalMs,
  };
}

const CONVERSATION_ONE_KEY = "019d2b8f-f8d0-72c3-bb67-a3f0d24a01f1";
const CONVERSATION_TWO_KEY = "019d2b8a-2df4-7580-bffc-6b4b1d8207c2";

const conversationOneHistory = [
  buildInvocationRecord({
    id: 501,
    invokeId: "invoke-pck-01-06",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:14:47.000Z",
    upstreamAccountId: 11,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "tokyo-edge-01",
    totalTokens: 65944,
    inputTokens: 61280,
    cacheInputTokens: 58624,
    outputTokens: 4664,
    reasoningTokens: 810,
    reasoningEffort: "high",
    cost: 0.0431,
    responseContentEncoding: "gzip, br",
    tUpstreamConnectMs: 612,
    tUpstreamTtfbMs: 126,
    tUpstreamStreamMs: 698,
    tTotalMs: 1492,
  }),
  buildInvocationRecord({
    id: 500,
    invokeId: "invoke-pck-01-05",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:14:42.000Z",
    upstreamAccountId: 11,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "tokyo-edge-01",
    totalTokens: 59790,
    inputTokens: 54870,
    cacheInputTokens: 52120,
    outputTokens: 4920,
    reasoningTokens: 740,
    reasoningEffort: "high",
    cost: 0.016,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 534,
    tUpstreamTtfbMs: 118,
    tUpstreamStreamMs: 620,
    tTotalMs: 1328,
  }),
  buildInvocationRecord({
    id: 499,
    invokeId: "invoke-pck-01-04",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:14:34.000Z",
    upstreamAccountId: 12,
    upstreamAccountName: "backup.f3x2@ops.example",
    proxyDisplayName: "osaka-edge-02",
    totalTokens: 59688,
    inputTokens: 55024,
    cacheInputTokens: 52310,
    outputTokens: 4664,
    reasoningTokens: 702,
    reasoningEffort: "medium",
    cost: 0.0161,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 688,
    tUpstreamTtfbMs: 144,
    tUpstreamStreamMs: 720,
    tTotalMs: 1586,
  }),
  buildInvocationRecord({
    id: 498,
    invokeId: "invoke-pck-01-03",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:14:27.000Z",
    upstreamAccountId: 13,
    upstreamAccountName: "audit.q9k8@ops.example",
    proxyDisplayName: "osaka-edge-02",
    endpoint: "/v1/chat/completions",
    status: "http_502",
    failureClass: "service_failure",
    errorMessage: "upstream gateway closed before first byte",
    totalTokens: 59549,
    inputTokens: 59549,
    cacheInputTokens: 0,
    outputTokens: 0,
    cost: 0.0161,
    responseContentEncoding: "identity",
    serviceTier: "auto",
    tUpstreamConnectMs: 1208,
    tUpstreamTtfbMs: null,
    tUpstreamStreamMs: null,
    tTotalMs: 30018,
    isActionable: true,
  }),
  buildInvocationRecord({
    id: 497,
    invokeId: "invoke-pck-01-02",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:14:02.000Z",
    upstreamAccountId: 11,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "singapore-edge-03",
    totalTokens: 59393,
    inputTokens: 54480,
    cacheInputTokens: 51120,
    outputTokens: 4913,
    reasoningTokens: 684,
    reasoningEffort: "medium",
    cost: 0.0276,
    responseContentEncoding: "gzip, br",
    tUpstreamConnectMs: 544,
    tUpstreamTtfbMs: 132,
    tUpstreamStreamMs: 603,
    tTotalMs: 1315,
  }),
  buildInvocationRecord({
    id: 496,
    invokeId: "invoke-pck-01-01",
    promptCacheKey: CONVERSATION_ONE_KEY,
    occurredAt: "2026-03-27T03:12:59.000Z",
    upstreamAccountId: 11,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "singapore-edge-03",
    totalTokens: 61120,
    inputTokens: 56240,
    cacheInputTokens: 53440,
    outputTokens: 4880,
    reasoningTokens: 701,
    reasoningEffort: "medium",
    cost: 0.0294,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 572,
    tUpstreamTtfbMs: 138,
    tUpstreamStreamMs: 648,
    tTotalMs: 1384,
  }),
];

const conversationTwoHistory = [
  buildInvocationRecord({
    id: 601,
    invokeId: "invoke-pck-02-06",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:19:19.000Z",
    upstreamAccountId: 21,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "frankfurt-edge-04",
    totalTokens: 74630,
    inputTokens: 69420,
    cacheInputTokens: 66200,
    outputTokens: 5210,
    reasoningTokens: 890,
    reasoningEffort: "high",
    cost: 0.0313,
    responseContentEncoding: "gzip, br",
    tUpstreamConnectMs: 618,
    tUpstreamTtfbMs: 141,
    tUpstreamStreamMs: 810,
    tTotalMs: 1686,
  }),
  buildInvocationRecord({
    id: 600,
    invokeId: "invoke-pck-02-05",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:18:56.000Z",
    upstreamAccountId: 21,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "frankfurt-edge-04",
    totalTokens: 72206,
    inputTokens: 67320,
    cacheInputTokens: 64100,
    outputTokens: 4886,
    reasoningTokens: 840,
    reasoningEffort: "high",
    cost: 0.0305,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 602,
    tUpstreamTtfbMs: 136,
    tUpstreamStreamMs: 774,
    tTotalMs: 1598,
  }),
  buildInvocationRecord({
    id: 599,
    invokeId: "invoke-pck-02-04",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:18:45.000Z",
    upstreamAccountId: 22,
    upstreamAccountName: "mia.7rmmq@support.example",
    proxyDisplayName: "frankfurt-edge-04",
    totalTokens: 71379,
    inputTokens: 66410,
    cacheInputTokens: 63144,
    outputTokens: 4969,
    reasoningTokens: 812,
    reasoningEffort: "medium",
    cost: 0.0275,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 644,
    tUpstreamTtfbMs: 149,
    tUpstreamStreamMs: 792,
    tTotalMs: 1642,
  }),
  buildInvocationRecord({
    id: 598,
    invokeId: "invoke-pck-02-03",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:18:32.000Z",
    upstreamAccountId: 22,
    upstreamAccountName: "mia.7rmmq@support.example",
    proxyDisplayName: "madrid-edge-05",
    totalTokens: 68983,
    inputTokens: 64210,
    cacheInputTokens: 61002,
    outputTokens: 4773,
    reasoningTokens: 788,
    reasoningEffort: "medium",
    cost: 0.0371,
    responseContentEncoding: "gzip, br",
    tUpstreamConnectMs: 700,
    tUpstreamTtfbMs: 155,
    tUpstreamStreamMs: 840,
    tTotalMs: 1764,
  }),
  buildInvocationRecord({
    id: 597,
    invokeId: "invoke-pck-02-02",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:18:15.000Z",
    upstreamAccountId: 21,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "madrid-edge-05",
    totalTokens: 63629,
    inputTokens: 59040,
    cacheInputTokens: 56120,
    outputTokens: 4589,
    reasoningTokens: 701,
    reasoningEffort: "medium",
    cost: 0.0327,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 582,
    tUpstreamTtfbMs: 133,
    tUpstreamStreamMs: 728,
    tTotalMs: 1503,
  }),
  buildInvocationRecord({
    id: 596,
    invokeId: "invoke-pck-02-01",
    promptCacheKey: CONVERSATION_TWO_KEY,
    occurredAt: "2026-03-27T03:17:44.000Z",
    upstreamAccountId: 21,
    upstreamAccountName: "growth.6vv4@relay.example",
    proxyDisplayName: "madrid-edge-05",
    totalTokens: 61208,
    inputTokens: 56990,
    cacheInputTokens: 53910,
    outputTokens: 4218,
    reasoningTokens: 655,
    reasoningEffort: "medium",
    cost: 0.0289,
    responseContentEncoding: "gzip",
    tUpstreamConnectMs: 560,
    tUpstreamTtfbMs: 129,
    tUpstreamStreamMs: 684,
    tTotalMs: 1436,
  }),
];

const conversationOnePreviews = conversationOneHistory
  .slice(0, 5)
  .map(buildPreviewFromRecord);
const conversationTwoPreviews = conversationTwoHistory
  .slice(0, 5)
  .map(buildPreviewFromRecord);

const historyRecordsByKey = new Map<string, ApiInvocation[]>([
  [
    CONVERSATION_ONE_KEY,
    conversationOneHistory,
  ],
  [
    CONVERSATION_TWO_KEY,
    conversationTwoHistory,
  ],
]);

function StorybookPromptCacheAccountMock({
  children,
}: {
  children: ReactNode;
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      const method = (
        init?.method ||
        (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const inputUrl =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      const parsedUrl = new URL(inputUrl, window.location.origin);
      const match = parsedUrl.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);

      if (match && method === "GET") {
        const accountId = Number(match[1]);
        const detail = accountDetails.get(accountId);
        if (!detail) {
          return jsonResponse({ message: "Not found" }, 404);
        }
        return jsonResponse(detail);
      }

      if (parsedUrl.pathname === "/api/invocations" && method === "GET") {
        const promptCacheKey = parsedUrl.searchParams.get("promptCacheKey");
        if (promptCacheKey) {
          const page = Number(parsedUrl.searchParams.get("page") ?? "1");
          const pageSize = Number(parsedUrl.searchParams.get("pageSize") ?? "20");
          const snapshotId = Number(
            parsedUrl.searchParams.get("snapshotId") ?? "8401",
          );
          const records = historyRecordsByKey.get(promptCacheKey) ?? [];
          const start = Math.max(0, (page - 1) * pageSize);
          const pagedRecords = records.slice(start, start + pageSize);

          return jsonResponse({
            snapshotId,
            total: records.length,
            page,
            pageSize,
            records: pagedRecords,
          });
        }
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init);
    };
  }

  useEffect(() => {
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
    };
  }, []);

  return <>{children}</>;
}

const stats: PromptCacheConversationsResponse = {
  rangeStart: "2026-03-26T03:00:00.000Z",
  rangeEnd: "2026-03-27T03:20:00.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: CONVERSATION_ONE_KEY,
      requestCount: 15,
      totalTokens: 784054,
      totalCost: 0.403,
      createdAt: "2026-03-27T03:12:32.000Z",
      lastActivityAt: "2026-03-27T03:14:47.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 11,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: 9,
          totalTokens: 431220,
          totalCost: 0.2214,
          lastActivityAt: "2026-03-27T03:14:47.000Z",
        },
        {
          upstreamAccountId: 12,
          upstreamAccountName: "backup.f3x2@ops.example",
          requestCount: 4,
          totalTokens: 221944,
          totalCost: 0.1137,
          lastActivityAt: "2026-03-27T03:14:34.000Z",
        },
        {
          upstreamAccountId: 13,
          upstreamAccountName: "audit.q9k8@ops.example",
          requestCount: 2,
          totalTokens: 130890,
          totalCost: 0.0679,
          lastActivityAt: "2026-03-27T03:14:27.000Z",
        },
      ],
      recentInvocations: conversationOnePreviews,
      last24hRequests: [
        {
          occurredAt: "2026-03-26T07:14:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 84210,
          cumulativeTokens: 84210,
        },
        {
          occurredAt: "2026-03-26T12:10:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 126430,
          cumulativeTokens: 210640,
        },
        {
          occurredAt: "2026-03-26T18:42:00.000Z",
          status: "http_502",
          isSuccess: false,
          requestTokens: 59549,
          cumulativeTokens: 270189,
        },
        {
          occurredAt: "2026-03-27T01:35:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 213920,
          cumulativeTokens: 484109,
        },
        {
          occurredAt: "2026-03-27T03:14:47.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 299945,
          cumulativeTokens: 784054,
        },
      ],
    },
    {
      promptCacheKey: CONVERSATION_TWO_KEY,
      requestCount: 13,
      totalTokens: 774794,
      totalCost: 0.4501,
      createdAt: "2026-03-27T03:07:14.000Z",
      lastActivityAt: "2026-03-27T03:19:19.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: 8,
          totalTokens: 452106,
          totalCost: 0.2623,
          lastActivityAt: "2026-03-27T03:19:19.000Z",
        },
        {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
          requestCount: 5,
          totalTokens: 322688,
          totalCost: 0.1878,
          lastActivityAt: "2026-03-27T03:18:45.000Z",
        },
      ],
      recentInvocations: conversationTwoPreviews,
      last24hRequests: [
        {
          occurredAt: "2026-03-26T08:22:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 102448,
          cumulativeTokens: 102448,
        },
        {
          occurredAt: "2026-03-26T12:38:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 148930,
          cumulativeTokens: 251378,
        },
        {
          occurredAt: "2026-03-26T18:55:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 168441,
          cumulativeTokens: 419819,
        },
        {
          occurredAt: "2026-03-27T03:19:19.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 354975,
          cumulativeTokens: 774794,
        },
      ],
    },
  ],
};

const sharedScaleStats: PromptCacheConversationsResponse = {
  rangeStart: "2026-03-02T00:00:00.000Z",
  rangeEnd: "2026-03-03T00:00:00.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: "019d2b69-ca16-73f2-bf97-0e9b9a1f0c31",
      requestCount: 3,
      totalTokens: 420,
      totalCost: 0.01,
      createdAt: "2026-03-02T03:00:00.000Z",
      lastActivityAt: "2026-03-02T05:00:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 31,
          upstreamAccountName: "sweep.q1h2@watch.example",
          requestCount: 3,
          totalTokens: 420,
          totalCost: 0.01,
          lastActivityAt: "2026-03-02T05:00:00.000Z",
        },
      ],
      recentInvocations: [
        buildPreviewFromRecord(
          buildInvocationRecord({
            id: 701,
            invokeId: "invoke-low-01",
            promptCacheKey: "019d2b69-ca16-73f2-bf97-0e9b9a1f0c31",
            occurredAt: "2026-03-02T05:00:00.000Z",
            totalTokens: 120,
            cost: 0.003,
            proxyDisplayName: "hong-kong-edge-01",
            upstreamAccountId: 31,
            upstreamAccountName: "sweep.q1h2@watch.example",
          }),
        ),
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T03:00:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 100,
          cumulativeTokens: 100,
        },
        {
          occurredAt: "2026-03-02T05:00:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 120,
          cumulativeTokens: 220,
        },
      ],
    },
    {
      promptCacheKey: "019d2b77-b081-7180-80bd-5cc31df7f9b4",
      requestCount: 8,
      totalTokens: 8600,
      totalCost: 0.21,
      createdAt: "2026-03-02T02:30:00.000Z",
      lastActivityAt: "2026-03-02T23:40:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 41,
          upstreamAccountName: "burst.f9m4@watch.example",
          requestCount: 8,
          totalTokens: 8600,
          totalCost: 0.21,
          lastActivityAt: "2026-03-02T23:40:00.000Z",
        },
      ],
      recentInvocations: [
        buildPreviewFromRecord(
          buildInvocationRecord({
            id: 801,
            invokeId: "invoke-high-01",
            promptCacheKey: "019d2b77-b081-7180-80bd-5cc31df7f9b4",
            occurredAt: "2026-03-02T23:40:00.000Z",
            totalTokens: 2200,
            cost: 0.052,
            proxyDisplayName: "london-edge-02",
            upstreamAccountId: 41,
            upstreamAccountName: "burst.f9m4@watch.example",
          }),
        ),
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T02:30:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1200,
          cumulativeTokens: 1200,
        },
        {
          occurredAt: "2026-03-02T09:10:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1800,
          cumulativeTokens: 3000,
        },
        {
          occurredAt: "2026-03-02T18:50:00.000Z",
          status: "upstream_stream_error",
          isSuccess: false,
          requestTokens: 900,
          cumulativeTokens: 3900,
        },
        {
          occurredAt: "2026-03-02T23:40:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 2200,
          cumulativeTokens: 6100,
        },
      ],
    },
  ],
};

const meta = {
  title: "Monitoring/PromptCacheConversationTable",
  component: PromptCacheConversationTable,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <MemoryRouter>
        <I18nProvider>
          <StorybookPromptCacheAccountMock>
            <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
              <main className="mx-auto w-full max-w-[1200px] space-y-4">
                <h2 className="text-xl font-semibold">
                  对话
                </h2>
                <Story />
              </main>
            </div>
          </StorybookPromptCacheAccountMock>
        </I18nProvider>
      </MemoryRouter>
    ),
  ],
} satisfies Meta<typeof PromptCacheConversationTable>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
};

export const SingleExpanded: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: [stats.conversations[0]?.promptCacheKey ?? ""],
  },
};

export const ExpandAll: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: stats.conversations.map(
      (conversation) => conversation.promptCacheKey,
    ),
  },
};

export const Empty: Story = {
  args: {
    stats: {
      rangeStart: stats.rangeStart,
      rangeEnd: stats.rangeEnd,
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [],
    },
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

export const ErrorState: Story = {
  args: {
    stats: null,
    isLoading: false,
    error: "Network error",
  },
};

export const SharedScaleComparison: Story = {
  args: {
    stats: sharedScaleStats,
    isLoading: false,
    error: null,
  },
};

export const TooltipEdgeDensity: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Hover or tap the final token segment to verify the shared tooltip flips inward near the right table edge without clipping.",
      },
    },
  },
};

export const DrawerOpen: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await expect(
      await documentScope.findByText(/All retained calls/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/Loaded 6 \/ 6 retained record\(s\)/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getAllByTestId("invocation-table-scroll").length,
    ).toBeGreaterThan(0);
  },
};
