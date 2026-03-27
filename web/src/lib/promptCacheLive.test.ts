import { describe, expect, it } from "vitest";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  PromptCacheConversationRequestPoint,
} from "./api";
import { mergePromptCacheConversationsResponse } from "./promptCacheLive";

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

describe("mergePromptCacheConversationsResponse", () => {
  it("does not let unseen live conversations displace count-capped authoritative rows", () => {
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
      "pck-newest",
      "pck-older",
    ]);
  });

  it("dedupes last24h request points when the same invocation is already present after resync", () => {
    const base = createResponse([
      createConversation("pck-live", {
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
        "pck-live": [
          createLiveRecord({
            id: 901,
            invokeId: "invoke-live-01",
            occurredAt: "2026-03-10T02:30:00Z",
            promptCacheKey: "pck-live",
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
    ]);
  });
});
