import { type ReactNode, useEffect, useRef } from "react";
import type {
  ApiInvocation,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationOperationEvent,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
} from "../../lib/api";

type StoryPromptCacheConversationPreview = PromptCacheConversationInvocationPreview &
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
const CONVERSATION_ROUTING_KEY = "019e239a-038c-7860-a185-routing-story";
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
    codexImagegenRewriteMode: overrides.codexImagegenRewriteMode ?? null,
    availableModels: overrides.availableModels ?? null,
    forwardProxyKey: overrides.forwardProxyKey ?? null,
    forwardProxyKeys:
      overrides.forwardProxyKeys ?? (overrides.forwardProxyKey ? [overrides.forwardProxyKey] : []),
    policyFieldSources: overrides.policyFieldSources,
    stickyRoutes: overrides.stickyRoutes ?? [],
    updatedAt: overrides.updatedAt ?? null,
  };
}

class MockEventSource implements EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;
  static instances = new Set<MockEventSource>();

  readonly url: string;
  readonly withCredentials = false;
  readyState = MockEventSource.CONNECTING;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null = null;
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null;

  #listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();

  constructor(url: string | URL) {
    this.url = typeof url === "string" ? url : url.toString();
    MockEventSource.instances.add(this);
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
    MockEventSource.instances.delete(this);
  }

  static emitMessage(payload: unknown) {
    for (const instance of MockEventSource.instances) {
      if (instance.readyState !== MockEventSource.OPEN) continue;
      instance.#emit("message", new MessageEvent("message", { data: JSON.stringify(payload) }));
    }
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
      stickyRoutes: [
        {
          modelKey: null,
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          createdAt: "2026-05-13T23:40:00.000Z",
          updatedAt: "2026-05-13T23:42:00.000Z",
          lastSeenAt: "2026-05-13T23:47:36.000Z",
        },
        {
          modelKey: "gpt-5.4",
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
          createdAt: "2026-05-13T23:43:00.000Z",
          updatedAt: "2026-05-13T23:47:36.000Z",
          lastSeenAt: "2026-05-13T23:47:39.000Z",
        },
        {
          modelKey: "gpt-5.1-codex-max",
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          createdAt: "2026-05-13T23:44:00.000Z",
          updatedAt: "2026-05-13T23:44:00.000Z",
          lastSeenAt: "2026-05-13T23:45:12.000Z",
        },
      ],
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

const operationEventsByPromptCacheKey = new Map<string, PromptCacheConversationOperationEvent[]>([
  [
    CONVERSATION_SHORT_KEY,
    [
      {
        id: 304,
        promptCacheKey: CONVERSATION_SHORT_KEY,
        action: "stickyMutationSuppressed",
        origin: "systemAuto",
        infoTypes: ["routing"],
        occurredAt: "2026-05-13T23:47:39.000Z",
        headline: "Sticky mutation suppressed",
        changedFields: [],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
        },
        stickyAfter: {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
        },
        invokeId: "invoke-short-33",
        routingContext: {
          reasonCode: "staleConcurrentCompletion",
          routingSource: "freshAssignment",
          httpStatus: null,
          triggerAttemptId: "LATE33",
          causingAttemptId: null,
          causingHttpStatus: null,
        },
        routingScope: {
          kind: "model",
          modelKey: "gpt-5.4",
          requestModel: "gpt-5.4-2026-05-01",
        },
      },
      {
        id: 303,
        promptCacheKey: CONVERSATION_SHORT_KEY,
        action: "stickyTargetChanged",
        origin: "systemAuto",
        infoTypes: ["routing"],
        occurredAt: "2026-05-13T23:47:36.000Z",
        headline: "Sticky target changed",
        changedFields: ["stickyTarget"],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: null,
        stickyAfter: {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
        },
        invokeId: "invoke-short-32",
        routingContext: {
          reasonCode: "freshAssignmentAfterFailure",
          routingSource: "freshAssignment",
          routingSelectionAudit: {
            selectedAccountId: 22,
            selectedAccountName: "mia.7rmmq@support.example",
            eligibleCandidateCount: 1,
            winnerReasonCode: "onlyEligibleCandidate",
            comparedAccountId: null,
            comparedAccountName: null,
            selectedScore: {
              eligibility: "assignable",
              routeBindingFailurePenalty: 0,
              modelRoutePenalty: 0,
              modelRoutePenaltyCode: "normal",
              routingPriorityRank: 0,
              capacityLane: "primary",
              dispatchState: "readyOnOwnedNode",
              secondaryResetProximitySecs: null,
              primaryResetProximitySecs: null,
              scarcityScore: "0.000000",
              effectiveLoad: 0,
              lastSelectedAt: null,
            },
            comparedScore: null,
            excludedCandidates: [
              {
                accountId: 21,
                accountName: "growth.6vv4@relay.example",
                reasonCode: "modelNotAllowed",
              },
            ],
          },
          httpStatus: null,
          triggerAttemptId: "SUCCESS32",
          causingAttemptId: "FAILED31",
          causingHttpStatus: 502,
        },
        routingScope: {
          kind: "model",
          modelKey: "gpt-5.4",
          requestModel: "gpt-5.4-2026-05-01",
        },
      },
      {
        id: 302,
        promptCacheKey: CONVERSATION_SHORT_KEY,
        action: "stickyTargetCleared",
        origin: "dashboardBulk",
        infoTypes: ["routing"],
        occurredAt: "2026-05-13T23:46:12.000Z",
        headline: "Sticky target cleared",
        changedFields: ["stickyTarget"],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: {
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
        },
        stickyAfter: null,
        invokeId: null,
        routingScope: { kind: "all", modelKey: null, requestModel: null },
      },
      {
        id: 301,
        promptCacheKey: CONVERSATION_SHORT_KEY,
        action: "affinityReset",
        origin: "dashboardBulk",
        infoTypes: ["routing"],
        occurredAt: "2026-05-13T23:46:12.000Z",
        headline: "Affinity reset",
        changedFields: ["bindingKind"],
        bindingBefore: {
          bindingKind: "upstreamAccount",
          groupName: null,
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
        },
        bindingAfter: {
          bindingKind: "none",
          groupName: null,
          upstreamAccountId: null,
          upstreamAccountName: null,
        },
        stickyBefore: {
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
        },
        stickyAfter: null,
        invokeId: null,
        routingScope: { kind: "all", modelKey: null, requestModel: null },
        stickyTransitions: [
          {
            modelKey: null,
            before: {
              upstreamAccountId: 21,
              upstreamAccountName: "growth.6vv4@relay.example",
            },
            after: null,
          },
          {
            modelKey: "gpt-5.1-codex-max",
            before: {
              upstreamAccountId: 21,
              upstreamAccountName: "growth.6vv4@relay.example",
            },
            after: null,
          },
        ],
      },
    ],
  ],
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

function buildPreviewFromRecord(record: ApiInvocation): StoryPromptCacheConversationPreview {
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

const conversationOnePreviews = conversationOneHistory.slice(0, 5).map(buildPreviewFromRecord);
const conversationTwoPreviews = conversationTwoHistory.slice(0, 5).map(buildPreviewFromRecord);
const shortSameDayPreviews = shortSameDayHistory.slice(0, 4).map(buildPreviewFromRecord);
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
  [CONVERSATION_ONE_KEY, conversationOneHistory],
  [CONVERSATION_TWO_KEY, conversationTwoHistory],
  [CONVERSATION_SHORT_KEY, shortSameDayHistory],
  [
    CONVERSATION_ROUTING_KEY,
    shortSameDayHistory.map((record) => ({ ...record, promptCacheKey: CONVERSATION_ROUTING_KEY })),
  ],
  [CONVERSATION_LARGE_HISTORY_KEY, largeHistory],
]);

const queuedLargeHistoryRecord = buildInvocationRecord({
  id: 30_001,
  invokeId: "invoke-large-history-live-queued",
  promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY,
  occurredAt: "2026-05-28T04:10:30.000Z",
  upstreamAccountId: 11,
  upstreamAccountName: "growth.6vv4@relay.example",
  proxyDisplayName: "tokyo-edge-large-01",
  model: "gpt-5.6-sol",
  totalTokens: 183_400,
  inputTokens: 176_000,
  cacheInputTokens: 169_400,
  outputTokens: 7_400,
  reasoningTokens: 512,
  reasoningEffort: "high",
  cost: 0.121,
  responseContentEncoding: "gzip",
  tTotalMs: 8_420,
});

function buildInvocationSummary(records: ApiInvocation[]) {
  const totalCost = records.reduce((sum, record) => sum + (record.cost ?? 0), 0);
  const totalTokens = records.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0);
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
      ? durationSamples.reduce((sum, value) => sum + value, 0) / durationSamples.length
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
      cacheInputTokens: records.reduce((sum, record) => sum + (record.cacheInputTokens ?? 0), 0),
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
      clientFailureCount: failedRecords.filter((record) => record.failureClass === "client_failure")
        .length,
      clientAbortCount: failedRecords.filter((record) => record.failureClass === "client_abort")
        .length,
      actionableFailureCount: failedRecords.filter((record) => record.isActionable).length,
    },
  };
}

