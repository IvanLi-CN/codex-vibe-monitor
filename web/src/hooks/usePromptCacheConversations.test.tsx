/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  BroadcastPayload,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationSelection,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { usePromptCacheConversations } from "./usePromptCacheConversations";

const apiMocks = vi.hoisted(() => ({
  fetchPromptCacheConversations:
    vi.fn<
      (
        selection: PromptCacheConversationSelection,
        signal?: AbortSignal,
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
    fetchPromptCacheConversations: apiMocks.fetchPromptCacheConversations,
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

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
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
  vi.useRealTimers();
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

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

async function rerenderAsync(ui: React.ReactNode) {
  await act(async () => {
    root?.render(ui);
    await Promise.resolve();
    await Promise.resolve();
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
  };
}

function createResponse(
  promptCacheKey: string,
  preview: PromptCacheConversationInvocationPreview[] = [],
): PromptCacheConversationsResponse {
  return createResponseWithConversations([
    {
      promptCacheKey,
      requestCount: 1,
      totalTokens: 30,
      totalCost: 0.12,
      createdAt: "2026-03-10T01:00:00Z",
      lastActivityAt: "2026-03-10T02:00:00Z",
      upstreamAccounts: [],
      recentInvocations: preview,
      last24hRequests: [],
    },
  ]);
}

function createConversation(
  promptCacheKey: string,
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: overrides.requestCount ?? 1,
    totalTokens: overrides.totalTokens ?? 30,
    totalCost: overrides.totalCost ?? 0.12,
    createdAt: overrides.createdAt ?? "2026-03-10T01:00:00Z",
    lastActivityAt: overrides.lastActivityAt ?? "2026-03-10T02:00:00Z",
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations: overrides.recentInvocations ?? [],
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

function createResponseWithConversations(
  conversations: PromptCacheConversation[],
  selectedLimit = 50,
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-03-10T00:00:00Z",
    rangeEnd: "2026-03-11T00:00:00Z",
    selectionMode: "count",
    selectedLimit,
    selectedActivityHours: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
  };
}

function Probe({ selection }: { selection: PromptCacheConversationSelection }) {
  const { stats, isLoading, error } = usePromptCacheConversations(selection);

  return (
    <div>
      <div data-testid="prompt-cache-key">
        {stats?.conversations[0]?.promptCacheKey ?? ""}
      </div>
      <div data-testid="conversation-keys">
        {stats?.conversations.map((item) => item.promptCacheKey).join(",") ?? ""}
      </div>
      <div data-testid="preview-invoke-id">
        {stats?.conversations[0]?.recentInvocations[0]?.invokeId ?? ""}
      </div>
      <div data-testid="preview-status">
        {stats?.conversations[0]?.recentInvocations[0]?.status ?? ""}
      </div>
      <div data-testid="preview-count">
        {String(stats?.conversations[0]?.recentInvocations.length ?? 0)}
      </div>
      <div data-testid="preview-service-tier">
        {stats?.conversations[0]?.recentInvocations[0]?.requestedServiceTier ?? ""}
      </div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
    </div>
  );
}

describe("usePromptCacheConversations", () => {
  it("ignores stale responses after the selection mode switches", async () => {
    const first = deferred<PromptCacheConversationsResponse>();
    const second = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversations
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe selection={{ mode: "count", limit: 50 }} />);
    expect(text("loading")).toBe("true");

    rerender(
      <Probe selection={{ mode: "activityWindow", activityHours: 3 }} />,
    );
    await flushAsync();

    second.resolve(createResponse("pck-window"));
    await flushAsync();
    expect(text("prompt-cache-key")).toBe("pck-window");

    first.resolve(createResponse("pck-count"));
    await flushAsync();
    expect(text("prompt-cache-key")).toBe("pck-window");
    expect(text("error")).toBe("");
  });

  it("applies live prompt cache records immediately and reconciles them after refetch", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-10T03:00:00Z"));
    const refresh = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversations
      .mockResolvedValueOnce(createResponse("pck-live"))
      .mockImplementationOnce(async () => refresh.promise);

    render(<Probe selection={{ mode: "count", limit: 50 }} />);
    await flushAsync();

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({
          type: "records",
          records: [
            {
              id: 901,
              invokeId: "invoke-live-01",
              occurredAt: "2026-03-10T02:30:00Z",
              createdAt: "2026-03-10T02:30:00Z",
              status: "running",
              promptCacheKey: "pck-live",
              totalTokens: 0,
              cost: 0,
              proxyDisplayName: "Proxy Running",
            },
          ],
        });
      });
    });

    expect(text("preview-invoke-id")).toBe("invoke-live-01");
    expect(text("preview-status")).toBe("running");
    expect(text("preview-count")).toBe("1");

    await act(async () => {
      sseMocks.listeners.forEach((listener) => {
        listener({
          type: "records",
          records: [
            {
              id: 901,
              invokeId: "invoke-live-01",
              occurredAt: "2026-03-10T02:30:00Z",
              createdAt: "2026-03-10T02:30:00Z",
              status: "completed",
              promptCacheKey: "pck-live",
              totalTokens: 182491,
              cost: 0.0484,
              proxyDisplayName: "Proxy Final",
              requestedServiceTier: "auto",
            },
          ],
        });
      });
      refresh.resolve(
        createResponse("pck-live", [
          createPreview({
            id: 901,
            invokeId: "invoke-live-01",
            occurredAt: "2026-03-10T02:30:00Z",
            status: "completed",
            totalTokens: 182491,
            cost: 0.0484,
            proxyDisplayName: "Proxy Final",
            upstreamAccountId: 17,
            upstreamAccountName: "pool-account-17",
          }),
        ]),
      );
      await refresh.promise;
    });
    await flushAsync();

    expect(text("preview-invoke-id")).toBe("invoke-live-01");
    expect(text("preview-status")).toBe("completed");
    expect(text("preview-count")).toBe("1");
    expect(text("preview-service-tier")).toBe("auto");
    expect(apiMocks.fetchPromptCacheConversations).toHaveBeenCalledTimes(2);
  });

  it("keeps a completed unseen live key visible while the initial load resolves stale data", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-10T03:00:00Z"));
    const initial = deferred<PromptCacheConversationsResponse>();
    const refresh = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversations
      .mockImplementationOnce(async () => initial.promise)
      .mockImplementationOnce(async () => refresh.promise);

    render(<Probe selection={{ mode: "count", limit: 50 }} />);
    await flushAsync();

    await act(async () => {
      vi.advanceTimersByTime(1_000);
      sseMocks.listeners.forEach((listener) => {
        listener({
          type: "records",
          records: [
            {
              id: 903,
              invokeId: "invoke-live-completed",
              occurredAt: "2026-03-10T02:30:00Z",
              createdAt: "2026-03-10T02:30:00Z",
              status: "completed",
              promptCacheKey: "pck-live-completed",
              totalTokens: 182491,
              cost: 0.0484,
              proxyDisplayName: "Proxy Final",
            },
          ],
        });
      });
    });

    initial.resolve(createResponse("pck-base"));
    await flushAsync();

    expect(text("conversation-keys")).toBe("pck-live-completed,pck-base");
    expect(text("preview-invoke-id")).toBe("invoke-live-completed");
    expect(text("preview-status")).toBe("completed");
    expect(apiMocks.fetchPromptCacheConversations).toHaveBeenCalledTimes(2);

    refresh.resolve(
      createResponseWithConversations([
        createConversation("pck-live-completed", {
          createdAt: "2026-03-10T02:30:00Z",
          lastActivityAt: "2026-03-10T02:30:00Z",
          recentInvocations: [
            createPreview({
              id: 903,
              invokeId: "invoke-live-completed",
              occurredAt: "2026-03-10T02:30:00Z",
              status: "completed",
              totalTokens: 182491,
              cost: 0.0484,
              proxyDisplayName: "Proxy Final",
            }),
          ],
        }),
        createConversation("pck-base"),
      ]),
    );
    await flushAsync();

    expect(text("conversation-keys")).toBe("pck-live-completed,pck-base");
    expect(text("preview-invoke-id")).toBe("invoke-live-completed");
    expect(text("preview-status")).toBe("completed");
  });

  it("forces an immediate refetch when an unseen live key arrives while the table is full", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-10T03:00:00Z"));
    const refresh = deferred<PromptCacheConversationsResponse>();
    apiMocks.fetchPromptCacheConversations
      .mockResolvedValueOnce(
        createResponseWithConversations(
          [
            createConversation("pck-newest", {
              createdAt: "2026-03-10T02:00:00Z",
              lastActivityAt: "2026-03-10T02:00:00Z",
            }),
            createConversation("pck-older", {
              createdAt: "2026-03-10T01:00:00Z",
              lastActivityAt: "2026-03-10T01:00:00Z",
            }),
          ],
          2,
        ),
      )
      .mockImplementationOnce(async () => refresh.promise);

    render(<Probe selection={{ mode: "count", limit: 2 }} />);
    await flushAsync();
    expect(text("conversation-keys")).toBe("pck-newest,pck-older");

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({
          type: "records",
          records: [
            {
              id: 902,
              invokeId: "invoke-live-hidden",
              occurredAt: "2026-03-10T02:30:00Z",
              createdAt: "2026-03-10T02:30:00Z",
              status: "running",
              promptCacheKey: "pck-live-hidden",
              totalTokens: 0,
              cost: 0,
            },
          ],
        });
      });
    });

    expect(text("conversation-keys")).toBe("pck-newest,pck-older");
    expect(apiMocks.fetchPromptCacheConversations).toHaveBeenCalledTimes(2);

    refresh.resolve(
      createResponseWithConversations(
        [
          createConversation("pck-live-hidden", {
            createdAt: "2026-03-10T02:30:00Z",
            lastActivityAt: "2026-03-10T02:30:00Z",
            recentInvocations: [
              createPreview({
                id: 902,
                invokeId: "invoke-live-hidden",
                occurredAt: "2026-03-10T02:30:00Z",
                status: "completed",
              }),
            ],
          }),
          createConversation("pck-newest", {
            createdAt: "2026-03-10T02:00:00Z",
            lastActivityAt: "2026-03-10T02:00:00Z",
          }),
        ],
        2,
      ),
    );
    await flushAsync();

    expect(text("conversation-keys")).toBe("pck-live-hidden,pck-newest");
  });

  it("clears the prior working set when the selection changes", async () => {
    const now = Date.now();
    const countOccurredAt = new Date(now - 20 * 60_000).toISOString();
    const hiddenOccurredAt = new Date(now - 10 * 60_000).toISOString();
    const windowOccurredAt = new Date(now - 5 * 60_000).toISOString();

    apiMocks.fetchPromptCacheConversations
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-count", {
            createdAt: countOccurredAt,
            lastActivityAt: countOccurredAt,
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-count", {
            createdAt: countOccurredAt,
            lastActivityAt: countOccurredAt,
          }),
        ]),
      )
      .mockResolvedValueOnce(
        createResponseWithConversations([
          createConversation("pck-window", {
            createdAt: windowOccurredAt,
            lastActivityAt: windowOccurredAt,
          }),
        ]),
      );

    render(<Probe selection={{ mode: "count", limit: 50 }} />);
    await flushAsync();

    act(() => {
      sseMocks.listeners.forEach((listener) => {
        listener({
          type: "records",
          records: [
            {
              id: 904,
              invokeId: "invoke-selection-hidden",
              occurredAt: hiddenOccurredAt,
              createdAt: hiddenOccurredAt,
              status: "running",
              promptCacheKey: "pck-selection-hidden",
              totalTokens: 200,
              cost: 0.02,
            },
          ],
        });
      });
    });

    expect(text("conversation-keys")).toBe("pck-selection-hidden,pck-count");

    await rerenderAsync(
      <Probe selection={{ mode: "activityWindow", activityHours: 3 }} />,
    );
    await flushAsync();

    expect(text("conversation-keys")).toBe("pck-window");
    expect(text("conversation-keys")).not.toContain("pck-selection-hidden");
    expect(text("conversation-keys")).not.toContain("pck-count");
  });
});
