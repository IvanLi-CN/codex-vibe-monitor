import { describe, expect, it } from "vitest";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "./api";
import { mapPromptCacheConversationsToDashboardCards } from "./dashboardWorkingConversations";

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
    totalTokens: overrides.totalTokens ?? 120,
    cost: overrides.cost ?? 0.012,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 17,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    inputTokens: overrides.inputTokens ?? 64,
    outputTokens: overrides.outputTokens ?? 56,
    cacheInputTokens: overrides.cacheInputTokens ?? 12,
    reasoningTokens: overrides.reasoningTokens ?? 8,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 12,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 130,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 90,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 280,
    tRespParseMs: overrides.tRespParseMs ?? 10,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 539,
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
    totalTokens: overrides.totalTokens ?? 1000,
    totalCost: overrides.totalCost ?? 0.048,
    createdAt: overrides.createdAt ?? recentInvocations[recentInvocations.length - 1]?.occurredAt ?? "2026-04-04T10:00:00Z",
    lastActivityAt: overrides.lastActivityAt ?? recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
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

describe("mapPromptCacheConversationsToDashboardCards", () => {
  it("builds stable WC short sequence ids from prompt cache keys", () => {
    const response = createResponse([
      createConversation("pck-alpha", [
        createPreview({
          id: 1,
          invokeId: "invoke-1",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
      ]),
    ]);

    const first = mapPromptCacheConversationsToDashboardCards(response);
    const second = mapPromptCacheConversationsToDashboardCards(response);

    expect(first[0]?.conversationSequenceId).toMatch(/^WC-[A-F0-9]{6}$/);
    expect(first[0]?.conversationSequenceId).toBe(second[0]?.conversationSequenceId);
    expect(first[0]?.hasPreviousPlaceholder).toBe(true);
  });

  it("appends a stable short suffix when visible WC short ids collide", () => {
    const response = createResponse([
      createConversation("pck-alpha", [
        createPreview({
          id: 1,
          invokeId: "invoke-1",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
      ]),
      createConversation("pck-beta", [
        createPreview({
          id: 2,
          invokeId: "invoke-2",
          occurredAt: "2026-04-04T10:03:00Z",
          status: "completed",
        }),
      ]),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response, {
      hashFn: () => "ABCDEF00",
      collisionHashFn: (key) => (key.includes("alpha") ? "11" : "22"),
    });

    expect(cards.map((card) => card.conversationSequenceId)).toEqual([
      "WC-ABCDEF-11",
      "WC-ABCDEF-22",
    ]);
  });

  it("sorts cards by conversation created time descending even when a running-only card is newer by activity", () => {
    const response = createResponse([
      createConversation(
        "pck-terminal-late",
        [
          createPreview({
            id: 11,
            invokeId: "invoke-11",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
          createPreview({
            id: 10,
            invokeId: "invoke-10",
            occurredAt: "2026-04-04T10:01:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:03:00Z",
        },
      ),
      createConversation("pck-running-only", [
        createPreview({
          id: 21,
          invokeId: "invoke-21",
          occurredAt: "2026-04-04T10:05:00Z",
          status: "running",
        }),
        createPreview({
          id: 20,
          invokeId: "invoke-20",
          occurredAt: "2026-04-04T09:54:00Z",
          status: "completed",
        }),
      ]),
      createConversation(
        "pck-terminal-early",
        [
          createPreview({
            id: 31,
            invokeId: "invoke-31",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:02:00Z",
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response);

    expect(cards.map((card) => card.promptCacheKey)).toEqual([
      "pck-terminal-late",
      "pck-terminal-early",
      "pck-running-only",
    ]);
    expect(cards[2]?.currentInvocation.displayStatus).toBe("running");
    expect(cards[2]?.previousInvocation?.displayStatus).toBe("completed");
    expect(cards[2]?.sortAnchorEpoch).toBe(Date.parse("2026-04-04T10:05:00Z"));
  });

  it("keeps the 20-card dashboard cap anchored on active work before display reordering", () => {
    const response = createResponse([
      ...Array.from({ length: 20 }, (_, index) =>
        createConversation(
          `pck-base-${index.toString().padStart(2, "0")}`,
          [
            createPreview({
              id: 100 + index,
              invokeId: `invoke-base-${index}`,
              occurredAt: `2026-04-04T10:00:${index.toString().padStart(2, "0")}Z`,
              status: "completed",
            }),
          ],
          {
            createdAt: `2026-04-04T10:00:${index.toString().padStart(2, "0")}Z`,
          },
        ),
      ),
      createConversation(
        "pck-old-running",
        [
          createPreview({
            id: 201,
            invokeId: "invoke-old-running",
            occurredAt: "2026-04-04T10:04:59Z",
            status: "running",
          }),
          createPreview({
            id: 200,
            invokeId: "invoke-old-running-previous",
            occurredAt: "2026-04-04T09:40:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T09:30:00Z",
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response);

    expect(cards).toHaveLength(20);
    expect(cards.map((card) => card.promptCacheKey)).toContain("pck-old-running");
    expect(cards.map((card) => card.promptCacheKey)).not.toContain("pck-base-00");
    expect(cards.at(-1)?.promptCacheKey).toBe("pck-old-running");
  });

  it("breaks dashboard cap ties by createdAt descending after the shared anchor", () => {
    const response = createResponse([
      ...Array.from({ length: 19 }, (_, index) =>
        createConversation(
          `pck-base-${index.toString().padStart(2, "0")}`,
          [
            createPreview({
              id: 300 + index,
              invokeId: `invoke-base-${index}`,
              occurredAt: `2026-04-04T10:04:${(59 - index).toString().padStart(2, "0")}Z`,
              status: "completed",
            }),
          ],
          {
            createdAt: `2026-04-04T10:${index.toString().padStart(2, "0")}:00Z`,
          },
        ),
      ),
      createConversation(
        "pck-tie-older",
        [
          createPreview({
            id: 401,
            invokeId: "invoke-tie-older-running",
            occurredAt: "2026-04-04T10:04:59Z",
            status: "running",
          }),
          createPreview({
            id: 400,
            invokeId: "invoke-tie-older-terminal",
            occurredAt: "2026-04-04T10:00:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T09:00:00Z",
          lastActivityAt: "2026-04-04T10:04:59Z",
        },
      ),
      createConversation(
        "pck-tie-newer",
        [
          createPreview({
            id: 402,
            invokeId: "invoke-tie-newer-terminal",
            occurredAt: "2026-04-04T10:00:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T09:59:00Z",
          lastActivityAt: "2026-04-04T10:00:00Z",
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response);

    expect(cards).toHaveLength(20);
    expect(cards.map((card) => card.promptCacheKey)).toContain("pck-tie-newer");
    expect(cards.map((card) => card.promptCacheKey)).not.toContain("pck-tie-older");
  });
});