function createDefaultStoryBinding(promptCacheKey: string) {
  return buildBindingResponse({
    promptCacheKey,
    bindingKind: "none",
    hasEncryptedSessionOwner: false,
    stickyRoutes: [],
  });
}

function buildStoryBindingPatch(
  current: PromptCacheConversationBindingResponse,
  payload: Record<string, unknown>,
) {
  const timeoutPatch = (payload.timeouts as Record<string, unknown> | undefined) ?? {};
  const timeoutKeys = [
    "responsesFirstByteTimeoutSecs",
    "compactFirstByteTimeoutSecs",
    "responsesStreamTimeoutSecs",
    "compactStreamTimeoutSecs",
  ] as const;
  const timeoutDefaults = {
    responsesFirstByteTimeoutSecs: "account",
    compactFirstByteTimeoutSecs: "account",
    responsesStreamTimeoutSecs: "account",
    compactStreamTimeoutSecs: "root",
  } as const;
  const timeouts = {
    ...current.timeouts,
    ...Object.fromEntries(
      timeoutKeys.map((key) => [
        key,
        timeoutPatch[key] == null ? current.timeouts[key] : timeoutPatch[key],
      ]),
    ),
  };
  const timeoutFieldSources = {
    ...current.timeoutFieldSources,
    ...Object.fromEntries(
      timeoutKeys.map((key) => [
        key,
        timeoutPatch[key] === null
          ? timeoutDefaults[key]
          : timeoutPatch[key] != null
            ? "conversation"
            : current.timeoutFieldSources[key],
      ]),
    ),
  };
  const currentPolicySources = current.policyFieldSources ?? {
    allowSwitchUpstream: "account",
    fastModeRewriteMode: "account",
    imageToolRewriteMode: "account",
    codexImagegenRewriteMode: "account",
    availableModels: "account",
    forwardProxyKey: "account",
  };
  const forwardProxyKeys = Array.isArray(payload.forwardProxyKeys)
    ? (payload.forwardProxyKeys as string[])
    : "forwardProxyKey" in payload && payload.forwardProxyKey
      ? [String(payload.forwardProxyKey)]
      : (current.forwardProxyKeys ?? []);
  const policyKeys = [
    "allowSwitchUpstream",
    "fastModeRewriteMode",
    "imageToolRewriteMode",
    "codexImagegenRewriteMode",
    "availableModels",
  ] as const;
  const policyOverrides = Object.fromEntries(
    policyKeys.map((key) => [key, key in payload ? payload[key] : current[key]]),
  ) as Partial<PromptCacheConversationBindingResponse>;
  const policyFieldSources = {
    ...currentPolicySources,
    ...Object.fromEntries(
      policyKeys.filter((key) => key in payload).map((key) => [key, "conversation"]),
    ),
    ...(Array.isArray(payload.forwardProxyKeys) || "forwardProxyKey" in payload
      ? { forwardProxyKey: "conversation" }
      : {}),
  };
  return {
    timeoutPatch,
    timeouts,
    timeoutFieldSources,
    policyOverrides,
    forwardProxyKey: forwardProxyKeys[0] ?? null,
    forwardProxyKeys,
    policyFieldSources,
  };
}

