import { describe, expect, it } from "vitest";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationRequestPoint,
  PromptCacheConversationsResponse,
} from "./api";
import {
  buildInvocationFromPromptCachePreview,
  buildPromptCachePreviewFromInvocation,
  mergePromptCacheConversationHistory,
  mergePromptCacheConversationsResponse,
  type PromptCacheConversationHistoryByKey,
  reconcilePromptCacheLiveRecordMap,
} from "./promptCacheLive";

function createRequestPoint(
  overrides: Partial<PromptCacheConversationRequestPoint> & {
    occurredAt: string;
    requestTokens: number;
    cumulativeTokens: number;
  },
): PromptCacheConversationRequestPoint {
  return {
    occurredAt: overrides.occurredAt,
    status: overrides.status ?? "completed",
    isSuccess: overrides.isSuccess ?? true,
    outcome:
      overrides.outcome ??
      ((overrides.status ?? "completed") === "running"
        ? "in_flight"
        : (overrides.status ?? "completed") === "unknown"
          ? "neutral"
          : (overrides.isSuccess ?? true)
            ? "success"
            : "failure"),
    requestTokens: overrides.requestTokens,
    cumulativeTokens: overrides.cumulativeTokens,
  };
}

function createConversation(
  promptCacheKey: string,
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: overrides.requestCount ?? 1,
    totalTokens: overrides.totalTokens ?? 100,
    totalCost: overrides.totalCost ?? 0.01,
    createdAt: overrides.createdAt ?? "2026-03-10T01:00:00Z",
    lastActivityAt: overrides.lastActivityAt ?? "2026-03-10T02:00:00Z",
    conversationId: overrides.conversationId,
    successCount: overrides.successCount,
    failureCount: overrides.failureCount,
    inputTokens: overrides.inputTokens,
    outputTokens: overrides.outputTokens,
    cacheInputTokens: overrides.cacheInputTokens,
    reportedCacheWriteTokens: overrides.reportedCacheWriteTokens,
    reasoningTokens: overrides.reasoningTokens,
    costInput: overrides.costInput,
    costCacheWrite: overrides.costCacheWrite,
    costCacheRead: overrides.costCacheRead,
    costOutput: overrides.costOutput,
    costReasoning: overrides.costReasoning,
    firstInvocationAt: overrides.firstInvocationAt,
    lastInvocationAt: overrides.lastInvocationAt,
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations: overrides.recentInvocations ?? [],
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

function createResponse(
  conversations: PromptCacheConversation[],
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-03-09T00:00:00Z",
    rangeEnd: "2026-03-10T03:00:00Z",
    selectionMode: "count",
    selectedLimit: 2,
    selectedActivityHours: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
  };
}

function createLiveRecord(
  overrides: Partial<ApiInvocation> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    promptCacheKey: string;
  },
): ApiInvocation {
  const {
    id,
    invokeId,
    occurredAt,
    promptCacheKey,
    createdAt,
    status,
    totalTokens,
    cost,
    ...rest
  } = overrides;
  return {
    id,
    invokeId,
    occurredAt,
    createdAt: createdAt ?? occurredAt,
    promptCacheKey,
    status: status ?? "completed",
    totalTokens: totalTokens ?? 100,
    cost: cost ?? 0.01,
    ...rest,
  };
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
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    proxyDisplayName: overrides.proxyDisplayName ?? null,
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    upstreamAccountName: overrides.upstreamAccountName ?? null,
    endpoint: overrides.endpoint ?? "/v1/responses",
    requestedServiceTier: overrides.requestedServiceTier,
  };
}

