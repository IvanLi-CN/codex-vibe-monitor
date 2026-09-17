/** @vitest-environment jsdom */

import { fireEvent } from "@testing-library/dom";
import { act } from "react";
import { expect, it, vi } from "vitest";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import {
  createConversation,
  createPreview,
  createResponse,
  createUpstreamAccountActivityResponse,
  host,
  renderSection,
  requireTestValue,
  rerenderSection,
  root,
  setHost,
  setRoot,
  UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS,
  upstreamAccountActivityMock,
} from "./DashboardWorkingConversationsSection.support";
import {
  DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY,
  readPersistedDashboardWorkspaceView,
} from "./dashboardActivityRange";

it("persists the preferred workspace view and restores it on remount", () => {
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  const response = createResponse([
    createConversation("pck-view-persist", [
      createPreview({
        id: 1,
        invokeId: "invoke-view-persist",
        occurredAt: "2026-04-04T10:04:00Z",
        status: "running",
      }),
    ]),
  ]);

  renderSection(response);

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  expect(readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY)).toBe(
    "upstreamAccounts",
  );

  act(() => {
    root?.unmount();
  });
  host?.remove();
  setHost(null);
  setRoot(null);

  renderSection(response);

  expect(host?.textContent).toContain("当前活动账号 1 个");
  expect(accountTab.getAttribute("aria-selected")).toBe("true");
});
it("preserves the upstream-account preference when usage temporarily forces conversations", () => {
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  const response = createResponse([
    createConversation("pck-usage-restore", [
      createPreview({
        id: 1,
        invokeId: "invoke-usage-restore",
        occurredAt: "2026-04-04T10:04:00Z",
        status: "running",
      }),
    ]),
  ]);

  renderSection(response);

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  rerenderSection(response, { activeRange: "usage" });
  expect(host?.textContent).toContain("当前对话 1 条");
  expect(readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY)).toBe(
    "upstreamAccounts",
  );

  rerenderSection(response, { activeRange: "today" });
  expect(host?.textContent).toContain("当前活动账号 1 个");
  const restoredAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (node) => node.textContent?.includes("上游账号"),
  );
  expect(restoredAccountTab?.getAttribute("aria-selected")).toBe("true");
});
it("switches workspace views without rendering the removed description", () => {
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  renderSection(
    createResponse([
      createConversation("pck-upstream-subtitle", [
        createPreview({
          id: 1,
          invokeId: "invoke-upstream-subtitle",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]),
  );

  expect(host?.textContent).not.toContain("展示最近 5 分钟内有终态调用");

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  expect(host?.textContent).not.toContain("展示当前总览范围内有调用的上游账号");
  expect(host?.textContent).not.toContain(
    "展示最近 5 分钟内有终态调用，或当前仍处于运行中 / 排队中的对话。",
  );
});
it("shows conversation short id, full request id, mismatch models, and real prompt cache key in upstream recent rows", () => {
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
  const onOpenInvocation = vi.fn();
  const onOpenConversation = vi.fn();

  renderSection(
    createResponse([
      createConversation("pck-upstream-anchor", [
        createPreview({
          id: 1,
          invokeId: "invoke-upstream-anchor",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]),
    { onOpenConversation, onOpenInvocation },
  );

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  const rows = Array.from(
    host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-row"]') ?? [],
  );
  const firstRow = rows[0];
  if (!(firstRow instanceof HTMLElement)) {
    throw new Error("missing first upstream recent row");
  }
  expect(firstRow.getAttribute("role")).toBeNull();
  expect(firstRow.getAttribute("tabindex")).toBeNull();
  const rowAction = firstRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-row-action"]',
  );
  expect(rowAction).toBeInstanceOf(HTMLButtonElement);

  const expectedConversationId = formatDashboardWorkingConversationSequenceId(
    `WC-${hashDashboardWorkingConversationKey("pck-upstream-running").slice(0, 6)}`,
  );
  const displayConversationId = expectedConversationId.replace(/^WC-/, "");

  const identity = firstRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-identity"]',
  );
  const identityChip = firstRow.querySelector(
    '[data-testid="dashboard-upstream-account-recent-identity-chip"]',
  );
  expect(identityChip).not.toBeNull();
  expect(identityChip?.className).toContain("rounded-full");
  expect(identityChip?.className).toContain("font-mono");
  expect(identity?.textContent).toContain(displayConversationId);
  expect(identity?.textContent).toContain("acct-invoke-1");
  expect(identity?.textContent).not.toContain("WC-");
  expect(firstRow.textContent).not.toContain("Pool Alpha");
  expect(firstRow.textContent).toContain("gpt-5.5-mini");
  expect(firstRow.textContent).toContain("gpt-5.5");
  expect(
    firstRow.querySelector(
      '[data-testid="dashboard-upstream-account-recent-model-routing-indicator"]',
    ),
  ).not.toBeNull();

  const reasoningBadge = firstRow.querySelector(
    '[data-testid="dashboard-working-conversation-reasoning-effort"]',
  );
  const endpointBadge = firstRow.querySelector('[data-testid="invocation-endpoint-badge"]');
  expect(reasoningBadge?.className).toContain("min-h-5");
  expect(endpointBadge?.className).toContain("min-h-5");

  act(() => {
    (rowAction as HTMLButtonElement).click();
  });

  expect(onOpenInvocation).toHaveBeenCalledWith(
    expect.objectContaining({
      promptCacheKey: "pck-upstream-running",
    }),
  );
  expect(onOpenInvocation.mock.calls[0]?.[0]?.promptCacheKey).not.toBe("acct-invoke-1");
  expect(onOpenConversation).not.toHaveBeenCalled();
});
it("opens conversation detail from the upstream recent identity chip only", () => {
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
  const onOpenInvocation = vi.fn();
  const onOpenConversation = vi.fn();

  renderSection(
    createResponse([
      createConversation("pck-upstream-anchor", [
        createPreview({
          id: 1,
          invokeId: "invoke-upstream-anchor",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]),
    { onOpenConversation, onOpenInvocation },
  );

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  const identityChip = host?.querySelector(
    '[data-testid="dashboard-upstream-account-recent-identity-chip"]',
  );
  if (!(identityChip instanceof HTMLButtonElement)) {
    throw new Error("missing upstream recent identity chip");
  }

  expect(identityChip.getAttribute("aria-label")).toContain("打开对话详情");
  expect(identityChip.getAttribute("aria-label")).toContain("pck-upstream-running");

  act(() => {
    identityChip.click();
  });

  expect(onOpenConversation).toHaveBeenCalledWith({
    conversationSequenceId: `WC-${hashDashboardWorkingConversationKey("pck-upstream-running").slice(0, 6)}`,
    promptCacheKey: "pck-upstream-running",
  });
  expect(onOpenInvocation).not.toHaveBeenCalled();

  onOpenConversation.mockClear();
  onOpenInvocation.mockClear();

  act(() => {
    identityChip.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });

  expect(onOpenConversation).toHaveBeenCalledTimes(1);
  expect(onOpenInvocation).not.toHaveBeenCalled();

  onOpenConversation.mockClear();

  act(() => {
    identityChip.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true }));
  });

  expect(onOpenConversation).toHaveBeenCalledTimes(1);
  expect(onOpenInvocation).not.toHaveBeenCalled();
});
it("spreads identity chip tones for prompt cache keys that used to collide on the same low-bit slot", () => {
  upstreamAccountActivityMock.data = {
    range: "today",
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    accounts: [
      {
        ...requireTestValue(
          createUpstreamAccountActivityResponse().accounts[0],
          "upstream account",
        ),
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        groupName: "Primary",
        planType: "enterprise",
        requestCount: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length,
        successCount: 0,
        failureCount: 0,
        nonSuccessCount: 0,
        totalTokens: 1600,
        successTokens: 0,
        nonSuccessTokens: 0,
        failureTokens: 0,
        failureCost: 0,
        totalCost: 0.12,
        cacheHitRate: 0.25,
        tokensPerMinute: 640,
        spendRate: 0.12,
        firstByteAvgMs: 420,
        currentFirstTokenAvgMs: 420,
        avgTotalMs: 860,
        currentAvgTotalMs: 860,
        inProgressInvocationCount: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length,
        retryInvocationCount: 0,
        recentInvocations: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.map((promptCacheKey, index) =>
          createPreview({
            id: 9100 + index,
            invokeId: `acct-tone-${index + 1}`,
            promptCacheKey,
            occurredAt: `2026-04-04T10:0${5 - index}:00Z`,
            status: "running",
            upstreamAccountName: "Pool Alpha",
          }),
        ),
      },
    ],
  };

  renderSection(
    createResponse([
      createConversation("pck-upstream-tone-anchor", [
        createPreview({
          id: 1,
          invokeId: "invoke-upstream-tone-anchor",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]),
    { recentPreviewLimit: UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length },
  );

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  const identityChips = Array.from(
    host?.querySelectorAll('[data-testid="dashboard-upstream-account-recent-identity-chip"]') ?? [],
  );
  expect(identityChips).toHaveLength(UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS.length);

  const toneClassNames = identityChips.map((chip) => chip.className);
  expect(new Set(toneClassNames).size).toBeGreaterThanOrEqual(4);

  const renderedShortIds = identityChips.map((chip) => chip.textContent?.trim());
  for (const promptCacheKey of UPSTREAM_IDENTITY_TONE_COLLISION_SEEDS) {
    const expectedShortId = formatDashboardWorkingConversationSequenceId(
      `WC-${hashDashboardWorkingConversationKey(promptCacheKey).slice(0, 6)}`,
    ).replace(/^WC-/, "");
    expect(renderedShortIds).toContain(expectedShortId);
  }
});
it("renders the WS transport badge only in websocket invocation slots", () => {
  renderSection(
    createResponse([
      createConversation("pck-ws-transport", [
        createPreview({
          id: 1,
          invokeId: "invoke-current-ws",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
          transport: "websocket",
        }),
        createPreview({
          id: 2,
          invokeId: "invoke-previous-http",
          occurredAt: "2026-04-04T10:03:00Z",
          status: "failed",
          transport: "http",
        }),
      ]),
    ]),
  );

  const badges = host?.querySelectorAll('[data-testid="invocation-transport-badge"]');
  expect(badges).toHaveLength(1);
  expect(badges?.[0]?.querySelector('[aria-hidden="true"]')?.textContent).toBe("WS");
  expect(badges?.[0]?.textContent).toContain("WebSocket transport");
  expect(badges?.[0]?.getAttribute("title")).toBe("WebSocket");
});
it("shows a bare hash in the card header while keeping the raw prompt cache key non-visible", () => {
  const cards = renderSection(
    createResponse([
      createConversation("019d68a9-9c32-7482-a353-71e4b6265f09", [
        createPreview({
          id: 1,
          invokeId: "invoke-header",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
          firstTokenMs: 720,
        }),
      ]),
    ]),
  );

  const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
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

  const currentPhaseBadge = card.querySelector(
    '[data-testid="invocation-phase-badge"][data-phase="responding"]',
  );
  expect(currentPhaseBadge).toBeInstanceOf(HTMLElement);
  expect(currentPhaseBadge?.getAttribute("aria-label")).toBe("响应中");
  expect(currentPhaseBadge?.getAttribute("data-phase-label-visible")).toBe("false");
  expect(card.textContent).toContain(expectedSortAnchorLabel);
  expect(card.textContent).toContain("请求");
  expect(card.textContent).toContain("Token");
  expect(card.textContent).toContain("成本");
  expect(card.textContent).not.toContain("累计请求");
  expect(card.textContent).not.toContain("对话 Tokens");
  expect(card.textContent).not.toContain("对话成本");
  expect(card.textContent).toContain(cards[0]?.conversationSequenceId.replace(/^WC-/, "") ?? "");
  expect(card.textContent).not.toContain("WC-");
  expect(card.textContent).not.toContain("019d68a9-9c32-7482-a353-71e4b6265f09");
  expect(card.getAttribute("data-prompt-cache-key")).toBeNull();
  expect(card.getAttribute("data-anchor-prompt-cache-key")).toBeNull();
  expect(card.getAttribute("data-conversation-sequence-id")).toBe(
    cards[0]?.conversationSequenceId.replace(/^WC-/, ""),
  );
});
it("keeps rendered cards visible while surfacing a non-blocking error banner", () => {
  renderSection(
    createResponse([
      createConversation("pck-inline-error", [
        createPreview({
          id: 1,
          invokeId: "invoke-inline-error",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
        }),
      ]),
    ]),
    {
      error: "load more temporarily unavailable",
    },
  );

  expect(host?.querySelector('[data-testid="dashboard-working-conversation-card"]')).toBeTruthy();
  expect(host?.textContent).toContain("load more temporarily unavailable");
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
  const fastIcon = currentSlot.querySelector('[data-testid="invocation-fast-icon"]');
  if (
    !(modelName instanceof HTMLElement) ||
    !(reasoningEffort instanceof HTMLElement) ||
    !(fastIcon instanceof HTMLElement)
  ) {
    throw new Error("missing model/reasoning/service-tier markers");
  }

  expect(reasoningEffort.textContent).toContain("medium");
  expect(
    modelName.compareDocumentPosition(reasoningEffort) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).not.toBe(0);
  expect(
    reasoningEffort.compareDocumentPosition(fastIcon) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).not.toBe(0);
});
it("groups GPT-5.6 model, reasoning effort, and FAST metadata", () => {
  renderSection(
    createResponse([
      createConversation("pck-gpt56-context", [
        createPreview({
          id: 1,
          invokeId: "invoke-gpt56-context",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
          model: "gpt-5.6-sol",
          requestModel: "gpt-5.6-sol",
          responseModel: "gpt-5.6-sol",
          reasoningEffort: "high",
          requestedServiceTier: "priority",
          serviceTier: "priority",
        }),
      ]),
    ]),
  );

  const cluster = host?.querySelector(
    '[data-testid="dashboard-working-conversation-model-context"]',
  );
  if (!(cluster instanceof HTMLElement)) {
    throw new Error("missing GPT-5.6 model context cluster");
  }

  expect(cluster.getAttribute("data-model-context-grouped")).toBe("true");
  expect(cluster.getAttribute("aria-label")).toContain("gpt-5.6-sol");
  expect(
    host
      ?.querySelector('[data-testid="dashboard-working-conversation-slot-header"]')
      ?.getAttribute("aria-label"),
  ).toEqual(expect.stringContaining("gpt-5.6-sol"));
  expect(
    host
      ?.querySelector('[data-testid="dashboard-working-conversation-slot-header"]')
      ?.getAttribute("aria-label"),
  ).toEqual(expect.stringContaining("Fast"));
  expect(cluster.querySelector('[data-model-icon="white-balance-sunny"]')).not.toBeNull();
  expect(cluster.querySelector('[data-model-context-part="model"]')?.className).toContain("w-5");
  expect(cluster.querySelector('[data-model-context-part="model"]')?.className).not.toContain(
    "flex-1",
  );
  expect(
    cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]'),
  ).not.toBeNull();
  expect(
    cluster
      .querySelector('[data-model-context-part="reasoning-effort-marker"]')
      ?.getAttribute("aria-hidden"),
  ).toBe("true");
  expect(
    cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]')?.className,
  ).toContain("bg-warning");
  expect(
    cluster.querySelector(
      '[data-testid="dashboard-working-conversation-model-context-reasoning-effort"]',
    )?.textContent,
  ).toContain("high");
  expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
  expect(
    Array.from(cluster.children).map((child) => child.getAttribute("data-model-context-part")),
  ).toEqual(["model", "reasoning-effort", "fast"]);
  expect(cluster.querySelectorAll('[class~="w-px"]')).toHaveLength(0);
});
it("uses the error marker tone for max reasoning effort", () => {
  renderSection(
    createResponse([
      createConversation("pck-gpt56-max-context", [
        createPreview({
          id: 2,
          invokeId: "invoke-gpt56-max-context",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
          model: "gpt-5.6-sol",
          requestModel: "gpt-5.6-sol",
          responseModel: "gpt-5.6-sol",
          reasoningEffort: "max",
          requestedServiceTier: "priority",
          serviceTier: "priority",
        }),
      ]),
    ]),
  );

  const marker = host?.querySelector(
    '[data-testid="dashboard-working-conversation-model-context"] [data-model-context-part="reasoning-effort-marker"]',
  );
  expect(marker).not.toBeNull();
  expect(marker?.className).toContain("bg-error");
});
it("reuses the grouped context cluster in upstream-account recent rows", () => {
  const upstreamActivity = createUpstreamAccountActivityResponse();
  const firstInvocation = upstreamActivity.accounts[0]?.recentInvocations[0];
  if (!firstInvocation) throw new Error("missing upstream recent invocation fixture");
  upstreamAccountActivityMock.data = {
    ...upstreamActivity,
    accounts: [
      {
        ...upstreamActivity.accounts[0],
        recentInvocations: [
          {
            ...firstInvocation,
            model: "gpt-5.6-sol",
            requestModel: "gpt-5.6-sol",
            responseModel: "gpt-5.6-sol",
            reasoningEffort: "high",
          },
        ],
      },
    ],
  };

  renderSection(createResponse([]));
  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) throw new Error("missing upstream account tab");
  act(() => {
    fireEvent.click(accountTab);
  });

  const cluster = host?.querySelector(
    '[data-testid="dashboard-upstream-account-recent-model-context"]',
  );
  if (!(cluster instanceof HTMLElement)) {
    throw new Error("missing upstream recent model context cluster");
  }
  expect(cluster.getAttribute("data-model-context-grouped")).toBe("true");
  expect(
    cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]'),
  ).not.toBeNull();
  expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
});
