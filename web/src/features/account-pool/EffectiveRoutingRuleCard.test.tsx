/** @vitest-environment jsdom */
import type * as React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { EffectiveRoutingRule } from "../../lib/api";
import {
  buildDefaultStatusChangeReasonFieldSources,
  buildDefaultStatusChangeReasons,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { EffectiveRoutingRuleCard } from "./EffectiveRoutingRuleCard";

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
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
  title: "Effective routing rule",
  description:
    "Merged routing constraints applied to the selected upstream account. Use account overrides when needed.",
  noTags: "No tags linked",
  allowCutOut: "Cut-out allowed",
  denyCutOut: "Cut-out blocked",
  allowCutIn: "Cut-in allowed",
  denyCutIn: "Cut-in blocked",
  sourceTags: "Source tags",
  priorityPrimary: "Primary",
  priorityNormal: "Normal",
  priorityFallback: "Fallback only",
  priorityNoNew: "No new",
  fastModeKeepOriginal: "Keep original",
  fastModeFillMissing: "Fill when missing",
  fastModeForceAdd: "Force add",
  fastModeForceRemove: "Force remove",
  imageToolKeepOriginal: "Keep original",
  imageToolFillMissing: "Fill when missing",
  imageToolForceAdd: "Force add",
  imageToolForceRemove: "Force remove",
  availableModelsInherited: "Inherited / unrestricted",
  availableModelsNoneAllowed: "No models allowed",
  availableModelsEmpty: "No matching models",
  systemDeniedModelsEmpty: "None",
  concurrencyLimit: (count: number) => `Concurrency ${count}`,
  concurrencyUnlimited: "Concurrency unlimited",
  sourceBreakdownTitle: "Field source breakdown",
  fieldAllowCutOut: "Cut out",
  fieldAllowCutIn: "Cut in",
  fieldPriority: "Priority",
  fieldFastMode: "FAST mode",
  fieldImageToolRewriteMode: "Image tools",
  fieldConcurrency: "Concurrency",
  fieldUpstream429: "Upstream 429 retry",
  fieldAvailableModels: "Available models",
  fieldSystemDeniedModels: "System denied models",
  fieldProxyBindings: "Account proxy",
  statusChangeReasonSectionTitle: "Status change trigger reasons",
  statusChangeReasonSectionHint:
    "Disabled reasons still keep evidence but no longer mutate account state.",
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
  statusChangeReasonSummary: (enabled: number, total: number) => `${enabled}/${total} enabled`,
  statusChangeReasonEnabledValue: "Triggers status change",
  statusChangeReasonDisabledValue: "Evidence only",
  statusChangeReasonToggleEnabled: "On",
  statusChangeReasonToggleDisabled: "Off",
  statusChangeReasonResetAction: "Reset",
  sourceRoot: "Root default",
  sourceGroup: "Group",
  sourceTag: "Tag",
  sourceAccount: "Account",
  sourceConversation: "Conversation",
  sourceSystem: "System",
  overrideEdit: "Edit account override",
  overrideClear: "Clear account override",
  overrideSaving: "Saving account override...",
  cutOutLabel: "Cut out",
  cutInLabel: "Cut in",
  upstream429RetryCountValue: (count: number) => String(count),
  currentValue: "Current value",
  availableModelsAddCustom: "Add model",
  availableModelsCustomLabel: (value: string) => value,
  availableModelsRemove: "Remove model",
  availableModelsPlaceholder: "Model id",
};

function buildRule(overrides: Partial<EffectiveRoutingRule> = {}): EffectiveRoutingRule {
  return {
    allowCutOut: true,
    allowCutIn: true,
    priorityTier: "normal",
    fastModeRewriteMode: "keep_original",
    imageToolRewriteMode: "keep_original",
    concurrencyLimit: 0,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableModels: [],
    systemDeniedModels: [],
    statusChangeReasons: buildDefaultStatusChangeReasons(),
    statusChangeReasonFieldSources: buildDefaultStatusChangeReasonFieldSources(),
    sourceTagIds: [],
    sourceTagNames: [],
    fieldSources: {
      allowCutOut: "root",
      allowCutIn: "root",
      priorityTier: "root",
      fastModeRewriteMode: "root",
      imageToolRewriteMode: "root",
      concurrencyLimit: "root",
      upstream429Retry: "root",
      availableModels: "root",
      systemDeniedModels: "root",
    },
    ...overrides,
  };
}

