import { useEffect, useRef, type ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fireEvent, userEvent, waitFor, within } from "storybook/test";
import { MemoryRouter } from "react-router-dom";
import { I18nProvider } from "../i18n";
import type {
  ApiInvocation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
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

const CONVERSATION_ONE_KEY = "019d2b8f-f8d0-72c3-bb67-a3f0d24a01f1";
const CONVERSATION_TWO_KEY = "019d2b8a-2df4-7580-bffc-6b4b1d8207c2";
const CONVERSATION_SHORT_KEY = "019e239a-038c-7860-a185-46a9d45553f7";
const CONVERSATION_LARGE_HISTORY_KEY = "019f0d8c-91f2-7f25-b2b7-large-history";

function buildBindingResponse(
  overrides: Partial<PromptCacheConversationBindingResponse> & {
    promptCacheKey: string;
    bindingKind: PromptCacheConversationBindingResponse["bindingKind"];
  },
): PromptCacheConversationBindingResponse {
  return {
    promptCacheKey: overrides.promptCacheKey,
    bindingKind: overrides.bindingKind,
    groupName: overrides.groupName ?? null,
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    upstreamAccountName: overrides.upstreamAccountName ?? null,
    hasEncryptedSessionOwner: overrides.hasEncryptedSessionOwner ?? false,
    encryptedOwnerAccountId: overrides.encryptedOwnerAccountId ?? null,
    encryptedOwnerAccountName: overrides.encryptedOwnerAccountName ?? null,
    encryptedOwnerGroupName: overrides.encryptedOwnerGroupName ?? null,
    timeouts: overrides.timeouts ?? {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 300,
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    timeoutFieldSources: overrides.timeoutFieldSources ?? {
      responsesFirstByteTimeoutSecs: "root",
      compactFirstByteTimeoutSecs: "root",
      responsesStreamTimeoutSecs: "root",
      compactStreamTimeoutSecs: "root",
    },
    allowSwitchUpstream: overrides.allowSwitchUpstream ?? null,
    fastModeRewriteMode: overrides.fastModeRewriteMode ?? null,
    imageToolRewriteMode: overrides.imageToolRewriteMode ?? null,
    availableModels: overrides.availableModels ?? null,
    forwardProxyKey: overrides.forwardProxyKey ?? null,
    forwardProxyKeys:
      overrides.forwardProxyKeys ??
      (overrides.forwardProxyKey ? [overrides.forwardProxyKey] : []),
    updatedAt: overrides.updatedAt ?? null,
  };
}

class MockEventSource implements EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readonly url: string;
  readonly withCredentials = false;
  readyState = MockEventSource.CONNECTING;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null = null;
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null;

  #listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();

  constructor(url: string | URL) {
    this.url = typeof url === "string" ? url : url.toString();
    window.setTimeout(() => {
      if (this.readyState === MockEventSource.CLOSED) return;
      this.readyState = MockEventSource.OPEN;
      this.#emit("open", new Event("open"));
    }, 40);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const bucket = this.#listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>();
    bucket.add(listener);
    this.#listeners.set(type, bucket);
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    this.#listeners.get(type)?.delete(listener);
  }

  dispatchEvent(event: Event) {
    this.#emit(event.type, event);
    return true;
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }

  #emit(type: string, event: Event) {
    if (type === "open") this.onopen?.call(this as unknown as EventSource, event);
    if (type === "error") this.onerror?.call(this as unknown as EventSource, event);
    if (type === "message") {
      this.onmessage?.call(this as unknown as EventSource, event as MessageEvent<string>);
    }

    for (const listener of this.#listeners.get(type) ?? []) {
      if (typeof listener === "function") {
        listener(event);
      } else {
        listener.handleEvent(event);
      }
    }
  }
}

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
      blockNewConversations: false,
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
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

function buildAccountSummary(
  detail: UpstreamAccountDetail,
  overrides?: Partial<UpstreamAccountSummary>,
): UpstreamAccountSummary {
  return {
    id: detail.id,
    kind: detail.kind,
    provider: "codex",
    displayName: detail.displayName,
    groupName: overrides?.groupName ?? detail.groupName,
    isMother: detail.isMother,
    status: detail.status,
    workStatus: "idle",
    enableStatus: "enabled",
    healthStatus: "normal",
    syncState: "idle",
    displayStatus: detail.status,
    enabled: detail.enabled,
    email: detail.email,
    chatgptAccountId: detail.chatgptAccountId,
    planType: detail.planType,
    maskedApiKey: detail.maskedApiKey,
    tags: detail.tags,
    effectiveRoutingRule: detail.effectiveRoutingRule,
    ...overrides,
  };
}

const accountSummaries = Array.from(accountDetails.values()).map((detail, index) =>
  buildAccountSummary(detail, {
    groupName: index < 3 ? "JOZ Team" : index < 5 ? "CIII" : "Overflow",
  }),
);

