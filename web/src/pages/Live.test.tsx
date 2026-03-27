/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import LivePage from "./Live";

const PROMPT_CACHE_SELECTION_STORAGE_KEY =
  "codex-vibe-monitor.live.prompt-cache-selection";
const hookMocks = vi.hoisted(() => ({
  useForwardProxyLiveStats: vi.fn(),
  useInvocationStream: vi.fn(),
  usePromptCacheConversations: vi.fn(),
  useSummary: vi.fn(),
}));
const componentMocks = vi.hoisted(() => ({
  promptCacheConversationTable: vi.fn(),
}));
const storage = new Map<string, string>();

vi.mock("../hooks/useForwardProxyLiveStats", () => ({
  useForwardProxyLiveStats: hookMocks.useForwardProxyLiveStats,
}));

vi.mock("../hooks/useInvocations", () => ({
  useInvocationStream: hookMocks.useInvocationStream,
}));

vi.mock("../hooks/usePromptCacheConversations", () => ({
  usePromptCacheConversations: hookMocks.usePromptCacheConversations,
}));

vi.mock("../hooks/useStats", () => ({
  useSummary: hookMocks.useSummary,
}));

vi.mock("../components/StatsCards", () => ({
  StatsCards: () => <div data-testid="stats-cards" />,
}));

vi.mock("../components/ForwardProxyLiveTable", () => ({
  ForwardProxyLiveTable: () => <div data-testid="forward-proxy-live-table" />,
}));

vi.mock("../components/PromptCacheConversationTable", () => ({
  PromptCacheConversationTable: (props: {
    stats: {
      conversations?: Array<{ promptCacheKey: string }>;
    } | null;
    expandedPromptCacheKeys?: string[];
    onToggleExpandedPromptCacheKey?: (promptCacheKey: string) => void;
  }) => {
    componentMocks.promptCacheConversationTable(props);
    const firstPromptCacheKey = props.stats?.conversations?.[0]?.promptCacheKey;

    return (
      <div
        data-testid="prompt-cache-conversation-table"
        data-expanded={(props.expandedPromptCacheKeys ?? []).join(",")}
      >
        <button
          type="button"
          data-testid="prompt-cache-conversation-toggle-first"
          onClick={() => {
            if (firstPromptCacheKey) {
              props.onToggleExpandedPromptCacheKey?.(firstPromptCacheKey);
            }
          }}
        >
          toggle first
        </button>
      </div>
    );
  },
}));

vi.mock("../components/InvocationChart", () => ({
  InvocationChart: () => <div data-testid="invocation-chart" />,
}));