describe("EffectiveRoutingRuleCard", () => {
  it("shows inherited copy when no available model constraint is defined", () => {
    render(<EffectiveRoutingRuleCard rule={buildRule()} labels={labels} />);

    expect(document.body.textContent).toContain("Inherited / unrestricted");
    expect(document.body.textContent).toContain("Image tools");
    expect(document.body.textContent).not.toContain("No models allowed");
  });

  it("shows deny-all copy for empty tag intersections", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: [],
          sourceTagIds: [1, 2],
          sourceTagNames: ["allow-gpt-4o", "allow-o3"],
          fieldSources: {
            allowCutOut: "root",
            allowCutIn: "root",
            priorityTier: "tag",
            fastModeRewriteMode: "tag",
            concurrencyLimit: "tag",
            upstream429Retry: "root",
            availableModels: "tag",
            systemDeniedModels: "root",
          },
        })}
        labels={labels}
      />,
    );

    expect(document.body.textContent).toContain("No models allowed");
    expect(document.body.textContent).not.toContain("Inherited / unrestricted");
  });

  it("shows deny-all copy for empty group model overrides", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: [],
          fieldSources: {
            ...buildRule().fieldSources,
            availableModels: "group",
          },
        })}
        labels={labels}
      />,
    );

    expect(document.body.textContent).toContain("No models allowed");
    expect(document.body.textContent).not.toContain("Inherited / unrestricted");
  });

  it("renders rule values as semantic badges without the blocking summary strip", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          allowCutOut: false,
          allowCutIn: false,
          priorityTier: "no_new",
          fastModeRewriteMode: "force_add",
          fieldSources: {
            ...buildRule().fieldSources,
            allowCutOut: "tag",
            allowCutIn: "account",
            priorityTier: "tag",
            fastModeRewriteMode: "account",
          },
        })}
        labels={labels}
      />,
    );

    const blockedValues = Array.from(document.querySelectorAll('[class*="bg-warning"]')).map(
      (node) => node.textContent,
    );
    expect(blockedValues).toContain("No new");
    expect(blockedValues).toContain("Cut-out blocked");
    expect(blockedValues).toContain("Cut-in blocked");
    expect(blockedValues).toContain("No new");

    const forceAddBadge = Array.from(document.querySelectorAll('[class*="bg-primary"]')).find(
      (node) => node.textContent === "Force add",
    );
    expect(forceAddBadge).toBeTruthy();
    expect(document.body.textContent?.match(/Cut-out blocked/g)).toHaveLength(1);
    expect(document.body.textContent?.match(/Cut-in blocked/g)).toHaveLength(1);
  });

  it("expands the priority override and saves the no-new tier directly", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard rule={buildRule()} labels={labels} editablePolicy={{ onChange }} />,
    );

    const editButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Priority"]',
    );
    expect(editButton).not.toBeNull();

    act(() => {
      editButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(document.body.textContent).not.toContain(
      "Default value starts from the inherited value.",
    );
    const noNewButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.trim() === "No new",
    );
    expect(noNewButton).not.toBeNull();

    act(() => {
      noNewButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("priorityTier", {
      priorityTier: "no_new",
    });
  });

  it("clears an account override when the active override button is clicked again", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          allowCutIn: false,
          fieldSources: {
            ...buildRule().fieldSources,
            allowCutIn: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    );

    const clearButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Clear account override: Cut in"]',
    );
    expect(clearButton).not.toBeNull();

    act(() => {
      clearButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("allowCutIn", { allowCutIn: null });
  });

  it("keeps inherited timeout rows collapsed until the user expands one", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule()}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    );

    expect(
      document.querySelector<HTMLInputElement>('input[name="responsesFirstByteTimeoutSecs"]'),
    ).toBeNull();

    const timeoutButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Standard response first byte timeout"]',
    );
    expect(timeoutButton).not.toBeNull();
    expect(timeoutButton?.disabled).toBe(false);
  });

  it("expands an inherited timeout row instead of clearing it", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard rule={buildRule()} labels={labels} editablePolicy={{ onChange }} />,
    );

    const timeoutButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Standard response first byte timeout"]',
    );

    act(() => {
      timeoutButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).not.toHaveBeenCalled();
    expect(
      document.querySelector<HTMLInputElement>('input[name="responsesFirstByteTimeoutSecs"]'),
    ).not.toBeNull();
  });

  it("clears an account timeout override when its active override button is clicked", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          timeouts: {
            responsesFirstByteTimeoutSecs: 180,
            compactFirstByteTimeoutSecs: 300,
            responsesStreamTimeoutSecs: 300,
            compactStreamTimeoutSecs: 300,
          },
          timeoutFieldSources: {
            responsesFirstByteTimeoutSecs: "account",
            compactFirstByteTimeoutSecs: "root",
            responsesStreamTimeoutSecs: "root",
            compactStreamTimeoutSecs: "root",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    );

    const timeoutButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Clear account override: Standard response first byte timeout"]',
    );

    act(() => {
      timeoutButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("timeoutResponsesFirstByte", {
      timeouts: {
        responsesFirstByteTimeoutSecs: null,
      },
    });
  });

  it("expands the first account override by default", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          fastModeRewriteMode: "force_add",
          fieldSources: {
            ...buildRule().fieldSources,
            fastModeRewriteMode: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    );

    expect(document.body.textContent).not.toContain(
      "Default value starts from the inherited value.",
    );
    expect(document.body.textContent).toContain("FAST modeForce addAccountFAST mode");
    const activeButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Clear account override: FAST mode"]',
    );
    expect(activeButton?.getAttribute("aria-pressed")).toBe("true");
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull();
  });

  it("expands every existing account override by default", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          allowCutOut: false,
          allowCutIn: false,
          priorityTier: "primary",
          fastModeRewriteMode: "force_add",
          concurrencyLimit: 3,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 5,
          fieldSources: {
            ...buildRule().fieldSources,
            allowCutOut: "account",
            allowCutIn: "account",
            priorityTier: "account",
            fastModeRewriteMode: "account",
            concurrencyLimit: "account",
            upstream429Retry: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    );

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull();
    expect(document.querySelector('[role="switch"][aria-label="Cut in"]')).not.toBeNull();
    expect(document.querySelector('[role="radiogroup"][aria-label="Priority"]')).not.toBeNull();
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull();
    expect(
      document.querySelector('[role="radiogroup"][aria-label="Upstream 429 retry"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('button[role="switch"][aria-label="Upstream 429 retry"]'),
    ).toBeNull();
    expect(document.body.textContent).toContain("Concurrency 3");
    expect(document.body.textContent).toContain("5");
    expect(document.body.textContent).not.toContain("Account override");
    expect(document.body.textContent).not.toContain(
      "Default value starts from the inherited value.",
    );
    expect(document.body.textContent).toContain("Cut outCut-out blockedAccountCut out");
    expect(document.body.textContent).toContain("FAST modeForce addAccountFAST mode");
  });

  it("saves 429 retry as a single 0..5 count selector", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 4,
          fieldSources: {
            ...buildRule().fieldSources,
            upstream429Retry: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    );

    const zeroButton = document.querySelector<HTMLButtonElement>(
      '[role="radiogroup"][aria-label="Upstream 429 retry"] button[role="radio"]',
    );
    expect(zeroButton?.textContent).toBe("0");

    act(() => {
      zeroButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("upstream429Retry", {
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
    });
  });

  it("renders available models override as a tag selector trigger with selected chips", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: ["gpt-5.5", "gpt-5.4-mini"],
          fieldSources: {
            ...buildRule().fieldSources,
            availableModels: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    );

    const trigger = document.querySelector<HTMLButtonElement>(
      'button[role="combobox"][aria-label="Available models"]',
    );
    expect(trigger).not.toBeNull();
    expect(trigger?.textContent).toContain("gpt-5.5");
    expect(trigger?.textContent).toContain("gpt-5.4-mini");
    expect(document.body.textContent).not.toContain("Add gpt-5.5");
    expect(document.body.textContent).not.toContain("Add gpt-5.4-mini");
  });

  it("renders status change reasons with their resolved source and evidence-only state", () => {
    const baseReasons = buildDefaultStatusChangeReasons();
    const baseSources = buildDefaultStatusChangeReasonFieldSources();
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          statusChangeReasons: {
            ...baseReasons,
            upstream_http_401: false,
          },
          statusChangeReasonFieldSources: {
            ...baseSources,
            upstream_http_401: "account",
          },
        })}
        labels={labels}
      />,
    );

    expect(document.body.textContent).toContain("Status change trigger reasons");
    expect(document.body.textContent).toContain("401 invalid credentials");
    expect(document.body.textContent).toContain("10/11 enabled");
  });

  it("saves an account status-change reason override through the nested reason payload", () => {
    const onChange = vi.fn();
    render(
      <EffectiveRoutingRuleCard rule={buildRule()} labels={labels} editablePolicy={{ onChange }} />,
    );

    const toggleButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="401 invalid credentials"]',
    );
    expect(toggleButton).not.toBeNull();

    act(() => {
      toggleButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("statusChangeReason:upstream_http_401", {
      statusChangeReasons: {
        upstream_http_401: false,
      },
    });
  });

  it("resets account status-change reason overrides from the section header", () => {
    const onChange = vi.fn();
    const baseReasons = buildDefaultStatusChangeReasons();
    const baseSources = buildDefaultStatusChangeReasonFieldSources();
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          statusChangeReasons: {
            ...baseReasons,
            upstream_http_401: false,
            upstream_http_403: false,
          },
          statusChangeReasonFieldSources: {
            ...baseSources,
            upstream_http_401: "account",
            upstream_http_403: "account",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    );

    const resetButton = document.querySelector<HTMLButtonElement>('button[aria-label="Reset"]');
    expect(resetButton).not.toBeNull();
    expect(resetButton?.textContent).toContain("Reset");

    act(() => {
      resetButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("statusChangeReasons", {
      statusChangeReasons: {
        upstream_http_401: null,
        upstream_http_403: null,
      },
    });
  });

  it("keeps a user-opened inherited field when editable policy identity changes", () => {
    const rule = buildRule({
      fastModeRewriteMode: "force_add",
      fieldSources: {
        ...buildRule().fieldSources,
        fastModeRewriteMode: "account",
      },
    });
    const onChange = vi.fn();

    render(
      <EffectiveRoutingRuleCard
        rule={rule}
        identityKey="account-a"
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    );

    const cutOutButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Cut out"]',
    );
    expect(cutOutButton).not.toBeNull();

    act(() => {
      cutOutButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull();

    act(() => {
      root?.render(
        <EffectiveRoutingRuleCard
          rule={{
            ...rule,
            fieldSources: {
              ...rule.fieldSources,
            },
          }}
          identityKey="account-a"
          labels={labels}
          editablePolicy={{ onChange, busyField: null }}
        />,
      );
    });

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull();
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull();

    act(() => {
      root?.render(
        <EffectiveRoutingRuleCard
          rule={{
            ...rule,
            fieldSources: {
              ...rule.fieldSources,
            },
          }}
          identityKey="account-b"
          labels={labels}
          editablePolicy={{ onChange }}
        />,
      );
    });

    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull();
  });

  it("keeps system denied models read-only even when account policy editing is enabled", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          systemDeniedModels: ["gpt-5.5"],
          fieldSources: {
            ...buildRule().fieldSources,
            systemDeniedModels: "system",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    );

    expect(document.body.textContent).toContain("gpt-5.5");
    expect(
      document.querySelector('button[aria-label="Edit account override: System denied models"]'),
    ).toBeNull();
  });

  it("expands conversation-owned rows by default when conversation is the local override source", () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          fastModeRewriteMode: "force_add",
          imageToolRewriteMode: "force_remove",
          availableModels: [],
          fieldSources: {
            ...buildRule().fieldSources,
            fastModeRewriteMode: "conversation",
            imageToolRewriteMode: "conversation",
            availableModels: "conversation",
          },
          timeouts: {
            responsesFirstByteTimeoutSecs: 45,
            compactFirstByteTimeoutSecs: 300,
            responsesStreamTimeoutSecs: 225,
            compactStreamTimeoutSecs: 300,
          },
          timeoutFieldSources: {
            responsesFirstByteTimeoutSecs: "conversation",
            compactFirstByteTimeoutSecs: "root",
            responsesStreamTimeoutSecs: "conversation",
            compactStreamTimeoutSecs: "root",
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
        localOverrideSource="conversation"
        visibleRows={[
          "allowCutOut",
          "fastModeRewriteMode",
          "imageToolRewriteMode",
          "availableModels",
        ]}
        visibleSections={{
          statusChangeReasons: false,
          sourceTags: false,
        }}
      />,
    );

    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull();
    expect(document.querySelector('[role="radiogroup"][aria-label="Image tools"]')).not.toBeNull();
    expect(
      document.querySelector<HTMLInputElement>('input[name="responsesFirstByteTimeoutSecs"]'),
    ).not.toBeNull();
    expect(
      document.querySelector<HTMLInputElement>('input[name="responsesStreamTimeoutSecs"]'),
    ).not.toBeNull();
    expect(document.body.textContent).toContain("Conversation");
    expect(document.body.textContent).toContain("No models allowed");
    expect(document.body.textContent).not.toContain("Status change trigger reasons");
    expect(document.body.textContent).not.toContain("Source tags");
    expect(document.body.textContent).not.toContain("Cut in");
  });
});