const storyForwardProxyNodes = [
  {
    key: "__direct__",
    displayName: "Direct",
    protocolLabel: "DIRECT",
    source: "direct",
    selectable: true,
    penalized: false,
    aliasKeys: [],
    last24h: [],
  },
  {
    key: "tokyo-edge-01",
    displayName: "Tokyo Edge 01",
    protocolLabel: "HTTP",
    source: "node",
    selectable: true,
    penalized: false,
    aliasKeys: ["jp-edge-01"],
    last24h: [],
  },
] as const;

const bindingByPromptCacheKey = new Map<string, PromptCacheConversationBindingResponse>([
  [
    CONVERSATION_ONE_KEY,
    buildBindingResponse({
      promptCacheKey: CONVERSATION_ONE_KEY,
      bindingKind: "group",
      groupName: "JOZ Team",
      timeouts: {
        responsesFirstByteTimeoutSecs: 90,
        compactFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "group",
        compactFirstByteTimeoutSecs: "root",
        responsesStreamTimeoutSecs: "root",
        compactStreamTimeoutSecs: "root",
      },
      updatedAt: "2026-03-27T03:16:00.000Z",
    }),
  ],
  [
    CONVERSATION_SHORT_KEY,
    buildBindingResponse({
      promptCacheKey: CONVERSATION_SHORT_KEY,
      bindingKind: "upstreamAccount",
      upstreamAccountId: 21,
      upstreamAccountName: "growth.6vv4@relay.example",
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      timeouts: {
        responsesFirstByteTimeoutSecs: 45,
        compactFirstByteTimeoutSecs: 180,
        responsesStreamTimeoutSecs: 240,
        compactStreamTimeoutSecs: 300,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "conversation",
        compactFirstByteTimeoutSecs: "account",
        responsesStreamTimeoutSecs: "conversation",
        compactStreamTimeoutSecs: "root",
      },
      updatedAt: "2026-05-13T23:42:00.000Z",
    }),
  ],
  [
    CONVERSATION_LARGE_HISTORY_KEY,
    buildBindingResponse({
      promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY,
      bindingKind: "upstreamAccount",
      upstreamAccountId: 11,
      upstreamAccountName: "growth.6vv4@relay.example",
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 11,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "JOZ Team",
      timeouts: {
        responsesFirstByteTimeoutSecs: 120,
        compactFirstByteTimeoutSecs: 240,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "root",
        compactFirstByteTimeoutSecs: "account",
        responsesStreamTimeoutSecs: "root",
        compactStreamTimeoutSecs: "root",
      },
      updatedAt: "2026-05-28T04:10:00.000Z",
    }),
  ],
] as const);

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

const shortSameDayStartMs = Date.parse("2026-05-13T23:26:12.000Z");
const shortSameDayEndMs = Date.parse("2026-05-13T23:40:47.000Z");
const shortSameDayOffsetsMs = [
  0,
  24_000,
  48_000,
  75_000,
  108_000,
  136_000,
  169_000,
  198_000,
  232_000,
  259_000,
  286_000,
  315_000,
  348_000,
  374_000,
  402_000,
  402_000,
  458_000,
  486_000,
  514_000,
  541_000,
  566_000,
  593_000,
  620_000,
  648_000,
  676_000,
  704_000,
  731_000,
  758_000,
  785_000,
  812_000,
  shortSameDayEndMs - shortSameDayStartMs,
];

const shortSameDayHistory = shortSameDayOffsetsMs
  .map((offsetMs, index) => {
    const newestId = 930 - index;
    const isFailure = index === 15 || index === 24 || index === 28;
    const isSecondAccount = index % 5 === 2 || index % 7 === 4;
    const totalTokens = 181_000 + ((index * 1_487) % 8_800);
    const outputTokens = isFailure ? 0 : 34 + ((index * 173) % 2_400);
    return buildInvocationRecord({
      id: newestId,
      invokeId: `invoke-short-${String(index + 1).padStart(2, "0")}`,
      promptCacheKey: CONVERSATION_SHORT_KEY,
      occurredAt: new Date(shortSameDayStartMs + offsetMs).toISOString(),
      upstreamAccountId: isSecondAccount ? 22 : 21,
      upstreamAccountName: isSecondAccount
        ? "mia.7rmmq@support.example"
        : "growth.6vv4@relay.example",
      proxyDisplayName: isSecondAccount ? "madrid-edge-05" : "frankfurt-edge-04",
      status: isFailure ? "http_502" : "completed",
      failureClass: isFailure ? "service_failure" : "none",
      isActionable: isFailure,
      totalTokens,
      inputTokens: totalTokens - outputTokens,
      cacheInputTokens: Math.max(0, totalTokens - outputTokens - 512),
      outputTokens,
      reasoningTokens: isFailure ? 0 : index % 4 === 0 ? 812 : 0,
      reasoningEffort: "medium",
      cost: Number((0.091 + (index % 9) * 0.0087).toFixed(4)),
      tTotalMs: isFailure ? 30_000 + index * 740 : 10_500 + (index % 11) * 1_920,
      responseContentEncoding: "identity",
    });
  })
  .reverse();

const conversationOnePreviews = conversationOneHistory
  .slice(0, 5)
  .map(buildPreviewFromRecord);
const conversationTwoPreviews = conversationTwoHistory
  .slice(0, 5)
  .map(buildPreviewFromRecord);
