/** @vitest-environment jsdom */
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type { PromptCacheConversationsResponse } from "../lib/api";
import type { SubscriptionTopicEvent } from "../lib/sse";
import LivePage from "./Live";

const LIVE_TAB_STORAGE_KEY = "codex-vibe-monitor.live.active-tab";
const fixedTimestamp = "2026-10-07T08:00:00Z";

const mocks = vi.hoisted(() => ({
  useForwardProxyLiveStats: vi.fn(),
  useInvocationStream: vi.fn(),
  useModelRoutingLive: vi.fn(),
  useSseStatus: vi.fn(),
  getCachedTopicState: vi.fn(),
  subscribeToTopic: vi.fn(),
  useSummary: vi.fn(),
}));

vi.mock("../lib/sse", async () => {
  const actual = await vi.importActual<typeof import("../lib/sse")>("../lib/sse");
  return {
    ...actual,
    getCachedTopicState: mocks.getCachedTopicState,
    subscribeToTopic: mocks.subscribeToTopic,
  };
});

vi.mock("../hooks/useCompactViewport", () => ({
  useCompactViewport: () => false,
}));

vi.mock("../hooks/useForwardProxyLiveStats", () => ({
  useForwardProxyLiveStats: mocks.useForwardProxyLiveStats,
}));

vi.mock("../hooks/useInvocations", () => ({
  useInvocationStream: mocks.useInvocationStream,
}));

vi.mock("../hooks/useModelRoutingLive", () => ({
  useModelRoutingLive: mocks.useModelRoutingLive,
}));

vi.mock("../hooks/useStats", () => ({
  useSummary: mocks.useSummary,
}));

vi.mock("../hooks/useSseStatus", () => ({
  default: mocks.useSseStatus,
}));

vi.mock("../features/forward-proxy/ForwardProxyLiveTable", () => ({
  ForwardProxyLiveTable: () => <div />,
}));

vi.mock("../features/invocations/InvocationChart", () => ({
  InvocationChart: () => <div />,
}));

vi.mock("../features/invocations/InvocationTable", () => ({
  InvocationCardList: () => <div />,
}));

vi.mock("../features/live/ModelRoutingLivePanel", () => ({
  ModelRoutingLivePanel: () => <div />,
}));

vi.mock("../features/stats/StatsCards", () => ({
  StatsCards: () => <div />,
}));

const promptCacheStats: PromptCacheConversationsResponse = {
  rangeStart: fixedTimestamp,
  rangeEnd: fixedTimestamp,
  snapshotAt: null,
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  selectedActivityMinutes: null,
  implicitFilter: {
    kind: null,
    filteredCount: 0,
  },
  conversations: [
    {
      promptCacheKey: "pck-integration",
      requestCount: 2,
      totalTokens: 120,
      totalCost: 0.12,
      createdAt: fixedTimestamp,
      lastActivityAt: fixedTimestamp,
      lastTerminalAt: fixedTimestamp,
      lastInFlightAt: null,
      conversationId: "conv-1",
      successCount: 2,
      failureCount: 1,
      inputTokens: 100,
      outputTokens: 20,
      cacheInputTokens: 10,
      reportedCacheWriteTokens: 0,
      reasoningTokens: 5,
      costInput: 0.01,
      costCacheWrite: 0.02,
      costCacheRead: 0.03,
      costOutput: 0.04,
      costReasoning: 0.05,
      firstInvocationAt: fixedTimestamp,
      lastInvocationAt: fixedTimestamp,
      inFlightPhaseCounts: null,
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      manualBinding: null,
      upstreamAccounts: [],
      recentInvocations: [],
      last24hRequests: [],
    },
  ],
};

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
  window.localStorage.clear();
  window.localStorage.setItem(LIVE_TAB_STORAGE_KEY, "conversations");
  mocks.useForwardProxyLiveStats.mockReturnValue({
    stats: null,
    isLoading: false,
    error: null,
  });
  mocks.useInvocationStream.mockReturnValue({
    records: [],
    isLoading: false,
    error: null,
  });
  mocks.useModelRoutingLive.mockReturnValue({
    data: null,
    isLoading: false,
    error: null,
    refresh: vi.fn(),
  });
  mocks.useSseStatus.mockReturnValue({
    phase: "disabled",
    downtimeMs: 0,
    nextRetryAt: null,
    autoReconnect: false,
  });
  mocks.getCachedTopicState.mockReturnValue(null);
  mocks.subscribeToTopic.mockReturnValue(vi.fn());
  mocks.useSummary.mockReturnValue({
    summary: null,
    isLoading: false,
    error: null,
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

function render(ui: ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter>
        <I18nProvider initialLocale="zh" persistLocale={false}>
          {ui}
        </I18nProvider>
      </MemoryRouter>,
    );
  });
}

describe("Live prompt-cache consumer integration", () => {
  it("renders the real subscription hook data through the real conversation table", () => {
    render(<LivePage />);

    const subscription = mocks.subscribeToTopic.mock.calls.find(
      ([descriptor]) => descriptor?.topic === "prompt-cache.window",
    );
    expect(subscription).toBeDefined();
    const listener = subscription?.[1] as
      | ((event: SubscriptionTopicEvent<PromptCacheConversationsResponse>) => void)
      | undefined;
    expect(listener).toBeDefined();
    mocks.getCachedTopicState.mockReturnValue({
      descriptor: subscription?.[0],
      topicKey: "prompt-cache.window?detail=full&limit=50&recentInvocationLimit=16",
      schemaEpoch: "prompt-cache.window/v1",
      cursor: 1,
      payload: promptCacheStats,
      lastKind: "snapshot",
      receivedAt: Date.parse(fixedTimestamp),
      error: null,
    });
    act(() => {
      listener?.({
        type: "snapshot",
        topic: subscription?.[0],
        topicKey: "prompt-cache.window?detail=full&limit=50&recentInvocationLimit=16",
        schemaEpoch: "prompt-cache.window/v1",
        cursor: 1,
        payload: promptCacheStats,
        deliverySource: "network",
      });
    });

    const renderedText = host?.textContent ?? "";
    expect(renderedText).toContain("pck-integration");
    expect(renderedText).toContain("conv-1");
    expect(renderedText).toContain("成功");
    expect(renderedText).toContain("失败");
    expect(renderedText).toContain("120");
    expect(subscription?.[0]).toEqual(expect.objectContaining({ topic: "prompt-cache.window" }));
  });
});
