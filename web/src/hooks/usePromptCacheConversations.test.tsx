/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  PromptCacheConversationSelection,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { usePromptCacheConversations } from "./usePromptCacheConversations";

const topicMocks = vi.hoisted(() => ({
  calls: [] as Array<{ descriptor: unknown; enabled: boolean }>,
  refresh: vi.fn(),
  state: {
    data: null as PromptCacheConversationsResponse | null,
    isLoading: false,
    error: null as string | null,
  },
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: unknown, enabled = true) => {
    topicMocks.calls.push({ descriptor, enabled });
    return {
      data: topicMocks.state.data,
      isLoading: topicMocks.state.isLoading,
      error: topicMocks.state.error,
      refresh: topicMocks.refresh,
    };
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  topicMocks.calls = [];
  topicMocks.refresh.mockReset();
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function render(ui: React.ReactNode) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function createResponse(promptCacheKey: string): PromptCacheConversationsResponse {
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
        upstreamAccounts: [],
        recentInvocations: [],
        last24hRequests: [],
      },
    ],
  };
}

function Probe({ selection }: { selection: PromptCacheConversationSelection }) {
  const { stats, isLoading, error, refresh } = usePromptCacheConversations(selection);

  return (
    <div>
      <div data-testid="prompt-cache-key">{stats?.conversations[0]?.promptCacheKey ?? ""}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <button type="button" data-testid="refresh" onClick={() => void refresh()} />
    </div>
  );
}

describe("usePromptCacheConversations", () => {
  it("subscribes to the prompt-cache topic for count selections", () => {
    topicMocks.state.data = createResponse("pck-count");

    render(<Probe selection={{ mode: "count", limit: 50 }} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "prompt-cache.window",
        params: {
          limit: "50",
          detail: "full",
          recentInvocationLimit: "16",
        },
      },
      enabled: true,
    });
    expect(text("prompt-cache-key")).toBe("pck-count");
    expect(text("loading")).toBe("false");
  });

  it("maps activity-window selections onto the topic descriptor and forwards refresh", () => {
    topicMocks.state.data = createResponse("pck-window");
    topicMocks.state.isLoading = true;
    topicMocks.state.error = "topic warning";

    render(<Probe selection={{ mode: "activityWindow", activityMinutes: 45 }} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "prompt-cache.window",
        params: {
          activityMinutes: "45",
          detail: "full",
          recentInvocationLimit: "16",
        },
      },
      enabled: true,
    });
    expect(text("prompt-cache-key")).toBe("pck-window");
    expect(text("loading")).toBe("true");
    expect(text("error")).toBe("topic warning");

    act(() => {
      const button = host?.querySelector('[data-testid="refresh"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("Missing refresh button");
      }
      button.click();
    });

    expect(topicMocks.refresh).toHaveBeenCalledTimes(1);
  });
});