const shortSameDayPreviews = shortSameDayHistory
  .slice(0, 4)
  .map(buildPreviewFromRecord);
const largeHistory = Array.from({ length: 15_000 }, (_, index) => {
  const isFailure = index % 41 === 0;
  const account = index % 3 === 0 ? accountSummaries[1] : accountSummaries[0];
  const occurredAt = new Date(
    Date.parse("2026-05-28T04:10:00.000Z") - index * 45_000,
  ).toISOString();
  return buildInvocationRecord({
    id: 20_000 - index,
    invokeId: `invoke-large-history-${String(index + 1).padStart(5, "0")}`,
    promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY,
    occurredAt,
    upstreamAccountId: account?.id ?? 11,
    upstreamAccountName: account?.displayName ?? "growth.6vv4@relay.example",
    proxyDisplayName: index % 2 === 0 ? "tokyo-edge-large-01" : "osaka-edge-large-02",
    status: isFailure ? "http_502" : "completed",
    failureClass: isFailure ? "service_failure" : "none",
    errorMessage: isFailure ? "[upstream_response_failed] gateway timeout" : undefined,
    isActionable: isFailure,
    totalTokens: 180_000 + (index % 700) * 37,
    inputTokens: 172_000 + (index % 500) * 29,
    cacheInputTokens: 168_000 + (index % 300) * 17,
    outputTokens: isFailure ? 0 : 300 + (index % 900),
    reasoningTokens: isFailure ? 0 : index % 5 === 0 ? 448 : 117,
    reasoningEffort: index % 4 === 0 ? "high" : "medium",
    cost: Number((0.11 + (index % 23) * 0.0047).toFixed(4)),
    responseContentEncoding: isFailure ? "identity" : "gzip",
    tTotalMs: isFailure ? 300_000 : 6_000 + (index % 70) * 200,
  });
});
const largeHistoryPreviews = largeHistory.slice(0, 5).map(buildPreviewFromRecord);

const historyRecordsByKey = new Map<string, ApiInvocation[]>([
  [
    CONVERSATION_ONE_KEY,
    conversationOneHistory,
  ],
  [
    CONVERSATION_TWO_KEY,
    conversationTwoHistory,
  ],
  [
    CONVERSATION_SHORT_KEY,
    shortSameDayHistory,
  ],
  [
    CONVERSATION_LARGE_HISTORY_KEY,
    largeHistory,
  ],
]);

function buildInvocationSummary(records: ApiInvocation[]) {
  const totalCost = records.reduce((sum, record) => sum + (record.cost ?? 0), 0);
  const totalTokens = records.reduce(
    (sum, record) => sum + (record.totalTokens ?? 0),
    0,
  );
  const completedRecords = records.filter((record) => record.status === "completed");
  const failedRecords = records.filter(
    (record) =>
      record.failureClass === "service_failure" ||
      record.failureClass === "client_failure" ||
      record.failureClass === "client_abort",
  );
  const durationSamples = records
    .map((record) => record.tTotalMs)
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  const avgTotalMs =
    durationSamples.length > 0
      ? durationSamples.reduce((sum, value) => sum + value, 0) /
        durationSamples.length
      : null;

  return {
    snapshotId: 8401,
    newRecordsCount: 0,
    totalCount: records.length,
    successCount: completedRecords.length,
    failureCount: failedRecords.length,
    totalCost,
    totalTokens,
    token: {
      requestCount: records.length,
      totalTokens,
      avgTokensPerRequest: records.length > 0 ? totalTokens / records.length : 0,
      cacheInputTokens: records.reduce(
        (sum, record) => sum + (record.cacheInputTokens ?? 0),
        0,
      ),
      totalCost,
    },
    network: {
      avgTtfbMs: null,
      p95TtfbMs: null,
      avgTotalMs,
      p95TotalMs: durationSamples.length > 0 ? Math.max(...durationSamples) : null,
    },
    exception: {
      failureCount: failedRecords.length,
      serviceFailureCount: failedRecords.filter(
        (record) => record.failureClass === "service_failure",
      ).length,
      clientFailureCount: failedRecords.filter(
        (record) => record.failureClass === "client_failure",
      ).length,
      clientAbortCount: failedRecords.filter(
        (record) => record.failureClass === "client_abort",
      ).length,
      actionableFailureCount: failedRecords.filter((record) => record.isActionable).length,
    },
  };
}

