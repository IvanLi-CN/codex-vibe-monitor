/** @vitest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { act, type ComponentProps, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import userEvent from "@testing-library/user-event";
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
  PromptCacheConversationBindingResponse,
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail:
    vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  fetchInvocationRecords: vi.fn(),
  fetchInvocationRecordsSummary: vi.fn(),
  fetchPromptCacheConversationBinding:
    vi.fn<(promptCacheKey: string) => Promise<PromptCacheConversationBindingResponse>>(),
  fetchUpstreamAccounts: vi.fn(),
  updatePromptCacheConversationBinding: vi.fn(),
}));

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}));

class MockPointerEvent extends MouseEvent {
  pointerType: string;

  constructor(
    type: string,
    init: MouseEventInit & { pointerType?: string } = {},
  ) {
    super(type, init);
    this.pointerType = init.pointerType ?? "mouse";
  }
}

function findSelectOption(label: string) {
  return Array.from(document.querySelectorAll('[role="option"]')).find(
    (option) => option.textContent?.includes(label),
  ) as HTMLElement | undefined;
}

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRecordsSummary: apiMocks.fetchInvocationRecordsSummary,
    fetchPromptCacheConversationBinding:
      apiMocks.fetchPromptCacheConversationBinding,
    fetchUpstreamAccounts: apiMocks.fetchUpstreamAccounts,
    updatePromptCacheConversationBinding:
      apiMocks.updatePromptCacheConversationBinding,
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

function createUpstreamAccountSummary(
  id: number,
  displayName: string,
  groupName: string,
  overrides: Partial<UpstreamAccountSummary> = {},
) {
  return {
    id,
    kind: "api_key_codex",
    provider: "codex",
    displayName,
    groupName,
    isMother: false,
    status: "active",
    workStatus: "idle",
    enableStatus: "enabled",
    healthStatus: "normal",
    syncState: "idle",
    displayStatus: "active",
    enabled: true,
    email: null,
    chatgptAccountId: null,
    planType: null,
    maskedApiKey: "sk-***",
    tags: [],
    effectiveRoutingRule: {
      blockNewConversations: false,
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
    },
    ...overrides,
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
  Object.defineProperty(window, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(globalThis, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    writable: true,
    value: () => false,
  });
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
});

describe("PromptCacheConversationTable", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-03T00:00:00Z"));
    apiMocks.fetchUpstreamAccountDetail.mockReset();
    apiMocks.fetchInvocationRecords.mockReset();
    apiMocks.fetchInvocationRecordsSummary.mockReset();
    apiMocks.fetchPromptCacheConversationBinding.mockReset();
    apiMocks.fetchUpstreamAccounts.mockReset();
    apiMocks.updatePromptCacheConversationBinding.mockReset();
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-history",
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      policyFieldSources: {
        allowSwitchUpstream: "account",
        fastModeRewriteMode: "account",
        imageToolRewriteMode: "account",
        availableModels: "account",
        forwardProxyKey: "account",
      },
      updatedAt: null,
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [],
      groups: [],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: 0,
      page: 1,
      pageSize: 500,
      metrics: { total: 0, oauth: 0, apiKey: 0, attention: 0 },
      routing: null,
    });
    apiMocks.updatePromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-history",
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      updatedAt: null,
    });
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 0,
      successCount: 0,
      failureCount: 0,
      totalCost: 0,
      totalTokens: 0,
      token: {
        requestCount: 0,
        totalTokens: 0,
        avgTokensPerRequest: 0,
        cacheInputTokens: 0,
        totalCost: 0,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: null,
        p95TotalMs: null,
      },
      exception: {
        failureCount: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
    });
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

  function findInputByAriaLabel(label: string) {
    return document.querySelector(
      `input[aria-label="${label}"]`,
    ) as HTMLInputElement | null;
  }

  async function clickDrawerTab(label: string) {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const tab = Array.from(document.querySelectorAll('[role="tab"]')).find(
      (node) => node.textContent?.includes(label),
    ) as HTMLElement | undefined;
    expect(tab).toBeTruthy();
    await user.click(tab!);
    await flushInteractive();
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
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 9,
      successCount: 7,
      failureCount: 2,
      totalCost: 1.25,
      totalTokens: 12000,
      token: {
        requestCount: 9,
        totalTokens: 12000,
        avgTokensPerRequest: 1333.33,
        cacheInputTokens: 4000,
        totalCost: 1.25,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: 12345,
        p95TotalMs: 22000,
      },
      exception: {
        failureCount: 2,
        serviceFailureCount: 2,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 2,
      },
    });
    apiMocks.fetchInvocationRecords.mockImplementation(async (query: {
      page?: number;
      snapshotId?: number;
      sortOrder?: string;
      pageSize?: number;
      signal?: AbortSignal;
    }) => {
      if (query.pageSize === 200) {
        return {
          snapshotId: 900,
          total: 0,
          page: 1,
          pageSize: 200,
          records: [],
        };
      }
      if (query.page === 1) {
        return {
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
        };
      }
      throw new Error("page 2 failed");
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

    expect(document.body.textContent).toContain("对话详情");
    expect(document.body.textContent).toContain("对话调用总览");
    expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledWith(
      expect.objectContaining({
        promptCacheKey: "pck-history",
      }),
    );

    await clickDrawerTab("调用");

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      promptCacheKey: "pck-history",
      page: 1,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
      signal: expect.any(AbortSignal),
    });
    expect(apiMocks.fetchInvocationRecords).not.toHaveBeenCalledWith(
      expect.objectContaining({
        promptCacheKey: "pck-history",
        page: 2,
        pageSize: 50,
      }),
    );

    const drawerBody = document.querySelector(".drawer-body");
    expect(drawerBody).toBeTruthy();
    Object.defineProperties(drawerBody as HTMLElement, {
      scrollHeight: { configurable: true, value: 1_000 },
      clientHeight: { configurable: true, value: 500 },
      scrollTop: { configurable: true, value: 500 },
    });
    await act(async () => {
      drawerBody?.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      promptCacheKey: "pck-history",
      page: 2,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
      snapshotId: 901,
      signal: expect.any(AbortSignal),
    });
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

  it("saves an upstream account binding from the history drawer", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding",
      bindingKind: "group",
      groupName: "prod",
      upstreamAccountId: null,
      upstreamAccountName: null,
      updatedAt: "2026-03-02T12:00:00Z",
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [
        createUpstreamAccountSummary(42, "Pool Alpha", "prod"),
        createUpstreamAccountSummary(77, "Pool Beta", "backup"),
        createUpstreamAccountSummary(88, "Pool Disabled", "disabled-only", {
          enabled: false,
          enableStatus: "disabled",
        }),
        createUpstreamAccountSummary(99, "Pool Inactive", "inactive-only", {
          status: "disabled",
          displayStatus: "disabled",
        }),
      ],
      groups: [
        { groupName: "prod", accountCount: 1 },
        { groupName: "backup", accountCount: 1 },
        { groupName: "disabled-only", accountCount: 1 },
        { groupName: "inactive-only", accountCount: 1 },
      ],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: 2,
      page: 1,
      pageSize: 500,
      metrics: { total: 2, oauth: 0, apiKey: 2, attention: 0 },
      routing: null,
    });
    apiMocks.updatePromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding",
      bindingKind: "upstreamAccount",
      groupName: null,
      upstreamAccountId: 77,
      upstreamAccountName: "Pool Beta",
      updatedAt: "2026-03-02T12:01:00Z",
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-binding",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const kindSelect = document.querySelector(
      '[role="combobox"][aria-label="绑定类型"]',
    ) as HTMLElement | null;
    const targetSelect = document.querySelector(
      '[role="combobox"][aria-label="分组绑定目标"]',
    ) as HTMLElement | null;
    expect(kindSelect?.textContent).toContain("分组");
    expect(targetSelect?.textContent).toContain("prod");
    expect(document.body.textContent).not.toContain(
      "保存后，下一个相同 Prompt Cache Key 请求生效。",
    );
    expect(document.body.textContent).not.toContain("不保留硬路由绑定。");
    expect(document.querySelectorAll("select")).toHaveLength(0);
    expect(document.querySelectorAll('[role="combobox"]')).toHaveLength(2);

    await user.click(kindSelect!);
    expect(findSelectOption("disabled-only")).toBeUndefined();
    expect(findSelectOption("inactive-only")).toBeUndefined();
    await user.click(findSelectOption("上游账号")!);
    await flushInteractive();

    const accountSelect = document.querySelector(
      '[role="combobox"][aria-label="账号绑定目标"]',
    ) as HTMLElement | null;
    await user.click(accountSelect!);
    expect(findSelectOption("Pool Disabled")).toBeUndefined();
    expect(findSelectOption("Pool Inactive")).toBeUndefined();
    await user.click(findSelectOption("Pool Beta")!);
    const saveButton = findButtonByAriaLabel("保存");
    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledWith(
      "pck-binding",
      {
        bindingKind: "upstreamAccount",
        upstreamAccountId: 77,
        timeouts: {},
      },
    );
    expect(document.body.textContent).toContain("当前：账号 Pool Beta");
  });

  it("allows retrying a binding save after a transient failure", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding",
      bindingKind: "group",
      groupName: "prod",
      upstreamAccountId: null,
      upstreamAccountName: null,
      updatedAt: "2026-03-02T12:00:00Z",
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [createUpstreamAccountSummary(42, "Pool Alpha", "prod")],
      groups: [{ groupName: "prod", accountCount: 1 }],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: 1,
      page: 1,
      pageSize: 500,
      metrics: { total: 1, oauth: 0, apiKey: 1, attention: 0 },
      routing: null,
    });
    apiMocks.updatePromptCacheConversationBinding
      .mockRejectedValueOnce(new Error("temporary save failure"))
      .mockResolvedValueOnce({
        promptCacheKey: "pck-binding",
        bindingKind: "group",
        groupName: "prod",
        upstreamAccountId: null,
        upstreamAccountName: null,
        updatedAt: "2026-03-02T12:01:00Z",
      });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-binding",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    const saveButton = findButtonByAriaLabel("保存");
    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("temporary save failure");
    expect(saveButton?.disabled).toBe(false);

    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("当前：分组 prod");
  });

  it("saves conversation routing policy overrides from the settings tab", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-policy",
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      allowSwitchUpstream: null,
      fastModeRewriteMode: "keep_original",
      imageToolRewriteMode: null,
      availableModels: null,
      forwardProxyKey: null,
      policyFieldSources: {
        allowSwitchUpstream: "account",
        fastModeRewriteMode: "account",
        imageToolRewriteMode: "account",
        availableModels: "account",
        forwardProxyKey: "account",
      },
      updatedAt: "2026-03-02T12:00:00Z",
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [createUpstreamAccountSummary(42, "Pool Alpha", "prod")],
      groups: [{ groupName: "prod", accountCount: 1 }],
      forwardProxyNodes: [
        {
          key: "__direct__",
          aliasKeys: [],
          source: "direct",
          displayName: "直连",
          protocolLabel: "DIRECT",
          penalized: false,
          selectable: true,
          last24h: [],
        },
      ],
      hasUngroupedAccounts: false,
      total: 1,
      page: 1,
      pageSize: 500,
      metrics: { total: 1, oauth: 0, apiKey: 1, attention: 0 },
      routing: null,
    });
    apiMocks.updatePromptCacheConversationBinding.mockImplementation(
      async (_promptCacheKey, payload) => ({
        promptCacheKey: "pck-policy",
        bindingKind: "none",
        groupName: null,
        upstreamAccountId: null,
        upstreamAccountName: null,
        allowSwitchUpstream:
          "allowSwitchUpstream" in payload ? payload.allowSwitchUpstream : null,
        fastModeRewriteMode:
          "fastModeRewriteMode" in payload
            ? payload.fastModeRewriteMode
            : "keep_original",
        imageToolRewriteMode:
          "imageToolRewriteMode" in payload ? payload.imageToolRewriteMode : null,
        availableModels:
          "availableModels" in payload ? payload.availableModels : null,
        forwardProxyKey:
          "forwardProxyKey" in payload ? payload.forwardProxyKey : null,
        policyFieldSources: {
          allowSwitchUpstream:
            "allowSwitchUpstream" in payload ? "conversation" : "account",
          fastModeRewriteMode:
            "fastModeRewriteMode" in payload ? "conversation" : "account",
          imageToolRewriteMode:
            "imageToolRewriteMode" in payload ? "conversation" : "account",
          availableModels:
            "availableModels" in payload ? "conversation" : "account",
          forwardProxyKey:
            "forwardProxyKey" in payload ? "conversation" : "account",
        },
        updatedAt: "2026-03-02T12:01:00Z",
      }),
    );
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-policy",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.click(findButtonByAriaLabel("编辑对话覆盖: 切出")!);
    await user.click(
      document.querySelector('[role="combobox"][aria-label="切出"]') as HTMLElement,
    );
    await user.click(findSelectOption("允许换上游")!);
    await flushInteractive();
    await vi.waitFor(() =>
      expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(1),
    );
    await vi.waitFor(() =>
      expect(findButtonByAriaLabel("编辑对话覆盖: FAST 模式")?.disabled).toBe(
        false,
      ),
    );
    await user.click(
      findButtonByAriaLabel("编辑对话覆盖: FAST 模式")!,
    );
    await user.click(
      document.querySelector('[role="combobox"][aria-label="FAST 模式"]') as HTMLElement,
    );
    await user.click(findSelectOption("强制添加")!);
    await flushInteractive();
    await vi.waitFor(() =>
      expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(2),
    );
    await user.click(findButtonByAriaLabel("编辑对话覆盖: 图片工具")!);
    await user.click(
      document.querySelector('[role="combobox"][aria-label="图片工具"]') as HTMLElement,
    );
    await user.click(findSelectOption("强制移除")!);
    await flushInteractive();
    await vi.waitFor(() =>
      expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(3),
    );
    await user.click(findButtonByAriaLabel("编辑对话覆盖: 代理")!);
    await user.click(
      document.querySelector('[role="combobox"][aria-label="代理"]') as HTMLElement,
    );
    await user.click(findSelectOption("直连 · DIRECT")!);
    await flushInteractive();
    await vi.waitFor(() =>
      expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(4),
    );
    await user.click(findButtonByAriaLabel("编辑对话覆盖: 可用模型")!);
    await vi.waitFor(() => expect(findInputByAriaLabel("可用模型")).toBeTruthy());
    const availableModelsInput = findInputByAriaLabel("可用模型")!;
    await user.clear(availableModelsInput);
    await user.type(
      availableModelsInput,
      "gpt-5.1-codex-max, gpt-5.1-codex-mini",
    );
    await user.click(findButtonByAriaLabel("应用覆盖")!);
    await flushInteractive();
    await vi.waitFor(() =>
      expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledTimes(5),
    );

    const policyPatchCalls = apiMocks.updatePromptCacheConversationBinding.mock.calls
      .filter(([key]) => key === "pck-policy")
      .map(([, payload]) => payload);
    expect(policyPatchCalls).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          allowSwitchUpstream: true,
          bindingKind: "none",
        }),
        expect.objectContaining({
          bindingKind: "none",
          fastModeRewriteMode: "force_add",
        }),
        expect.objectContaining({
          bindingKind: "none",
          imageToolRewriteMode: "force_remove",
        }),
        expect.objectContaining({
          bindingKind: "none",
          forwardProxyKeys: ["__direct__"],
        }),
        expect.objectContaining({
          availableModels: ["gpt-5.1-codex-max", "gpt-5.1-codex-mini"],
          bindingKind: "none",
        }),
      ]),
    );
  });

  it("does not allow saving a default binding draft after load failure", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockRejectedValue(
      new Error("binding load failed"),
    );
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-binding",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    expect(document.body.textContent).toContain("binding load failed");
    const saveButton = findButtonByAriaLabel("保存");
    expect(saveButton?.disabled).toBe(true);
    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.updatePromptCacheConversationBinding).not.toHaveBeenCalled();
  });

  it("defaults the drawer to overview and resets tabs after reopening", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-tab-reset",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("对话调用总览");
    expect(document.body.textContent).not.toContain("路由绑定");

    await clickDrawerTab("设置");
    expect(document.body.textContent).toContain("路由绑定");

    const closeButton = findButtonByAriaLabel("关闭调用记录抽屉");
    await act(async () => {
      closeButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("对话调用总览");
    expect(document.body.textContent).not.toContain("路由绑定");
  });

  it("keeps a single historical invocation visible in the activity chart", async () => {
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValueOnce({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 1,
      successCount: 1,
      failureCount: 0,
      totalCost: 0.31,
      totalTokens: 1500,
      token: {
        requestCount: 1,
        totalTokens: 1500,
        avgTokensPerRequest: 1500,
        cacheInputTokens: 300,
        totalCost: 0.31,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: 12345,
        p95TotalMs: 12345,
      },
      exception: {
        failureCount: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 901,
      total: 1,
      page: 1,
      pageSize: 200,
      records: [
        {
          id: 71,
          invokeId: "single-history-71",
          occurredAt: "2026-02-01T12:30:00Z",
          status: "completed",
          failureClass: "none",
          totalTokens: 1500,
          cost: 0.31,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-single-history",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy West",
          createdAt: "2026-02-01T12:30:00Z",
        },
      ],
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
          promptCacheKey: "pck-single-history",
          requestCount: 1,
          totalTokens: 1500,
          totalCost: 0.31,
          createdAt: "2026-02-01T12:30:00Z",
          lastActivityAt: "2026-02-01T12:30:00Z",
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
    await clickDrawerTab("概览");

    const chart = document.querySelector(
      '[data-testid="conversation-activity-chart"]',
    );
    expect(chart?.getAttribute("data-visible-total-count")).toBe("1");
    expect(chart?.getAttribute("data-visible-span")).toBe("1");
    expect(chart?.getAttribute("data-chart-range-start")).toBe(
      "2026-02-01T12:30:00.000Z",
    );
    expect(chart?.getAttribute("data-chart-range-end")).toBe(
      "2026-02-01T12:31:00.000Z",
    );

    await clickDrawerTab("调用");
    expect(document.body.textContent).toContain("Proxy West");
  });

  it("confirms before saving a group binding when an encrypted owner exists in the same group", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding",
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 42,
      encryptedOwnerAccountName: "Pool Alpha",
      encryptedOwnerGroupName: "prod",
      updatedAt: "2026-03-02T12:00:00Z",
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [
        createUpstreamAccountSummary(42, "Pool Alpha", "prod"),
        createUpstreamAccountSummary(77, "Pool Beta", "prod"),
      ],
      groups: [{ groupName: "prod", accountCount: 2 }],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: 2,
      page: 1,
      pageSize: 500,
      metrics: { total: 2, oauth: 0, apiKey: 2, attention: 0 },
      routing: null,
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
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
          promptCacheKey: "pck-binding",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const kindSelect = document.querySelector(
      '[role="combobox"][aria-label="绑定类型"]',
    ) as HTMLElement | null;
    await user.click(kindSelect!);
    await user.click(findSelectOption("分组")!);
    await flushInteractive();

    const groupSelect = document.querySelector(
      '[role="combobox"][aria-label="分组绑定目标"]',
    ) as HTMLElement | null;
    await user.click(groupSelect!);
    await user.click(findSelectOption("prod")!);

    const saveButton = findButtonByAriaLabel("保存");
    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    const confirmDialog = document.body.querySelector('[role="alertdialog"]');
    expect(confirmDialog?.textContent).toContain(
      "这个对话已经绑定加密会话 owner：Pool Alpha · prod",
    );
    expect(confirmDialog?.textContent).toContain("invalid_encrypted_content");
    expect(apiMocks.updatePromptCacheConversationBinding).not.toHaveBeenCalled();

    const cancelButton = Array.from(confirmDialog?.querySelectorAll("button") ?? [])
      .find((button) => button.textContent?.includes("取消"));
    await act(async () => {
      cancelButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    expect(document.body.querySelector('[role="alertdialog"]')).toBeNull();
    expect(apiMocks.updatePromptCacheConversationBinding).not.toHaveBeenCalled();

    apiMocks.updatePromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding",
      bindingKind: "group",
      groupName: "prod",
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 42,
      encryptedOwnerAccountName: "Pool Alpha",
      encryptedOwnerGroupName: "prod",
      updatedAt: "2026-03-02T12:05:00Z",
    });

    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    const reopenedDialog = document.body.querySelector('[role="alertdialog"]');
    const continueButton = Array.from(reopenedDialog?.querySelectorAll("button") ?? [])
      .find((button) => button.textContent?.includes("继续更改"));
    await act(async () => {
      continueButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledWith(
      "pck-binding",
      {
        bindingKind: "group",
        groupName: "prod",
        timeouts: {},
      },
    );
  });

  it("does not confirm when clearing a manual override back to the encrypted owner lock", async () => {
    apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding-clear",
      bindingKind: "group",
      groupName: "prod",
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 42,
      encryptedOwnerAccountName: "Pool Alpha",
      encryptedOwnerGroupName: "prod",
      updatedAt: "2026-03-02T12:00:00Z",
    });
    apiMocks.fetchUpstreamAccounts.mockResolvedValue({
      writesEnabled: true,
      items: [
        createUpstreamAccountSummary(42, "Pool Alpha", "prod"),
        createUpstreamAccountSummary(77, "Pool Beta", "prod"),
      ],
      groups: [{ groupName: "prod", accountCount: 2 }],
      forwardProxyNodes: [],
      hasUngroupedAccounts: false,
      total: 2,
      page: 1,
      pageSize: 500,
      metrics: { total: 2, oauth: 0, apiKey: 2, attention: 0 },
      routing: null,
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 1,
      total: 0,
      page: 1,
      pageSize: 200,
      records: [],
    });
    apiMocks.updatePromptCacheConversationBinding.mockResolvedValue({
      promptCacheKey: "pck-binding-clear",
      bindingKind: "none",
      groupName: null,
      upstreamAccountId: null,
      upstreamAccountName: null,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 42,
      encryptedOwnerAccountName: "Pool Alpha",
      encryptedOwnerGroupName: "prod",
      updatedAt: "2026-03-02T12:05:00Z",
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
          promptCacheKey: "pck-binding-clear",
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.01,
          createdAt: "2026-03-02T10:00:00Z",
          lastActivityAt: "2026-03-02T12:30:00Z",
          last24hRequests: [],
        }),
      ],
    });

    const historyButton = findButtonByAriaLabel("打开全部调用记录");
    await act(async () => {
      historyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();
    await clickDrawerTab("设置");

    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const kindSelect = document.querySelector(
      '[role="combobox"][aria-label="绑定类型"]',
    ) as HTMLElement | null;
    expect(kindSelect?.textContent).toContain("分组");
    await user.click(kindSelect!);
    await user.click(findSelectOption("清空")!);
    await flushInteractive();
    await clickDrawerTab("设置");

    const saveButton = findButtonByAriaLabel("保存");
    await act(async () => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushInteractive();

    expect(document.body.querySelector('[role="alertdialog"]')).toBeNull();
    expect(apiMocks.updatePromptCacheConversationBinding).toHaveBeenCalledWith(
      "pck-binding-clear",
      {
        bindingKind: "none",
        timeouts: {},
      },
    );
  });

  it("uses the first and latest same-day invocation timestamps as the history chart range", async () => {
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValueOnce({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 4,
      successCount: 4,
      failureCount: 0,
      totalCost: 0.48,
      totalTokens: 749_360,
      token: {
        requestCount: 4,
        totalTokens: 749_360,
        avgTokensPerRequest: 187_340,
        cacheInputTokens: 735_000,
        totalCost: 0.48,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: 15_800,
        p95TotalMs: 33_860,
      },
      exception: {
        failureCount: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 901,
      total: 4,
      page: 1,
      pageSize: 200,
      records: [
        {
          id: 84,
          invokeId: "same-day-84",
          occurredAt: "2026-05-13T23:40:47.000Z",
          status: "completed",
          failureClass: "none",
          totalTokens: 187_327,
          cost: 0.0972,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-same-day-short",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy Latest",
          createdAt: "2026-05-13T23:40:47.000Z",
        },
        {
          id: 83,
          invokeId: "same-day-83",
          occurredAt: "2026-05-13T23:38:49.000Z",
          status: "completed",
          failureClass: "none",
          totalTokens: 187_080,
          cost: 0.1558,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-same-day-short",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy Mid",
          createdAt: "2026-05-13T23:38:49.000Z",
        },
        {
          id: 82,
          invokeId: "same-day-82",
          occurredAt: "2026-05-13T23:37:43.000Z",
          status: "completed",
          failureClass: "none",
          totalTokens: 187_002,
          cost: 0.11,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-same-day-short",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy Mid",
          createdAt: "2026-05-13T23:37:43.000Z",
        },
        {
          id: 81,
          invokeId: "same-day-81",
          occurredAt: "2026-05-13T23:26:12.000Z",
          status: "completed",
          failureClass: "none",
          totalTokens: 188_951,
          cost: 0.117,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-same-day-short",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy First",
          createdAt: "2026-05-13T23:26:12.000Z",
        },
      ],
    });

    renderInteractive({
      rangeStart: "2026-05-13T16:00:00Z",
      rangeEnd: "2026-05-14T15:59:59Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-same-day-short",
          requestCount: 4,
          totalTokens: 749_360,
          totalCost: 0.48,
          createdAt: "2026-05-13T23:26:12.000Z",
          lastActivityAt: "2026-05-13T23:40:47.000Z",
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
    await clickDrawerTab("概览");

    const chart = document.querySelector(
      '[data-testid="conversation-activity-chart"]',
    );
    expect(chart?.getAttribute("data-visible-total-count")).toBe("4");
    expect(chart?.getAttribute("data-chart-range-start")).toBe(
      "2026-05-13T23:26:12.000Z",
    );
    expect(chart?.getAttribute("data-chart-range-end")).toBe(
      "2026-05-13T23:40:47.000Z",
    );
    expect(chart?.getAttribute("data-chart-range-start")).not.toBe(
      "2026-05-13T16:00:00.000Z",
    );
  });

  it("renders neutral activity buckets in the chart", async () => {
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValueOnce({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 1,
      successCount: 0,
      failureCount: 0,
      totalCost: 0.07,
      totalTokens: 700,
      token: {
        requestCount: 1,
        totalTokens: 700,
        avgTokensPerRequest: 700,
        cacheInputTokens: 200,
        totalCost: 0.07,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: 4321,
        p95TotalMs: 4321,
      },
      exception: {
        failureCount: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue({
      snapshotId: 901,
      total: 1,
      page: 1,
      pageSize: 200,
      records: [
        {
          id: 72,
          invokeId: "neutral-history-72",
          occurredAt: "2026-02-02T12:30:00Z",
          status: "",
          failureClass: "none",
          totalTokens: 700,
          cost: 0.07,
          endpoint: "/v1/responses",
          promptCacheKey: "pck-neutral-history",
          upstreamAccountId: 101,
          upstreamAccountName: "Pool Alpha",
          proxyDisplayName: "Proxy Neutral",
          createdAt: "2026-02-02T12:30:00Z",
        },
      ],
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
          promptCacheKey: "pck-neutral-history",
          requestCount: 1,
          totalTokens: 700,
          totalCost: 0.07,
          createdAt: "2026-02-02T12:30:00Z",
          lastActivityAt: "2026-02-02T12:30:00Z",
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

    expect(document.body.textContent).toContain("中性");
    await clickDrawerTab("调用");
    expect(document.body.textContent).toContain("Proxy Neutral");
  });

  it("preserves the retained first-to-latest chart range when history records are sampled", async () => {
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValueOnce({
      snapshotId: 900,
      newRecordsCount: 0,
      totalCount: 1001,
      successCount: 1001,
      failureCount: 0,
      totalCost: 100.1,
      totalTokens: 100100,
      token: {
        requestCount: 1001,
        totalTokens: 100100,
        avgTokensPerRequest: 100,
        cacheInputTokens: 0,
        totalCost: 100.1,
      },
      network: {
        avgTtfbMs: null,
        p95TtfbMs: null,
        avgTotalMs: 1000,
        p95TotalMs: 1000,
      },
      exception: {
        failureCount: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
    });
    apiMocks.fetchInvocationRecords.mockImplementation(async (query: {
      page?: number;
      pageSize?: number;
      snapshotId?: number;
      sortOrder?: string;
      signal?: AbortSignal;
    }) => {
      if (!query.signal) {
        const listRecords = [
          {
            id: 1001,
            invokeId: "sampled-history-1001",
            occurredAt: "2026-03-02T23:00:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 100,
            cost: 0.1,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-sampled-history",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: "Proxy 1001",
            createdAt: "2026-03-02T23:00:00Z",
          },
          {
            id: 1000,
            invokeId: "sampled-history-1000",
            occurredAt: "2026-03-02T22:59:00Z",
            status: "completed",
            failureClass: "none",
            totalTokens: 100,
            cost: 0.1,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-sampled-history",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: "Proxy 1000",
            createdAt: "2026-03-02T22:59:00Z",
          },
        ];
        return {
          snapshotId: 901,
          total: listRecords.length,
          page: query.page ?? 1,
          pageSize: query.pageSize ?? 200,
          records: query.page === 1 ? listRecords : [],
        };
      }
      const page = query.page ?? 1;
      const pageSize = query.pageSize ?? 200;
      const total = 1001;
      const startOffset = (page - 1) * pageSize;
      const count = Math.max(0, Math.min(pageSize, total - startOffset));
      return {
        snapshotId: 901,
        total,
        page,
        pageSize,
        records: Array.from({ length: count }, (_, index) => {
          const offset = startOffset + index;
          const id = total - offset;
          const occurredAt =
            offset === total - 1
              ? "2026-01-01T00:00:00Z"
              : new Date(Date.parse("2026-03-03T12:00:00Z") - offset * 60_000)
                  .toISOString();
          return {
            id,
            invokeId: `sampled-history-${id}`,
            occurredAt,
            status: "completed",
            failureClass: "none",
            totalTokens: 100,
            cost: 0.1,
            endpoint: "/v1/responses",
            promptCacheKey: "pck-sampled-history",
            upstreamAccountId: null,
            upstreamAccountName: null,
            proxyDisplayName: `Proxy ${id}`,
            createdAt: occurredAt,
          };
        }),
      };
    });

    renderInteractive({
      rangeStart: "2026-01-01T00:00:00Z",
      rangeEnd: "2026-03-03T12:00:00Z",
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [
        createConversation({
          promptCacheKey: "pck-sampled-history",
          requestCount: 1001,
          totalTokens: 100100,
          totalCost: 100.1,
          createdAt: "2026-01-01T00:00:00Z",
          lastActivityAt: "2026-03-03T12:00:00Z",
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

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith(
      expect.objectContaining({
        promptCacheKey: "pck-sampled-history",
        page: 6,
        pageSize: 200,
        sortBy: "occurredAt",
        sortOrder: "desc",
        snapshotId: 901,
      }),
    );
    const chart = document.querySelector(
      '[data-testid="conversation-activity-chart"]',
    );
    expect(chart?.getAttribute("data-chart-range-start")).toBe(
      "2026-01-01T00:00:00.000Z",
    );
    expect(chart?.getAttribute("data-chart-range-end")).toBe(
      "2026-03-03T12:00:00.000Z",
    );
    expect(document.body.textContent).toContain("01/01");
    expect(document.body.textContent).toContain("03/03");
    expect(document.body.textContent).toContain(
      "图表采样最近 1,000 / 1,001 条匹配调用",
    );
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

    apiMocks.fetchInvocationRecords.mockImplementation(async (query: {
      page?: number;
      snapshotId?: number;
      sortOrder?: string;
      pageSize?: number;
      signal?: AbortSignal;
    }) => {
      if (query.pageSize === 200) {
        return {
          snapshotId: 900,
          total: 0,
          page: 1,
          pageSize: 200,
          records: [],
        };
      }
      if (query.page === 1 && query.snapshotId == null) {
        return {
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
        };
      }
      return refreshPromise;
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

    await clickDrawerTab("调用");
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
        pageSize: 50,
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

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      promptCacheKey: "pck-history-live",
      page: 1,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
      signal: expect.any(AbortSignal),
    });
    expect(document.body.textContent).toContain("Proxy Final");
    expect(document.body.textContent).not.toContain("Proxy Running");
  });

  it("continues from page 2 after a silent refresh adopts a new history snapshot", async () => {
    let pageOneRequests = 0;
    apiMocks.fetchInvocationRecords.mockImplementation(async (query: {
      page?: number;
      snapshotId?: number;
      pageSize?: number;
    }) => {
      if (query.pageSize === 200) {
        return {
          snapshotId: 900,
          total: 0,
          page: 1,
          pageSize: 200,
          records: [],
        };
      }
      if (query.page === 1 && query.snapshotId == null) {
        pageOneRequests += 1;
        const snapshotId = pageOneRequests === 1 ? 902 : 903;
        return {
          snapshotId,
          total: 120,
          page: 1,
          pageSize: 50,
          records: [
            {
              id: snapshotId,
              invokeId: `history-snapshot-${snapshotId}`,
              occurredAt: "2026-03-02T12:35:00Z",
              status: "completed",
              promptCacheKey: "pck-history-snapshot",
              totalTokens: 1500,
              cost: 0.31,
              proxyDisplayName: `Proxy Snapshot ${snapshotId}`,
              createdAt: "2026-03-02T12:35:00Z",
            },
          ],
        };
      }
      if (query.page === 2) {
        const snapshotId = query.snapshotId ?? 903;
        return {
          snapshotId,
          total: 120,
          page: 2,
          pageSize: 50,
          records: [
            {
              id: 51,
              invokeId: "history-snapshot-page-2",
              occurredAt: "2026-03-02T12:10:00Z",
              status: "completed",
              promptCacheKey: "pck-history-snapshot",
              totalTokens: 900,
              cost: 0.2,
              proxyDisplayName: `Proxy Snapshot Page 2 ${snapshotId}`,
              createdAt: "2026-03-02T12:10:00Z",
            },
          ],
        };
      }
      throw new Error(`unexpected page ${query.page}`);
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
          promptCacheKey: "pck-history-snapshot",
          requestCount: 120,
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
    });
    await flushInteractive();

    await clickDrawerTab("调用");
    expect(document.body.textContent).toContain("Proxy Snapshot 902");

    const drawerBody = document.querySelector(".drawer-body");
    expect(drawerBody).toBeTruthy();
    Object.defineProperties(drawerBody as HTMLElement, {
      scrollHeight: { configurable: true, value: 1_000 },
      clientHeight: { configurable: true, value: 500 },
      scrollTop: { configurable: true, value: 500 },
    });
    await act(async () => {
      drawerBody?.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      promptCacheKey: "pck-history-snapshot",
      page: 2,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
      snapshotId: 902,
      signal: expect.any(AbortSignal),
    });
    await flushInteractive();
    expect(document.body.textContent).toContain("已加载 2 / 120");

    emitSseRecords({
      type: "records",
      records: [
        {
          id: 121,
          invokeId: "history-snapshot-live",
          occurredAt: "2026-03-02T12:40:00Z",
          createdAt: "2026-03-02T12:40:00Z",
          status: "completed",
          promptCacheKey: "pck-history-snapshot",
          totalTokens: 0,
          cost: 0,
          proxyDisplayName: "Proxy Snapshot Live",
        },
      ],
    });
    await flushInteractive();

    expect(document.body.textContent).toContain("Proxy Snapshot 903");
    expect(document.body.textContent).toContain("已加载 4 / 121");

    await act(async () => {
      drawerBody?.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await flushInteractive();

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      promptCacheKey: "pck-history-snapshot",
      page: 2,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
      snapshotId: 903,
      signal: expect.any(AbortSignal),
    });
    expect(apiMocks.fetchInvocationRecords).not.toHaveBeenCalledWith(
      expect.objectContaining({
        promptCacheKey: "pck-history-snapshot",
        page: 3,
        pageSize: 50,
        snapshotId: 903,
      }),
    );
    await flushInteractive();
    expect(document.body.textContent).toContain("已加载 4 / 121");
  });
});
