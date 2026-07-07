/** @vitest-environment jsdom */
import * as React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { GroupAccountRoutingRule } from "../../lib/api";
import {
  buildDefaultStatusChangeReasons,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { GroupAccountRoutingRuleDialog } from "./GroupAccountRoutingRuleDialog";

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

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

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
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
});

let root: Root | null = null;
let host: HTMLDivElement | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  root = null;
  host = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

const labels = {
  allowCutOut: "Cut out is not blocked",
  allowCutIn: "Cut in is not blocked",
  forbidCutOut: "Block cut out",
  forbidCutIn: "Block cut in",
  priorityTier: "Preferred usage",
  priorityPrimary: "Primary",
  priorityNormal: "Normal",
  priorityFallback: "Fallback only",
  priorityNoNew: "No new",
  fastModeRewriteMode: "Fast mode",
  fastModeKeepOriginal: "Keep original",
  fastModeFillMissing: "Fill when missing",
  fastModeForceAdd: "Force add",
  fastModeForceRemove: "Force remove",
  imageToolRewriteMode: "Image tools",
  imageToolKeepOriginal: "Keep original",
  imageToolFillMissing: "Fill when missing",
  imageToolForceAdd: "Force add",
  imageToolForceRemove: "Force remove",
  imageToolRewriteHint:
    "Keep original follows the account's own image capability. Fill when missing only injects image tools when image intent is confirmed; force add always injects; force remove always strips it.",
  concurrencyLimit: "Concurrency limit",
  concurrencyHint:
    "Use 1-30 to cap fresh assignments. The last slider step means unlimited.",
  currentValue: "Current",
  unlimited: "Unlimited",
  availableModels: "Available models",
  availableModelsHint:
    "Leave empty to inherit. Automatic and sticky routing only consider matching accounts.",
  availableModelsSearchPlaceholder: "Search models",
  availableModelsEmpty: "No matching models",
  availableModelsAll: "Inherited / unrestricted",
  availableModelsCustomLabel: (value: string) => value,
  availableModelsAddCustom: "Add custom model id",
  availableModelsInherited: "Clear and inherit",
  availableModelsRemove: "Remove model",
  statusChangeReasonSectionTitle: "Status change trigger reasons",
  statusChangeReasonSectionHint:
    "Disabled reasons keep evidence only and do not change account state.",
  statusChangeReasonLabel: (reason: StatusChangeReasonCode) =>
    ({
      upstream_http_401: "401 invalid credentials",
      upstream_http_402: "402 plan or billing rejected",
      upstream_http_403: "403 permission rejected",
      reauth_required: "Reauthentication required",
      upstream_http_429_rate_limit: "429 rate limit",
      upstream_http_429_quota_exhausted: "429 quota exhausted",
      usage_snapshot_exhausted: "Usage snapshot exhausted",
      quota_still_exhausted: "Quota still exhausted",
      transport_failure: "Transport failure",
      upstream_server_overloaded: "Upstream overloaded",
      upstream_http_5xx: "Upstream 5xx",
    })[reason],
  statusChangeReasonToggleEnabled: "On",
  statusChangeReasonToggleDisabled: "Off",
  upstream429Retry: "Upstream 429 retry",
  upstream429RetryHint:
    "Retry the same upstream account before cooldown and failover.",
  upstream429RetryToggle: "Retry after upstream 429",
  upstream429RetryCount: "Retry count",
  upstream429RetryCountOnce: "1 retry",
  upstream429RetryCountMany: (count: number) => `${count} retries`,
  cancel: "Cancel",
  validation: "Review the routing policy before saving.",
};

const defaultRule: GroupAccountRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: "normal",
  fastModeRewriteMode: "keep_original",
  imageToolRewriteMode: "keep_original",
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  availableModels: [],
  statusChangeReasons: buildDefaultStatusChangeReasons(),
};

describe("GroupAccountRoutingRuleDialog", () => {
  it("submits the default image tool rewrite mode", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={defaultRule}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    expect(document.body.textContent).toContain("Image tools");
    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);

    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        imageToolRewriteMode: "keep_original",
        priorityTier: "normal",
        fastModeRewriteMode: "keep_original",
      }),
    );
    expect(onSubmit).toHaveBeenCalledWith(
      expect.not.objectContaining({
        availableModels: [],
      }),
    );
  });

  it("saves upstream 429 retry as a single 0..5 selector where 0 disables retry", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={{
          ...defaultRule,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 3,
        }}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    const retryGroup = document.querySelector(
      '[role="radiogroup"][aria-label="Upstream 429 retry"]',
    ) as HTMLElement | null;
    expect(retryGroup).not.toBeNull();
    const zeroRetry = Array.from(
      retryGroup?.querySelectorAll<HTMLButtonElement>('[role="radio"]') ?? [],
    ).find((button) => button.textContent?.trim() === "0");
    expect(zeroRetry).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      zeroRetry!.click();
    });

    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
      }),
    );
  });

  it("preserves an explicit empty model override after clearing existing models", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={{ ...defaultRule, availableModels: ["gpt-5.5"] }}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    const removeButton = Array.from(document.querySelectorAll("button")).find(
      (button) => button.getAttribute("aria-label") === "Remove model gpt-5.5",
    );
    expect(removeButton).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      removeButton!.click();
    });

    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        availableModels: [],
      }),
    );
  });

  it("submits an explicit empty model override after touching an inherited empty model list", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        changedFieldsOnly
        rule={defaultRule}
        availableModelOptions={["gpt-5.5"]}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    const input = document.querySelector('input[name="availableModelInput"]');
    expect(input).toBeInstanceOf(HTMLInputElement);
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    expect(valueSetter).toBeTypeOf("function");
    act(() => {
      valueSetter!.call(input, "gpt-5.5");
      input!.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const addButton = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Add custom model id",
    );
    expect(addButton).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      addButton!.click();
    });

    const removeButton = Array.from(document.querySelectorAll("button")).find(
      (button) => button.getAttribute("aria-label") === "Remove model gpt-5.5",
    );
    expect(removeButton).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      removeButton!.click();
    });

    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        availableModels: [],
      }),
    );
  });

  it("preserves an untouched explicit empty model override when changing another field", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={{
          ...defaultRule,
          availableModels: [],
          availableModelsDefined: true,
        }}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    const cutOutSwitch = Array.from(
      document.querySelectorAll('button[role="switch"]'),
    ).find((button) =>
      button.closest("div")?.textContent?.includes("Block cut out"),
    );
    expect(cutOutSwitch).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      cutOutSwitch!.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });

    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        allowCutOut: false,
        availableModels: [],
      }),
    );
  });

  it("renders a flat reason list without category headings and still updates individual reasons", () => {
    const onSubmit = vi.fn();
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={defaultRule}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    );

    expect(document.body.textContent).not.toContain("Auth & permission");
    expect(document.body.textContent).not.toContain("429 & quota");
    expect(document.body.textContent).not.toContain("Transport & 5xx");

    const authToggle = document.querySelector<HTMLButtonElement>(
      'button[aria-label="401 invalid credentials"]',
    );
    expect(authToggle).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      authToggle!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const submit = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "Apply group policy",
    );
    expect(submit).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      submit!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        statusChangeReasons: expect.objectContaining({
          upstream_http_401: false,
          upstream_http_402: true,
          upstream_http_403: true,
          reauth_required: true,
        }),
      }),
    );
  });
});
