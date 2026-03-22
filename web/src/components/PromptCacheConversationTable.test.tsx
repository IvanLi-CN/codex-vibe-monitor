/** @vitest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn<
    (accountId: number) => Promise<UpstreamAccountDetail>
  >(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
  };
});

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
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    vi.useRealTimers();
  });

  function renderInteractive(stats: PromptCacheConversationsResponse | null) {
    if (!host) {
      host = document.createElement("div");
      document.body.appendChild(host);
      root = createRoot(host);
    }
    act(() => {
      root?.render(
        <MemoryRouter>
          <I18nProvider>
            <PromptCacheConversationTable
              stats={stats}
              isLoading={false}
              error={null}
            />
          </I18nProvider>
        </MemoryRouter>,
      );
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
              requestTokens: 80,
              cumulativeTokens: 200,
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

    expect(html).toContain("暂无 Prompt Cache Key 对话数据。");
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

  it("opens and closes the upstream account drawer from prompt cache rows", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue({
      id: 101,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "Pool Alpha",
      groupName: "group-a",
      isMother: false,
      status: "active",
      enabled: true,
      email: "pool-alpha@example.com",
      chatgptAccountId: "org_pool_alpha",
      chatgptUserId: "user_pool_alpha",
      planType: "team",
      maskedApiKey: null,
      lastSyncedAt: "2026-03-02T16:20:00Z",
      lastSuccessfulSyncAt: "2026-03-02T16:18:00Z",
      lastActivityAt: "2026-03-02T16:00:00Z",
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: "2026-03-02T22:00:00Z",
      lastRefreshedAt: "2026-03-02T16:19:00Z",
      primaryWindow: {
        usedPercent: 22,
        usedText: "22 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-02T18:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 38,
        usedText: "38 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-09T00:00:00Z",
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    });

    renderInteractive({
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
    });

    const trigger = Array.from(document.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("Pool Alpha"),
    );
    expect(trigger).toBeTruthy();

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledWith(101);
    expect(document.body.textContent).toContain("上游账号");
    expect(document.body.textContent).toContain("Pool Alpha");
    expect(document.body.textContent).toContain("去号池查看完整详情");

    const drawerWrapper = document
      .querySelector('section[role="dialog"]')
      ?.parentElement;
    expect(drawerWrapper).toBeTruthy();

    await act(async () => {
      drawerWrapper?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.textContent).not.toContain("去号池查看完整详情");
  });
});
