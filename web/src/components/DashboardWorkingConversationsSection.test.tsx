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
    upstreamAccountName:
      overrides.upstreamAccountName ?? "pool-alpha@example.com",
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
    createdAt:
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
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
  it("shows a bare hash in the card header while keeping the raw prompt cache key non-visible", () => {
    const cards = renderSection(
      createResponse([
        createConversation("019d68a9-9c32-7482-a353-71e4b6265f09", [
          createPreview({
            id: 1,
            invokeId: "invoke-header",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const card = host?.querySelector(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    if (!(card instanceof HTMLElement)) {
      throw new Error("missing working conversation card");
    }

    const expectedSortAnchorLabel = new Intl.DateTimeFormat("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(new Date("2026-04-04T10:04:00Z"));

    expect(card.textContent).toContain("运行中");
    expect(card.textContent).toContain(expectedSortAnchorLabel);
    expect(card.textContent).toContain("请求");
    expect(card.textContent).toContain("Token");
    expect(card.textContent).toContain("成本");
    expect(card.textContent).not.toContain("累计请求");
    expect(card.textContent).not.toContain("对话 Tokens");
    expect(card.textContent).not.toContain("对话成本");
    expect(card.textContent).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(card.textContent).not.toContain("WC-");
    expect(card.textContent).not.toContain("019d68a9-9c32-7482-a353-71e4b6265f09");
    expect(card.getAttribute("data-prompt-cache-key")).toBeNull();
    expect(card.getAttribute("data-conversation-sequence-id")).toBe(
      cards[0]?.conversationSequenceId.replace(/^WC-/, ""),
    );
  });

  it("places reasoning effort between the model name and service-tier indicator", () => {
    renderSection(
      createResponse([
        createConversation("pck-reasoning-layout", [
          createPreview({
            id: 1,
            invokeId: "invoke-reasoning-layout",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
            reasoningEffort: "medium",
            requestedServiceTier: "priority",
            serviceTier: "priority",
          }),
        ]),
      ]),
    );

    const currentSlot = host?.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLDivElement)) {
      throw new Error("missing current invocation slot");
    }

    const modelName = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-model-name"]',
    );
    const reasoningEffort = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-reasoning-effort"]',
    );
    const fastIcon = currentSlot.querySelector(
      '[data-testid="invocation-fast-icon"]',
    );
    if (
      !(modelName instanceof HTMLElement) ||
      !(reasoningEffort instanceof HTMLElement) ||
      !(fastIcon instanceof HTMLElement)
    ) {
      throw new Error("missing model/reasoning/service-tier markers");
    }

    expect(reasoningEffort.textContent).toContain("medium");
    expect(
      modelName.compareDocumentPosition(reasoningEffort) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(
      reasoningEffort.compareDocumentPosition(fastIcon) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
  });

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

  it("renders interrupted slots with the dedicated interrupted label", () => {
    renderSection(
      createResponse([
        createConversation("pck-interrupted", [
          createPreview({
            id: 2,
            invokeId: "invoke-interrupted",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "interrupted",
            failureClass: "service_failure",
            failureKind: "proxy_interrupted",
            errorMessage:
              "proxy request was interrupted before completion and was recovered on startup",
          }),
          createPreview({
            id: 1,
            invokeId: "invoke-interrupted-old",
            occurredAt: "2026-04-04T10:02:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("已中断");
    expect(text).not.toContain("失败");
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

    const accountButton = Array.from(
      host?.querySelectorAll("button") ?? [],
    ).find((button) => {
      const text = button.textContent ?? "";
      const title = button.getAttribute("title") ?? "";
      return (
        text.includes("pool-account-77") ||
        title.includes("pool-account-77@example.com")
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

    expect(currentSlot.getAttribute("aria-label")).toContain(
      cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "",
    );
    expect(currentSlot.getAttribute("aria-label")).not.toContain("WC-");

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
    expect(
      onOpenInvocation.mock.calls[0]?.[0]?.invocation?.record?.invokeId,
    ).toBe("invoke-slot-current");

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

    const card = host?.querySelector(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    const placeholder = host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    const placeholderLine = host?.querySelector(
      ".working-conversation-placeholder-line",
    );

    expect(card?.className).toContain("working-conversation-card-surface");
    expect(card?.className).not.toContain("bg-[linear-gradient");
    expect(placeholder?.className).toContain(
      "working-conversation-slot-surface",
    );
    expect(placeholderLine).not.toBeNull();
  });

  it("keeps the wide-screen grid contract on the conversations section", () => {
    renderSection(
      createResponse([
        createConversation("pck-grid-one", [
          createPreview({
            id: 1,
            invokeId: "invoke-grid-one",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "completed",
          }),
        ]),
      ]),
    );

    const grid = host?.querySelector(
      '[data-testid="dashboard-working-conversations-grid"]',
    );

    expect(grid).not.toBeNull();
    expect(grid?.className).toContain("xl:grid-cols-2");
    expect(grid?.className).toContain("2xl:grid-cols-3");
    expect(grid?.className).toContain("desktop1660:grid-cols-4");
  });
});
