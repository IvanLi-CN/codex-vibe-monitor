import { describe, expect, it } from "vitest";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  PromptCacheConversationRequestPoint,
} from "./api";
import {
  buildPromptCachePreviewFromInvocation,
  mergePromptCacheConversationsResponse,
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
  it("lets unseen live conversations displace older rows in count-capped mode", () => {
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
      "pck-live-new",
      "pck-newest",
    ]);
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
        "pck-live": [
          authoritativeRecord,
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
        requestTokens: 2400,
        cumulativeTokens: 2400,
      }),
    ]);
  });
});

describe("reconcilePromptCacheLiveRecordMap", () => {
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
      createResponse([
        createConversation("pck-visible-a"),
        createConversation("pck-visible-b"),
      ]),
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
      createResponse([
        createConversation("pck-visible-a"),
        createConversation("pck-visible-b"),
      ]),
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
});
