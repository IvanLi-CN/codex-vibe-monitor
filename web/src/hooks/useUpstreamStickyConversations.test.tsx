/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  StickyKeyConversationSelection,
  UpstreamStickyConversationsResponse,
} from "../lib/api";
import {
  getUpstreamStickySseRefreshDelay,
  shouldTriggerUpstreamStickyOpenResync,
  UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS,
  UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS,
  useUpstreamStickyConversations,
} from "./useUpstreamStickyConversations";

const topicMocks = vi.hoisted(() => ({
  calls: [] as Array<{ descriptor: unknown; enabled: boolean }>,
  refresh: vi.fn(),
  state: {
    data: null as UpstreamStickyConversationsResponse | null,
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

function createResponse(stickyKey: string): UpstreamStickyConversationsResponse {
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

function Probe({
  accountId,
  selection,
  enabled = true,
}: {
  accountId: number | null;
  selection: StickyKeyConversationSelection;
  enabled?: boolean;
}) {
  const { stats, isLoading, error, refresh } = useUpstreamStickyConversations(
    accountId,
    selection,
    enabled,
  );

  return (
    <div>
      <div data-testid="sticky-key">{stats?.conversations[0]?.stickyKey ?? ""}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <button type="button" data-testid="refresh" onClick={() => void refresh()} />
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
});

describe("useUpstreamStickyConversations", () => {
  it("subscribes to the sticky conversation topic and exposes topic state", () => {
    topicMocks.state.data = createResponse("sticky-routing");
    topicMocks.state.error = "topic warning";

    render(<Probe accountId={101} selection={{ mode: "count", limit: 20 }} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "prompt-cache.sticky.window",
        params: {
          accountId: "101",
          limit: "20",
        },
      },
      enabled: true,
    });
    expect(text("sticky-key")).toBe("sticky-routing");
    expect(text("error")).toBe("topic warning");
  });

  it("disables the topic until an account is available and forwards manual refresh", () => {
    render(<Probe accountId={null} selection={{ mode: "activityWindow", activityHours: 6 }} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: null,
      enabled: false,
    });
    expect(text("sticky-key")).toBe("");
    expect(text("loading")).toBe("false");

    topicMocks.state.data = createResponse("sticky-window");
    topicMocks.state.isLoading = true;
    render(<Probe accountId={202} selection={{ mode: "activityWindow", activityHours: 6 }} />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "prompt-cache.sticky.window",
        params: {
          accountId: "202",
          activityHours: "6",
        },
      },
      enabled: true,
    });

    act(() => {
      const button = host?.querySelector('[data-testid="refresh"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("Missing refresh button");
      }
      button.click();
    });

    expect(text("sticky-key")).toBe("sticky-window");
    expect(text("loading")).toBe("true");
    expect(topicMocks.refresh).toHaveBeenCalledTimes(1);
  });
});
