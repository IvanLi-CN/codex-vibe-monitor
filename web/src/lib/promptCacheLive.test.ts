import { describe, expect, it } from "vitest";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  PromptCacheConversationRequestPoint,
} from "./api";
import {
  buildPromptCachePreviewFromInvocation,
  buildInvocationFromPromptCachePreview,
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
      downstreamErrorMessage:
        "[downstream_closed] downstream closed while streaming upstream response",
    });

    const preview = buildPromptCachePreviewFromInvocation(record);
    const rebuilt = buildInvocationFromPromptCachePreview(preview);

    expect(preview.downstreamStatusCode).toBe(200);
    expect(preview.downstreamErrorMessage).toContain("downstream closed");
    expect(rebuilt.downstreamStatusCode).toBe(200);
    expect(rebuilt.downstreamErrorMessage).toContain("downstream closed");
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

    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain(
      "pck-running-old",
    );
    expect(merged?.conversations.find((item) => item.promptCacheKey === "pck-running-old")?.recentInvocations[0]?.status).toBe(
      "running",
    );
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
    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain(
      "pck-old-running",
    );
    expect(merged?.conversations.map((item) => item.promptCacheKey)).not.toContain(
      "pck-base-00",
    );
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
    expect(merged?.conversations.map((item) => item.promptCacheKey)).toContain(
      "pck-tie-newer",
    );
    expect(merged?.conversations.map((item) => item.promptCacheKey)).not.toContain(
      "pck-tie-older",
    );
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
      createResponse([
        createConversation("pck-visible-a"),
        createConversation("pck-visible-b"),
      ]),
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