function StorybookPromptCacheAccountMock({
  children,
}: {
  children: ReactNode;
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);
    originalEventSourceRef.current = window.EventSource;
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
      const bindingMatch = parsedUrl.pathname.match(
        /^\/api\/stats\/prompt-cache-conversation-bindings\/(.+)$/,
      );
      if (bindingMatch && method === "GET") {
        const promptCacheKey = decodeURIComponent(bindingMatch[1] ?? "");
        return jsonResponse(
          bindingByPromptCacheKey.get(promptCacheKey) ?? {
            promptCacheKey,
            bindingKind: "none",
            groupName: null,
            upstreamAccountId: null,
            upstreamAccountName: null,
            hasEncryptedSessionOwner: false,
            encryptedOwnerAccountId: null,
            encryptedOwnerAccountName: null,
            encryptedOwnerGroupName: null,
            timeouts: {
              responsesFirstByteTimeoutSecs: 120,
              compactFirstByteTimeoutSecs: 300,
              responsesStreamTimeoutSecs: 300,
              compactStreamTimeoutSecs: 300,
            },
            timeoutFieldSources: {
              responsesFirstByteTimeoutSecs: "root",
              compactFirstByteTimeoutSecs: "root",
              responsesStreamTimeoutSecs: "root",
              compactStreamTimeoutSecs: "root",
            },
            updatedAt: null,
          },
        );
      }
      if (bindingMatch && method === "PATCH") {
        const promptCacheKey = decodeURIComponent(bindingMatch[1] ?? "");
        const payload = init?.body ? JSON.parse(String(init.body)) : {};
        const timeoutPatch = payload.timeouts ?? {};
        const current =
          bindingByPromptCacheKey.get(promptCacheKey) ??
          buildBindingResponse({
            promptCacheKey,
            bindingKind: "none",
            hasEncryptedSessionOwner: true,
            encryptedOwnerAccountId: 21,
            encryptedOwnerAccountName: "growth.6vv4@relay.example",
            encryptedOwnerGroupName: "CIII",
          });
        const nextTimeouts = {
          ...current.timeouts,
          ...Object.fromEntries(
            Object.entries(timeoutPatch).map(([key, value]) => [
              key,
              value == null ? current.timeouts[key as keyof typeof current.timeouts] : value,
            ]),
          ),
        };
        const nextTimeoutSources = {
          ...current.timeoutFieldSources,
          responsesFirstByteTimeoutSecs:
            timeoutPatch.responsesFirstByteTimeoutSecs === null
              ? "account"
              : timeoutPatch.responsesFirstByteTimeoutSecs != null
                ? "conversation"
                : current.timeoutFieldSources.responsesFirstByteTimeoutSecs,
          compactFirstByteTimeoutSecs:
            timeoutPatch.compactFirstByteTimeoutSecs === null
              ? "account"
              : timeoutPatch.compactFirstByteTimeoutSecs != null
                ? "conversation"
                : current.timeoutFieldSources.compactFirstByteTimeoutSecs,
          responsesStreamTimeoutSecs:
            timeoutPatch.responsesStreamTimeoutSecs === null
              ? "account"
              : timeoutPatch.responsesStreamTimeoutSecs != null
                ? "conversation"
                : current.timeoutFieldSources.responsesStreamTimeoutSecs,
          compactStreamTimeoutSecs:
            timeoutPatch.compactStreamTimeoutSecs === null
              ? "root"
              : timeoutPatch.compactStreamTimeoutSecs != null
                ? "conversation"
                : current.timeoutFieldSources.compactStreamTimeoutSecs,
        };
        const currentPolicySources = current.policyFieldSources ?? {
          allowSwitchUpstream: "account",
          fastModeRewriteMode: "account",
          imageToolRewriteMode: "account",
          availableModels: "account",
          forwardProxyKey: "account",
        };
        const policyOverrides = {
          allowSwitchUpstream:
            "allowSwitchUpstream" in payload
              ? payload.allowSwitchUpstream
              : current.allowSwitchUpstream,
          fastModeRewriteMode:
            "fastModeRewriteMode" in payload
              ? payload.fastModeRewriteMode
              : current.fastModeRewriteMode,
          imageToolRewriteMode:
            "imageToolRewriteMode" in payload
              ? payload.imageToolRewriteMode
              : current.imageToolRewriteMode,
          availableModels:
            "availableModels" in payload
              ? payload.availableModels
              : current.availableModels,
          forwardProxyKey: Array.isArray(payload.forwardProxyKeys)
            ? (payload.forwardProxyKeys[0] ?? null)
            : "forwardProxyKey" in payload
              ? payload.forwardProxyKey
              : current.forwardProxyKey,
          forwardProxyKeys: Array.isArray(payload.forwardProxyKeys)
            ? payload.forwardProxyKeys
            : "forwardProxyKey" in payload && payload.forwardProxyKey
              ? [payload.forwardProxyKey]
              : current.forwardProxyKeys,
          policyFieldSources: {
            ...currentPolicySources,
            ...(Array.isArray(payload.forwardProxyKeys) ||
            "forwardProxyKey" in payload
              ? { forwardProxyKey: "conversation" as const }
              : {}),
          },
        };
        const response =
          payload.bindingKind === "upstreamAccount"
            ? buildBindingResponse({
                promptCacheKey,
                bindingKind: "upstreamAccount",
                upstreamAccountId: Number(payload.upstreamAccountId),
                upstreamAccountName:
                  accountSummaries.find(
                    (account) => account.id === Number(payload.upstreamAccountId),
                  )?.displayName ?? null,
                hasEncryptedSessionOwner: true,
                encryptedOwnerAccountId: 21,
                encryptedOwnerAccountName: "growth.6vv4@relay.example",
                encryptedOwnerGroupName: "CIII",
                timeouts: nextTimeouts,
                timeoutFieldSources: nextTimeoutSources,
                ...policyOverrides,
                updatedAt: new Date().toISOString(),
              })
            : payload.bindingKind === "group"
              ? buildBindingResponse({
                  promptCacheKey,
                  bindingKind: "group",
                  groupName: String(payload.groupName ?? ""),
                  hasEncryptedSessionOwner: true,
                  encryptedOwnerAccountId: 21,
                  encryptedOwnerAccountName: "growth.6vv4@relay.example",
                  encryptedOwnerGroupName: "CIII",
                  timeouts: nextTimeouts,
                  timeoutFieldSources: nextTimeoutSources,
                  ...policyOverrides,
                  updatedAt: new Date().toISOString(),
                })
              : buildBindingResponse({
                  promptCacheKey,
                  bindingKind: "none",
                  hasEncryptedSessionOwner: true,
                  encryptedOwnerAccountId: 21,
                  encryptedOwnerAccountName: "growth.6vv4@relay.example",
                  encryptedOwnerGroupName: "CIII",
                  timeouts: nextTimeouts,
                  timeoutFieldSources: nextTimeoutSources,
                  ...policyOverrides,
                  updatedAt:
                    Object.values(timeoutPatch).some((value) => value !== undefined)
                      ? new Date().toISOString()
                      : null,
                });
        bindingByPromptCacheKey.set(promptCacheKey, response);
        return jsonResponse(response);
      }

      if (parsedUrl.pathname === "/api/pool/upstream-accounts" && method === "GET") {
        return jsonResponse({
          writesEnabled: true,
          items: accountSummaries,
          groups: [
            { groupName: "JOZ Team", accountCount: 3 },
            { groupName: "CIII", accountCount: 2 },
            { groupName: "Overflow", accountCount: 2 },
          ],
          forwardProxyNodes: storyForwardProxyNodes,
          hasUngroupedAccounts: false,
          total: accountSummaries.length,
          page: 1,
          pageSize: accountSummaries.length,
          routing: null,
        });
      }

      const match = parsedUrl.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);

      if (match && method === "GET") {
        const accountId = Number(match[1]);
        const detail = accountDetails.get(accountId);
        if (!detail) {
          return jsonResponse({ message: "Not found" }, 404);
        }
        return jsonResponse(detail);
      }

      if (parsedUrl.pathname === "/api/invocations/summary" && method === "GET") {
        const promptCacheKey = parsedUrl.searchParams.get("promptCacheKey");
        const records = promptCacheKey
          ? historyRecordsByKey.get(promptCacheKey) ?? []
          : [];
        return jsonResponse(buildInvocationSummary(records));
      }

      if (parsedUrl.pathname === "/api/invocations" && method === "GET") {
        const promptCacheKey = parsedUrl.searchParams.get("promptCacheKey");
        if (promptCacheKey) {
          const storyWindow = window as typeof window & {
            __promptCacheInvocationRequests?: string[];
          };
          storyWindow.__promptCacheInvocationRequests ??= [];
          storyWindow.__promptCacheInvocationRequests.push(parsedUrl.search);
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
    window.EventSource = MockEventSource as unknown as typeof EventSource;
  }

  useEffect(() => {
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
      if (originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current;
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
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
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
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
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
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
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
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
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

const shortSameDayStats: PromptCacheConversationsResponse = {
  rangeStart: "2026-05-13T16:00:00.000Z",
  rangeEnd: "2026-05-14T15:59:59.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: CONVERSATION_SHORT_KEY,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      requestCount: shortSameDayHistory.length,
      totalTokens: shortSameDayHistory.reduce(
        (sum, record) => sum + (record.totalTokens ?? 0),
        0,
      ),
      totalCost: shortSameDayHistory.reduce(
        (sum, record) => sum + (record.cost ?? 0),
        0,
      ),
      createdAt: shortSameDayHistory.at(-1)?.occurredAt ?? "",
      lastActivityAt: shortSameDayHistory[0]?.occurredAt ?? "",
      upstreamAccounts: [
        {
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: shortSameDayHistory.filter(
            (record) => record.upstreamAccountId === 21,
          ).length,
          totalTokens: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 21)
            .reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
          totalCost: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 21)
            .reduce((sum, record) => sum + (record.cost ?? 0), 0),
          lastActivityAt: shortSameDayHistory[0]?.occurredAt ?? "",
        },
        {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
          requestCount: shortSameDayHistory.filter(
            (record) => record.upstreamAccountId === 22,
          ).length,
          totalTokens: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 22)
            .reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
          totalCost: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 22)
            .reduce((sum, record) => sum + (record.cost ?? 0), 0),
          lastActivityAt:
            shortSameDayHistory.find((record) => record.upstreamAccountId === 22)
              ?.occurredAt ?? "",
        },
      ],
      recentInvocations: shortSameDayPreviews,
      last24hRequests: shortSameDayHistory
        .slice()
        .reverse()
        .map((record, index, records) => ({
          occurredAt: record.occurredAt,
          status: record.status ?? "completed",
          isSuccess: record.failureClass === "none",
          requestTokens: record.totalTokens ?? 0,
          cumulativeTokens: records
            .slice(0, index + 1)
            .reduce((sum, item) => sum + (item.totalTokens ?? 0), 0),
        })),
    },
  ],
};

