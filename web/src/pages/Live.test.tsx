/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import LivePage from "./Live";

const hookMocks = vi.hoisted(() => ({
  useForwardProxyLiveStats: vi.fn(),
  useInvocationStream: vi.fn(),
  usePromptCacheConversations: vi.fn(),
  useSummary: vi.fn(),
}));

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
  PromptCacheConversationTable: () => (
    <div data-testid="prompt-cache-conversation-table" />
  ),
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
          return "Prompt Cache Key 对话";
        case "live.conversations.description":
          return "对话说明";
        case "live.conversations.selectionLabel":
          return "对话筛选";
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

describe("LivePage", () => {
  it("offers mutually exclusive count and activity-window prompt-cache filters", () => {
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

    render(<LivePage />);

    const select = host?.querySelector(
      '[data-testid="live-prompt-cache-selection"]',
    );
    if (!(select instanceof HTMLSelectElement)) {
      throw new Error("missing prompt cache selection");
    }

    expect(
      Array.from(select.options).map((option) => option.textContent),
    ).toEqual([
      "20 个对话",
      "50 个对话",
      "100 个对话",
      "近 1 小时活动",
      "近 3 小时活动",
      "近 6 小时活动",
      "近 12 小时活动",
      "近 24 小时活动",
    ]);
    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "count",
      limit: 50,
    });

    act(() => {
      select.value = "activityWindow:6";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });

    expect(hookMocks.usePromptCacheConversations).toHaveBeenLastCalledWith({
      mode: "activityWindow",
      activityHours: 6,
    });
  });
});