function updateStoryBinding(
  promptCacheKey: string,
  payload: Record<string, unknown>,
): PromptCacheConversationBindingResponse {
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
  const patch = buildStoryBindingPatch(current, payload);
  const bindingKind = (payload.bindingKind ??
    "none") as PromptCacheConversationBindingResponse["bindingKind"];
  const response = buildBindingResponse({
    promptCacheKey,
    bindingKind,
    groupName: bindingKind === "group" ? String(payload.groupName ?? "") : null,
    upstreamAccountId: bindingKind === "upstreamAccount" ? Number(payload.upstreamAccountId) : null,
    upstreamAccountName:
      bindingKind === "upstreamAccount"
        ? (accountSummaries.find((account) => account.id === Number(payload.upstreamAccountId))
            ?.displayName ?? null)
        : null,
    hasEncryptedSessionOwner: true,
    encryptedOwnerAccountId: 21,
    encryptedOwnerAccountName: "growth.6vv4@relay.example",
    encryptedOwnerGroupName: "CIII",
    timeouts: patch.timeouts,
    timeoutFieldSources: patch.timeoutFieldSources,
    ...patch.policyOverrides,
    forwardProxyKey: patch.forwardProxyKey,
    forwardProxyKeys: patch.forwardProxyKeys,
    policyFieldSources: patch.policyFieldSources,
    updatedAt:
      bindingKind === "none" &&
      !Object.values(patch.timeoutPatch).some((value) => value !== undefined)
        ? null
        : new Date().toISOString(),
  });
  bindingByPromptCacheKey.set(promptCacheKey, response);
  return response;
}