const largeHistoryStats: PromptCacheConversationsResponse = {
  rangeStart: largeHistory.at(-1)?.occurredAt ?? "",
  rangeEnd: largeHistory[0]?.occurredAt ?? "",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY,
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      requestCount: largeHistory.length,
      totalTokens: largeHistory.reduce(
        (sum, record) => sum + (record.totalTokens ?? 0),
        0,
      ),
      totalCost: largeHistory.reduce((sum, record) => sum + (record.cost ?? 0), 0),
      createdAt: largeHistory.at(-1)?.occurredAt ?? "",
      lastActivityAt: largeHistory[0]?.occurredAt ?? "",
      upstreamAccounts: [
        {
          upstreamAccountId: 11,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: Math.ceil(largeHistory.length / 2),
          totalTokens: 1_384_000_000,
          totalCost: 910.24,
          lastActivityAt: largeHistory[0]?.occurredAt ?? "",
        },
        {
          upstreamAccountId: 12,
          upstreamAccountName: "backup.f3x2@ops.example",
          requestCount: Math.floor(largeHistory.length / 2),
          totalTokens: 1_216_000_000,
          totalCost: 801.18,
          lastActivityAt: largeHistory[1]?.occurredAt ?? "",
        },
      ],
      recentInvocations: largeHistoryPreviews,
      last24hRequests: largeHistory
        .slice(0, 120)
        .reverse()
        .map((record, index, records) => ({
          occurredAt: record.occurredAt,
          status: record.status ?? "completed",
          isSuccess: record.failureClass === "none",
          requestTokens: record.totalTokens ?? 0,
          cumulativeTokens: records
            .slice(0, index + 1)
            .reduce((sum, item) => sum + (item.totalTokens ?? 0), 0),
        })),
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
              <main className="app-shell-boundary space-y-4">
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

const liveSyncSettledStats: PromptCacheConversationsResponse = {
  ...stats,
  conversations: stats.conversations.map((conversation, index) =>
    index !== 0
      ? conversation
      : {
          ...conversation,
          lastActivityAt: "2026-03-27T03:15:19.000Z",
          recentInvocations: [
            buildPreviewFromRecord(
              buildInvocationRecord({
                id: 507,
                invokeId: "invoke-pck-01-live-sync",
                promptCacheKey: CONVERSATION_ONE_KEY,
                occurredAt: "2026-03-27T03:15:19.000Z",
                upstreamAccountId: 11,
                upstreamAccountName: "growth.6vv4@relay.example",
                proxyDisplayName: "tokyo-edge-live-sync",
                totalTokens: 70214,
                inputTokens: 64810,
                cacheInputTokens: 61504,
                outputTokens: 5404,
                reasoningTokens: 924,
                reasoningEffort: "high",
                cost: 0.0468,
                responseContentEncoding: "gzip, br",
                tUpstreamConnectMs: 544,
                tUpstreamTtfbMs: 118,
                tUpstreamStreamMs: 680,
                tTotalMs: 1436,
              }),
            ),
            ...conversation.recentInvocations.slice(0, 4),
          ],
          last24hRequests: [
            ...conversation.last24hRequests.slice(0, -1),
            {
              occurredAt: "2026-03-27T03:15:19.000Z",
              status: "completed",
              isSuccess: true,
              requestTokens: 70214,
              cumulativeTokens: 854323,
            },
          ],
        },
  ),
};

export const LiveSyncSettled: Story = {
  args: {
    stats: liveSyncSettledStats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: [CONVERSATION_ONE_KEY],
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable post-sync state after the Prompt Cache row consumes live `records` SSE updates and converges onto the final persisted invocation.",
      },
    },
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
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await expect(
      await documentScope.findByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/已加载 6 \/ 6 条保留调用记录|Loaded 6 \/ 6 retained record\(s\)/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getAllByTestId("invocation-table-scroll").length,
    ).toBeGreaterThan(0);
  },
};

