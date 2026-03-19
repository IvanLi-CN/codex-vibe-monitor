/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
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

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchPromptCacheConversations: apiMocks.fetchPromptCacheConversations,
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function createResponse(
  promptCacheKey: string,
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-03-10T00:00:00Z",
    rangeEnd: "2026-03-11T00:00:00Z",
    selectionMode: "count",
    selectedLimit: 50,
    selectedActivityHours: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations: [
      {
        promptCacheKey,
        requestCount: 1,
        totalTokens: 30,
        totalCost: 0.12,
        createdAt: "2026-03-10T01:00:00Z",
        lastActivityAt: "2026-03-10T02:00:00Z",
        last24hRequests: [],
      },
    ],
  };
}

function Probe({ selection }: { selection: PromptCacheConversationSelection }) {
  const { stats, isLoading, error } = usePromptCacheConversations(selection);

  return (
    <div>
      <div data-testid="prompt-cache-key">
        {stats?.conversations[0]?.promptCacheKey ?? ""}
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
});
