/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE } from "../lib/dashboardWorkingConversations";
import { useDashboardWorkingConversations } from "./useDashboardWorkingConversations";

const topicMocks = vi.hoisted(() => ({
  state: {
    data: null as PromptCacheConversationsResponse | null,
    isLoading: false,
    error: null as string | null,
    refresh: vi.fn(),
  },
  lastDescriptor: null as Record<string, unknown> | null,
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: Record<string, unknown> | null) => {
    topicMocks.lastDescriptor = descriptor;
    return topicMocks.state;
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

beforeEach(() => {
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
  topicMocks.state.refresh.mockReset();
  topicMocks.lastDescriptor = null;
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

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function createPreview(
  invokeId: string,
  status: string,
  occurredAt: string,
): PromptCacheConversationInvocationPreview {
  return {
    id: Number(invokeId.replace(/\D/g, "")) || 1,
    invokeId,
    promptCacheKey: "conversation-a",
    occurredAt,
    status,
    failureClass: "none",
    routeMode: "pool",
    model: "gpt-5.4",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.4",
    totalTokens: 12,
    cost: 0.01,
    proxyDisplayName: "proxy-a",
    upstreamAccountId: 7,
    upstreamAccountName: "Pool A",
    upstreamAccountPlanType: null,
    endpoint: "/v1/responses",
    inputTokens: 8,
    outputTokens: 4,
    cacheInputTokens: 0,
    reasoningTokens: 0,
    reasoningEffort: "medium",
    source: "pool",
    livePhase: null,
  } as PromptCacheConversationInvocationPreview;
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: 120,
    totalCost: 0.2,
    createdAt: "2026-07-16T10:00:00Z",
    lastActivityAt: recentInvocations[0]?.occurredAt ?? "2026-07-16T10:00:00Z",
    lastTerminalAt: "2026-07-16T10:01:00Z",
    lastInFlightAt: recentInvocations.find((item) => item.status === "running")?.occurredAt ?? null,
    cursor: `${promptCacheKey}-cursor`,
    upstreamAccounts: [],
    recentInvocations,
  } as PromptCacheConversation;
}

function createResponse(): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-07-16T09:55:00Z",
    rangeEnd: "2026-07-16T10:05:00Z",
    snapshotAt: "2026-07-16T10:05:00Z",
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: 7,
    hasMore: true,
    nextCursor: "next-page",
    conversations: [
      createConversation("conversation-a", [
        createPreview("invoke-1", "running", "2026-07-16T10:04:00Z"),
        createPreview("invoke-2", "running", "2026-07-16T10:03:30Z"),
        createPreview("invoke-3", "running", "2026-07-16T10:03:00Z"),
        createPreview("invoke-4", "running", "2026-07-16T10:02:30Z"),
        createPreview("invoke-5", "running", "2026-07-16T10:02:00Z"),
        createPreview("invoke-6", "running", "2026-07-16T10:01:30Z"),
      ]),
      createConversation("conversation-b", [
        createPreview("invoke-7", "success", "2026-07-16T10:01:00Z"),
      ]),
    ],
  };
}

function Probe() {
  const { cards, totalMatched, hasMore, recentPreviewLimit, loadMore, setRefreshTargetCount } =
    useDashboardWorkingConversations();

  return (
    <div>
      <div data-testid="cards">{String(cards.length)}</div>
      <div data-testid="first-key">{cards[0]?.promptCacheKey ?? ""}</div>
      <div data-testid="total">{String(totalMatched)}</div>
      <div data-testid="has-more">{hasMore ? "true" : "false"}</div>
      <div data-testid="recent-limit">{String(recentPreviewLimit)}</div>
      <button type="button" data-testid="load-more" onClick={() => loadMore()} />
      <button type="button" data-testid="target-more" onClick={() => setRefreshTargetCount(25)} />
    </div>
  );
}

describe("useDashboardWorkingConversations", () => {
  it("subscribes to the dashboard.working-conversations.current topic and maps cards", () => {
    topicMocks.state.data = createResponse();

    render(<Probe />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "dashboard.working-conversations.current",
      params: {
        pageSize: String(DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE),
        recentInvocationLimit: "16",
      },
    });
    expect(text("cards")).toBe("2");
    expect(text("first-key")).toBe("conversation-b");
    expect(text("total")).toBe("7");
    expect(text("has-more")).toBe("true");
    expect(text("recent-limit")).toBe("6");
  });

  it("grows the requested topic window when loadMore is triggered", () => {
    topicMocks.state.data = createResponse();

    render(<Probe />);

    act(() => {
      host?.querySelector<HTMLButtonElement>('[data-testid="load-more"]')?.click();
    });
    rerender(<Probe />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "dashboard.working-conversations.current",
      params: {
        pageSize: "40",
        recentInvocationLimit: "16",
      },
    });
  });

  it("rounds the refresh target up to the next page boundary", () => {
    topicMocks.state.data = createResponse();

    render(<Probe />);

    act(() => {
      host?.querySelector<HTMLButtonElement>('[data-testid="target-more"]')?.click();
    });
    rerender(<Probe />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "dashboard.working-conversations.current",
      params: {
        pageSize: "40",
        recentInvocationLimit: "16",
      },
    });
  });
});