export const ShortSameDayDrawerOpen: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Conversation history whose retained calls all occur within a short same-day window; the drawer chart should use the first and latest retained call timestamps instead of expanding to the full day.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await expect(
      await documentScope.findByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    const chart = await documentScope.findByTestId("conversation-activity-chart");
    await expect(chart).toHaveAttribute(
      "data-chart-range-start",
      "2026-05-13T23:26:12.000Z",
    );
    await expect(chart).toHaveAttribute(
      "data-chart-range-end",
      "2026-05-13T23:40:47.000Z",
    );
    await waitFor(() => {
      const successBars = Array.from(
        chart.querySelectorAll<SVGGraphicsElement>(
          'path[fill="#22c55e"], rect[fill="#22c55e"]',
        ),
      )
        .map((element) => element.getBBox())
        .filter((box) => box.width > 0 && box.height > 0);
      const failureBars = Array.from(
        chart.querySelectorAll<SVGGraphicsElement>(
          'path[fill="#f87171"], rect[fill="#f87171"]',
        ),
      )
        .map((element) => element.getBBox())
        .filter((box) => box.width > 0 && box.height > 0);
      expect(successBars.length).toBeGreaterThan(0);
      expect(failureBars.length).toBeGreaterThan(0);

      const alignedMiddleBucket = failureBars.some((failureBox) => {
        const failureCenter = failureBox.x + failureBox.width / 2;
        if (failureCenter < 200) return false;
        return successBars.some((successBox) => {
          const successCenter = successBox.x + successBox.width / 2;
          return Math.abs(successCenter - failureCenter) < 1;
        });
      });
      expect(alignedMiddleBucket).toBe(true);
    });
  },
};

