/** @vitest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { act, type ComponentProps, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { I18nProvider } from "../i18n";
import type {
  BroadcastPayload,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail:
    vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  fetchInvocationRecords: vi.fn(),
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
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
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

function renderTable(stats: PromptCacheConversationsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <PromptCacheConversationTable
        stats={stats}
        isLoading={false}
        error={null}
      />
    </I18nProvider>,
  );
}

function formatZhDateTime(raw: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(raw));
}

function createConversation(
  overrides: Partial<PromptCacheConversation> & {
    promptCacheKey: string;
    createdAt: string;
    lastActivityAt: string;
  },
): PromptCacheConversation {
  return {
    promptCacheKey: overrides.promptCacheKey,
    requestCount: overrides.requestCount ?? 1,
    totalTokens: overrides.totalTokens ?? 0,
    totalCost: overrides.totalCost ?? 0,
    createdAt: overrides.createdAt,
    lastActivityAt: overrides.lastActivityAt,
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations: overrides.recentInvocations ?? [],
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

describe("PromptCacheConversationTable", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-03T00:00:00Z"));
    apiMocks.fetchUpstreamAccountDetail.mockReset();
    apiMocks.fetchInvocationRecords.mockReset();
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
  });

  function renderInteractiveElement(element: React.ReactNode) {
    if (!host) {
      host = document.createElement("div");
      document.body.appendChild(host);
      root = createRoot(host);
    }
    act(() => {
      root?.render(
        <MemoryRouter>
          <I18nProvider>{element}</I18nProvider>
        </MemoryRouter>,
      );
    });
  }

  function renderInteractive(
    stats: PromptCacheConversationsResponse | null,
    props: Partial<ComponentProps<typeof PromptCacheConversationTable>> = {},
  ) {
    renderInteractiveElement(
      <PromptCacheConversationTable
        stats={stats}
        isLoading={false}
        error={null}
        {...props}
      />,
    );
  }

  function findButtonByAriaLabel(label: string, index = 0) {
    return (
      Array.from(document.querySelectorAll("button")).filter(
        (button): button is HTMLButtonElement =>
          button.getAttribute("aria-label") === label ||
          button.textContent?.includes(label) === true,
      )[index] ?? null
    );
  }

  function emitSseRecords(payload: BroadcastPayload) {
    act(() => {
      sseMocks.listeners.forEach((listener) => listener(payload));
    });
  }

  async function flushInteractive() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it("renders conversation metrics and unified 24h sparkline surfaces", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-chat-001",
          requestCount: 12,
          totalTokens: 3456,
          totalCost: 1.2345,
          createdAt: "2026-03-02T00:00:00Z",
          lastActivityAt: "2026-03-02T16:00:00Z",
          upstreamAccounts: [
            {
              upstreamAccountId: 101,
              upstreamAccountName: "Pool Alpha",
              requestCount: 7,
              totalTokens: 2000,
              totalCost: 0.7,
              lastActivityAt: "2026-03-02T16:00:00Z",
            },
            {
              upstreamAccountId: 102,
              upstreamAccountName: "Pool Beta",
              requestCount: 5,
              totalTokens: 1456,
              totalCost: 0.5345,
              lastActivityAt: "2026-03-02T14:00:00Z",
            },
          ],
          last24hRequests: [
            {
              occurredAt: "2026-03-02T10:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 120,
            },
            {
              occurredAt: "2026-03-02T12:00:00Z",
              status: "failed",
              isSuccess: false,
              outcome: "failure",
              requestTokens: 80,
              cumulativeTokens: 200,
            },
            {
              occurredAt: "2026-03-02T14:00:00Z",
              status: "unknown",
              isSuccess: false,
              outcome: "neutral",
              requestTokens: 30,
              cumulativeTokens: 230,
            },
            {
              occurredAt: "2026-03-02T16:00:00Z",
              status: "running",
              isSuccess: false,
              outcome: "in_flight",
              requestTokens: 16,
              cumulativeTokens: 246,
            },
          ],
        }),
      ],
    };

    const html = renderTable(stats);

    expect(html).toContain("pck-chat-001");
    expect(html).toContain("Prompt Cache Key");
    expect(html).toContain("24 小时 Token 累计");
    expect(html).toContain("sm:hidden");
    expect(html).toContain("sm:table");
    expect(html).toContain('data-chart-kind="keyed-conversation-sparkline"');
    expect(html).toContain('aria-label="pck-chat-001 24 小时 Token 累计图"');
    expect(html).not.toContain("<title>");
    expect(html).toContain('stroke="oklch(var(--color-success) / 0.95)"');
    expect(html).toContain('stroke="oklch(var(--color-error) / 0.92)"');
    expect(html).toContain('stroke="oklch(var(--color-base-content) / 0.58)"');
    expect(html).toContain('stroke="oklch(var(--color-primary) / 0.88)"');
  });

  it("shares the 24h token chart scale across visible conversations", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-low",
          requestCount: 1,
          totalTokens: 50,
          totalCost: 0.01,
          createdAt: "2026-03-02T01:00:00Z",
          lastActivityAt: "2026-03-02T01:00:00Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-02T01:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 50,
              cumulativeTokens: 50,
            },
          ],
        }),
        createConversation({
          promptCacheKey: "pck-high",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.02,
          createdAt: "2026-03-02T02:00:00Z",
          lastActivityAt: "2026-03-02T02:00:00Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-02T02:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 100,
              cumulativeTokens: 100,
            },
          ],
        }),
      ],
    };

    const html = renderTable(stats);

    expect(html).toContain('aria-label="pck-low 23 小时 Token 累计图"');
    expect(html).toContain('y1="24"');
  });

  it("ignores malformed timestamps when computing the shared chart scale", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-low-valid",
          requestCount: 1,
          totalTokens: 50,
          totalCost: 0.01,
          createdAt: "2026-03-02T01:00:00Z",
          lastActivityAt: "2026-03-02T01:00:00Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-02T01:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 50,
              cumulativeTokens: 50,
            },
          ],
        }),
        createConversation({
          promptCacheKey: "pck-bad-point",
          requestCount: 2,
          totalTokens: 100,
          totalCost: 0.02,
          createdAt: "2026-03-02T02:00:00Z",
          lastActivityAt: "2026-03-02T02:00:00Z",
          last24hRequests: [
            {
              occurredAt: "not-a-date",
              status: "success",
              isSuccess: true,
              requestTokens: 9999,
              cumulativeTokens: 10000,
            },
            {
              occurredAt: "2026-03-02T02:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 100,
              cumulativeTokens: 100,
            },
          ],
        }),
      ],
    };

    const html = renderTable(stats);

    expect(html).toContain('aria-label="pck-low-valid 23 小时 Token 累计图"');
    expect(html).toContain('y1="24"');
  });

  it("renders empty state when there are no conversations", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: "inactiveOutside24h", filteredCount: 2 },
      conversations: [],
    };

    const html = renderTable(stats);

    expect(html).toContain("暂无对话数据。");
    expect(html).toContain("有 2 个更新创建的对话因未在近 24 小时活动而未显示");
  });

  it("renders the implicit filter note when time mode is capped to 50 conversations", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T21:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "activityWindow",
      selectedLimit: null,
      selectedActivityHours: 3,
      implicitFilter: { kind: "cappedTo50", filteredCount: 7 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-window-cap",
          requestCount: 2,
          totalTokens: 120,
          totalCost: 0.01,
          createdAt: "2026-03-02T21:30:00Z",
          lastActivityAt: "2026-03-02T23:00:00Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-02T23:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 120,
            },
          ],
        }),
      ],
    };

    const html = renderTable(stats);

    expect(html).toContain("3 小时 Token 累计");
    expect(html).toContain(
      "有 7 个对话命中了活动时间筛选，但因时间模式最多只展示 50 个对话而未显示。",
    );
  });

  it("refreshes the chart range end when stats arrive after mount", async () => {
    const nextStats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-03T00:00:05Z",
      rangeEnd: "2026-03-03T00:00:05Z",
      selectionMode: "activityWindow",
      selectedLimit: null,
      selectedActivityHours: 1,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-live-arrival",
          requestCount: 1,
          totalTokens: 120,
          totalCost: 0.01,
          createdAt: "2026-03-03T00:00:05Z",
          lastActivityAt: "2026-03-03T00:00:05Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-03T00:00:05Z",
              status: "success",
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 120,
            },
          ],
        }),
      ],
    };

    renderInteractive(null);

    vi.setSystemTime(new Date("2026-03-03T00:00:10Z"));

    renderInteractive(nextStats);

    await act(async () => {
      await Promise.resolve();
    });

    expect(host?.textContent).toContain("1 小时 Token 累计");
  });

  it("caps the shared chart window to 24 hours for older active conversations", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T23:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "activityWindow",
      selectedLimit: null,
      selectedActivityHours: 1,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-active-old",
          requestCount: 4,
          totalTokens: 240,
          totalCost: 0.02,
          createdAt: "2026-03-01T00:00:00Z",
          lastActivityAt: "2026-03-02T23:50:00Z",
          last24hRequests: [
            {
              occurredAt: "2026-03-02T01:00:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 120,
            },
            {
              occurredAt: "2026-03-02T23:50:00Z",
              status: "success",
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 240,
            },
          ],
        }),
      ],
    };

    const html = renderTable(stats);

    expect(html).toContain("24 小时 Token 累计");
    expect(html).toContain('aria-label="pck-active-old 24 小时 Token 累计图"');
    expect(html).not.toContain("48 小时 Token 累计");
  });

  it("renders upstream account rows and three-line totals with fallbacks", () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-account-lines",
          requestCount: 12,
          totalTokens: 3456,
          totalCost: 1.2345,
          createdAt: "2026-03-02T00:00:00Z",
          lastActivityAt: "2026-03-02T16:00:00Z",
          upstreamAccounts: [
            {
              upstreamAccountId: 101,
              upstreamAccountName: "Pool Alpha",
              requestCount: 5,
              totalTokens: 1600,
              totalCost: 0.56,
              lastActivityAt: "2026-03-02T16:00:00Z",
            },
            {
              upstreamAccountId: 202,
              upstreamAccountName: null,
              requestCount: 4,
              totalTokens: 1200,
              totalCost: 0.44,
              lastActivityAt: "2026-03-02T15:00:00Z",
            },
            {
              upstreamAccountId: null,
              upstreamAccountName: null,
              requestCount: 3,
              totalTokens: 656,
              totalCost: 0.2345,
              lastActivityAt: "2026-03-02T14:00:00Z",
            },
            {
              upstreamAccountId: 303,
              upstreamAccountName: "Pool Hidden",
              requestCount: 1,
              totalTokens: 1,
              totalCost: 0.0001,
              lastActivityAt: "2026-03-02T13:00:00Z",
            },
          ],
          last24hRequests: [],
        }),
      ],
    };

    const html = renderTable(stats);
    const createdAtLabel = formatZhDateTime("2026-03-02T00:00:00Z");
    const lastActivityLabel = formatZhDateTime("2026-03-02T16:00:00Z");

    expect(html).toContain("上游账号");
    expect(html).toContain("总计");
    expect(html).toContain("时间");
    expect(html).toContain("Pool Alpha");
    expect(html).toContain("账号 #202");
    expect(html).toContain("—");
    expect(html).not.toContain("Pool Hidden");
    expect(html).toContain("5 请求 · Token 1,600 · US$0.56");
    expect(html).toContain("4 请求 · Token 1,200 · US$0.44");
    expect(html).toContain("请求数");
    expect(html).toContain("3,456");
    expect(html).toContain("US$1.2345");
    expect(html).toContain("创建");
    expect(html).toContain("活动");
    expect(html).toContain(createdAtLabel);
    expect(html).toContain(lastActivityLabel);
    expect(html).toContain("w-[15%]");
    expect(html).toContain("tabular-nums");
  });

  it("forwards prompt cache account clicks to the shared upstream account controller", async () => {
    const onOpenUpstreamAccount = vi.fn();

    renderInteractive(
      {
        rangeStart: "2026-03-02T00:00:00Z",
        rangeEnd: "2026-03-03T00:00:00Z",
        selectionMode: "count",
        selectedLimit: 50,
        selectedActivityHours: null,
        implicitFilter: { kind: null, filteredCount: 0 },
        conversations: [
          createConversation({
            promptCacheKey: "pck-clickable-account",
            requestCount: 12,
            totalTokens: 3456,
            totalCost: 1.2345,
            createdAt: "2026-03-02T00:00:00Z",
            lastActivityAt: "2026-03-02T16:00:00Z",
            upstreamAccounts: [
              {
                upstreamAccountId: 101,
                upstreamAccountName: "Pool Alpha",
                requestCount: 5,
                totalTokens: 1600,
                totalCost: 0.56,
                lastActivityAt: "2026-03-02T16:00:00Z",
              },
              {
                upstreamAccountId: null,
                upstreamAccountName: "匿名账号",
                requestCount: 4,
                totalTokens: 1200,
                totalCost: 0.44,
                lastActivityAt: "2026-03-02T15:00:00Z",
              },
            ],
            last24hRequests: [],
          }),
        ],
      },
      { onOpenUpstreamAccount },
    );

    const trigger = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.includes("Pool Alpha"),
    );
    expect(trigger).toBeTruthy();

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(101, "Pool Alpha");
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
    expect(document.body.textContent).not.toContain("去号池查看完整详情");
  });

  it("toggles recent invocation previews inline", async () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-preview-toggle",
          requestCount: 2,
          totalTokens: 3210,
          totalCost: 0.42,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:00:00Z",
          recentInvocations: [
            {
              id: 11,
              invokeId: "preview-11",
              occurredAt: "2026-03-02T12:00:00Z",
              status: "success",
              failureClass: "service_failure",
              routeMode: "pool",
              model: "gpt-5.4",
              totalTokens: 3210,
              cost: 0.42,
              proxyDisplayName: "Proxy West",
              upstreamAccountId: 101,
              upstreamAccountName: "Pool Alpha",
              endpoint: "/v1/responses",
            },
          ],
          last24hRequests: [],
        }),
      ],
    };

    function Harness() {
      const [expandedPromptCacheKeys, setExpandedPromptCacheKeys] = useState<
        string[]
      >([]);

      return (
        <PromptCacheConversationTable
          stats={stats}
          isLoading={false}
          error={null}
          expandedPromptCacheKeys={expandedPromptCacheKeys}
          onToggleExpandedPromptCacheKey={(promptCacheKey) => {
            setExpandedPromptCacheKeys((current) =>
              current.includes(promptCacheKey)
                ? current.filter((value) => value !== promptCacheKey)
                : [...current, promptCacheKey],
            );
          }}
        />
      );
    }

    renderInteractiveElement(<Harness />);

    const expandButton = findButtonByAriaLabel("展开最近调用记录");
    expect(expandButton).toBeTruthy();

    await act(async () => {
      expandButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(
      document.querySelector('[data-testid="invocation-table-scroll"]'),
    ).toBeTruthy();
    expect(document.body.textContent).toContain("首字总耗时 / HTTP 压缩");
    expect(document.body.textContent).not.toContain("输入 / 缓存");
    expect(document.body.textContent).toContain("gpt-5.4");
    expect(document.body.textContent).toContain("Proxy West");
    expect(document.body.textContent).toContain("3,210");

    const detailToggle = document.querySelector(
      'button[aria-controls^="invocation-table-details-"]',
    ) as HTMLButtonElement | null;
    expect(detailToggle).toBeTruthy();

    await act(async () => {
      detailToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    const accountButtons = Array.from(
      document.querySelectorAll("button"),
    ).filter((button) => button.textContent?.includes("Pool Alpha"));
    expect(accountButtons.length).toBeGreaterThan(0);

    const collapseButton = findButtonByAriaLabel("收起最近调用记录");
    expect(collapseButton).toBeTruthy();

    await act(async () => {
      collapseButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.textContent).not.toContain("总时延");
  });

  it("toggles recent invocation previews inline without external expansion state", async () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-preview-uncontrolled",
          requestCount: 1,
          totalTokens: 1184,
          totalCost: 0.028,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:00:00Z",
          recentInvocations: [
            {
              id: 21,
              invokeId: "preview-21",
              occurredAt: "2026-03-02T12:00:00Z",
              status: "success",
              failureClass: "none",
              routeMode: "pool",
              model: "gpt-5.4",
              totalTokens: 1184,
              cost: 0.028,
              proxyDisplayName: "Proxy Central",
              upstreamAccountId: 101,
              upstreamAccountName: "Pool Alpha",
              endpoint: "/v1/responses",
            },
          ],
          last24hRequests: [],
        }),
      ],
    };

    renderInteractive(stats);

    const expandButton = findButtonByAriaLabel("展开最近调用记录");
    expect(expandButton).toBeTruthy();

    await act(async () => {
      expandButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(
      document.querySelector('[data-testid="invocation-table-scroll"]'),
    ).toBeTruthy();
    expect(document.body.textContent).toContain("首字总耗时 / HTTP 压缩");
    expect(document.body.textContent).toContain("Proxy Central");

    const collapseButton = findButtonByAriaLabel("收起最近调用记录");
    expect(collapseButton).toBeTruthy();

    await act(async () => {
      collapseButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.textContent).not.toContain("总时延");
  });

  it("opens the history drawer and preserves loaded records when later pages fail", async () => {
    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 901,
        total: 3,
        page: 1,
        pageSize: 2,
        records: [
          {
            id: 71,
            invokeId: "history-71",
            occurredAt: "2026-03-02T12:30:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 1500,
            cost: 0.31,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-history",
            upstreamAccountId: 101,
            upstreamAccountName: "Pool Alpha",
            proxyDisplayName: "Proxy West",
            createdAt: "2026-03-02T12:30:00Z",
          },
          {
            id: 70,
            invokeId: "history-70",
            occurredAt: "2026-03-02T12:10:00Z",
            status: "http_502",
            failureClass: "service_failure",
            totalTokens: 900,
            cost: 0.2,
            endpoint: "/v1/chat/completions",
            promptCacheKey: "pck-history",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: "Proxy East",
            createdAt: "2026-03-02T12:10:00Z",
          },
        ],
      })
      .mockRejectedValueOnce(new Error("page 2 failed"));

    renderInteractive({
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-history",
          requestCount: 3,
          totalTokens: 2400,
          totalCost: 0.51,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    expect(historyButton).toBeTruthy();

    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(1, {
      promptCacheKey: "pck-history",
      page: 1,
      pageSize: 200,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });
    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(2, {
      promptCacheKey: "pck-history",
      page: 2,
      pageSize: 200,
      sortBy: "occurredAt",
      sortOrder: "desc",
      snapshotId: 901,
    });
    expect(document.body.textContent).toContain("全部保留调用记录");
    expect(
      document.querySelector('[data-testid="invocation-table-scroll"]'),
    ).toBeTruthy();
    expect(document.body.textContent).toContain("首字总耗时 / HTTP 压缩");
    expect(document.body.textContent).not.toContain("输入 / 缓存");
    expect(document.body.textContent).toContain("Proxy West");
    expect(document.body.textContent).toContain("HTTP 502");
    expect(document.body.textContent).toContain("page 2 failed");
    expect(document.body.textContent).toContain("已加载 2 / 3 条保留调用记录");

    const closeButton = findButtonByAriaLabel("关闭调用记录抽屉");
    expect(closeButton).toBeTruthy();

    await act(async () => {
      closeButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.textContent).not.toContain("全部保留调用记录");
  });

  it("streams live history rows into the open drawer and replaces running snapshots with final records", async () => {
    let resolveRefresh!: (value: {
      snapshotId: number;
      total: number;
      page: number;
      pageSize: number;
      records: Array<Record<string, unknown>>;
    }) => void;
    const refreshPromise = new Promise<{
      snapshotId: number;
      total: number;
      page: number;
      pageSize: number;
      records: Array<Record<string, unknown>>;
    }>((resolve) => {
      resolveRefresh = resolve;
    });

    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 902,
        total: 1,
        page: 1,
        pageSize: 200,
        records: [
          {
            id: 61,
            invokeId: "history-base-61",
            occurredAt: "2026-03-02T12:10:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 900,
            cost: 0.2,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-history-live",
            upstreamAccountId: 101,
            upstreamAccountName: "Pool Alpha",
            proxyDisplayName: "Proxy Base",
            createdAt: "2026-03-02T12:10:00Z",
          },
        ],
      })
      .mockImplementationOnce(async () => refreshPromise);

    renderInteractive({
      rangeStart: "2026-03-02T00:00:00Z",
      rangeEnd: "2026-03-03T00:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-history-live",
          requestCount: 2,
          totalTokens: 2400,
          totalCost: 0.51,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:35:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    expect(historyButton).toBeTruthy();

    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(document.body.textContent).toContain("Proxy Base");

    emitSseRecords({
      type: "records",
      records: [
        {
          id: 62,
          invokeId: "history-live-62",
          occurredAt: "2026-03-02T12:35:00Z",
          createdAt: "2026-03-02T12:35:00Z",
          status: "running",
          promptCacheKey: "pck-history-live",
          totalTokens: 0,
          cost: 0,
          proxyDisplayName: "Proxy Running",
        },
      ],
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("Proxy Running");
    expect(document.body.textContent).toContain("共 2 条保留调用记录");

    emitSseRecords({
      type: "records",
      records: [
        {
          id: 62,
          invokeId: "history-live-62",
          occurredAt: "2026-03-02T12:35:00Z",
          createdAt: "2026-03-02T12:35:00Z",
          status: "completed",
          promptCacheKey: "pck-history-live",
          totalTokens: 1500,
          cost: 0.31,
          proxyDisplayName: "Proxy Final",
        },
      ],
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("Proxy Final");
    expect(document.body.textContent).not.toContain("Proxy Running");

    await act(async () => {
      resolveRefresh({
        snapshotId: 903,
        total: 2,
        page: 1,
        pageSize: 200,
        records: [
          {
            id: 62,
            invokeId: "history-live-62",
            occurredAt: "2026-03-02T12:35:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 1500,
            cost: 0.31,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-history-live",
            upstreamAccountId: 101,
            upstreamAccountName: "Pool Alpha",
            proxyDisplayName: "Proxy Final",
            createdAt: "2026-03-02T12:35:00Z",
          },
          {
            id: 61,
            invokeId: "history-base-61",
            occurredAt: "2026-03-02T12:10:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 900,
            cost: 0.2,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-history-live",
            upstreamAccountId: 101,
            upstreamAccountName: "Pool Alpha",
            proxyDisplayName: "Proxy Base",
            createdAt: "2026-03-02T12:10:00Z",
          },
        ],
      });
      await refreshPromise;
    });
    await flushInteractive();

    expect(apiMocks.fetchInvocationRecords).toHaveBeenNthCalledWith(2, {
      promptCacheKey: "pck-history-live",
      page: 1,
      pageSize: 200,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });
    expect(document.body.textContent).toContain("Proxy Final");
    expect(document.body.textContent).not.toContain("Proxy Running");
  });
});
