/** @vitest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type { PromptCacheConversationsResponse } from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

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
        <I18nProvider>
          <PromptCacheConversationTable
            stats={stats}
            isLoading={false}
            error={null}
          />
        </I18nProvider>,
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
        {
          promptCacheKey: "pck-chat-001",
          requestCount: 12,
          totalTokens: 3456,
          totalCost: 1.2345,
          createdAt: "2026-03-02T00:00:00Z",
          lastActivityAt: "2026-03-02T16:00:00Z",
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
        },
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
        {
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
        },
        {
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
        },
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
        {
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
        },
        {
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
        },
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
        {
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
        },
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
        {
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
        },
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
});
