/** @vitest-environment jsdom */

import { waitFor } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import {
  createBulkConversationFetchMock,
  createConversation,
  createPreview,
  createResponse,
  host,
  renderSection,
} from "./DashboardWorkingConversationsSection.support";

it("confirms manual binding clear from the route bind dialog footer", async () => {
  const originalFetch = globalThis.fetch;
  let capturedPayload: Record<string, unknown> | null = null;
  const fetchMock = createBulkConversationFetchMock({
    onBulkPayload: (payload) => {
      capturedPayload = payload;
    },
  });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  const onConversationsChanged = vi.fn();

  try {
    renderSection(
      createResponse([
        createConversation("pck-clear-binding-1", [
          createPreview({
            id: 121,
            invokeId: "invoke-clear-binding-1",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
      { onConversationsChanged },
    );

    const user = userEvent.setup();
    await user.click(
      host?.querySelector(
        '[data-testid="dashboard-working-conversations-selection-mode-button"]',
      ) as HTMLElement,
    );
    await user.click(
      host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
    );
    await user.click(
      document.body.querySelector(
        '[data-testid="dashboard-working-conversations-route-bind-button"]',
      ) as HTMLElement,
    );

    await waitFor(() =>
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
        ),
      ).not.toBeNull(),
    );

    await user.click(
      document.body.querySelector(
        '[data-testid="dashboard-working-conversations-route-bind-clear-button"]',
      ) as HTMLElement,
    );

    const confirmDialog = document.body.querySelector(
      '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
    );
    expect(confirmDialog?.textContent).toContain("手工绑定");
    expect(confirmDialog?.textContent).toContain("sticky route");
    expect(confirmDialog?.textContent).toContain("owner lock");
    expect(confirmDialog?.textContent).not.toContain("重选");

    const confirmButton = Array.from(confirmDialog?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.includes("确认清空绑定"),
    );
    if (!(confirmButton instanceof HTMLButtonElement)) {
      throw new Error("missing clear binding confirm button");
    }
    await user.click(confirmButton);

    await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
    expect(capturedPayload).toMatchObject({
      action: "bind",
      bindingKind: "none",
      promptCacheKeys: ["pck-clear-binding-1"],
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-working-conversations-bulk-panel"]'),
    ).toBeNull();
  } finally {
    globalThis.fetch = originalFetch;
  }
});
it("opens the destructive clear flow from the bulk route bind dialog", async () => {
  const originalFetch = globalThis.fetch;
  const fetchMock = createBulkConversationFetchMock();
  globalThis.fetch = fetchMock as unknown as typeof fetch;

  try {
    renderSection(
      createResponse([
        createConversation("pck-bind-clear-shortcut-1", [
          createPreview({
            id: 111,
            invokeId: "invoke-bind-clear-shortcut-1",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
    );

    const user = userEvent.setup();
    await user.click(
      host?.querySelector(
        '[data-testid="dashboard-working-conversations-selection-mode-button"]',
      ) as HTMLElement,
    );
    await user.click(
      host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
    );
    await user.click(
      document.body.querySelector(
        '[data-testid="dashboard-working-conversations-route-bind-button"]',
      ) as HTMLElement,
    );

    await waitFor(() =>
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
        ),
      ).not.toBeNull(),
    );

    const clearShortcutButton = document.body.querySelector(
      '[data-testid="dashboard-working-conversations-route-bind-clear-button"]',
    );
    if (!(clearShortcutButton instanceof HTMLButtonElement)) {
      throw new Error("missing route bind clear shortcut button");
    }

    await user.click(clearShortcutButton);

    await waitFor(() => {
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
        ),
      ).toBeNull();
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
        ),
      ).not.toBeNull();
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

it.each([
  { label: "null", reasoningEffort: null },
  { label: "blank", reasoningEffort: " " },
])("omits $label grouped reasoning while keeping model and FAST semantics", ({
  reasoningEffort,
}) => {
  renderSection(
    createResponse([
      createConversation("pck-gpt56-missing-context", [
        createPreview({
          id: 3,
          invokeId: "invoke-gpt56-missing-context",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
          model: "gpt-5.6-sol",
          requestModel: "gpt-5.6-sol",
          responseModel: "gpt-5.6-sol",
          reasoningEffort: reasoningEffort ?? undefined,
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
  expect(cluster.querySelector('[data-model-context-part="reasoning-effort"]')).toBeNull();
  expect(cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]')).toBeNull();
  expect(cluster.textContent).not.toContain("—");
  expect(cluster.getAttribute("aria-label")).toContain("gpt-5.6-sol");
  expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
  expect(
    host
      ?.querySelector('[data-testid="dashboard-working-conversation-slot-model"]')
      ?.getAttribute("title"),
  ).toBe("gpt-5.6-sol");
});
it("submits the selected bulk FAST mode override", async () => {
  const originalFetch = globalThis.fetch;
  let capturedPayload: Record<string, unknown> | null = null;
  const fetchMock = createBulkConversationFetchMock({
    onBulkPayload: (payload) => {
      capturedPayload = payload;
    },
  });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  const onConversationsChanged = vi.fn();

  try {
    renderSection(
      createResponse([
        createConversation("pck-fast-mode-1", [
          createPreview({
            id: 111,
            invokeId: "invoke-fast-mode-1",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
          }),
        ]),
      ]),
      { onConversationsChanged },
    );

    const user = userEvent.setup();
    await user.click(
      host?.querySelector(
        '[data-testid="dashboard-working-conversations-selection-mode-button"]',
      ) as HTMLElement,
    );
    await user.click(
      host?.querySelector('[data-testid="dashboard-working-conversation-card"]') as HTMLElement,
    );
    await user.click(
      document.body.querySelector(
        '[data-testid="dashboard-working-conversations-fast-mode-button"]',
      ) as HTMLElement,
    );

    const fastModeOptions = document.body.querySelectorAll(
      '[data-testid="dashboard-working-conversations-fast-mode-option"]',
    );
    expect(fastModeOptions).toHaveLength(4);
    const forceRemoveOption = Array.from(fastModeOptions).find(
      (option) => (option as HTMLElement).dataset.value === "force_remove",
    );
    if (!(forceRemoveOption instanceof HTMLButtonElement)) {
      throw new Error("missing force remove fast mode option");
    }
    await user.click(forceRemoveOption);

    await waitFor(() => expect(onConversationsChanged).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(
        document.body.querySelector(
          '[data-testid="dashboard-working-conversations-fast-mode-popover"]',
        ),
      ).toBeNull(),
    );
    expect(capturedPayload).toMatchObject({
      action: "setFastModeRewriteMode",
      fastModeRewriteMode: "force_remove",
      promptCacheKeys: ["pck-fast-mode-1"],
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});