describe("mergePromptCacheConversationsResponse", () => {
  it("preserves downstream-facing error metadata across prompt-cache preview adapters", () => {
    const record = createLiveRecord({
      id: 250,
      invokeId: "invoke-downstream-preview",
      occurredAt: "2026-03-10T02:10:00Z",
      promptCacheKey: "pck-downstream-preview",
      status: "failed",
      failureClass: "client_abort",
      failureKind: "downstream_closed",
      downstreamStatusCode: 200,
      firstTokenMs: 742,
      downstreamErrorMessage:
        "[downstream_closed] downstream closed while streaming upstream response",
    });

    const preview = buildPromptCachePreviewFromInvocation(record);
    const rebuilt = buildInvocationFromPromptCachePreview(preview);

    expect(preview.downstreamStatusCode).toBe(200);
    expect(preview.downstreamErrorMessage).toContain("downstream closed");
    expect(rebuilt.downstreamStatusCode).toBe(200);
    expect(preview.firstTokenMs).toBe(742);
    expect(rebuilt.firstTokenMs).toBe(742);
    expect(rebuilt.downstreamErrorMessage).toContain("downstream closed");
  });

  it("uses persisted first-invocation time for count-mode ordering", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([
        createConversation("pck-first-old", {
          createdAt: "2026-03-10T02:00:00Z",
          firstInvocationAt: "2026-03-10T01:00:00Z",
        }),
        createConversation("pck-first-new", {
          createdAt: "2026-03-10T01:00:00Z",
          firstInvocationAt: "2026-03-10T01:30:00Z",
        }),
      ]),
      {},
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations.map((conversation) => conversation.promptCacheKey)).toEqual([
      "pck-first-new",
      "pck-first-old",
    ]);
  });

  it("hydrates detailed metrics for an unseen live conversation", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([]),
      {
        "pck-live-stats": [
          createLiveRecord({
            id: 401,
            invokeId: "invoke-live-success",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live-stats",
            status: "completed",
            inputTokens: 120,
            outputTokens: 30,
            cacheInputTokens: 20,
            reportedCacheWriteTokens: 10,
            reasoningTokens: 4,
            costInput: 0.01,
            costCacheWrite: 0.02,
            costCacheRead: 0.03,
            costOutput: 0.04,
            costReasoning: 0.05,
          }),
          createLiveRecord({
            id: 402,
            invokeId: "invoke-live-failure",
            occurredAt: "2026-03-10T02:40:00Z",
            promptCacheKey: "pck-live-stats",
            status: "failed",
            failureClass: "service_failure",
            inputTokens: 80,
            outputTokens: 10,
            cacheInputTokens: 5,
            reportedCacheWriteTokens: 2,
            reasoningTokens: 1,
            costInput: 0.11,
            costCacheWrite: 0.12,
            costCacheRead: 0.13,
            costOutput: 0.14,
            costReasoning: 0.15,
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    const conversation = merged?.conversations[0];
    expect(conversation?.successCount).toBeUndefined();
    expect(conversation?.failureCount).toBeUndefined();
    expect(conversation?.inputTokens).toBe(200);
    expect(conversation?.outputTokens).toBe(40);
    expect(conversation?.reportedCacheWriteTokens).toBe(12);
    expect(conversation?.costReasoning).toBeCloseTo(0.2);
    expect(conversation?.firstInvocationAt).toBe("2026-03-10T02:30:00Z");
    expect(conversation?.lastInvocationAt).toBe("2026-03-10T02:40:00Z");
  });

  it("adds deduplicated live aggregates to an existing authoritative conversation", () => {
    const authoritativeRecord = createLiveRecord({
      id: 404,
      invokeId: "invoke-authoritative-existing",
      occurredAt: "2026-03-10T02:10:00Z",
      promptCacheKey: "pck-existing-stats",
      status: "completed",
      upstreamAccountId: 7,
      upstreamAccountName: "primary",
    });
    const liveRecord = createLiveRecord({
      id: 405,
      invokeId: "invoke-live-existing",
      occurredAt: "2026-03-10T02:20:00Z",
      promptCacheKey: "pck-existing-stats",
      status: "failed",
      failureClass: "service_failure",
      upstreamAccountId: 7,
      upstreamAccountName: "primary",
      inputTokens: 40,
      outputTokens: 20,
      cacheInputTokens: 10,
      reportedCacheWriteTokens: 4,
      reasoningTokens: 2,
      costInput: 0.01,
      costCacheWrite: 0.02,
      costCacheRead: 0.03,
      costOutput: 0.04,
      costReasoning: 0.05,
    });
    const merged = mergePromptCacheConversationsResponse(
      createResponse([
        createConversation("pck-existing-stats", {
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          successCount: 1,
          failureCount: 0,
          inputTokens: 80,
          outputTokens: 30,
          cacheInputTokens: 20,
          reportedCacheWriteTokens: 5,
          reasoningTokens: 3,
          costInput: 0.1,
          costCacheWrite: 0.2,
          costCacheRead: 0.3,
          costOutput: 0.4,
          costReasoning: 0.5,
          firstInvocationAt: "2026-03-10T02:00:00Z",
          lastInvocationAt: "2026-03-10T02:10:00Z",
          recentInvocations: [buildPromptCachePreviewFromInvocation(authoritativeRecord)],
          upstreamAccounts: [
            {
              upstreamAccountId: 7,
              upstreamAccountName: "primary",
              requestCount: 1,
              totalTokens: 100,
              totalCost: 0.01,
              lastActivityAt: "2026-03-10T02:10:00Z",
            },
          ],
        }),
      ]),
      {
        "pck-existing-stats": [authoritativeRecord, liveRecord],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    const conversation = merged?.conversations[0];
    expect(conversation?.requestCount).toBe(2);
    expect(conversation?.totalTokens).toBe(200);
    expect(conversation?.totalCost).toBeCloseTo(0.02);
    expect(conversation?.successCount).toBe(1);
    expect(conversation?.failureCount).toBe(1);
    expect(conversation?.inputTokens).toBe(120);
    expect(conversation?.outputTokens).toBe(50);
    expect(conversation?.cacheInputTokens).toBe(30);
    expect(conversation?.reportedCacheWriteTokens).toBe(9);
    expect(conversation?.reasoningTokens).toBe(5);
    expect(conversation?.costReasoning).toBeCloseTo(0.55);
    expect(conversation?.firstInvocationAt).toBe("2026-03-10T02:00:00Z");
    expect(conversation?.lastInvocationAt).toBe("2026-03-10T02:20:00Z");
    expect(conversation?.upstreamAccounts).toEqual([
      {
        upstreamAccountId: 7,
        upstreamAccountName: "primary",
        requestCount: 2,
        totalTokens: 200,
        totalCost: 0.02,
        lastActivityAt: "2026-03-10T02:20:00Z",
      },
    ]);
  });

  it("leaves delayed outcome counts absent for an optimistic in-flight conversation", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([]),
      {
        "pck-live-running": [
          createLiveRecord({
            id: 406,
            invokeId: "invoke-live-running",
            occurredAt: "2026-03-10T02:50:00Z",
            promptCacheKey: "pck-live-running",
            status: "running",
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations[0]?.successCount).toBeUndefined();
    expect(merged?.conversations[0]?.failureCount).toBeUndefined();
  });

  it("preserves the durable first-invocation time for a retained live conversation", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([]),
      {
        "pck-retained": [
          createLiveRecord({
            id: 403,
            invokeId: "invoke-retained",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-retained",
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
      {
        "pck-retained": {
          createdAt: "2026-03-10T02:00:00Z",
          lastActivityAt: "2026-03-10T02:05:00Z",
          firstInvocationAt: "2026-03-09T01:00:00Z",
          lastInvocationAt: "2026-03-10T02:05:00Z",
        },
      },
    );

    expect(merged?.conversations[0]?.createdAt).toBe("2026-03-09T01:00:00Z");
    expect(merged?.conversations[0]?.firstInvocationAt).toBe("2026-03-09T01:00:00Z");
  });

  it("lets unseen live conversations displace older rows in count-capped mode", () => {
    const base = createResponse([
      createConversation("pck-newest", {
        createdAt: "2026-03-10T02:00:00Z",
        lastActivityAt: "2026-03-10T02:00:00Z",
      }),
    ]);

    const merged = mergePromptCacheConversationsResponse(
      base,
      {
        "pck-live-new": [
          createLiveRecord({
            id: 301,
            invokeId: "invoke-live-new",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live-new",
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toEqual([
      "pck-live-new",
      "pck-newest",
    ]);
  });

  it("keeps full capped rows stable until an unseen live key gets authoritative history", () => {
    const base = createResponse([
      createConversation("pck-newest", {
        createdAt: "2026-03-10T02:00:00Z",
        lastActivityAt: "2026-03-10T02:00:00Z",
      }),
      createConversation("pck-older", {
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T01:00:00Z",
      }),
    ]);

    const merged = mergePromptCacheConversationsResponse(
      base,
      {
        "pck-live-unknown": [
          createLiveRecord({
            id: 302,
            invokeId: "invoke-live-unknown",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live-unknown",
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toEqual([
      "pck-newest",
      "pck-older",
    ]);
  });

  it("uses known conversation history when an old key reappears outside the current snapshot", () => {
    const base = createResponse([
      createConversation("pck-newest", {
        createdAt: "2026-03-10T02:00:00Z",
        lastActivityAt: "2026-03-10T02:00:00Z",
      }),
      createConversation("pck-older", {
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T01:00:00Z",
      }),
    ]);

    const merged = mergePromptCacheConversationsResponse(
      base,
      {
        "pck-live-old": [
          createLiveRecord({
            id: 303,
            invokeId: "invoke-live-old",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live-old",
          }),
        ],
      },
      { mode: "count", limit: 3 },
      Date.parse("2026-03-10T03:00:00Z"),
      {
        "pck-live-old": {
          createdAt: "2026-03-01T00:00:00Z",
          lastActivityAt: "2026-03-01T00:00:00Z",
        },
      },
    );

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toEqual([
      "pck-newest",
      "pck-older",
      "pck-live-old",
    ]);
  });

  it("keeps a reactivated known key visible when the activity-window working set is full", () => {
    const now = Date.parse("2026-03-10T03:00:00Z");
    const base = createResponse(
      Array.from({ length: 50 }, (_, index) =>
        createConversation(`pck-visible-${index}`, {
          createdAt: new Date(Date.parse("2026-03-10T00:00:00Z") + index * 60_000).toISOString(),
          lastActivityAt: new Date(
            Date.parse("2026-03-10T02:05:00Z") + index * 20_000,
          ).toISOString(),
          recentInvocations: [
            createPreview({
              id: 4000 + index,
              invokeId: `invoke-visible-${index}`,
              occurredAt: new Date(
                Date.parse("2026-03-10T02:05:00Z") + index * 20_000,
              ).toISOString(),
              status: "completed",
            }),
          ],
        }),
      ),
    );

    const merged = mergePromptCacheConversationsResponse(
      base,
      {
        "pck-reactivated": [
          createLiveRecord({
            id: 905,
            invokeId: "invoke-reactivated",
            occurredAt: "2026-03-10T02:59:30Z",
            promptCacheKey: "pck-reactivated",
            status: "running",
          }),
        ],
      },
      { mode: "activityWindow", activityMinutes: 30 },
      now,
      {
        "pck-reactivated": {
          createdAt: "2026-03-10T00:10:00Z",
          lastActivityAt: "2026-03-10T01:40:00Z",
        },
      },
    );

    expect(merged?.conversations).toHaveLength(50);
    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain("pck-reactivated");
    expect(merged?.conversations.map((item) => item.promptCacheKey)).not.toContain("pck-visible-0");
  });

  it("dedupes last24h request points when the same invocation is already present after resync", () => {
    const authoritativeRecord = createLiveRecord({
      id: 901,
      invokeId: "invoke-live-01",
      occurredAt: "2026-03-10T02:30:00Z",
      promptCacheKey: "pck-live",
      totalTokens: 182491,
    });
    const base = createResponse([
      createConversation("pck-live", {
        recentInvocations: [buildPromptCachePreviewFromInvocation(authoritativeRecord)],
        last24hRequests: [
          createRequestPoint({
            occurredAt: "2026-03-10T02:30:00Z",
            requestTokens: 182491,
            cumulativeTokens: 182491,
          }),
        ],
      }),
    ]);

    const merged = mergePromptCacheConversationsResponse(
      base,
      {
        "pck-live": [authoritativeRecord],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations[0]?.last24hRequests).toEqual([
      createRequestPoint({
        occurredAt: "2026-03-10T02:30:00Z",
        requestTokens: 182491,
        cumulativeTokens: 182491,
      }),
    ]);
  });

  it("keeps a distinct live request point even when an authoritative point shares the same shape", () => {
    const authoritativeRecord = createLiveRecord({
      id: 1001,
      invokeId: "invoke-authoritative",
      occurredAt: "2026-03-10T02:30:00Z",
      promptCacheKey: "pck-live-points",
      totalTokens: 182491,
    });
    const merged = mergePromptCacheConversationsResponse(
      createResponse([
        createConversation("pck-live-points", {
          recentInvocations: [buildPromptCachePreviewFromInvocation(authoritativeRecord)],
          last24hRequests: [
            createRequestPoint({
              occurredAt: "2026-03-10T02:30:00Z",
              requestTokens: 182491,
              cumulativeTokens: 182491,
            }),
          ],
        }),
      ]),
      {
        "pck-live-points": [
          createLiveRecord({
            id: 1002,
            invokeId: "invoke-live-b",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live-points",
            totalTokens: 182491,
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations[0]?.last24hRequests).toEqual([
      createRequestPoint({
        occurredAt: "2026-03-10T02:30:00Z",
        requestTokens: 182491,
        cumulativeTokens: 182491,
      }),
      createRequestPoint({
        occurredAt: "2026-03-10T02:30:00Z",
        requestTokens: 182491,
        cumulativeTokens: 364982,
      }),
    ]);
  });

  it("marks running live request points as in-flight instead of successful", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([createConversation("pck-running")]),
      {
        "pck-running": [
          createLiveRecord({
            id: 1101,
            invokeId: "invoke-running",
            occurredAt: "2026-03-10T02:45:00Z",
            promptCacheKey: "pck-running",
            status: "running",
            totalTokens: 2400,
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations[0]?.last24hRequests).toEqual([
      createRequestPoint({
        occurredAt: "2026-03-10T02:45:00Z",
        status: "running",
        isSuccess: false,
        outcome: "in_flight",
        requestTokens: 2400,
        cumulativeTokens: 2400,
      }),
    ]);
  });

  it("keeps blank-status live request points neutral instead of treating them as failures", () => {
    const merged = mergePromptCacheConversationsResponse(
      createResponse([createConversation("pck-neutral")]),
      {
        "pck-neutral": [
          createLiveRecord({
            id: 1102,
            invokeId: "invoke-neutral",
            occurredAt: "2026-03-10T02:46:00Z",
            promptCacheKey: "pck-neutral",
            status: "",
            failureClass: "none",
            totalTokens: 32,
          }),
        ],
      },
      { mode: "count", limit: 2 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations[0]?.last24hRequests).toEqual([
      createRequestPoint({
        occurredAt: "2026-03-10T02:46:00Z",
        status: "unknown",
        isSuccess: false,
        outcome: "neutral",
        requestTokens: 32,
        cumulativeTokens: 32,
      }),
    ]);
  });

  it("keeps running-only conversations visible in the precise 5-minute dashboard window", () => {
    const merged = mergePromptCacheConversationsResponse(
      {
        rangeStart: "2026-03-10T02:55:00Z",
        rangeEnd: "2026-03-10T03:00:00Z",
        selectionMode: "activityWindow",
        selectedLimit: null,
        selectedActivityHours: null,
        selectedActivityMinutes: 5,
        implicitFilter: { kind: null, filteredCount: 0 },
        conversations: [
          createConversation("pck-terminal", {
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1300,
                  invokeId: "invoke-terminal",
                  occurredAt: "2026-03-10T02:58:00Z",
                  promptCacheKey: "pck-terminal",
                  status: "completed",
                }),
              ),
            ],
          }),
        ],
      },
      {
        "pck-running-old": [
          createLiveRecord({
            id: 1301,
            invokeId: "invoke-running-old",
            occurredAt: "2026-03-10T02:40:00Z",
            promptCacheKey: "pck-running-old",
            status: "running",
          }),
        ],
      },
      { mode: "activityWindow", activityMinutes: 5 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain("pck-running-old");
    expect(
      merged?.conversations.find((item) => item.promptCacheKey === "pck-running-old")
        ?.recentInvocations[0]?.status,
    ).toBe("running");
  });

  it("sorts the precise 5-minute dashboard window by conversation created time descending", () => {
    const merged = mergePromptCacheConversationsResponse(
      {
        rangeStart: "2026-03-10T02:55:00Z",
        rangeEnd: "2026-03-10T03:00:00Z",
        selectionMode: "activityWindow",
        selectedLimit: null,
        selectedActivityHours: null,
        selectedActivityMinutes: 5,
        implicitFilter: { kind: null, filteredCount: 0 },
        conversations: [
          createConversation("pck-terminal-early", {
            createdAt: "2026-03-10T02:56:00Z",
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1401,
                  invokeId: "invoke-terminal-early",
                  occurredAt: "2026-03-10T02:57:00Z",
                  promptCacheKey: "pck-terminal-early",
                  status: "completed",
                }),
              ),
            ],
          }),
          createConversation("pck-running-only", {
            createdAt: "2026-03-10T02:40:00Z",
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1402,
                  invokeId: "invoke-running-only",
                  occurredAt: "2026-03-10T02:59:00Z",
                  promptCacheKey: "pck-running-only",
                  status: "running",
                }),
              ),
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1403,
                  invokeId: "invoke-running-only-old-terminal",
                  occurredAt: "2026-03-10T02:48:00Z",
                  promptCacheKey: "pck-running-only",
                  status: "completed",
                }),
              ),
            ],
          }),
          createConversation("pck-terminal-late", {
            createdAt: "2026-03-10T02:58:00Z",
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1404,
                  invokeId: "invoke-terminal-late",
                  occurredAt: "2026-03-10T02:58:30Z",
                  promptCacheKey: "pck-terminal-late",
                  status: "completed",
                }),
              ),
            ],
          }),
        ],
      },
      {},
      { mode: "activityWindow", activityMinutes: 5 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toEqual([
      "pck-terminal-late",
      "pck-terminal-early",
      "pck-running-only",
    ]);
  });

  it("keeps reactivated older conversations inside the capped 5-minute working set", () => {
    const baseConversations = Array.from({ length: 50 }, (_, index) =>
      createConversation(`pck-base-${index.toString().padStart(2, "0")}`, {
        createdAt: `2026-03-10T02:${(10 + index).toString().padStart(2, "0")}:00Z`,
        lastActivityAt: `2026-03-10T02:55:${index.toString().padStart(2, "0")}Z`,
        recentInvocations: [
          buildPromptCachePreviewFromInvocation(
            createLiveRecord({
              id: 1500 + index,
              invokeId: `invoke-base-${index}`,
              occurredAt: `2026-03-10T02:55:${index.toString().padStart(2, "0")}Z`,
              promptCacheKey: `pck-base-${index.toString().padStart(2, "0")}`,
              status: "completed",
            }),
          ),
        ],
      }),
    );

    const merged = mergePromptCacheConversationsResponse(
      {
        rangeStart: "2026-03-10T02:55:00Z",
        rangeEnd: "2026-03-10T03:00:00Z",
        selectionMode: "activityWindow",
        selectedLimit: null,
        selectedActivityHours: null,
        selectedActivityMinutes: 5,
        implicitFilter: { kind: null, filteredCount: 0 },
        conversations: baseConversations,
      },
      {
        "pck-old-running": [
          createLiveRecord({
            id: 1701,
            invokeId: "invoke-old-running",
            occurredAt: "2026-03-10T02:59:30Z",
            promptCacheKey: "pck-old-running",
            status: "running",
          }),
        ],
      },
      { mode: "activityWindow", activityMinutes: 5 },
      Date.parse("2026-03-10T03:00:00Z"),
      {
        "pck-old-running": {
          createdAt: "2026-03-09T01:00:00Z",
          lastActivityAt: "2026-03-09T01:00:00Z",
        },
      },
    );

    expect(merged?.conversations).toHaveLength(50);
    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain("pck-old-running");
    expect(merged?.conversations.map((item) => item.promptCacheKey)).not.toContain("pck-base-00");
    expect(merged?.conversations.at(-1)?.promptCacheKey).toBe("pck-old-running");
  });

  it("breaks capped working-set ties by createdAt descending after the shared anchor", () => {
    const baseConversations = Array.from({ length: 49 }, (_, index) =>
      createConversation(`pck-base-${index.toString().padStart(2, "0")}`, {
        createdAt: `2026-03-10T02:${(10 + index).toString().padStart(2, "0")}:00Z`,
        lastActivityAt: `2026-03-10T02:59:${(59 - index).toString().padStart(2, "0")}Z`,
        recentInvocations: [
          buildPromptCachePreviewFromInvocation(
            createLiveRecord({
              id: 1800 + index,
              invokeId: `invoke-base-${index}`,
              occurredAt: `2026-03-10T02:59:${(59 - index).toString().padStart(2, "0")}Z`,
              promptCacheKey: `pck-base-${index.toString().padStart(2, "0")}`,
              status: "completed",
            }),
          ),
        ],
      }),
    );

    const merged = mergePromptCacheConversationsResponse(
      {
        rangeStart: "2026-03-10T02:55:00Z",
        rangeEnd: "2026-03-10T03:00:00Z",
        selectionMode: "activityWindow",
        selectedLimit: null,
        selectedActivityHours: null,
        selectedActivityMinutes: 5,
        implicitFilter: { kind: null, filteredCount: 0 },
        conversations: [
          ...baseConversations,
          createConversation("pck-tie-older", {
            createdAt: "2026-03-09T01:00:00Z",
            lastActivityAt: "2026-03-10T02:59:59Z",
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1900,
                  invokeId: "invoke-tie-older-running",
                  occurredAt: "2026-03-10T02:59:59Z",
                  promptCacheKey: "pck-tie-older",
                  status: "running",
                }),
              ),
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1899,
                  invokeId: "invoke-tie-older-terminal",
                  occurredAt: "2026-03-10T02:55:00Z",
                  promptCacheKey: "pck-tie-older",
                  status: "completed",
                }),
              ),
            ],
          }),
          createConversation("pck-tie-newer", {
            createdAt: "2026-03-10T02:54:59Z",
            lastActivityAt: "2026-03-10T02:55:00Z",
            recentInvocations: [
              buildPromptCachePreviewFromInvocation(
                createLiveRecord({
                  id: 1901,
                  invokeId: "invoke-tie-newer-terminal",
                  occurredAt: "2026-03-10T02:55:00Z",
                  promptCacheKey: "pck-tie-newer",
                  status: "completed",
                }),
              ),
            ],
          }),
        ],
      },
      {},
      { mode: "activityWindow", activityMinutes: 5 },
      Date.parse("2026-03-10T03:00:00Z"),
    );

    expect(merged?.conversations).toHaveLength(50);
    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain("pck-tie-newer");
    expect(merged?.conversations.map((item) => item.promptCacheKey)).not.toContain("pck-tie-older");
  });
});

describe("mergePromptCacheConversationHistory", () => {
  it("preserves durable invocation timestamps when a snapshot omits or regresses them", () => {
    const current: PromptCacheConversationHistoryByKey = {
      "pck-history": {
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T03:00:00Z",
        firstInvocationAt: "2026-03-10T01:10:00Z",
        lastInvocationAt: "2026-03-10T02:50:00Z",
      },
    };

    const omitted = mergePromptCacheConversationHistory(
      current,
      createResponse([
        createConversation("pck-history", {
          createdAt: "2026-03-10T01:30:00Z",
          lastActivityAt: "2026-03-10T02:00:00Z",
        }),
      ]),
    );
    expect(omitted["pck-history"]?.firstInvocationAt).toBe("2026-03-10T01:10:00Z");
    expect(omitted["pck-history"]?.lastInvocationAt).toBe("2026-03-10T02:50:00Z");

    const regressed = mergePromptCacheConversationHistory(
      omitted,
      createResponse([
        createConversation("pck-history", {
          createdAt: "2026-03-10T01:30:00Z",
          lastActivityAt: "2026-03-10T02:00:00Z",
          firstInvocationAt: "2026-03-10T01:20:00Z",
          lastInvocationAt: "2026-03-10T02:10:00Z",
        }),
      ]),
    );
    expect(regressed["pck-history"]?.firstInvocationAt).toBe("2026-03-10T01:10:00Z");
    expect(regressed["pck-history"]?.lastInvocationAt).toBe("2026-03-10T02:50:00Z");
  });

  it("retains only the most recent inactive history entries within the configured bound", () => {
    const merged = mergePromptCacheConversationHistory(
      {
        "pck-old-a": {
          createdAt: "2026-03-10T00:00:00Z",
          lastActivityAt: "2026-03-10T00:01:00Z",
        },
        "pck-old-b": {
          createdAt: "2026-03-10T00:04:00Z",
          lastActivityAt: "2026-03-10T00:05:00Z",
        },
        "pck-live": {
          createdAt: "2026-03-10T00:02:00Z",
          lastActivityAt: "2026-03-10T00:03:00Z",
        },
      },
      createResponse([
        createConversation("pck-authoritative", {
          createdAt: "2026-03-10T01:00:00Z",
          lastActivityAt: "2026-03-10T01:05:00Z",
        }),
      ]),
      ["pck-live"],
      1,
    );

    expect(merged).toEqual({
      "pck-authoritative": {
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T01:05:00Z",
      },
      "pck-live": {
        createdAt: "2026-03-10T00:02:00Z",
        lastActivityAt: "2026-03-10T00:03:00Z",
      },
      "pck-old-b": {
        createdAt: "2026-03-10T00:04:00Z",
        lastActivityAt: "2026-03-10T00:05:00Z",
      },
    });
  });

  it("stays bounded under high churn while keeping recent inactive history for reactivation", () => {
    let current: PromptCacheConversationHistoryByKey = {
      "pck-live": {
        createdAt: "2026-03-10T00:02:00Z",
        lastActivityAt: "2026-03-10T00:03:00Z",
      },
    };

    for (let index = 0; index < 128; index += 1) {
      const createdAt = new Date(Date.parse("2026-03-10T00:00:00Z") + index * 60_000).toISOString();
      const lastActivityAt = new Date(
        Date.parse("2026-03-10T00:00:30Z") + index * 60_000,
      ).toISOString();
      current = mergePromptCacheConversationHistory(
        current,
        createResponse([
          createConversation(`pck-${index}`, {
            createdAt,
            lastActivityAt,
          }),
        ]),
        ["pck-live"],
        50,
      );
    }

    expect(Object.keys(current)).toHaveLength(52);
    expect(Object.keys(current)).toContain("pck-127");
    expect(Object.keys(current)).toContain("pck-live");
    expect(Object.keys(current)).toContain("pck-126");
    expect(Object.keys(current)).not.toContain("pck-0");
  });
});

describe("reconcilePromptCacheLiveRecordMap", () => {
  it("keeps unseen completed keys when the authoritative response started before the live record arrived", () => {
    const completedRecord = createLiveRecord({
      id: 1200,
      invokeId: "invoke-hidden-completed",
      occurredAt: "2026-03-10T02:30:00Z",
      promptCacheKey: "pck-hidden-completed",
      status: "completed",
    });

    const reconciled = reconcilePromptCacheLiveRecordMap(
      { "pck-hidden-completed": [completedRecord] },
      createResponse([createConversation("pck-visible-a"), createConversation("pck-visible-b")]),
      {
        requestStartedAtMs: 100,
        liveRecordObservedAtByKey: { "pck-hidden-completed": 101 },
      },
    );

    expect(reconciled).toEqual({
      "pck-hidden-completed": [completedRecord],
    });
  });

  it("drops unseen terminal-only keys when the authoritative resync still omits them", () => {
    const reconciled = reconcilePromptCacheLiveRecordMap(
      {
        "pck-hidden": [
          createLiveRecord({
            id: 1201,
            invokeId: "invoke-hidden",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-hidden",
            status: "completed",
          }),
        ],
      },
      createResponse([createConversation("pck-visible-a"), createConversation("pck-visible-b")]),
    );

    expect(reconciled).toEqual({});
  });

  it("keeps unseen running keys until a later authoritative resync can confirm them", () => {
    const liveRecord = createLiveRecord({
      id: 1202,
      invokeId: "invoke-running-hidden",
      occurredAt: "2026-03-10T02:30:00Z",
      promptCacheKey: "pck-hidden-running",
      status: "running",
    });

    const reconciled = reconcilePromptCacheLiveRecordMap(
      { "pck-hidden-running": [liveRecord] },
      createResponse([createConversation("pck-visible-a"), createConversation("pck-visible-b")]),
    );

    expect(reconciled).toEqual({
      "pck-hidden-running": [liveRecord],
    });
  });

  it("drops completed live records once they fall outside a full authoritative preview window", () => {
    const droppedRecord = createLiveRecord({
      id: 2001,
      invokeId: "invoke-preview-tail-drop",
      occurredAt: "2026-03-10T02:24:00Z",
      promptCacheKey: "pck-preview-full",
      status: "completed",
      totalTokens: 3200,
    });

    const reconciled = reconcilePromptCacheLiveRecordMap(
      { "pck-preview-full": [droppedRecord] },
      createResponse([
        createConversation("pck-preview-full", {
          recentInvocations: [
            createLiveRecord({
              id: 2105,
              invokeId: "invoke-preview-5",
              occurredAt: "2026-03-10T02:29:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2104,
              invokeId: "invoke-preview-4",
              occurredAt: "2026-03-10T02:28:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2103,
              invokeId: "invoke-preview-3",
              occurredAt: "2026-03-10T02:27:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2102,
              invokeId: "invoke-preview-2",
              occurredAt: "2026-03-10T02:26:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2101,
              invokeId: "invoke-preview-1",
              occurredAt: "2026-03-10T02:25:00Z",
              promptCacheKey: "pck-preview-full",
            }),
          ].map(buildPromptCachePreviewFromInvocation),
        }),
      ]),
    );

    expect(reconciled).toEqual({});
  });

  it("keeps transient running records when preview tie-break IDs are still database-only", () => {
    const liveRecord = createLiveRecord({
      id: 0,
      invokeId: "invoke-preview-transient-running",
      occurredAt: "2026-03-10T02:25:00Z",
      promptCacheKey: "pck-preview-full",
      status: "running",
      totalTokens: 0,
    });

    const reconciled = reconcilePromptCacheLiveRecordMap(
      { "pck-preview-full": [liveRecord] },
      createResponse([
        createConversation("pck-preview-full", {
          recentInvocations: [
            createLiveRecord({
              id: 2105,
              invokeId: "invoke-preview-5",
              occurredAt: "2026-03-10T02:29:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2104,
              invokeId: "invoke-preview-4",
              occurredAt: "2026-03-10T02:28:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2103,
              invokeId: "invoke-preview-3",
              occurredAt: "2026-03-10T02:27:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2102,
              invokeId: "invoke-preview-2",
              occurredAt: "2026-03-10T02:26:00Z",
              promptCacheKey: "pck-preview-full",
            }),
            createLiveRecord({
              id: 2101,
              invokeId: "invoke-preview-tail",
              occurredAt: "2026-03-10T02:25:00Z",
              promptCacheKey: "pck-preview-full",
            }),
          ].map(buildPromptCachePreviewFromInvocation),
        }),
      ]),
    );

    expect(reconciled).toEqual({
      "pck-preview-full": [liveRecord],
    });
  });

  it("keeps live records until authoritative previews include downstream diagnostics", () => {
    const liveRecord = createLiveRecord({
      id: 2301,
      invokeId: "invoke-preview-downstream-gap",
      occurredAt: "2026-03-10T02:30:00Z",
      promptCacheKey: "pck-preview-downstream-gap",
      status: "failed",
      errorMessage: "failed to contact oauth codex upstream",
      failureKind: "failed_contact_upstream",
      downstreamStatusCode: 502,
      downstreamErrorMessage:
        "pool upstream responded with 502: failed to contact oauth codex upstream",
    });

    const reconciled = reconcilePromptCacheLiveRecordMap(
      { "pck-preview-downstream-gap": [liveRecord] },
      createResponse([
        createConversation("pck-preview-downstream-gap", {
          recentInvocations: [
            createPreview({
              id: 2301,
              invokeId: "invoke-preview-downstream-gap",
              occurredAt: "2026-03-10T02:30:00Z",
              status: "failed",
              failureClass: "service_failure",
            }),
          ],
        }),
      ]),
    );

    expect(reconciled).toEqual({
      "pck-preview-downstream-gap": [liveRecord],
    });
  });
});