export const DrawerBindingControls: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "History drawer with prompt-cache conversation route binding controls visible and preloaded with an upstream-account binding.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(
      await documentScope.findByRole("tab", { name: /设置|Settings/i }),
    );
    await expect(
      await documentScope.findByText(/路由绑定|Route binding/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/当前：账号 growth\.6vv4@relay\.example|Current: account growth\.6vv4@relay\.example/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(
        /加密会话 owner：growth\.6vv4@relay\.example · CIII|Encrypted session owner: growth\.6vv4@relay\.example · CIII/i,
      ),
    ).toBeInTheDocument();
    const bindingKindSelect = documentScope.getByRole("combobox", {
      name: /绑定类型|Binding type/i,
    });
    await expect(bindingKindSelect).toHaveTextContent(/上游账号|Account/i);

    await userEvent.click(bindingKindSelect);
    const bindingOptions = await documentScope.findByRole("listbox");
    await expect(bindingOptions).toHaveTextContent(/清空|Clear/i);
    await expect(bindingOptions).toHaveTextContent(/分组|Group/i);
    await expect(bindingOptions).toHaveTextContent(/上游账号|Account/i);
  },
};

export const DrawerEncryptedOwnerDangerConfirm: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Encrypted-session owner warning flow that uses the project dialog before changing a manually bound route.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    const originalConfirm = window.confirm;
    window.confirm = (message?: string) => {
      throw new Error(`Native confirm should not be used: ${String(message ?? "")}`);
    };

    try {
      await userEvent.click(historyButton);
      await userEvent.click(
        await documentScope.findByRole("tab", { name: /设置|Settings/i }),
      );
      await expect(
        await documentScope.findByText(
          /加密会话 owner：growth\.6vv4@relay\.example · CIII|Encrypted session owner: growth\.6vv4@relay\.example · CIII/i,
        ),
      ).toBeInTheDocument();

      const bindingKindSelect = documentScope.getByRole("combobox", {
        name: /绑定类型|Binding type/i,
      });
      await userEvent.click(bindingKindSelect);
      await userEvent.click(
        await documentScope.findByRole("option", { name: /分组|Group/i }),
      );

      await userEvent.click(
        documentScope.getByRole("button", { name: /保存|Save/i }),
      );

      const confirmDialog = await documentScope.findByRole("alertdialog", {
        name: /要更改加密会话的路由绑定吗|change encrypted-session route binding/i,
      });
      await expect(confirmDialog).toHaveTextContent(
        /growth\.6vv4@relay\.example · CIII/i,
      );
      await expect(confirmDialog).toHaveTextContent(/invalid_encrypted_content/i);
      await userEvent.click(
        within(confirmDialog).getByRole("button", { name: /取消|Cancel/i }),
      );
      await waitFor(() => {
        expect(documentScope.queryByRole("alertdialog")).toBeNull();
      });
      await expect(
        documentScope.getByText(
          /当前：账号 growth\.6vv4@relay\.example|Current: account growth\.6vv4@relay\.example/i,
        ),
      ).toBeInTheDocument();
    } finally {
      window.confirm = originalConfirm;
    }
  },
};

export const DrawerEncryptedOwnerDangerDialogOpen: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable visual state for the encrypted-session owner warning dialog inside the conversation route-binding drawer.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    const originalConfirm = window.confirm;
    window.confirm = (message?: string) => {
      throw new Error(`Native confirm should not be used: ${String(message ?? "")}`);
    };

    try {
      await userEvent.click(historyButton);
      await userEvent.click(
        await documentScope.findByRole("tab", { name: /设置|Settings/i }),
      );
      const bindingKindSelect = documentScope.getByRole("combobox", {
        name: /绑定类型|Binding type/i,
      });
      await userEvent.click(bindingKindSelect);
      await userEvent.click(
        await documentScope.findByRole("option", { name: /分组|Group/i }),
      );

      await userEvent.click(
        documentScope.getByRole("button", { name: /保存|Save/i }),
      );

      const confirmDialog = await documentScope.findByRole("alertdialog", {
        name: /要更改加密会话的路由绑定吗|change encrypted-session route binding/i,
      });
      await expect(confirmDialog).toHaveTextContent(
        /growth\.6vv4@relay\.example · CIII/i,
      );
      await expect(confirmDialog).toHaveTextContent(/invalid_encrypted_content/i);
    } finally {
      window.confirm = originalConfirm;
    }
  },
};