function handleStoryBindingRequest(
  parsedUrl: URL,
  method: string,
  init?: RequestInit,
): Response | null {
  const resetMatch = parsedUrl.pathname.match(
    /^\/api\/stats\/prompt-cache-conversation-bindings\/reset-affinity\/(.+)$/,
  );
  if (resetMatch && method === "POST") {
    const promptCacheKey = decodeURIComponent(resetMatch[1] ?? "");
    const response = buildBindingResponse({
      ...(bindingByPromptCacheKey.get(promptCacheKey) ?? {
        promptCacheKey,
        bindingKind: "none" as const,
      }),
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      stickyRoutes: [],
      updatedAt: new Date().toISOString(),
    });
    bindingByPromptCacheKey.set(promptCacheKey, response);
    return jsonResponse(response);
  }
  const bindingMatch = parsedUrl.pathname.match(
    /^\/api\/stats\/prompt-cache-conversation-bindings\/(.+)$/,
  );
  if (!bindingMatch) return null;
  const promptCacheKey = decodeURIComponent(bindingMatch[1] ?? "");
  if (method === "GET") {
    return jsonResponse(
      bindingByPromptCacheKey.get(promptCacheKey) ?? createDefaultStoryBinding(promptCacheKey),
    );
  }
  if (method !== "PATCH") return null;
  const payload = init?.body ? JSON.parse(String(init.body)) : {};
  return jsonResponse(updateStoryBinding(promptCacheKey, payload));
}

