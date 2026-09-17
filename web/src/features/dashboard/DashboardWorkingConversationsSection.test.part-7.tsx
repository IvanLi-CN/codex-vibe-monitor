/** @vitest-environment jsdom */

import { waitFor } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { expect, it, vi } from "vitest";
import {
  createBulkConversationFetchMock,
  createConversation,
  createPreview,
  createResponse,
  createUpstreamAccountActivityResponse,
  host,
  renderSection,
  requireTestValue,
  rerenderSection,
  root,
  upstreamAccountActivityMock,
} from "./DashboardWorkingConversationsSection.support";

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
  expect(placeholder.getAttribute("role")).toBe("group");
  expect(onOpenInvocation).not.toHaveBeenCalled();
});
it("renders interrupted slots with the dedicated interrupted status icon semantics", () => {
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

  const card = host?.querySelector('[data-testid="dashboard-working-conversation-card"]');
  if (!(card instanceof HTMLElement)) {
    throw new Error("missing interrupted conversation card");
  }

  const headerStatusIcon = card.querySelector('[data-testid="dashboard-inline-invocation-status"]');
  if (!(headerStatusIcon instanceof HTMLElement)) {
    throw new Error("missing interrupted header status icon");
  }

  expect(headerStatusIcon.getAttribute("aria-label")).toContain("已中断");
  expect(headerStatusIcon.getAttribute("aria-label")).not.toContain("失败");
  expect(card.textContent ?? "").not.toContain("已中断");
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
    return text.includes("pool-account-77") || title.includes("pool-account-77@example.com");
  });
  if (!(accountButton instanceof HTMLButtonElement)) {
    throw new Error("missing account button");
  }

  act(() => {
    accountButton.click();
  });

  expect(onOpenUpstreamAccount).toHaveBeenCalledWith(77, "pool-account-77@example.com");
  expect(onOpenInvocation).not.toHaveBeenCalled();
});
it("opens health events when clicking upstream account attention badges", async () => {
  const onOpenUpstreamAccount = vi.fn();
  const onOpenInvocation = vi.fn();
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  renderSection(createResponse([]), {
    onOpenUpstreamAccount,
    onOpenInvocation,
  });

  const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
  );
  if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    upstreamAccountTab.click();
  });

  const attentionBadges = host?.querySelector(
    '[data-testid="dashboard-upstream-account-attention-badges"]',
  );
  if (!(attentionBadges instanceof HTMLDivElement)) {
    throw new Error("missing upstream account attention badges");
  }
  expect(attentionBadges.className).not.toContain("rounded-full");
  expect(attentionBadges.className).not.toContain("border-base-300/70");
  expect(attentionBadges.className).not.toContain("bg-base-100/86");
  const attentionBadgeButtons = attentionBadges.querySelectorAll(
    '[data-testid="dashboard-upstream-account-attention-badge"]',
  );
  expect(attentionBadgeButtons).toHaveLength(2);
  expect(attentionBadges.querySelector("button")).toBe(attentionBadgeButtons[0]);

  act(() => {
    (attentionBadgeButtons[0] as HTMLButtonElement).click();
  });

  expect(onOpenUpstreamAccount).toHaveBeenCalledWith(42, "Pool Alpha", {
    tab: "healthEvents",
  });
  expect(onOpenInvocation).not.toHaveBeenCalled();
});
it("debounces upstream account quick policy writes as account-level overrides", async () => {
  vi.useFakeTimers();
  const originalFetch = globalThis.fetch;
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit) =>
      new Response(
        JSON.stringify({
          id: 42,
          displayName: "Pool Alpha",
          status: "active",
          routingRule: {},
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      ),
  );
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  try {
    renderSection(createResponse([]));

    const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
    );
    if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      upstreamAccountTab.click();
    });

    const policyBadge = host?.querySelector(
      '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="priority-new-conversations"]',
    );
    if (!(policyBadge instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account policy badge");
    }

    expect(policyBadge.textContent?.trim()).toBe("禁新");
    expect(policyBadge.dataset.policyTone).toBe("warning");

    act(() => {
      policyBadge.click();
    });
    await act(async () => {});
    expect(policyBadge.textContent?.trim()).toBe("普通");
    expect(policyBadge.dataset.policyTone).toBe("neutral");

    act(() => {
      policyBadge.click();
    });
    expect(policyBadge.textContent?.trim()).toBe("兜底");
    expect(policyBadge.dataset.policyTone).toBe("success");

    expect(fetchMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1000);
    });

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = requireTestValue(fetchMock.mock.calls[0], "bulk binding request");
    expect(String(init?.body)).toContain('"priorityTier":"fallback"');
  } finally {
    globalThis.fetch = originalFetch;
    vi.useRealTimers();
  }
});
it("keeps a confirmed quick policy when a same-version stale activity frame arrives", () => {
  const version = {
    epoch: "2026-09-05T12:00:00.000000000Z",
    generation: "4",
  };
  const confirmed = createUpstreamAccountActivityResponse();
  confirmed.routingStateVersion = version;
  const account = requireTestValue(confirmed.accounts[0], "confirmed upstream account");
  account.effectiveRoutingRule = {
    ...requireTestValue(account.effectiveRoutingRule, "effective routing rule"),
    priorityTier: "fallback",
  };
  upstreamAccountActivityMock.data = confirmed;

  renderSection(createResponse([]));
  const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
  );
  if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }
  act(() => {
    upstreamAccountTab.click();
  });
  const policyBadge = host?.querySelector(
    '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="priority-new-conversations"]',
  );
  if (!(policyBadge instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account policy badge");
  }
  expect(policyBadge.textContent?.trim()).toBe("兜底");

  upstreamAccountActivityMock.data = {
    ...confirmed,
    accounts: [
      {
        ...account,
        effectiveRoutingRule: {
          ...requireTestValue(account.effectiveRoutingRule, "effective routing rule"),
          priorityTier: "normal",
        },
      },
    ],
  };
  rerenderSection(createResponse([]));

  expect(policyBadge.textContent?.trim()).toBe("兜底");
});
it("cycles upstream account Fast mode as an account-level quick policy", async () => {
  vi.useFakeTimers();
  const originalFetch = globalThis.fetch;
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit) =>
      new Response(
        JSON.stringify({
          id: 42,
          displayName: "Pool Alpha",
          status: "active",
          routingRule: {},
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      ),
  );
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  try {
    renderSection(createResponse([]));

    const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
    );
    if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      upstreamAccountTab.click();
    });

    const fastBadge = host?.querySelector(
      '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="fast-mode-rewrite"]',
    );
    if (!(fastBadge instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account Fast policy badge");
    }

    expect(fastBadge.textContent?.trim()).toBe("强制Fast");
    expect(fastBadge.dataset.policyTone).toBe("primary");
    expect(fastBadge.getAttribute("title")).toContain("Fast 改写策略");
    expect(fastBadge.getAttribute("aria-label")).toContain("Fast 改写策略：强制Fast");

    act(() => {
      fastBadge.click();
    });
    expect(fastBadge.textContent?.trim()).toBe("禁Fast");
    expect(fastBadge.dataset.policyTone).toBe("warning");
    expect(fastBadge.disabled).toBe(false);

    act(() => {
      fastBadge.click();
    });
    expect(fastBadge.textContent?.trim()).toBe("不改Fast");
    expect(fastBadge.dataset.policyTone).toBe("neutral");
    expect(fastBadge.getAttribute("aria-label")).toContain("Fast 改写策略：不改Fast");
    expect(fastBadge.disabled).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1000);
    });

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = requireTestValue(fetchMock.mock.calls[0], "routing fallback request");
    expect(String(init?.body)).toContain('"routingRule"');
    expect(String(init?.body)).toContain('"fastModeRewriteMode":"keep_original"');
  } finally {
    globalThis.fetch = originalFetch;
    vi.useRealTimers();
  }
});
it("flushes a pending upstream account quick policy write on unmount", async () => {
  vi.useFakeTimers();
  const originalFetch = globalThis.fetch;
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit) =>
      new Response(
        JSON.stringify({
          id: 42,
          displayName: "Pool Alpha",
          status: "active",
          routingRule: {},
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      ),
  );
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();

  try {
    renderSection(createResponse([]));

    const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /上游账号|upstream account/i.test(candidate.textContent ?? ""),
    );
    if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }

    act(() => {
      upstreamAccountTab.click();
    });

    const policyBadge = host?.querySelector(
      '[data-testid="dashboard-upstream-account-policy-badge"][data-policy-key="priority-new-conversations"]',
    );
    if (!(policyBadge instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account policy badge");
    }

    act(() => {
      policyBadge.click();
    });
    expect(fetchMock).not.toHaveBeenCalled();

    act(() => {
      root?.unmount();
    });

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = requireTestValue(fetchMock.mock.calls[0], "routing normal request");
    expect(String(init?.body)).toContain('"routingRule"');
    expect(String(init?.body)).toContain('"priorityTier":"normal"');
  } finally {
    globalThis.fetch = originalFetch;
    vi.useRealTimers();
  }
});
it("keeps the concrete upstream account label on assigned-account blocked dashboard cards", () => {
  renderSection(
    createResponse([
      createConversation("pck-assigned-account-blocked", [
        createPreview({
          id: 61,
          invokeId: "invoke-assigned-account-blocked",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "failed",
          failureClass: "service_failure",
          failureKind: "pool_assigned_account_blocked",
          errorMessage:
            '[pool_assigned_account_blocked] upstream account group "sticky-preflight-missing" has no bound forward proxy nodes',
          upstreamAccountId: 52,
          upstreamAccountName: "sticky-account-52@example.com",
          proxyDisplayName: "tokyo-edge-blocked",
          tUpstreamTtfbMs: null,
          tUpstreamStreamMs: null,
          tTotalMs: 42,
        }),
        createPreview({
          id: 60,
          invokeId: "invoke-assigned-account-blocked-previous",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "completed",
          upstreamAccountId: 52,
          upstreamAccountName: "sticky-account-52@example.com",
        }),
      ]),
    ]),
  );

  const accountLabel = host?.querySelector('[title="sticky-account-52@example.com"]');
  expect(accountLabel).not.toBeNull();
  expect(accountLabel?.className).not.toContain("invocation-account-routing-in-progress");
  expect(host?.textContent ?? "").not.toContain("未分配上游账号");
});
it("marks the concrete upstream account as routing in progress on running dashboard cards", () => {
  renderSection(
    createResponse([
      createConversation("pck-routing-account", [
        createPreview({
          id: 81,
          invokeId: "invoke-routing-account",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "running",
          upstreamAccountId: 52,
          upstreamAccountName: "sticky-account-52@example.com",
        }),
      ]),
    ]),
  );

  const accountLabel = host?.querySelector('[title="sticky-account-52@example.com"]');
  expect(accountLabel).not.toBeNull();
  expect(accountLabel?.className).toContain("invocation-account-routing-in-progress");
  expect(host?.textContent ?? "").not.toContain("号池路由中");
});
it("uses the unassigned-account fallback only for true no-account dashboard cards", () => {
  renderSection(
    createResponse([
      createConversation("pck-true-no-account", [
        createPreview({
          id: 71,
          invokeId: "invoke-true-no-account",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "failed",
          failureClass: "service_failure",
          failureKind: "pool_no_available_account",
          errorMessage: "[pool_no_available_account] no assignable upstream account remains",
          upstreamAccountId: null,
          upstreamAccountName: null,
          proxyDisplayName: null,
          tUpstreamTtfbMs: null,
          tUpstreamStreamMs: null,
          tTotalMs: 38,
        }),
        createPreview({
          id: 70,
          invokeId: "invoke-true-no-account-previous",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "completed",
          upstreamAccountId: null,
          upstreamAccountName: null,
          proxyDisplayName: null,
        }),
      ]),
    ]),
  );

  const accountLabel = host?.querySelector('[title="未分配上游账号"]');
  expect(accountLabel).not.toBeNull();
});
it("shows the blocked-binding recovery banner and reuses the clear-affinity confirmation flow", async () => {
  const originalFetch = globalThis.fetch;
  let capturedPayload: Record<string, unknown> | null = null;
  const fetchMock = createBulkConversationFetchMock({
    onBulkPayload: (payload) => {
      capturedPayload = payload;
    },
  });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  const onClearBlockedBindingFilter = vi.fn();
  try {
    renderSection(
      createResponse([
        createConversation("pck-banner-blocked", [
          createPreview({
            id: 91,
            invokeId: "invoke-banner-blocked",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "failed",
            failureClass: "service_failure",
            failureKind: "pool_assigned_account_blocked",
            blockedBinding: {
              constraintSource: "encryptedSessionOwner",
              upstreamAccountId: 2890,
              upstreamAccountLabel: "dzw",
              promptCacheKey: "pck-banner-blocked",
              recoveryAction: "clearAndResetAffinity",
            },
          }),
        ]),
      ]),
      {
        activeBlockedBindingFilter: {
          upstreamAccountId: 2890,
          constraintSource: "encryptedSessionOwner",
        },
        onClearBlockedBindingFilter,
        onConversationsChanged: vi.fn(),
      },
    );

    const upstreamAccountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (node) => node.textContent?.includes("上游账号"),
    );
    if (!(upstreamAccountTab instanceof HTMLButtonElement)) {
      throw new Error("missing upstream account tab");
    }
    expect(upstreamAccountTab.disabled).toBe(true);
    expect(
      host?.querySelector('[data-testid="dashboard-blocked-binding-banner"]')?.textContent ?? "",
    ).toContain("单账号会话约束阻塞");
    expect(host?.textContent ?? "").toContain("dzw");

    const user = userEvent.setup();
    await user.click(
      host?.querySelector(
        '[data-testid="dashboard-blocked-binding-clear-filter-button"]',
      ) as HTMLElement,
    );
    expect(onClearBlockedBindingFilter).toHaveBeenCalledTimes(1);

    await user.click(
      host?.querySelector(
        '[data-testid="dashboard-blocked-binding-clear-and-reselect-button"]',
      ) as HTMLElement,
    );

    const confirmDialog = document.body.querySelector(
      '[data-testid="dashboard-working-conversations-clear-affinity-dialog"]',
    );
    expect(confirmDialog?.textContent).toContain("加密 owner 约束");
    expect(confirmDialog?.textContent).toContain("sticky route");
    expect(confirmDialog?.textContent).toContain("重选");

    const confirmButton = Array.from(confirmDialog?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.includes("确认清空并重选"),
    );
    if (!(confirmButton instanceof HTMLButtonElement)) {
      throw new Error("missing clear affinity confirm button");
    }
    await user.click(confirmButton);

    await waitFor(() =>
      expect(capturedPayload).toMatchObject({
        action: "clearAndResetAffinity",
        promptCacheKeys: ["pck-banner-blocked"],
      }),
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
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
  expect(onOpenInvocation.mock.calls[0]?.[0]?.invocation?.record?.invokeId).toBe(
    "invoke-slot-current",
  );

  act(() => {
    currentSlot.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });

  expect(onOpenInvocation).toHaveBeenCalledTimes(2);
});
