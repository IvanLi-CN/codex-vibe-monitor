/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import type {
  ApiInvocation,
  BroadcastPayload,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { useDashboardWorkingConversations } from "./useDashboardWorkingConversations";

const apiMocks = vi.hoisted(() => ({
  fetchPromptCacheConversationsPage: vi.fn<
    (
      selection: { mode: "activityWindow"; activityMinutes: number },
      options?: {
        pageSize?: number;
        cursor?: string | null;
        snapshotAt?: string | null;
        detail?: "compact" | "full";
        signal?: AbortSignal;
      },
    ) => Promise<PromptCacheConversationsResponse>
  >(),
}));

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchPromptCacheConversationsPage:
      apiMocks.fetchPromptCacheConversationsPage,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: (listener: (payload: BroadcastPayload) => void) => {
    sseMocks.listeners.add(listener);
    return () => sseMocks.listeners.delete(listener);
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListeners.add(listener);
    return () => sseMocks.openListeners.delete(listener);
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let visibilityState: DocumentVisibilityState = "visible";
const fixedNowMs = Date.parse("2026-04-10T02:05:00Z");
const realDateNow = Date.now.bind(Date);

beforeAll(() => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => visibilityState,
  });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  Date.now = () => fixedNowMs;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  sseMocks.listeners.clear();
  sseMocks.openListeners.clear();
  visibilityState = "visible";
  vi.useRealTimers();
  Date.now = realDateNow;
  apiMocks.fetchPromptCacheConversationsPage.mockReset();
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
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
    source: overrides.source ?? "pool",
    inputTokens: overrides.inputTokens ?? 0,
    outputTokens: overrides.outputTokens ?? 0,
    cacheInputTokens: overrides.cacheInputTokens ?? 0,
    reasoningTokens: overrides.reasoningTokens ?? 0,
    responseContentEncoding: overrides.responseContentEncoding,
    serviceTier: overrides.serviceTier,
    tReqReadMs: overrides.tReqReadMs,
    tReqParseMs: overrides.tReqParseMs,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs,
    tRespParseMs: overrides.tRespParseMs,
    tPersistMs: overrides.tPersistMs,
    tTotalMs: overrides.tTotalMs,
  };
}

function createConversation(
  promptCacheKey: string,
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
    requestCount: overrides.requestCount ?? 1,
    totalTokens: overrides.totalTokens ?? 30,
    totalCost: overrides.totalCost ?? 0.12,
    createdAt: overrides.createdAt ?? "2026-04-10T02:00:00Z",
    lastActivityAt: overrides.lastActivityAt ?? "2026-04-10T02:04:00Z",
    lastTerminalAt: hasLastTerminalAt
      ? (overrides.lastTerminalAt ?? null)
      : "2026-04-10T02:04:00Z",
    lastInFlightAt: hasLastInFlightAt
      ? (overrides.lastInFlightAt ?? null)
      : null,
    cursor: overrides.cursor ?? `${promptCacheKey}-cursor`,
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations: overrides.recentInvocations ?? [
      createPreview({
        id: 1,
        invokeId: `${promptCacheKey}-invoke-1`,
        occurredAt: overrides.lastActivityAt ?? "2026-04-10T02:04:00Z",
        status: "completed",
        totalTokens: overrides.totalTokens ?? 30,
        cost: overrides.totalCost ?? 0.12,
      }),
    ],
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

function createResponseWithConversations(
  conversations: PromptCacheConversation[],
  overrides: Partial<PromptCacheConversationsResponse> = {},
): PromptCacheConversationsResponse {
  return {
    rangeStart: overrides.rangeStart ?? "2026-04-10T02:00:00Z",
    rangeEnd: overrides.rangeEnd ?? "2026-04-10T02:05:00Z",
    snapshotAt: overrides.snapshotAt ?? "2026-04-10T02:05:00Z",
    selectionMode: overrides.selectionMode ?? "activityWindow",
    selectedLimit: overrides.selectedLimit ?? null,
    selectedActivityHours: overrides.selectedActivityHours ?? null,
    selectedActivityMinutes: overrides.selectedActivityMinutes ?? 5,
    implicitFilter: overrides.implicitFilter ?? {
      kind: null,
      filteredCount: 0,
    },
    totalMatched: overrides.totalMatched ?? conversations.length,
    hasMore: overrides.hasMore ?? false,
    nextCursor: overrides.nextCursor ?? null,
    conversations,
  };
}

function createConversationBatch(
  prefix: string,
  count: number,
  overrides?: (index: number) => Partial<PromptCacheConversation>,
) {
  return Array.from({ length: count }, (_, index) =>
    createConversation(`${prefix}-${index + 1}`, overrides?.(index)),
  );
}

function createRecord(
  promptCacheKey: string,
  overrides: Partial<ApiInvocation> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    promptCacheKey,
    occurredAt: overrides.occurredAt,
    createdAt: overrides.createdAt ?? overrides.occurredAt,
    status: overrides.status,
    source: overrides.source ?? "pool",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    endpoint: overrides.endpoint ?? "/v1/responses",
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    proxyDisplayName: overrides.proxyDisplayName,
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    upstreamAccountName: overrides.upstreamAccountName,
    requestedServiceTier: overrides.requestedServiceTier,
    serviceTier: overrides.serviceTier,
    responseContentEncoding: overrides.responseContentEncoding,
    inputTokens: overrides.inputTokens,
    outputTokens: overrides.outputTokens,
    cacheInputTokens: overrides.cacheInputTokens,
    reasoningTokens: overrides.reasoningTokens,
    reasoningEffort: overrides.reasoningEffort,
    failureKind: overrides.failureKind,
    errorMessage: overrides.errorMessage,
    failureClass: overrides.failureClass,
    isActionable: overrides.isActionable,
    tReqReadMs: overrides.tReqReadMs,
    tReqParseMs: overrides.tReqParseMs,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs,
    tRespParseMs: overrides.tRespParseMs,
    tPersistMs: overrides.tPersistMs,
    tTotalMs: overrides.tTotalMs,
  };
}