function handleStoryOperationEvents(parsedUrl: URL): Response | null {
  const match = parsedUrl.pathname.match(
    /^\/api\/stats\/prompt-cache-conversation-binding-events\/(.+)$/,
  );
  if (!match) return null;
  const promptCacheKey = decodeURIComponent(match[1] ?? "");
  const infoType = parsedUrl.searchParams.get("infoType");
  const routingScope = parsedUrl.searchParams.get("routingScope");
  const routingModel = parsedUrl.searchParams.get("routingModel");
  const page = Number(parsedUrl.searchParams.get("page") ?? "1");
  const pageSize = Number(parsedUrl.searchParams.get("pageSize") ?? "20");
  const allItems = operationEventsByPromptCacheKey.get(promptCacheKey) ?? [];
  const filteredItems = allItems.filter((item) => {
    if (infoType && !item.infoTypes.includes(infoType as never)) return false;
    if (routingScope && item.routingScope?.kind !== routingScope) return false;
    return !routingModel || item.routingScope?.modelKey === routingModel;
  });
  const start = Math.max(0, (page - 1) * pageSize);
  return jsonResponse({
    items: filteredItems.slice(start, start + pageSize),
    total: filteredItems.length,
    page,
    pageSize,
    routingModelFacets: Array.from(
      new Set(
        allItems.flatMap((item) =>
          item.routingScope?.kind === "model" && item.routingScope.modelKey
            ? [item.routingScope.modelKey]
            : [],
        ),
      ),
    ).sort(),
  });
}

function createPromptCacheStoryFetchHandler(
  originalFetch: typeof window.fetch,
): typeof window.fetch {
  return async (input, init) => {
    const method = (
      init?.method || (input instanceof Request ? input.method : "GET")
    ).toUpperCase();
    const inputUrl =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const parsedUrl = new URL(inputUrl, window.location.origin);
    const operationResponse = method === "GET" ? handleStoryOperationEvents(parsedUrl) : null;
    if (operationResponse) return operationResponse;
    const bindingResponse = handleStoryBindingRequest(parsedUrl, method, init);
    if (bindingResponse) return bindingResponse;
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
    const accountMatch = parsedUrl.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);
    if (accountMatch && method === "GET") {
      const detail = accountDetails.get(Number(accountMatch[1]));
      return detail ? jsonResponse(detail) : jsonResponse({ message: "Not found" }, 404);
    }
    if (parsedUrl.pathname === "/api/invocations/summary" && method === "GET") {
      const promptCacheKey = parsedUrl.searchParams.get("promptCacheKey");
      return jsonResponse(
        buildInvocationSummary(
          promptCacheKey ? (historyRecordsByKey.get(promptCacheKey) ?? []) : [],
        ),
      );
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
        const snapshotId = Number(parsedUrl.searchParams.get("snapshotId") ?? "8401");
        const records = historyRecordsByKey.get(promptCacheKey) ?? [];
        const start = Math.max(0, (page - 1) * pageSize);
        return jsonResponse({
          snapshotId,
          total: records.length,
          page,
          pageSize,
          records: records.slice(start, start + pageSize),
        });
      }
    }
    return originalFetch(input, init);
  };
}

function StorybookPromptCacheAccountMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);
    originalEventSourceRef.current = window.EventSource;
    window.fetch = createPromptCacheStoryFetchHandler(
      originalFetchRef.current as typeof window.fetch,
    );
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

export type { StoryPromptCacheConversationPreview };
export {
  accountDetails,
  accountSummaries,
  bindingByPromptCacheKey,
  buildAccountDetail,
  buildAccountSummary,
  buildBindingResponse,
  buildInvocationRecord,
  buildInvocationSummary,
  buildPreviewFromRecord,
  CONVERSATION_LARGE_HISTORY_KEY,
  CONVERSATION_ONE_KEY,
  CONVERSATION_ROUTING_KEY,
  CONVERSATION_SHORT_KEY,
  CONVERSATION_TWO_KEY,
  conversationOneHistory,
  conversationOnePreviews,
  conversationTwoHistory,
  conversationTwoPreviews,
  historyRecordsByKey,
  jsonResponse,
  largeHistory,
  largeHistoryPreviews,
  MockEventSource,
  operationEventsByPromptCacheKey,
  queuedLargeHistoryRecord,
  StorybookPromptCacheAccountMock,
  shortSameDayEndMs,
  shortSameDayHistory,
  shortSameDayOffsetsMs,
  shortSameDayPreviews,
  shortSameDayStartMs,
  stats,
  storyForwardProxyNodes,
};
