import { describe, expect, it } from "vitest";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "./api";
import {
  formatDashboardWorkingConversationSequenceId,
  mapPromptCacheConversationsToDashboardCards,
} from "./dashboardWorkingConversations";

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
    upstreamAccountName:
      overrides.upstreamAccountName ?? "pool-alpha@example.com",
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
  const hasLastTerminalAt = Object.prototype.hasOwnProperty.call(
    overrides,
    "lastTerminalAt",
  );
  const hasLastInFlightAt = Object.prototype.hasOwnProperty.call(
    overrides,
    "lastInFlightAt",
  );
  return {
    promptCacheKey,
    requestCount: overrides.requestCount ?? recentInvocations.length,
    totalTokens: overrides.totalTokens ?? 1000,
    totalCost: overrides.totalCost ?? 0.048,
    createdAt:
      overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt:
      overrides.lastActivityAt ??
      recentInvocations[0]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastTerminalAt: hasLastTerminalAt
      ? (overrides.lastTerminalAt ?? null)
      : undefined,
    lastInFlightAt: hasLastInFlightAt
      ? (overrides.lastInFlightAt ?? null)
      : undefined,
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
  it("strips the WC prefix from the display sequence id without changing the raw id", () => {
    expect(formatDashboardWorkingConversationSequenceId("WC-ABCDEF")).toBe(
      "ABCDEF",
    );
    expect(formatDashboardWorkingConversationSequenceId("WC-ABCDEF-11")).toBe(
      "ABCDEF-11",
    );
    expect(formatDashboardWorkingConversationSequenceId("ABCDEF")).toBe(
      "ABCDEF",
    );
  });

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
    expect(first[0]?.conversationSequenceId).toBe(
      second[0]?.conversationSequenceId,
    );
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

  it("keeps cards in visible-set order so newer activity stays ahead of newer createdAt", () => {
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

  it("uses the latest of terminal and in-flight anchors when both are present", () => {
    const response = createResponse([
      createConversation(
        "pck-has-newer-inflight",
        [
          createPreview({
            id: 41,
            invokeId: "invoke-41",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:00:00Z",
          lastTerminalAt: "2026-04-04T10:02:00Z",
          lastInFlightAt: "2026-04-04T10:04:00Z",
        },
      ),
      createConversation(
        "pck-terminal-only",
        [
          createPreview({
            id: 42,
            invokeId: "invoke-42",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:01:00Z",
          lastTerminalAt: "2026-04-04T10:03:00Z",
          lastInFlightAt: null,
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response);
    const inflightAnchored = cards.find(
      (card) => card.promptCacheKey === "pck-has-newer-inflight",
    );

    expect(inflightAnchored?.sortAnchorEpoch).toBe(
      Date.parse("2026-04-04T10:04:00Z"),
    );
  });

  it("keeps the explicit 20-card cap anchored on active work before final display sorting", () => {
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

    const cards = mapPromptCacheConversationsToDashboardCards(response, {
      limit: 20,
    });

    expect(cards).toHaveLength(20);
    expect(cards.map((card) => card.promptCacheKey)).toContain(
      "pck-old-running",
    );
    expect(cards.map((card) => card.promptCacheKey)).not.toContain(
      "pck-base-00",
    );
    expect(cards.at(-1)?.promptCacheKey).toBe("pck-old-running");
  });

  it("re-sorts loaded cards by createdAt descending even when later pages arrive behind the visible set", () => {
    const response = createResponse([
      createConversation(
        "pck-head-visible",
        [
          createPreview({
            id: 101,
            invokeId: "invoke-head-visible",
            occurredAt: "2026-04-04T10:04:50Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:04:50Z",
          lastTerminalAt: "2026-04-04T10:04:50Z",
        },
      ),
      createConversation(
        "pck-head-second",
        [
          createPreview({
            id: 102,
            invokeId: "invoke-head-second",
            occurredAt: "2026-04-04T10:04:40Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:04:40Z",
          lastTerminalAt: "2026-04-04T10:04:40Z",
        },
      ),
      createConversation(
        "pck-page-two-new-created",
        [
          createPreview({
            id: 103,
            invokeId: "invoke-page-two",
            occurredAt: "2026-04-04T10:03:30Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T10:05:30Z",
          lastTerminalAt: "2026-04-04T10:03:30Z",
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response);

    expect(cards.map((card) => card.promptCacheKey)).toEqual([
      "pck-page-two-new-created",
      "pck-head-visible",
      "pck-head-second",
    ]);
    expect(cards[0]?.createdAtEpoch).toBe(Date.parse("2026-04-04T10:05:30Z"));
    expect(cards[0]?.sortAnchorEpoch).toBe(Date.parse("2026-04-04T10:03:30Z"));
  });

  it("breaks explicit cap ties by createdAt descending after the shared anchor", () => {
    const response = createResponse([
      ...Array.from({ length: 19 }, (_, index) =>
        createConversation(
          `pck-base-${index.toString().padStart(2, "0")}`,
          [
            createPreview({
              id: 300 + index,
              invokeId: `invoke-base-${index}`,
              occurredAt: `2026-04-04T10:05:${(19 - index).toString().padStart(2, "0")}Z`,
              status: "completed",
            }),
          ],
          {
            createdAt: `2026-04-04T09:${index.toString().padStart(2, "0")}:00Z`,
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
            occurredAt: "2026-04-04T09:59:00Z",
            status: "completed",
          }),
        ],
        {
          createdAt: "2026-04-04T09:00:00Z",
          lastActivityAt: "2026-04-04T10:05:00Z",
          lastTerminalAt: "2026-04-04T09:59:00Z",
          lastInFlightAt: "2026-04-04T10:05:00Z",
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
          lastActivityAt: "2026-04-04T10:05:00Z",
          lastTerminalAt: "2026-04-04T10:00:00Z",
          lastInFlightAt: "2026-04-04T10:05:00Z",
        },
      ),
    ]);

    const cards = mapPromptCacheConversationsToDashboardCards(response, {
      limit: 20,
    });

    expect(cards).toHaveLength(20);
    expect(cards.map((card) => card.promptCacheKey)).toContain("pck-tie-newer");
    expect(cards.map((card) => card.promptCacheKey)).not.toContain(
      "pck-tie-older",
    );
  });
});