function emitRecords(records: ApiInvocation[]) {
  sseMocks.listeners.forEach((listener) => {
    listener({ type: "records", records });
  });
}

function emitOpen() {
  sseMocks.openListeners.forEach((listener) => {
    listener();
  });
}

function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`Missing button: ${testId}`);
  }
  act(() => {
    element.click();
  });
}

function Probe() {
  const {
    cards,
    stats,
    totalMatched,
    hasMore,
    isLoading,
    isLoadingMore,
    error,
    loadMore,
    refresh,
    setRefreshTargetCount,
  } = useDashboardWorkingConversations();

  return (
    <div>
      <button type="button" data-testid="load-more" onClick={() => loadMore()}>
        load more
      </button>
      <button type="button" data-testid="refresh" onClick={() => refresh()}>
        refresh
      </button>
      <button
        type="button"
        data-testid="set-refresh-target-20"
        onClick={() => setRefreshTargetCount(20)}
      >
        set refresh target 20
      </button>
      <button
        type="button"
        data-testid="set-refresh-target-25"
        onClick={() => setRefreshTargetCount(25)}
      >
        set refresh target 25
      </button>
      <div data-testid="cards-length">{String(cards.length)}</div>
      <div data-testid="card-keys">
        {cards.map((item) => item.promptCacheKey).join(",")}
      </div>
      <div data-testid="conversation-keys">
        {stats?.conversations.map((item) => item.promptCacheKey).join(",") ??
          ""}
      </div>
      <div data-testid="conversation-summary">
        {stats?.conversations
          .map(
            (item) =>
              `${item.promptCacheKey}:${item.recentInvocations[0]?.invokeId ?? ""}:${item.recentInvocations[0]?.status ?? ""}:${item.requestCount}:${item.lastInFlightAt ?? ""}`,
          )
          .join("|") ?? ""}
      </div>
      <div data-testid="preview-invoke-id">
        {stats?.conversations[0]?.recentInvocations[0]?.invokeId ?? ""}
      </div>
      <div data-testid="preview-status">
        {stats?.conversations[0]?.recentInvocations[0]?.status ?? ""}
      </div>
      <div data-testid="request-count">
        {String(stats?.conversations[0]?.requestCount ?? 0)}
      </div>
      <div data-testid="total-tokens">
        {String(stats?.conversations[0]?.totalTokens ?? 0)}
      </div>
      <div data-testid="last-terminal-at">
        {stats?.conversations[0]?.lastTerminalAt ?? ""}
      </div>
      <div data-testid="last-in-flight-at">
        {stats?.conversations[0]?.lastInFlightAt ?? ""}
      </div>
      <div data-testid="total-matched">{String(totalMatched)}</div>
      <div data-testid="has-more">{hasMore ? "true" : "false"}</div>
      <div data-testid="next-cursor">{stats?.nextCursor ?? ""}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="loading-more">{isLoadingMore ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
    </div>
  );
}

describe("useDashboardWorkingConversations", () => {
  it("keeps card display order on createdAt descending even when the backing rows remain activity-sorted", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-created-middle", {
          createdAt: "2026-04-10T02:02:00Z",
          lastActivityAt: "2026-04-10T02:04:58Z",
          lastTerminalAt: "2026-04-10T02:03:40Z",
          lastInFlightAt: "2026-04-10T02:04:58Z",
          recentInvocations: [
            createPreview({
              id: 52,
              invokeId: "invoke-created-middle-running",
              occurredAt: "2026-04-10T02:04:58Z",
              status: "running",
            }),
            createPreview({
              id: 51,
              invokeId: "invoke-created-middle-previous",
              occurredAt: "2026-04-10T02:03:40Z",
              status: "completed",
            }),
          ],
        }),
        createConversation("pck-created-oldest", {
          createdAt: "2026-04-10T01:58:00Z",
          lastActivityAt: "2026-04-10T02:03:20Z",
          lastTerminalAt: "2026-04-10T02:03:20Z",
          recentInvocations: [
            createPreview({
              id: 61,
              invokeId: "invoke-created-oldest",
              occurredAt: "2026-04-10T02:03:20Z",
              status: "completed",
            }),
          ],
        }),
        createConversation("pck-created-newest", {
          createdAt: "2026-04-10T02:03:00Z",
          lastActivityAt: "2026-04-10T02:01:00Z",
          lastTerminalAt: "2026-04-10T02:01:00Z",
          recentInvocations: [
            createPreview({
              id: 71,
              invokeId: "invoke-created-newest",
              occurredAt: "2026-04-10T02:01:00Z",
              status: "completed",
            }),
          ],
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    expect(text("conversation-keys")).toBe(
      "pck-created-middle,pck-created-oldest,pck-created-newest",
    );
    expect(text("card-keys")).toBe(
      "pck-created-newest,pck-created-middle,pck-created-oldest",
    );
  });

  it("loads the head page with compact pagination defaults", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations(
        createConversationBatch("pck-head", 20, (index) => ({
          cursor: `cursor-head-${index + 1}`,
          createdAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          lastActivityAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          lastTerminalAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
        })),
        { totalMatched: 23, hasMore: true, nextCursor: "cursor-page-1" },
      ),
    );

    render(<Probe />);
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledWith(
      { mode: "activityWindow", activityMinutes: 5 },
      expect.objectContaining({
        pageSize: 20,
        detail: "compact",
        signal: expect.any(AbortSignal),
      }),
    );
    expect(text("cards-length")).toBe("20");
    expect(text("conversation-keys")).toContain("pck-head-1");
    expect(text("total-matched")).toBe("23");
    expect(text("has-more")).toBe("true");
    expect(text("next-cursor")).toBe("cursor-page-1");
    expect(text("loading")).toBe("false");
  });

  it("loads the next page with snapshotAt and cursor", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => ({
            cursor: `cursor-head-${index + 1}`,
            createdAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-tail", 5, (index) => ({
            cursor: `cursor-tail-${index + 1}`,
            createdAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();

    click("load-more");
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenLastCalledWith(
      { mode: "activityWindow", activityMinutes: 5 },
      expect.objectContaining({
        pageSize: 20,
        cursor: "cursor-page-1",
        snapshotAt: "2026-04-10T02:05:00Z",
        detail: "compact",
        signal: expect.any(AbortSignal),
      }),
    );
    expect(text("conversation-keys")).toContain("pck-head-1");
    expect(text("conversation-keys")).toContain("pck-tail-1");
    expect(text("has-more")).toBe("false");
    expect(text("loading-more")).toBe("false");
  });

  it("backfills immediately when the refresh target grows after the head page is loaded", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => ({
            cursor: `cursor-head-${index + 1}`,
            createdAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-tail", 5, (index) => ({
            cursor: `cursor-tail-${index + 1}`,
            createdAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    click("set-refresh-target-25");
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenLastCalledWith(
      { mode: "activityWindow", activityMinutes: 5 },
      expect.objectContaining({
        pageSize: 20,
        cursor: "cursor-page-1",
        snapshotAt: "2026-04-10T02:05:00Z",
        detail: "compact",
        signal: expect.any(AbortSignal),
      }),
    );
    expect(text("cards-length")).toBe("25");
    expect(text("conversation-keys")).toContain("pck-tail-1");
    expect(text("has-more")).toBe("false");
  });

  it("keeps already loaded cards visible when a later page fetch fails", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => ({
            cursor: `cursor-head-${index + 1}`,
            createdAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockRejectedValueOnce(new Error("load more temporarily unavailable"));

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    click("load-more");
    await flushAsync();
    await flushAsync();

    expect(text("conversation-keys")).toContain("pck-head-1");
    expect(text("cards-length")).toBe("20");
    expect(text("error")).toBe("load more temporarily unavailable");
    expect(text("next-cursor")).toBe("cursor-page-1");
  });

  it("keeps snapshot pagination stable when a later page loads after the live clock advances", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => {
            const occurredAt = `2026-04-10T02:04:${String(59 - index).padStart(2, "0")}Z`;
            return {
              cursor: `cursor-head-${index + 1}`,
              createdAt: occurredAt,
              lastActivityAt: occurredAt,
              lastTerminalAt: occurredAt,
            };
          }),
          {
            totalMatched: 21,
            hasMore: true,
            nextCursor: "cursor-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-tail-1", {
              cursor: "cursor-tail-1",
              createdAt: "2026-04-10T02:04:39Z",
              lastActivityAt: "2026-04-10T02:04:39Z",
              lastTerminalAt: "2026-04-10T02:04:39Z",
            }),
          ],
          {
            totalMatched: 21,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    Date.now = () => Date.parse("2026-04-10T02:10:30Z");

    click("load-more");
    await flushAsync();
    await flushAsync();

    expect(text("cards-length")).toBe("21");
    expect(text("conversation-keys")).toContain("pck-head-1");
    expect(text("conversation-keys")).toContain("pck-tail-1");
    expect(text("has-more")).toBe("false");
  });

  it("realigns nextCursor to the refreshed snapshot after a head resync", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => ({
            cursor: `cursor-head-${index + 1}`,
            createdAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(59 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-old-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-tail", 5, (index) => ({
            cursor: `cursor-tail-${index + 1}`,
            createdAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T02:${String(39 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-old-2",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head-new", 20, (index) => ({
            cursor: `cursor-new-head-${index + 1}`,
            createdAt: `2026-04-10T03:${String(19 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: `2026-04-10T03:${String(19 - index).padStart(2, "0")}:00Z`,
            lastTerminalAt: `2026-04-10T03:${String(19 - index).padStart(2, "0")}:00Z`,
          })),
          {
            totalMatched: 26,
            hasMore: true,
            nextCursor: "cursor-new-1",
            snapshotAt: "2026-04-10T02:06:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    click("load-more");
    await flushAsync();
    expect(text("next-cursor")).toBe("cursor-old-2");

    act(() => {
      emitOpen();
    });
    await flushAsync();
    await flushAsync();

    expect(text("next-cursor")).toBe("cursor-new-1");
    expect(text("has-more")).toBe("true");
  });

  it("recomputes hasMore and nextCursor when a head refresh lands in the same snapshot second", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head-old", 20, (index) => ({
            cursor: `cursor-old-head-${index + 1}`,
            createdAt: `2026-04-10T02:04:${String(59 - index).padStart(2, "0")}Z`,
            lastActivityAt: `2026-04-10T02:04:${String(59 - index).padStart(2, "0")}Z`,
            lastTerminalAt: `2026-04-10T02:04:${String(59 - index).padStart(2, "0")}Z`,
          })),
          {
            totalMatched: 21,
            hasMore: true,
            nextCursor: "cursor-old-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head-new", 20, (index) => ({
            cursor: `cursor-new-head-${index + 1}`,
            createdAt: `2026-04-10T02:04:${String(39 - index).padStart(2, "0")}Z`,
            lastActivityAt: `2026-04-10T02:04:${String(39 - index).padStart(2, "0")}Z`,
            lastTerminalAt: `2026-04-10T02:04:${String(39 - index).padStart(2, "0")}Z`,
          })),
          {
            totalMatched: 20,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();
    expect(text("has-more")).toBe("true");
    expect(text("next-cursor")).toBe("cursor-old-page-1");

    act(() => {
      emitOpen();
    });
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);
    expect(text("cards-length")).toBe("20");
    expect(text("has-more")).toBe("false");
    expect(text("next-cursor")).toBe("");
    expect(text("conversation-keys")).toContain("pck-head-new-1");
    expect(text("conversation-keys")).not.toContain("pck-head-old-1");
  });

  it("preserves loaded tail rows while a newer snapshot backfills fresh pages", async () => {
    const refreshedTailPage = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [createConversation("pck-head-old", { cursor: "cursor-old-1" })],
          {
            totalMatched: 3,
            hasMore: true,
            nextCursor: "cursor-old-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-tail-overlap", {
              cursor: "cursor-old-2",
              requestCount: 1,
              totalTokens: 30,
              totalCost: 0.12,
              recentInvocations: [
                createPreview({
                  id: 11,
                  invokeId: "invoke-tail-stale",
                  occurredAt: "2026-04-10T02:03:00Z",
                  status: "completed",
                  totalTokens: 30,
                  cost: 0.12,
                }),
              ],
              lastActivityAt: "2026-04-10T02:03:00Z",
              lastTerminalAt: "2026-04-10T02:03:00Z",
            }),
          ],
          {
            totalMatched: 3,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [createConversation("pck-head-new", { cursor: "cursor-new-1" })],
          {
            totalMatched: 3,
            hasMore: true,
            nextCursor: "cursor-new-1",
            snapshotAt: "2026-04-10T02:06:00Z",
          },
        ),
      )
      .mockImplementationOnce(() => refreshedTailPage.promise);

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    click("load-more");
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-head-old,pck-tail-overlap");
    expect(text("conversation-summary")).toContain(
      "pck-tail-overlap:invoke-tail-stale:completed:1:",
    );

    act(() => {
      emitOpen();
    });
    await flushAsync();

    expect(text("conversation-keys")).toContain("pck-head-new");
    expect(text("conversation-keys")).toContain("pck-head-old");
    expect(text("conversation-keys")).toContain("pck-tail-overlap");
    expect(text("conversation-summary")).toContain(
      "pck-tail-overlap:invoke-tail-stale:completed:1:",
    );
    expect(text("next-cursor")).toBe("cursor-new-1");

    refreshedTailPage.resolve(
      createResponseWithConversations(
        [
          createConversation("pck-tail-overlap", {
            cursor: "cursor-new-2",
            requestCount: 2,
            totalTokens: 75,
            totalCost: 0.3,
            recentInvocations: [
              createPreview({
                id: 12,
                invokeId: "invoke-tail-fresh",
                occurredAt: "2026-04-10T02:05:30Z",
                status: "running",
                totalTokens: 45,
                cost: 0.18,
              }),
            ],
            lastActivityAt: "2026-04-10T02:05:30Z",
            lastTerminalAt: "2026-04-10T02:03:00Z",
            lastInFlightAt: "2026-04-10T02:05:30Z",
          }),
        ],
        {
          totalMatched: 3,
          hasMore: false,
          nextCursor: null,
          snapshotAt: "2026-04-10T02:06:00Z",
        },
      ),
    );
    await flushAsync();
    await flushAsync();

    expect(text("conversation-keys")).toContain("pck-head-new");
    expect(text("conversation-keys")).toContain("pck-tail-overlap");
    expect(text("conversation-summary")).toContain(
      "pck-tail-overlap:invoke-tail-fresh:running:2:2026-04-10T02:05:30Z",
    );
    expect(text("conversation-summary")).not.toContain(
      "pck-tail-overlap:invoke-tail-stale:completed:1:",
    );
  });

  it("shrinks future snapshot backfills to the current refresh target instead of the deepest historical load", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          Array.from({ length: 20 }, (_, index) =>
            createConversation(`pck-old-head-${index + 1}`, {
              cursor: `cursor-old-head-${index + 1}`,
              createdAt: `2026-04-10T02:59:${String(59 - index).padStart(2, "0")}Z`,
              lastActivityAt: `2026-04-10T02:59:${String(59 - index).padStart(2, "0")}Z`,
              lastTerminalAt: `2026-04-10T02:59:${String(59 - index).padStart(2, "0")}Z`,
            }),
          ),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-old-page-1",
            snapshotAt: "2026-04-10T03:00:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          Array.from({ length: 5 }, (_, index) =>
            createConversation(`pck-old-tail-${index + 1}`, {
              cursor: `cursor-old-tail-${index + 1}`,
              createdAt: `2026-04-10T02:58:${String(59 - index).padStart(2, "0")}Z`,
              lastActivityAt: `2026-04-10T02:58:${String(59 - index).padStart(2, "0")}Z`,
              lastTerminalAt: `2026-04-10T02:58:${String(59 - index).padStart(2, "0")}Z`,
            }),
          ),
          {
            totalMatched: 25,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T03:00:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          Array.from({ length: 20 }, (_, index) =>
            createConversation(`pck-new-head-${index + 1}`, {
              cursor: `cursor-new-head-${index + 1}`,
              createdAt: `2026-04-10T03:04:${String(59 - index).padStart(2, "0")}Z`,
              lastActivityAt: `2026-04-10T03:04:${String(59 - index).padStart(2, "0")}Z`,
              lastTerminalAt: `2026-04-10T03:04:${String(59 - index).padStart(2, "0")}Z`,
            }),
          ),
          {
            totalMatched: 25,
            hasMore: true,
            nextCursor: "cursor-new-page-1",
            snapshotAt: "2026-04-10T03:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    click("load-more");
    await flushAsync();
    await flushAsync();
    expect(text("cards-length")).toBe("25");

    click("set-refresh-target-25");
    click("set-refresh-target-20");

    act(() => {
      emitOpen();
    });
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(3);
    expect(text("cards-length")).toBe("20");
    expect(text("conversation-keys")).toContain("pck-new-head-1");
    expect(text("conversation-keys")).not.toContain("pck-old-tail-1");
    expect(text("next-cursor")).toBe("cursor-new-page-1");
    expect(text("has-more")).toBe("true");
  });

  it("patches loaded conversations from records SSE without forcing a refetch", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-live", {
          recentInvocations: [
            createPreview({
              id: 1,
              invokeId: "invoke-1",
              occurredAt: "2026-04-10T02:04:00Z",
              status: "completed",
              totalTokens: 30,
              cost: 0.12,
            }),
          ],
          requestCount: 1,
          totalTokens: 30,
          totalCost: 0.12,
          lastTerminalAt: "2026-04-10T02:04:00Z",
          lastInFlightAt: null,
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 2,
          invokeId: "invoke-2",
          occurredAt: "2026-04-10T02:59:30Z",
          status: "running",
          totalTokens: 0,
          cost: 0,
          proxyDisplayName: "proxy-live",
        }),
      ]);
    });

    expect(text("preview-invoke-id")).toBe("invoke-2");
    expect(text("preview-status")).toBe("running");
    expect(text("request-count")).toBe("2");
    expect(text("last-in-flight-at")).toBe("2026-04-10T02:59:30Z");
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(1);
  });

  it("resyncs the head page when a loaded key receives a hidden pre-snapshot update", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-live", {
            recentInvocations: [
              createPreview({
                id: 4,
                invokeId: "invoke-4",
                occurredAt: "2026-04-10T02:04:00Z",
                status: "completed",
                totalTokens: 55,
                cost: 0.22,
              }),
              createPreview({
                id: 3,
                invokeId: "invoke-3",
                occurredAt: "2026-04-10T02:03:00Z",
                status: "completed",
                totalTokens: 45,
                cost: 0.18,
              }),
            ],
            requestCount: 4,
            totalTokens: 140,
            totalCost: 0.56,
            lastActivityAt: "2026-04-10T02:04:00Z",
            lastTerminalAt: "2026-04-10T02:04:00Z",
            lastInFlightAt: null,
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-live", {
            recentInvocations: [
              createPreview({
                id: 4,
                invokeId: "invoke-4",
                occurredAt: "2026-04-10T02:04:00Z",
                status: "completed",
                totalTokens: 55,
                cost: 0.22,
              }),
              createPreview({
                id: 3,
                invokeId: "invoke-3",
                occurredAt: "2026-04-10T02:03:00Z",
                status: "completed",
                totalTokens: 45,
                cost: 0.18,
              }),
            ],
            requestCount: 4,
            totalTokens: 140,
            totalCost: 0.56,
            lastActivityAt: "2026-04-10T02:04:00Z",
            lastTerminalAt: "2026-04-10T02:04:00Z",
            lastInFlightAt: "2026-04-10T02:02:00Z",
          }),
        ]),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 2,
          invokeId: "invoke-2",
          occurredAt: "2026-04-10T02:02:00Z",
          status: "running",
          totalTokens: 0,
          cost: 0,
        }),
      ]);
    });
    await flushAsync();
    await flushAsync();

    expect(text("request-count")).toBe("4");
    expect(text("preview-invoke-id")).toBe("invoke-4");
    expect(text("last-in-flight-at")).toBe("2026-04-10T02:02:00Z");
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);
  });

  it("treats late-persisted same-second SSE records as post-snapshot aggregate updates without forcing a refetch", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-live", {
            requestCount: 3,
            totalTokens: 120,
            totalCost: 0.48,
            recentInvocations: [
              createPreview({
                id: 5,
                invokeId: "invoke-5",
                occurredAt: "2026-04-10T02:05:00Z",
                status: "completed",
                totalTokens: 50,
                cost: 0.2,
              }),
              createPreview({
                id: 4,
                invokeId: "invoke-4",
                occurredAt: "2026-04-10T02:05:00Z",
                status: "completed",
                totalTokens: 40,
                cost: 0.16,
              }),
            ],
            lastActivityAt: "2026-04-10T02:05:00Z",
            lastTerminalAt: "2026-04-10T02:05:00Z",
            lastInFlightAt: null,
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-live", {
            requestCount: 3,
            totalTokens: 120,
            totalCost: 0.48,
            recentInvocations: [
              createPreview({
                id: 5,
                invokeId: "invoke-5",
                occurredAt: "2026-04-10T02:05:00Z",
                status: "completed",
                totalTokens: 50,
                cost: 0.2,
              }),
              createPreview({
                id: 4,
                invokeId: "invoke-4",
                occurredAt: "2026-04-10T02:05:00Z",
                status: "completed",
                totalTokens: 40,
                cost: 0.16,
              }),
            ],
            lastActivityAt: "2026-04-10T02:05:00Z",
            lastTerminalAt: "2026-04-10T02:05:00Z",
            lastInFlightAt: null,
          }),
        ]),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 3,
          invokeId: "invoke-3",
          occurredAt: "2026-04-10T02:05:00Z",
          createdAt: "2026-04-10T02:05:00.200Z",
          status: "completed",
          totalTokens: 30,
          cost: 0.12,
        }),
      ]);
    });
    await flushAsync();
    await flushAsync();

    expect(text("preview-invoke-id")).toBe("invoke-5");
    expect(text("request-count")).toBe("4");
    expect(text("total-tokens")).toBe("150");
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(1);
  });

  it("clears lastInFlightAt once no visible in-flight preview remains", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-10T03:12:00Z"));
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-live", {
          recentInvocations: [
            createPreview({
              id: 2,
              invokeId: "invoke-visible-running",
              occurredAt: "2026-04-10T03:07:00Z",
              status: "running",
              totalTokens: 0,
              cost: 0,
            }),
            createPreview({
              id: 1,
              invokeId: "invoke-terminal",
              occurredAt: "2026-04-10T03:06:00Z",
              status: "completed",
              totalTokens: 30,
              cost: 0.12,
            }),
          ],
          lastActivityAt: "2026-04-10T03:07:00Z",
          lastTerminalAt: "2026-04-10T03:06:00Z",
          lastInFlightAt: "2026-04-10T03:07:00Z",
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-live");

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 3,
          invokeId: "invoke-visible-running",
          occurredAt: "2026-04-10T03:07:00Z",
          status: "completed",
          totalTokens: 40,
          cost: 0.16,
        }),
      ]);
    });

    expect(text("conversation-keys")).toBe("pck-live");
    expect(text("last-terminal-at")).toBe("2026-04-10T03:07:00Z");
    expect(text("last-in-flight-at")).toBe("");
  });

  it("preserves lastInFlightAt when compact previews still hide a running record update", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-live", {
          requestCount: 4,
          recentInvocations: [
            createPreview({
              id: 4,
              invokeId: "invoke-visible-completed-newer",
              occurredAt: "2026-04-10T03:09:00Z",
              status: "completed",
              totalTokens: 60,
              cost: 0.2,
            }),
            createPreview({
              id: 3,
              invokeId: "invoke-visible-completed-older",
              occurredAt: "2026-04-10T03:08:00Z",
              status: "completed",
              totalTokens: 50,
              cost: 0.18,
            }),
          ],
          lastActivityAt: "2026-04-10T03:09:00Z",
          lastTerminalAt: "2026-04-10T03:09:00Z",
          lastInFlightAt: "2026-04-10T03:07:00Z",
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 5,
          invokeId: "invoke-hidden-running",
          occurredAt: "2026-04-10T03:07:00Z",
          status: "running",
          totalTokens: 0,
          cost: 0,
        }),
      ]);
    });

    expect(text("conversation-keys")).toBe("pck-live");
    expect(text("preview-status")).toBe("completed");
    expect(text("last-in-flight-at")).toBe("2026-04-10T03:07:00Z");
  });

  it("preserves hidden lastInFlightAt when a newer terminal preview lands", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-live", {
          requestCount: 4,
          recentInvocations: [
            createPreview({
              id: 4,
              invokeId: "invoke-visible-completed-newer",
              occurredAt: "2026-04-10T03:09:00Z",
              status: "completed",
              totalTokens: 60,
              cost: 0.2,
            }),
            createPreview({
              id: 3,
              invokeId: "invoke-visible-completed-older",
              occurredAt: "2026-04-10T03:08:00Z",
              status: "completed",
              totalTokens: 50,
              cost: 0.18,
            }),
          ],
          lastActivityAt: "2026-04-10T03:09:00Z",
          lastTerminalAt: "2026-04-10T03:09:00Z",
          lastInFlightAt: "2026-04-10T03:07:00Z",
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 5,
          invokeId: "invoke-visible-completed-latest",
          occurredAt: "2026-04-10T03:10:00Z",
          status: "completed",
          totalTokens: 70,
          cost: 0.24,
        }),
      ]);
    });

    expect(text("conversation-keys")).toBe("pck-live");
    expect(text("preview-invoke-id")).toBe("invoke-visible-completed-latest");
    expect(text("last-terminal-at")).toBe("2026-04-10T03:10:00Z");
    expect(text("last-in-flight-at")).toBe("2026-04-10T03:07:00Z");
  });

  it("clears hidden lastInFlightAt when a compact-hidden running invoke completes", async () => {
    apiMocks.fetchPromptCacheConversationsPage.mockResolvedValueOnce(
      createResponseWithConversations([
        createConversation("pck-live", {
          requestCount: 4,
          recentInvocations: [
            createPreview({
              id: 4,
              invokeId: "invoke-visible-completed-newer",
              occurredAt: "2026-04-10T03:09:00Z",
              status: "completed",
              totalTokens: 60,
              cost: 0.2,
            }),
            createPreview({
              id: 3,
              invokeId: "invoke-visible-completed-older",
              occurredAt: "2026-04-10T03:08:00Z",
              status: "completed",
              totalTokens: 50,
              cost: 0.18,
            }),
          ],
          lastActivityAt: "2026-04-10T03:09:00Z",
          lastTerminalAt: "2026-04-10T03:09:00Z",
          lastInFlightAt: "2026-04-10T03:07:00Z",
        }),
      ]),
    );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-live", {
          id: 5,
          invokeId: "invoke-hidden-running",
          occurredAt: "2026-04-10T03:07:00Z",
          status: "completed",
          totalTokens: 40,
          cost: 0.16,
        }),
      ]);
    });

    expect(text("conversation-keys")).toBe("pck-live");
    expect(text("last-in-flight-at")).toBe("");
  });

  it("keeps the server nextCursor pinned after SSE reorders loaded rows", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-head", {
              cursor: "cursor-row-head",
              createdAt: "2026-04-10T02:59:00Z",
              lastActivityAt: "2026-04-10T02:59:00Z",
              lastTerminalAt: "2026-04-10T02:59:00Z",
            }),
            createConversation("pck-tail", {
              cursor: "cursor-row-tail",
              createdAt: "2026-04-10T02:58:00Z",
              lastActivityAt: "2026-04-10T02:58:00Z",
              lastTerminalAt: "2026-04-10T02:58:00Z",
            }),
            ...createConversationBatch("pck-filler", 18, (index) => ({
              cursor: `cursor-row-filler-${index + 1}`,
              createdAt: `2026-04-10T02:${String(57 - index).padStart(2, "0")}:00Z`,
              lastActivityAt: `2026-04-10T02:${String(57 - index).padStart(2, "0")}:00Z`,
              lastTerminalAt: `2026-04-10T02:${String(57 - index).padStart(2, "0")}:00Z`,
            })),
          ],
          {
            totalMatched: 21,
            hasMore: true,
            nextCursor: "cursor-page-1",
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [createConversation("pck-after", { cursor: "cursor-row-after" })],
          {
            totalMatched: 21,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T02:05:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();
    expect(text("next-cursor")).toBe("cursor-page-1");

    act(() => {
      emitRecords([
        createRecord("pck-tail", {
          id: 11,
          invokeId: "invoke-tail-live",
          occurredAt: "2026-04-10T03:05:30Z",
          status: "running",
          totalTokens: 0,
          cost: 0,
        }),
      ]);
    });

    expect(text("conversation-keys")).toContain("pck-tail,pck-head");
    expect(text("next-cursor")).toBe("cursor-page-1");

    click("load-more");
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenLastCalledWith(
      { mode: "activityWindow", activityMinutes: 5 },
      expect.objectContaining({
        cursor: "cursor-page-1",
        snapshotAt: "2026-04-10T02:05:00Z",
        detail: "compact",
        signal: expect.any(AbortSignal),
      }),
    );
  });

  it("throttles unseen-key refreshes into at most one extra head resync while a fetch is in flight", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-10T03:00:00Z"));
    const refresh = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-base", {
            cursor: "cursor-base",
            createdAt: "2026-04-10T02:00:00Z",
            lastActivityAt: "2026-04-10T02:59:00Z",
            lastTerminalAt: "2026-04-10T02:59:00Z",
          }),
        ]),
      )
      .mockImplementationOnce(async () => refresh.promise)
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-newer", {
              cursor: "cursor-newer",
              createdAt: "2026-04-10T02:03:00Z",
              lastActivityAt: "2026-04-10T02:59:45Z",
              lastTerminalAt: "2026-04-10T02:59:45Z",
            }),
            createConversation("pck-base", {
              cursor: "cursor-base",
              createdAt: "2026-04-10T02:00:00Z",
              lastActivityAt: "2026-04-10T02:59:00Z",
              lastTerminalAt: "2026-04-10T02:59:00Z",
            }),
          ],
          {
            totalMatched: 3,
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitRecords([
        createRecord("pck-new", {
          id: 11,
          invokeId: "invoke-new-1",
          occurredAt: "2026-04-10T02:59:30Z",
          status: "running",
        }),
      ]);
    });
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);

    act(() => {
      emitRecords([
        createRecord("pck-newer", {
          id: 12,
          invokeId: "invoke-new-2",
          occurredAt: "2026-04-10T02:59:45Z",
          status: "running",
        }),
      ]);
    });
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);

    act(() => {
      vi.advanceTimersByTime(1_499);
    });
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);

    refresh.resolve(
      createResponseWithConversations([
        createConversation("pck-new", {
          cursor: "cursor-new",
          createdAt: "2026-04-10T02:02:30Z",
          lastActivityAt: "2026-04-10T02:59:30Z",
          lastTerminalAt: null,
          lastInFlightAt: "2026-04-10T02:59:30Z",
        }),
        createConversation("pck-base", {
          cursor: "cursor-base",
          createdAt: "2026-04-10T02:00:00Z",
          lastActivityAt: "2026-04-10T02:59:00Z",
          lastTerminalAt: "2026-04-10T02:59:00Z",
        }),
      ]),
    );
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(3);
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-newer,pck-new,pck-base");
  });

  it("stops exposing hasMore once the backend reports the last page for a fixed snapshot", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-10T03:00:00Z"));
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          createConversationBatch("pck-head", 20, (index) => ({
            cursor: `cursor-head-${index + 1}`,
            createdAt: `2026-04-10T03:${String(19 - index).padStart(2, "0")}:00Z`,
            lastActivityAt: "2026-04-10T03:00:00Z",
            lastTerminalAt: "2026-04-10T03:00:00Z",
          })),
          {
            totalMatched: 21,
            hasMore: true,
            nextCursor: "cursor-1",
            snapshotAt: "2026-04-10T03:00:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-tail", {
              cursor: "cursor-2",
              lastActivityAt: "2026-04-10T02:59:00Z",
              lastTerminalAt: "2026-04-10T02:59:00Z",
            }),
          ],
          {
            totalMatched: 21,
            hasMore: false,
            nextCursor: null,
            snapshotAt: "2026-04-10T03:00:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    vi.setSystemTime(new Date("2026-04-10T03:07:00Z"));
    click("load-more");
    await flushAsync();

    expect(text("cards-length")).toBe("21");
    expect(text("has-more")).toBe("false");
    expect(text("next-cursor")).toBe("");
  });

  it("resyncs on SSE open and suppresses duplicate reconnect refreshes inside the cooldown window", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-10T03:00:00Z"));
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-open", {
            lastActivityAt: "2026-04-10T02:59:00Z",
            lastTerminalAt: "2026-04-10T02:59:00Z",
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-open", {
            recentInvocations: [
              createPreview({
                id: 2,
                invokeId: "invoke-open-2",
                occurredAt: "2026-04-10T02:59:30Z",
                status: "completed",
              }),
            ],
            lastActivityAt: "2026-04-10T02:59:30Z",
            lastTerminalAt: "2026-04-10T02:59:30Z",
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-open", {
            recentInvocations: [
              createPreview({
                id: 3,
                invokeId: "invoke-open-3",
                occurredAt: "2026-04-10T02:59:45Z",
                status: "completed",
              }),
            ],
            lastActivityAt: "2026-04-10T02:59:45Z",
            lastTerminalAt: "2026-04-10T02:59:45Z",
          }),
        ]),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    act(() => {
      emitOpen();
    });
    await flushAsync();

    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);
    expect(text("preview-invoke-id")).toBe("invoke-open-2");

    act(() => {
      vi.advanceTimersByTime(2_999);
      emitOpen();
    });
    await flushAsync();
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(2);

    act(() => {
      vi.advanceTimersByTime(1);
      emitOpen();
    });
    await flushAsync();
    expect(apiMocks.fetchPromptCacheConversationsPage).toHaveBeenCalledTimes(3);
    expect(text("preview-invoke-id")).toBe("invoke-open-3");
  });

  it("keeps cached cards visible when a background head refresh fails", async () => {
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-cached", {
            lastActivityAt: "2026-04-10T02:59:00Z",
            lastTerminalAt: "2026-04-10T02:59:00Z",
          }),
        ]),
      )
      .mockRejectedValueOnce(new Error("temporary outage"));

    render(<Probe />);
    await flushAsync();
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-cached");

    act(() => {
      emitOpen();
    });
    await flushAsync();
    await flushAsync();

    expect(text("conversation-keys")).toBe("pck-cached");
    expect(text("cards-length")).toBe("1");
    expect(text("error")).toBe("temporary outage");
  });

  it("prunes stale terminal conversations after a resync", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-10T03:00:00Z"));
    apiMocks.fetchPromptCacheConversationsPage
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-stale", {
              lastInFlightAt: "2026-04-10T02:50:00Z",
              lastActivityAt: "2026-04-10T02:56:00Z",
              lastTerminalAt: "2026-04-10T02:56:00Z",
            }),
          ],
          {
            snapshotAt: "2026-04-10T03:00:00Z",
          },
        ),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-stale", {
              lastInFlightAt: null,
              lastActivityAt: "2026-04-10T02:56:00Z",
              lastTerminalAt: "2026-04-10T02:56:00Z",
            }),
          ],
          {
            snapshotAt: "2026-04-10T03:07:00Z",
          },
        ),
      );

    render(<Probe />);
    await flushAsync();
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-stale");

    vi.setSystemTime(new Date("2026-04-10T03:07:00Z"));
    act(() => {
      emitOpen();
    });
    await flushAsync();

    expect(text("conversation-keys")).toBe("");
    expect(text("cards-length")).toBe("0");
  });
});
