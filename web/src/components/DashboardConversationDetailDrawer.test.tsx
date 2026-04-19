/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationSelection,
} from "../lib/dashboardWorkingConversations";
import { DashboardConversationDetailDrawer } from "./DashboardConversationDetailDrawer";

const { historyMocks } = vi.hoisted(() => ({
  historyMocks: {
    usePromptCacheConversationHistory: vi.fn(),
  },
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, values?: Record<string, string | number>) => {
      if (key === "dashboard.workingConversations.conversationDrawer.title") {
        return `conversation:${values?.sequenceId ?? ""}`;
      }
      if (values?.loaded != null && values?.total != null) {
        return `${key}:${values.loaded}/${values.total}`;
      }
      if (values?.count != null) return `${key}:${values.count}`;
      if (values?.id != null) return `${key}:${values.id}`;
      return key;
    },
  }),
}));

vi.mock("./prompt-cache-conversation-history-shared", () => ({
  PromptCacheConversationInvocationTable: ({
    records,
  }: {
    records: Array<{ invokeId?: string }>;
  }) => (
    <div data-testid="dashboard-history-table">
      {records.map((record, index) => (
        <div key={record.invokeId ?? index}>{record.invokeId ?? "record"}</div>
      ))}
    </div>
  ),
  usePromptCacheConversationHistory:
    historyMocks.usePromptCacheConversationHistory,
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
  vi.restoreAllMocks();
  historyMocks.usePromptCacheConversationHistory.mockReset();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function createInvocation(): DashboardWorkingConversationInvocationModel {
  return {
    preview: {
      id: 1,
      invokeId: "invoke-dashboard-current",
      occurredAt: "2026-04-19T09:00:00Z",
      status: "running",
      failureClass: null,
      routeMode: "forward_proxy",
      model: "gpt-5.4",
      totalTokens: 1234,
      cost: 1.2345,
      proxyDisplayName: "tokyo-edge-01",
      upstreamAccountId: 42,
      upstreamAccountName: "pool-alpha@example.com",
      endpoint: "/v1/responses",
      source: "proxy",
      inputTokens: 600,
      outputTokens: 634,
      cacheInputTokens: 0,
      reasoningTokens: 20,
      reasoningEffort: "high",
      errorMessage: undefined,
      failureKind: undefined,
      isActionable: undefined,
      responseContentEncoding: "gzip",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      tReqReadMs: 10,
      tReqParseMs: 5,
      tUpstreamConnectMs: 30,
      tUpstreamTtfbMs: 80,
      tUpstreamStreamMs: 200,
      tRespParseMs: 12,
      tPersistMs: 6,
      tTotalMs: 343,
    },
    record: {
      id: 1,
      invokeId: "invoke-dashboard-current",
      occurredAt: "2026-04-19T09:00:00Z",
      createdAt: "2026-04-19T09:00:00Z",
      status: "running",
      source: "proxy",
      routeMode: "forward_proxy",
      proxyDisplayName: "tokyo-edge-01",
      upstreamAccountId: 42,
      upstreamAccountName: "pool-alpha@example.com",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      inputTokens: 600,
      outputTokens: 634,
      cacheInputTokens: 0,
      reasoningTokens: 20,
      reasoningEffort: "high",
      totalTokens: 1234,
      cost: 1.2345,
      responseContentEncoding: "gzip",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      billingServiceTier: "priority",
      tReqReadMs: 10,
      tReqParseMs: 5,
      tUpstreamConnectMs: 30,
      tUpstreamTtfbMs: 80,
      tUpstreamStreamMs: 200,
      tRespParseMs: 12,
      tPersistMs: 6,
      tTotalMs: 343,
    },
    displayStatus: "running",
    occurredAtEpoch: Date.parse("2026-04-19T09:00:00Z"),
    isInFlight: true,
    isTerminal: false,
    tone: "running",
  };
}

function createSelection(): DashboardWorkingConversationSelection {
  const currentInvocation = createInvocation();
  return {
    conversationSequenceId: "WC-4E063D",
    promptCacheKey:
      "prompt_cache_key_that_should_wrap_inside_the_summary_panel_without_truncating_the_visible_identifier",
    createdAtEpoch: Date.parse("2026-04-19T08:55:00Z"),
    lastActivityAtEpoch: Date.parse("2026-04-19T09:01:00Z"),
    requestCount: 50,
    totalTokens: 5313107,
    totalCost: 2.7894,
    currentInvocation,
    previousInvocation: null,
  };
}

describe("DashboardConversationDetailDrawer", () => {
  it("keeps the dashboard conversation drawer width constrained", () => {
    historyMocks.usePromptCacheConversationHistory.mockReturnValue({
      visibleRecords: [],
      effectiveTotal: 0,
      loadedCount: 0,
      isLoading: false,
      error: null,
      hasHydrated: true,
    });

    render(
      <DashboardConversationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    const drawerShell = document.body.querySelector(".drawer-shell");
    expect(drawerShell?.className).toContain("max-w-[72rem]");

    const promptCacheField = Array.from(
      document.body.querySelectorAll("span"),
    ).find((element) =>
      element.textContent?.includes("prompt_cache_key_that_should_wrap_inside"),
    );
    expect(promptCacheField?.className).toContain("break-all");
  });
});
