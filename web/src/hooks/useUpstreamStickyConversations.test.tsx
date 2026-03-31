/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  StickyKeyConversationSelection,
  UpstreamStickyConversationsResponse,
} from "../lib/api";
import {
  UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS,
  UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS,
  getUpstreamStickySseRefreshDelay,
  shouldTriggerUpstreamStickyOpenResync,
  useUpstreamStickyConversations,
} from "./useUpstreamStickyConversations";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamStickyConversations:
    vi.fn<
      (
        accountId: number,
        selection: StickyKeyConversationSelection,
        signal?: AbortSignal,
      ) => Promise<UpstreamStickyConversationsResponse>
    >(),
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamStickyConversations: apiMocks.fetchUpstreamStickyConversations,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: () => () => {},
  subscribeToSseOpen: () => () => {},
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

function createResponse(
  stickyKey: string,
): UpstreamStickyConversationsResponse {
  return {
    rangeStart: "2026-03-10T00:00:00Z",
    rangeEnd: "2026-03-11T00:00:00Z",
    selectionMode: "count",
    selectedLimit: 20,
    selectedActivityHours: null,
    implicitFilter: {
      kind: null,
      filteredCount: 0,
    },
    conversations: [
      {
        stickyKey,
        requestCount: 1,
        totalTokens: 30,
        totalCost: 0.12,
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T02:00:00Z",
        recentInvocations: [],
        last24hRequests: [],
      },
    ],
  };
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

function Probe({
  accountId,
  selection,
  enabled = true,
}: {
  accountId: number | null;
  selection: StickyKeyConversationSelection;
  enabled?: boolean;
}) {
  const { stats, isLoading, error } = useUpstreamStickyConversations(
    accountId,
    selection,
    enabled,
  );

  return (
    <div>
      <div data-testid="sticky-key">
        {stats?.conversations[0]?.stickyKey ?? ""}
      </div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
    </div>
  );
}

describe("useUpstreamStickyConversations sync guards", () => {
  it("returns zero delay when the SSE refresh window is already open", () => {
    const delay = getUpstreamStickySseRefreshDelay(
      10_000,
      10_000 + UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS,
    );
    expect(delay).toBe(0);
  });

  it("returns the remaining delay when refreshes are too dense", () => {
    const delay = getUpstreamStickySseRefreshDelay(20_000, 22_000);
    expect(delay).toBe(UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS - 2_000);
  });

  it("throttles open resync inside the cooldown window", () => {
    const allowed = shouldTriggerUpstreamStickyOpenResync(
      30_000,
      30_000 + UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS - 1,
    );
    expect(allowed).toBe(false);
  });

  it("allows forced open resync regardless of cooldown", () => {
    const allowed = shouldTriggerUpstreamStickyOpenResync(40_000, 40_100, true);
    expect(allowed).toBe(true);
  });

  it("ignores stale responses after account switches", async () => {
    const first = deferred<UpstreamStickyConversationsResponse>();
    const second = deferred<UpstreamStickyConversationsResponse>();
    apiMocks.fetchUpstreamStickyConversations
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe accountId={101} selection={{ mode: "count", limit: 20 }} />);
    expect(text("loading")).toBe("true");

    rerender(<Probe accountId={202} selection={{ mode: "count", limit: 20 }} />);
    await flushAsync();

    second.resolve(createResponse("sticky-b"));
    await flushAsync();
    expect(text("sticky-key")).toBe("sticky-b");

    first.resolve(createResponse("sticky-a"));
    await flushAsync();
    expect(text("sticky-key")).toBe("sticky-b");
    expect(text("error")).toBe("");
  });
});