vi.mock("../components/InvocationTable", () => ({
  InvocationTable: () => <div data-testid="invocation-table" />,
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, values?: Record<string, string | number>) => {
      switch (key) {
        case "live.summary.title":
          return "实时概览";
        case "live.summary.current":
          return "当前";
        case "live.summary.30m":
          return "30 分钟";
        case "live.summary.1h":
          return "1 小时";
        case "live.summary.1d":
          return "1 天";
        case "live.proxy.title":
          return "代理运行态";
        case "live.proxy.description":
          return "代理说明";
        case "live.conversations.title":
          return "对话";
        case "live.conversations.description":
          return "对话说明";
        case "live.conversations.selectionLabel":
          return "对话筛选";
        case "live.conversations.actions.expandAllRecords":
          return "展开所有记录";
        case "live.conversations.actions.collapseAllRecords":
          return "收起所有记录";
        case "live.conversations.option.count":
          return `${values?.count ?? 0} 个对话`;
        case "live.conversations.option.activityHours":
          return `近 ${values?.hours ?? 0} 小时活动`;
        case "live.chart.title":
          return "实时图表";
        case "live.window.label":
          return "窗口大小";
        case "live.option.records":
          return `${values?.count ?? 0} 条记录`;
        case "live.latest.title":
          return "最新记录";
        default:
          return key;
      }
    },
  }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) => storage.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => {
        storage.set(key, value);
      }),
      removeItem: vi.fn((key: string) => {
        storage.delete(key);
      }),
      clear: vi.fn(() => {
        storage.clear();
      }),
    },
  });
  if (typeof globalThis.PointerEvent === "undefined") {
    Object.defineProperty(window, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
    Object.defineProperty(globalThis, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
  }
  if (typeof HTMLElement.prototype.hasPointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
      configurable: true,
      writable: true,
      value: () => false,
    });
  }
  if (typeof HTMLElement.prototype.setPointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
  if (typeof HTMLElement.prototype.releasePointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  storage.clear();
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

function pressElement(element: HTMLElement) {
  act(() => {
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    }
    element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function setupLivePageHooks() {
  hookMocks.useForwardProxyLiveStats.mockReturnValue({
    stats: null,
    isLoading: false,
    error: null,
  });
  hookMocks.useSummary.mockReturnValue({
    summary: null,
    isLoading: false,
    error: null,
  });
  hookMocks.useInvocationStream.mockReturnValue({
    records: [],
    isLoading: false,
    error: null,
  });
  hookMocks.usePromptCacheConversations.mockReturnValue({
    stats: null,
    isLoading: false,
    error: null,
  });
}

function getPromptCacheSelectionTrigger() {
  const select = host?.querySelector(
    '[data-testid="live-prompt-cache-selection"]',
  );
  if (!(select instanceof HTMLButtonElement)) {
    throw new Error("missing prompt cache selection");
  }
  return select;
}

function remount(ui: React.ReactNode) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  render(ui);
}

function buildConversationStats(promptCacheKeys: string[]) {
  return {
    rangeStart: "2026-03-02T00:00:00Z",
    rangeEnd: "2026-03-03T00:00:00Z",
    selectionMode: "count",
    selectedLimit: 50,
    selectedActivityHours: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations: promptCacheKeys.map((promptCacheKey, index) => ({
      promptCacheKey,
      requestCount: index + 1,
      totalTokens: (index + 1) * 100,
      totalCost: Number(((index + 1) * 0.01).toFixed(2)),
      createdAt: "2026-03-02T00:00:00Z",
      lastActivityAt: "2026-03-02T12:00:00Z",
      upstreamAccounts: [],
      recentInvocations: [],
      last24hRequests: [],
    })),
  };
}

function getPromptCacheExpandAllButton() {
  const button = host?.querySelector(
    '[data-testid="live-prompt-cache-expand-all"]',
  );
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error("missing expand-all button");
  }
  return button;
}

function getPromptCacheExpandAllButtonIcon() {
  const icon = host?.querySelector(
    '[data-testid="live-prompt-cache-expand-all-icon"]',
  );
  if (!(icon instanceof HTMLElement)) {
    throw new Error("missing expand-all icon");
  }
  return icon;
}

function getPromptCacheConversationTable() {
  const table = host?.querySelector('[data-testid="prompt-cache-conversation-table"]');
  if (!(table instanceof HTMLDivElement)) {
    throw new Error("missing prompt cache conversation table");
  }
  return table;
}

describe("LivePage", () => {
  it("defaults to 50 conversations when storage is empty", () => {
    setupLivePageHooks();

    render(<LivePage />);

    const select = getPromptCacheSelectionTrigger();

    expect(window.localStorage.getItem).toHaveBeenCalledWith(
      PROMPT_CACHE_SELECTION_STORAGE_KEY,
    );
    expect(select.textContent).toContain("50 个对话");
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "count",
      limit: 50,
    });
  });

  it("falls back to 50 conversations when storage contains an invalid value", () => {
    setupLivePageHooks();
    storage.set(PROMPT_CACHE_SELECTION_STORAGE_KEY, "count:999");

    render(<LivePage />);

    const select = getPromptCacheSelectionTrigger();

    expect(select.textContent).toContain("50 个对话");
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "count",
      limit: 50,
    });
  });

  it("persists the selected count option and restores it after remount", () => {
    setupLivePageHooks();

    render(<LivePage />);

    const select = getPromptCacheSelectionTrigger();

    expect(host?.querySelector("select")).toBeNull();

    pressElement(select);
    const option = Array.from(
      document.body.querySelectorAll("[role='option']"),
    ).find(
      (candidate) =>
        candidate instanceof HTMLElement &&
        candidate.textContent?.includes("20 个对话"),
    );
    if (!(option instanceof HTMLElement)) {
      throw new Error("missing count option");
    }
    pressElement(option);

    expect(window.localStorage.setItem).toHaveBeenCalledWith(
      PROMPT_CACHE_SELECTION_STORAGE_KEY,
      "count:20",
    );
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "count",
      limit: 20,
    });

    remount(<LivePage />);

    const restoredSelect = getPromptCacheSelectionTrigger();
    expect(restoredSelect.textContent).toContain("20 个对话");
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "count",
      limit: 20,
    });
  });

  it("restores a stored activity-window selection on initial render", () => {
    setupLivePageHooks();
    storage.set(PROMPT_CACHE_SELECTION_STORAGE_KEY, "activityWindow:6");

    render(<LivePage />);

    const select = getPromptCacheSelectionTrigger();
    expect(select.textContent).toContain("近 6 小时活动");
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "activityWindow",
      activityHours: 6,
    });
  });

  it("offers mutually exclusive count and activity-window prompt-cache filters", () => {
    setupLivePageHooks();

    render(<LivePage />);

    const select = getPromptCacheSelectionTrigger();

    pressElement(select);
    const option = Array.from(
      document.body.querySelectorAll("[role='option']"),
    ).find(
      (candidate) =>
        candidate instanceof HTMLElement &&
        candidate.textContent?.includes("近 6 小时活动"),
    );
    if (!(option instanceof HTMLElement)) {
      throw new Error("missing activity window option");
    }
    pressElement(option);

    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "activityWindow",
      activityHours: 6,
    });
    expect(window.localStorage.setItem).toHaveBeenCalledWith(
      PROMPT_CACHE_SELECTION_STORAGE_KEY,
      "activityWindow:6",
    );
  });

  it("toggles expand-all for the current visible prompt cache conversations", () => {
    setupLivePageHooks();
    hookMocks.usePromptCacheConversations.mockReturnValue({
      stats: buildConversationStats(["pck-1", "pck-2"]),
      isLoading: false,
      error: null,
    });

    render(<LivePage />);

    const expandAllButton = getPromptCacheExpandAllButton();
    const expandAllIcon = getPromptCacheExpandAllButtonIcon();
    const table = getPromptCacheConversationTable();

    expect(host?.textContent).toContain("对话");
    expect(expandAllButton.textContent).toContain("展开所有记录");
    expect(expandAllIcon.dataset.iconName).toBe("chevron-down");
    expect(table.dataset.expanded).toBe("");

    pressElement(expandAllButton);

    expect(expandAllButton.textContent).toContain("收起所有记录");
    expect(expandAllIcon.dataset.iconName).toBe("chevron-up");
    expect(table.dataset.expanded).toBe("pck-1,pck-2");

    pressElement(expandAllButton);

    expect(expandAllButton.textContent).toContain("展开所有记录");
    expect(expandAllIcon.dataset.iconName).toBe("chevron-down");
    expect(table.dataset.expanded).toBe("");
  });

  it("keeps expanded rows that stay visible and prunes keys removed by refreshed results", () => {
    setupLivePageHooks();
    let currentConversationStats = buildConversationStats(["pck-1", "pck-2"]);
    hookMocks.usePromptCacheConversations.mockImplementation(() => ({
      stats: currentConversationStats,
      isLoading: false,
      error: null,
    }));

    render(<LivePage />);

    const toggleFirstButton = host?.querySelector(
      '[data-testid="prompt-cache-conversation-toggle-first"]',
    );
    if (!(toggleFirstButton instanceof HTMLButtonElement)) {
      throw new Error("missing table toggle button");
    }

    pressElement(toggleFirstButton);
    expect(getPromptCacheConversationTable().dataset.expanded).toBe("pck-1");

    currentConversationStats = buildConversationStats(["pck-1", "pck-3"]);
    rerender(<LivePage />);
    expect(getPromptCacheConversationTable().dataset.expanded).toBe("pck-1");

    currentConversationStats = buildConversationStats(["pck-3"]);
    rerender(<LivePage />);
    expect(getPromptCacheConversationTable().dataset.expanded).toBe("");
  });
});
