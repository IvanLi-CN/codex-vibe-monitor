/** @vitest-environment jsdom */
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type { PromptCacheConversationsResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import LivePage from "./Live";

const LIVE_TAB_STORAGE_KEY = "codex-vibe-monitor.live.active-tab";
const fixedTimestamp = "2026-10-07T08:00:00Z";

const mocks = vi.hoisted(() => ({
  useForwardProxyLiveStats: vi.fn(),
  useInvocationStream: vi.fn(),
  useModelRoutingLive: vi.fn(),
  useSseStatus: vi.fn(),
  useSummary: vi.fn(),
  createEventSource: vi.fn(),
}));

class FakeEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  readonly listeners = new Map<string, Set<EventListener>>();
  readyState = FakeEventSource.CONNECTING;

  constructor(readonly path: string) {}

  addEventListener(type: string, listener: EventListener) {
    const bucket = this.listeners.get(type) ?? new Set<EventListener>();
    bucket.add(listener);
    this.listeners.set(type, bucket);
  }

  removeEventListener(type: string, listener: EventListener) {
    this.listeners.get(type)?.delete(listener);
  }

  close() {
    this.readyState = FakeEventSource.CLOSED;
  }

  emitMessage(data: string) {
    const event = new MessageEvent("message", { data });
    for (const listener of this.listeners.get("message") ?? []) {
      listener(event);
    }
  }
}

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    createEventSource: mocks.createEventSource,
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
  mocks.createEventSource.mockReset();
  mocks.createEventSource.mockImplementation((path: string) => new FakeEventSource(path));
  vi.stubGlobal("EventSource", FakeEventSource);
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
  vi.unstubAllGlobals();
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

    const source = mocks.createEventSource.mock.results[0]?.value as FakeEventSource | undefined;
    expect(source).toBeDefined();
    const descriptor = buildTopicDescriptor("prompt-cache.window", {
      detail: "full",
      limit: 50,
      recentInvocationLimit: 16,
    });
    act(() => {
      source?.emitMessage(
        JSON.stringify({
          type: "snapshot",
          topic: descriptor,
          topicKey: "prompt-cache.window?detail=full&limit=50&recentInvocationLimit=16",
          schemaEpoch: "prompt-cache.window/v1",
          cursor: 1,
          payload: promptCacheStats,
        }),
      );
    });

    const renderedText = host?.textContent ?? "";
    expect(renderedText).toContain("pck-integration");
    expect(renderedText).toContain("conv-1");
    expect(renderedText).toContain("成功");
    expect(renderedText).toContain("失败");
    expect(renderedText).toContain("120");
    expect(descriptor).toEqual(expect.objectContaining({ topic: "prompt-cache.window" }));
  });
});