export const DrawerOwnerLockWithoutManualBinding: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Expanded Prompt Cache drawer that shows the encrypted owner hint after manual binding has been cleared.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    bindingByPromptCacheKey.set(
      CONVERSATION_SHORT_KEY,
      buildBindingResponse({
        promptCacheKey: CONVERSATION_SHORT_KEY,
        bindingKind: "none",
        hasEncryptedSessionOwner: true,
        encryptedOwnerAccountId: 21,
        encryptedOwnerAccountName: "growth.6vv4@relay.example",
        encryptedOwnerGroupName: "CIII",
        timeouts: {
          responsesFirstByteTimeoutSecs: 35,
          compactFirstByteTimeoutSecs: 180,
          responsesStreamTimeoutSecs: 210,
          compactStreamTimeoutSecs: 300,
        },
        timeoutFieldSources: {
          responsesFirstByteTimeoutSecs: "conversation",
          compactFirstByteTimeoutSecs: "account",
          responsesStreamTimeoutSecs: "conversation",
          compactStreamTimeoutSecs: "root",
        },
      }),
    );
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(
      await documentScope.findByRole("tab", { name: /设置|Settings/i }),
    );
    await expect(
      await documentScope.findByText(/路由绑定|Route binding/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/当前：无手工绑定|Current: no manual binding/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(
        /清空手工绑定不会清除加密会话 owner 锁|Clearing the manual binding does not remove the encrypted session owner lock/i,
      ),
    ).toBeInTheDocument();
  },
};

export const DrawerBindingAndTimeouts: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Prompt Cache drawer showing both a manual binding and conversation-level timeout overrides, including mixed source badges across conversation, account, and global layers.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    bindingByPromptCacheKey.set(
      CONVERSATION_SHORT_KEY,
      buildBindingResponse({
        promptCacheKey: CONVERSATION_SHORT_KEY,
        bindingKind: "upstreamAccount",
        upstreamAccountId: 21,
        upstreamAccountName: "growth.6vv4@relay.example",
        hasEncryptedSessionOwner: true,
        encryptedOwnerAccountId: 21,
        encryptedOwnerAccountName: "growth.6vv4@relay.example",
        encryptedOwnerGroupName: "CIII",
        timeouts: {
          responsesFirstByteTimeoutSecs: 40,
          compactFirstByteTimeoutSecs: 180,
          responsesStreamTimeoutSecs: 225,
          compactStreamTimeoutSecs: 300,
        },
        timeoutFieldSources: {
          responsesFirstByteTimeoutSecs: "conversation",
          compactFirstByteTimeoutSecs: "account",
          responsesStreamTimeoutSecs: "conversation",
          compactStreamTimeoutSecs: "root",
        },
        allowSwitchUpstream: true,
        fastModeRewriteMode: "force_add",
        imageToolRewriteMode: "force_remove",
        availableModels: ["gpt-5.1-codex-max", "gpt-5.1-codex-mini"],
        forwardProxyKey: "__direct__",
        forwardProxyKeys: ["__direct__", "tokyo-edge-01"],
        updatedAt: "2026-05-13T23:42:00.000Z",
      }),
    );
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(
      await documentScope.findByRole("tab", { name: /设置|Settings/i }),
    );
    await expect(
      await documentScope.findByText(/路由绑定|Route binding/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/当前对话覆盖|Conversation overrides/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/允许换上游|Allow switching upstream/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/强制添加|Force add/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/强制移除|Force remove/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getAllByText(/对话|Conversation/i).length,
    ).toBeGreaterThan(0);
    await expect(
      documentScope.getByText(/gpt-5\.1-codex-max, gpt-5\.1-codex-mini/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/40s/),
    ).toBeInTheDocument();
    await expect(
      documentScope.getAllByText(/对话|Conversation/i).length,
    ).toBeGreaterThan(0);
  },
};

export const LargeHistoryVirtualizedDrawer: Story = {
  args: {
    stats: largeHistoryStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Large retained conversation history with 15,000 total rows; the drawer loads 50 rows first and relies on virtualized visible rows while preserving route-binding controls.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(
      await documentScope.findByRole("tab", { name: /调用|Calls/i }),
    );
    await expect(
      await documentScope.findByText(
        /已加载 50 \/ 15,?000 条保留调用记录|Loaded 50 \/ 15,?000 retained record\(s\)/i,
      ),
    ).toBeInTheDocument();
    expect(
      canvasElement.ownerDocument.body.querySelectorAll("tbody tr").length,
    ).toBeLessThan(90);

    const drawerBody = canvasElement.ownerDocument.body.querySelector(".drawer-body");
    expect(drawerBody).toBeTruthy();
    if (drawerBody instanceof HTMLElement) {
      drawerBody.scrollTop = drawerBody.scrollHeight;
      fireEvent.scroll(drawerBody);
    }

    await expect(
      await documentScope.findByText(/已加载 100 \/ 15,?000 条保留调用记录|Loaded 100 \/ 15,?000 retained record\(s\)/i),
    ).toBeInTheDocument();
  },
};
