/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { mapPromptCacheConversationsToDashboardCards } from "../lib/dashboardWorkingConversations";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    totalTokens: overrides.totalTokens ?? 200,
    cost: overrides.cost ?? 0.02,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    inputTokens: overrides.inputTokens ?? 120,
    outputTokens: overrides.outputTokens ?? 80,
    cacheInputTokens: overrides.cacheInputTokens ?? 30,
    reasoningTokens: overrides.reasoningTokens ?? 14,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 10,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 90,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 70,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 220,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 418,
  };
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): PromptCacheConversation {
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: 200,
    totalCost: 0.02,
    createdAt: recentInvocations[recentInvocations.length - 1]?.occurredAt ?? "2026-04-04T10:00:00Z",
    lastActivityAt: recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: [],
  };
}

function createResponse(
  conversations: PromptCacheConversation[],
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
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

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.restoreAllMocks();
});

function renderSection(
  response: PromptCacheConversationsResponse,
  options?: {
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
    onOpenInvocation?: (selection: {
      slotKind: "current" | "previous";
      conversationSequenceId: string;
      promptCacheKey: string;
      invocation: { record: { invokeId: string } };
    }) => void;
  },
) {
  const cards = mapPromptCacheConversationsToDashboardCards(response);
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <DashboardWorkingConversationsSection
          cards={cards}
          isLoading={false}
          error={null}
          onOpenUpstreamAccount={options?.onOpenUpstreamAccount}
          onOpenInvocation={options?.onOpenInvocation}
        />
      </I18nProvider>,
    );
  });
  return cards;
}

describe("DashboardWorkingConversationsSection", () => {
  it("renders a fixed previous-invocation placeholder when a conversation has only one call", () => {
    renderSection(
      createResponse([
        createConversation("pck-single", [
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );

    expect(placeholder).not.toBeNull();
    expect(placeholder?.textContent).toContain("上一条调用");
    expect(placeholder?.textContent).toContain("等高占位");
  });

  it("keeps the placeholder slot non-interactive when there is no previous invocation", () => {
    const onOpenInvocation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-single", [
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
      { onOpenInvocation },
    );

    const previousSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="previous"]',
    );
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    if (!(placeholder instanceof HTMLDivElement)) {
      throw new Error("missing placeholder");
    }

    act(() => {
      placeholder.click();
    });

    expect(previousSlot).toBeNull();
    expect(placeholder.getAttribute("role")).toBeNull();
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("keeps upstream account buttons interactive so the shared drawer can open", () => {
    const onOpenUpstreamAccount = vi.fn();
    const onOpenInvocation = vi.fn();
    renderSection(
      createResponse([
        createConversation("pck-account", [
          createPreview({
            id: 2,
            invokeId: "invoke-2",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
            upstreamAccountId: 77,
            upstreamAccountName: "pool-account-77@example.com",
          }),
          createPreview({
            id: 1,
            invokeId: "invoke-1",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
            upstreamAccountId: 77,
            upstreamAccountName: "pool-account-77@example.com",
          }),
        ]),
      ]),
      {
        onOpenUpstreamAccount,
        onOpenInvocation,
      },
    );

    const accountButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) => {
      const text = button.textContent ?? "";
      const title = button.getAttribute("title") ?? "";
      return (
        text.includes("pool-account-77") || title.includes("pool-account-77@example.com")
      );
    });
    if (!(accountButton instanceof HTMLButtonElement)) {
      throw new Error("missing account button");
    }

    act(() => {
      accountButton.click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(
      77,
      "pool-account-77@example.com",
    );
    expect(onOpenInvocation).not.toHaveBeenCalled();
  });

  it("opens invocation details from the slot container by click and keyboard", () => {
    const onOpenInvocation = vi.fn();
    const response = createResponse([
      createConversation("pck-slot-open", [
        createPreview({
          id: 2,
          invokeId: "invoke-slot-current",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
        createPreview({
          id: 1,
          invokeId: "invoke-slot-previous",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "completed",
        }),
      ]),
    ]);
    const cards = renderSection(response, { onOpenInvocation });

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLDivElement)) {
      throw new Error("missing current invocation slot");
    }

    act(() => {
      currentSlot.click();
    });

    expect(onOpenInvocation).toHaveBeenCalledWith(
      expect.objectContaining({
        slotKind: "current",
        conversationSequenceId: cards[0]?.conversationSequenceId,
        promptCacheKey: "pck-slot-open",
      }),
    );
    expect(onOpenInvocation.mock.calls[0]?.[0]?.invocation?.record?.invokeId).toBe(
      "invoke-slot-current",
    );

    act(() => {
      currentSlot.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });

    expect(onOpenInvocation).toHaveBeenCalledTimes(2);
  });

  it("uses theme-aware surface classes instead of a hardcoded dark canvas surface", () => {
    renderSection(
      createResponse([
        createConversation("pck-theme-aware", [
          createPreview({
            id: 1,
            invokeId: "invoke-theme-aware",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    const placeholderLine = host?.querySelector(".working-conversation-placeholder-line");

    expect(card?.className).toContain("working-conversation-card-surface");
    expect(card?.className).not.toContain("bg-[linear-gradient");
    expect(placeholder?.className).toContain("working-conversation-slot-surface");
    expect(placeholderLine).not.toBeNull();
  });
});
