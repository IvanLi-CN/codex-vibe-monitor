import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import { OverlayHostProvider } from "../../components/ui/overlay-host";
import type { EffectiveRoutingRule, UpdateGroupAccountRoutingRulePayload } from "../../lib/api";
import {
  buildDefaultStatusChangeReasonFieldSources,
  buildDefaultStatusChangeReasons,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";
import {
  type AvailableModelOption,
  EffectiveRoutingRuleCard,
  type EffectiveRoutingRuleCardRowKey,
} from "./EffectiveRoutingRuleCard";

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
  availableModelsAllowlist: "Allowlist",
  availableModelsDenylist: "Denylist",
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
  imageToolRewriteHint:
    "These modes rewrite hosted Full Responses tools only. Configure Codex Full and Lite imagegen separately.",
  fieldCodexImagegenRewriteMode: "Codex imagegen",
  codexImagegenRewriteHint:
    "Controls the Codex client imagegen namespace separately from hosted image tools. Full uses top-level tools; Lite uses developer additional tools.",
  fieldConcurrency: "Concurrency",
  fieldUpstream429: "Upstream 429 retry",
  fieldAvailableModels: "Available models",
  fieldSystemDeniedModels: "System denied models",
  fieldProxyBindings: "Account proxy",
  fieldRequestCompression: "Request compression",
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
  sourceRoot: "Root default",
  sourceGroup: "Group",
  sourceTag: "Tag",
  sourceAccount: "Account",
  sourceSystem: "System",
  overrideEdit: "Edit account override",
  overrideActive: "Account override",
  overrideClear: "Clear account override",
  statusChangeReasonResetAction: "Reset",
  overrideSaving: "Saving account override...",
  overrideRetry: "Retry save",
  overrideRevert: "Revert to saved",
  inheritValue: "Default value starts from the inherited value.",
  cutOutLabel: "Cut out",
  cutInLabel: "Cut in",
  requestCompressionFollow: "Follow",
  requestCompressionIdentity: "Identity",
  requestCompressionGzip: "Gzip",
  requestCompressionDeflate: "Deflate",
  requestCompressionZstd: "Zstd",
  upstream429RetryCountValue: (count: number) => String(count),
  availableModelsAddCustom: "Add model",
  availableModelsCustomLabel: (value: string) => value,
  availableModelsRemove: "Remove model",
  availableModelsPlaceholder: "Model id",
  availableModelsSourceAll: "All sources",
  availableModelsSourceProject: "Project presets",
  availableModelsSourceAccount: "This account",
  availableModelsRefresh: "Refresh",
  availableModelsRefreshing: "Refreshing...",
  availableModelsLastRefreshed: (value: string) => `Last refreshed ${value}`,
  availableModelsRefreshError: "The account model catalog refresh failed.",
  availableModelsStale: "This account catalog is older than 24 hours.",
  availableModelsUnmatched: "Selected value",
  currentValue: "Current value",
};

const relaxedRule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: "normal",
  fastModeRewriteMode: "keep_original",
  imageToolRewriteMode: "keep_original",
  codexImagegenRewriteMode: "keep_original",
  requestCompressionAlgorithm: "identity",
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  availableModels: [],
  availableModelsMode: "denylist",
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
    codexImagegenRewriteMode: "root",
    requestCompressionAlgorithm: "root",
    concurrencyLimit: "root",
    upstream429Retry: "root",
    availableModels: "root",
    systemDeniedModels: "root",
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 120,
    compactFirstByteTimeoutSecs: 300,
    imageFirstByteTimeoutSecs: 300,
    responsesStreamTimeoutSecs: 300,
    compactStreamTimeoutSecs: 300,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: "root",
    compactFirstByteTimeoutSecs: "root",
    imageFirstByteTimeoutSecs: "root",
    responsesStreamTimeoutSecs: "root",
    compactStreamTimeoutSecs: "root",
  },
};

const strictRule: EffectiveRoutingRule = {
  allowCutOut: false,
  allowCutIn: false,
  priorityTier: "no_new",
  fastModeRewriteMode: "force_remove",
  imageToolRewriteMode: "force_add",
  codexImagegenRewriteMode: "force_add",
  requestCompressionAlgorithm: "gzip",
  concurrencyLimit: 2,
  upstream429RetryEnabled: true,
  upstream429MaxRetries: 4,
  availableModels: ["gpt-5.5", "gpt-5.4-mini"],
  availableModelsMode: "allowlist",
  systemDeniedModels: ["gpt-5.5"],
  statusChangeReasons: {
    ...buildDefaultStatusChangeReasons(),
    upstream_http_401: false,
    upstream_http_429_quota_exhausted: false,
  },
  statusChangeReasonFieldSources: {
    ...buildDefaultStatusChangeReasonFieldSources(),
    upstream_http_401: "account",
    upstream_http_429_quota_exhausted: "group",
  },
  sourceTagIds: [1, 2],
  sourceTagNames: ["vip-routing", "handoff-blocked"],
  fieldSources: {
    allowCutOut: "tag",
    allowCutIn: "account",
    priorityTier: "account",
    fastModeRewriteMode: "account",
    imageToolRewriteMode: "tag",
    codexImagegenRewriteMode: "account",
    requestCompressionAlgorithm: "account",
    concurrencyLimit: "tag",
    upstream429Retry: "account",
    availableModels: "account",
    availableModelsMode: "account",
    systemDeniedModels: "system",
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 45,
    compactFirstByteTimeoutSecs: 300,
    imageFirstByteTimeoutSecs: 360,
    responsesStreamTimeoutSecs: 210,
    compactStreamTimeoutSecs: 300,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: "account",
    compactFirstByteTimeoutSecs: "root",
    imageFirstByteTimeoutSecs: "account",
    responsesStreamTimeoutSecs: "account",
    compactStreamTimeoutSecs: "group",
  },
};

const strictFieldSources = {
  allowCutOut: "tag",
  allowCutIn: "account",
  priorityTier: "tag",
  fastModeRewriteMode: "account",
  imageToolRewriteMode: "account",
  codexImagegenRewriteMode: "account",
  requestCompressionAlgorithm: "account",
  concurrencyLimit: "tag",
  upstream429Retry: "account",
  availableModels: "account",
  availableModelsMode: "account",
  systemDeniedModels: "system",
} as const;

const denyAllTagIntersectionRule: EffectiveRoutingRule = {
  ...strictRule,
  availableModels: [],
  systemDeniedModels: [],
  sourceTagIds: [1, 2],
  sourceTagNames: ["allow-gpt-4o", "allow-o3"],
  fieldSources: {
    ...strictFieldSources,
    availableModels: "tag",
    systemDeniedModels: "root",
  },
};

const multipleAccountOverridesRule: EffectiveRoutingRule = {
  ...strictRule,
  allowCutOut: false,
  allowCutIn: false,
  priorityTier: "primary",
  fastModeRewriteMode: "force_add",
  requestCompressionAlgorithm: "deflate",
  concurrencyLimit: 3,
  upstream429RetryEnabled: true,
  upstream429MaxRetries: 5,
  fieldSources: {
    allowCutOut: "account",
    allowCutIn: "account",
    priorityTier: "account",
    fastModeRewriteMode: "account",
    imageToolRewriteMode: strictRule.fieldSources?.imageToolRewriteMode ?? "root",
    codexImagegenRewriteMode: strictRule.fieldSources?.codexImagegenRewriteMode ?? "root",
    requestCompressionAlgorithm: "account",
    concurrencyLimit: "account",
    upstream429Retry: "account",
    availableModels: strictRule.fieldSources?.availableModels ?? "root",
    systemDeniedModels: strictRule.fieldSources?.systemDeniedModels ?? "root",
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 30,
    compactFirstByteTimeoutSecs: 180,
    imageFirstByteTimeoutSecs: 420,
    responsesStreamTimeoutSecs: 180,
    compactStreamTimeoutSecs: 240,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: "account",
    compactFirstByteTimeoutSecs: "account",
    imageFirstByteTimeoutSecs: "account",
    responsesStreamTimeoutSecs: "account",
    compactStreamTimeoutSecs: "group",
  },
};

const meta = {
  title: "Account Pool/Components/Effective Routing Rule Card",
  component: EffectiveRoutingRuleCard,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "账号详情页里的最终生效规则卡片。服务端已经合并多个 tag 规则，前端只负责展示最终约束与来源 tag。",
      },
    },
  },
  decorators: [
    (Story) => {
      const EvidenceSurface = () => {
        const [host, setHost] = useState<HTMLDivElement | null>(null);
        return (
          <OverlayHostProvider value={host}>
            <div
              ref={setHost}
              data-visual-evidence-surface="effective-routing-rule-card"
              className="min-h-screen bg-base-200 px-12 py-12 text-base-content"
            >
              <div
                className="effective-routing-rule-story-surface mx-auto w-full max-w-3xl"
                data-visual-evidence-target="effective-routing-rule-card"
              >
                <Story />
              </div>
            </div>
          </OverlayHostProvider>
        );
      };

      return <EvidenceSurface />;
    },
  ],
  args: {
    labels,
    rule: relaxedRule,
  },
} satisfies Meta<typeof EffectiveRoutingRuleCard>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const StrictMergedRule: Story = {
  args: {
    rule: strictRule,
  },
  play: async ({ canvasElement }) => {
    const warningValues = Array.from(canvasElement.querySelectorAll('[class*="bg-warning"]')).map(
      (node) => node.textContent,
    );

    expect(warningValues).toContain("No new");
    expect(warningValues).toContain("Cut-out blocked");
    expect(warningValues).not.toContain("New conversations blocked");
    expect((canvasElement.textContent ?? "").match(/Cut-out blocked/g)).toHaveLength(1);

    const forceRemoveBadge = Array.from(
      canvasElement.querySelectorAll('[class*="bg-primary"]'),
    ).find((node) => node.textContent === "Force remove");
    expect(forceRemoveBadge).toBeTruthy();
  },
};

export const DenyAllTagIntersection: Story = {
  args: {
    rule: denyAllTagIntersectionRule,
  },
};

export const PrimaryRule: Story = {
  args: {
    rule: {
      ...relaxedRule,
      priorityTier: "primary",
      fastModeRewriteMode: "force_add",
      requestCompressionAlgorithm: "follow",
      sourceTagIds: [9],
      sourceTagNames: ["priority-lane"],
    },
  },
};

export const FillMissingRule: Story = {
  args: {
    rule: {
      ...relaxedRule,
      fastModeRewriteMode: "fill_missing",
      sourceTagIds: [12],
      sourceTagNames: ["overflow-guard"],
    },
  },
};

const editableOptions = ["gpt-5.5", "gpt-5.4-mini", "o3", "gpt-4.1"];
type StoryFieldSources = NonNullable<EffectiveRoutingRule["fieldSources"]>;
type EditablePolicyConfig = NonNullable<
  Parameters<typeof EffectiveRoutingRuleCard>[0]["editablePolicy"]
>;

function applyPatchToRule(
  rule: EffectiveRoutingRule,
  patch: UpdateGroupAccountRoutingRulePayload,
): EffectiveRoutingRule {
  const fieldSources: StoryFieldSources = {
    allowCutOut: rule.fieldSources?.allowCutOut ?? "root",
    allowCutIn: rule.fieldSources?.allowCutIn ?? "root",
    priorityTier: rule.fieldSources?.priorityTier ?? "root",
    fastModeRewriteMode: rule.fieldSources?.fastModeRewriteMode ?? "root",
    imageToolRewriteMode: rule.fieldSources?.imageToolRewriteMode ?? "root",
    codexImagegenRewriteMode: rule.fieldSources?.codexImagegenRewriteMode ?? "root",
    requestCompressionAlgorithm: rule.fieldSources?.requestCompressionAlgorithm ?? "root",
    concurrencyLimit: rule.fieldSources?.concurrencyLimit ?? "root",
    upstream429Retry: rule.fieldSources?.upstream429Retry ?? "root",
    availableModels: rule.fieldSources?.availableModels ?? "root",
    systemDeniedModels: rule.fieldSources?.systemDeniedModels ?? "root",
  };
  const statusChangeReasons = {
    ...buildDefaultStatusChangeReasons(),
    ...(rule.statusChangeReasons ?? {}),
  };
  const statusChangeReasonFieldSources = {
    ...buildDefaultStatusChangeReasonFieldSources(),
    ...(rule.statusChangeReasonFieldSources ?? {}),
  };
  const next: EffectiveRoutingRule = {
    ...rule,
    fieldSources,
    statusChangeReasons,
    statusChangeReasonFieldSources,
  };
  const nextSources = fieldSources;
  const sourceFor = (value: unknown): "root" | "account" => (value === null ? "root" : "account");
  if ("allowCutOut" in patch) {
    if (typeof patch.allowCutOut === "boolean") next.allowCutOut = patch.allowCutOut;
    nextSources.allowCutOut = sourceFor(patch.allowCutOut);
  }
  if ("allowCutIn" in patch) {
    if (typeof patch.allowCutIn === "boolean") next.allowCutIn = patch.allowCutIn;
    nextSources.allowCutIn = sourceFor(patch.allowCutIn);
  }
  if ("priorityTier" in patch) {
    if (patch.priorityTier !== null) next.priorityTier = patch.priorityTier ?? next.priorityTier;
    nextSources.priorityTier = sourceFor(patch.priorityTier);
  }
  if ("fastModeRewriteMode" in patch) {
    if (patch.fastModeRewriteMode !== null)
      next.fastModeRewriteMode = patch.fastModeRewriteMode ?? next.fastModeRewriteMode;
    nextSources.fastModeRewriteMode = sourceFor(patch.fastModeRewriteMode);
  }
  if ("imageToolRewriteMode" in patch) {
    if (patch.imageToolRewriteMode !== null)
      next.imageToolRewriteMode = patch.imageToolRewriteMode ?? next.imageToolRewriteMode;
    nextSources.imageToolRewriteMode = sourceFor(patch.imageToolRewriteMode);
  }
  if ("codexImagegenRewriteMode" in patch) {
    if (patch.codexImagegenRewriteMode !== null) {
      next.codexImagegenRewriteMode =
        patch.codexImagegenRewriteMode ?? next.codexImagegenRewriteMode;
    }
    nextSources.codexImagegenRewriteMode = sourceFor(patch.codexImagegenRewriteMode);
  }
  if ("requestCompressionAlgorithm" in patch) {
    if (patch.requestCompressionAlgorithm !== null) {
      next.requestCompressionAlgorithm =
        patch.requestCompressionAlgorithm ?? next.requestCompressionAlgorithm;
    }
    nextSources.requestCompressionAlgorithm = sourceFor(patch.requestCompressionAlgorithm);
  }
  if ("concurrencyLimit" in patch) {
    if (patch.concurrencyLimit !== null)
      next.concurrencyLimit = patch.concurrencyLimit ?? next.concurrencyLimit;
    nextSources.concurrencyLimit = sourceFor(patch.concurrencyLimit);
  }
  if ("upstream429RetryEnabled" in patch || "upstream429MaxRetries" in patch) {
    const hasEnabled = Object.hasOwn(patch, "upstream429RetryEnabled");
    const hasRetries = Object.hasOwn(patch, "upstream429MaxRetries");
    const enabledValue = patch.upstream429RetryEnabled;
    const retryValue = patch.upstream429MaxRetries;
    if (enabledValue === null || retryValue === null) {
      next.upstream429RetryEnabled = false;
      next.upstream429MaxRetries = 0;
      nextSources.upstream429Retry = "root";
    } else {
      if (enabledValue !== undefined) {
        next.upstream429RetryEnabled = enabledValue;
      }
      if (retryValue !== undefined) {
        next.upstream429MaxRetries = retryValue;
      }
      if (hasEnabled || hasRetries) {
        nextSources.upstream429Retry = "account";
      }
    }
  }
  if ("availableModels" in patch) {
    next.availableModels = patch.availableModels ?? [];
    nextSources.availableModels = sourceFor(patch.availableModels);
  }
  if ("availableModelsMode" in patch && patch.availableModelsMode !== null) {
    next.availableModelsMode = patch.availableModelsMode;
  } else if ("availableModelsMode" in patch && patch.availableModelsMode === null) {
    next.availableModelsMode = "denylist";
  }
  if ("statusChangeReasons" in patch && patch.statusChangeReasons) {
    for (const [reason, value] of Object.entries(patch.statusChangeReasons)) {
      if (value === null) {
        next.statusChangeReasons![reason as StatusChangeReasonCode] = true;
        next.statusChangeReasonFieldSources![reason as StatusChangeReasonCode] = "root";
      } else if (typeof value === "boolean") {
        next.statusChangeReasons![reason as StatusChangeReasonCode] = value;
        next.statusChangeReasonFieldSources![reason as StatusChangeReasonCode] = "account";
      }
    }
  }
  if ("timeouts" in patch && patch.timeouts) {
    const nextTimeoutSources = {
      responsesFirstByteTimeoutSecs:
        next.timeoutFieldSources?.responsesFirstByteTimeoutSecs ?? "root",
      compactFirstByteTimeoutSecs: next.timeoutFieldSources?.compactFirstByteTimeoutSecs ?? "root",
      responsesStreamTimeoutSecs: next.timeoutFieldSources?.responsesStreamTimeoutSecs ?? "root",
      compactStreamTimeoutSecs: next.timeoutFieldSources?.compactStreamTimeoutSecs ?? "root",
    };
    const nextTimeoutValues = {
      responsesFirstByteTimeoutSecs:
        next.timeouts?.responsesFirstByteTimeoutSecs ??
        relaxedRule.timeouts?.responsesFirstByteTimeoutSecs ??
        0,
      compactFirstByteTimeoutSecs:
        next.timeouts?.compactFirstByteTimeoutSecs ??
        relaxedRule.timeouts?.compactFirstByteTimeoutSecs ??
        0,
      responsesStreamTimeoutSecs:
        next.timeouts?.responsesStreamTimeoutSecs ??
        relaxedRule.timeouts?.responsesStreamTimeoutSecs ??
        0,
      compactStreamTimeoutSecs:
        next.timeouts?.compactStreamTimeoutSecs ??
        relaxedRule.timeouts?.compactStreamTimeoutSecs ??
        0,
    };
    for (const [key, value] of Object.entries(patch.timeouts)) {
      const timeoutKey = key as keyof typeof nextTimeoutValues;
      if (value === null) {
        nextTimeoutValues[timeoutKey] = relaxedRule.timeouts?.[timeoutKey] ?? 0;
        nextTimeoutSources[timeoutKey] = "root";
      } else if (typeof value === "number") {
        nextTimeoutValues[timeoutKey] = value;
        nextTimeoutSources[timeoutKey] = "account";
      }
    }
    next.timeouts = nextTimeoutValues;
    next.timeoutFieldSources = nextTimeoutSources;
  }
  return next;
}

function EditableRoutingRuleDemo({
  initialRule,
  busyField,
  saveStatusByField,
  errorByField,
  onRetry,
  onRevert,
  visibleRows,
  availableModelCatalog,
  availableModelCatalogStatus,
  availableModelCatalogError,
  availableModelCatalogStale,
  onRefreshAvailableModelCatalog,
}: {
  initialRule: EffectiveRoutingRule;
  busyField?: EditablePolicyConfig["busyField"];
  saveStatusByField?: EditablePolicyConfig["saveStatusByField"];
  errorByField?: EditablePolicyConfig["errorByField"];
  onRetry?: EditablePolicyConfig["onRetry"];
  onRevert?: EditablePolicyConfig["onRevert"];
  visibleRows?: readonly EffectiveRoutingRuleCardRowKey[];
  availableModelCatalog?: AvailableModelOption[];
  availableModelCatalogStatus?: string;
  availableModelCatalogError?: string | null;
  availableModelCatalogStale?: boolean;
  onRefreshAvailableModelCatalog?: () => void;
}) {
  const [rule, setRule] = useState(initialRule);
  return (
    <EffectiveRoutingRuleCard
      rule={rule}
      labels={labels}
      visibleRows={visibleRows ? [...visibleRows] : undefined}
      editablePolicy={{
        busyField,
        saveStatusByField,
        errorByField,
        onRetry,
        onRevert,
        availableModelOptions: editableOptions,
        availableModelCatalog,
        availableModelCatalogStatus,
        availableModelCatalogError,
        availableModelCatalogStale,
        availableModelCatalogLastSuccessfulAt: "2026-09-05 10:00",
        onRefreshAvailableModelCatalog,
        onChange: (_field, payload) => setRule((current) => applyPatchToRule(current, payload)),
      }}
    />
  );
}

export const EditableInherited: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={relaxedRule} />,
  play: async ({ canvasElement }) => {
    const timeoutButton = canvasElement.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Standard response first byte timeout"]',
    );
    if (!timeoutButton) {
      throw new Error("missing inherited timeout edit button");
    }

    await userEvent.click(timeoutButton);

    expect(
      canvasElement.querySelector<HTMLInputElement>('input[name="responsesFirstByteTimeoutSecs"]'),
    ).not.toBeNull();
  },
};

export const EditableImageToolHelp: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    await expect(documentScope.getByText("Codex imagegen", { exact: true })).toBeVisible();
    const help = documentScope.getByRole("button", {
      name: "Image tools help",
    });
    await userEvent.click(help);
    await expect(
      documentScope.getByText(/Configure Codex Full and Lite imagegen separately/i),
    ).toBeVisible();
  },
};

export const EditableImagegenRewritePolicies: Story = {
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={strictRule}
      visibleRows={["imageToolRewriteMode", "codexImagegenRewriteMode"]}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    await expect(documentScope.getByText("Image tools", { exact: true })).toBeVisible();
    await expect(documentScope.getByText("Codex imagegen", { exact: true })).toBeVisible();
    await expect(documentScope.getByRole("radiogroup", { name: "Codex imagegen" })).toBeVisible();
  },
};

export const EditableAccountOverrides: Story = {
  tags: ["test"],
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("radiogroup", { name: "FAST mode" })).toBeVisible();
    await expect(canvas.getByRole("radiogroup", { name: "Upstream 429 retry" })).toBeVisible();
  },
};

export const EditableAvailableModels: Story = {
  tags: ["test"],
  render: () => (
    <EditableRoutingRuleDemo initialRule={strictRule} visibleRows={["availableModels"]} />
  ),
  play: async ({ canvasElement }) => {
    const modeToggle = canvasElement.querySelector<HTMLButtonElement>(
      'button[data-testid="available-models-mode-toggle"]',
    );
    if (!modeToggle) {
      throw new Error("missing desktop available-models mode toggle");
    }

    expect(modeToggle.classList.contains("hidden")).toBe(true);
    expect(modeToggle.classList.contains("min-[769px]:inline-flex")).toBe(true);
    expect(modeToggle.textContent).toContain("Allowlist");

    await userEvent.click(modeToggle);

    expect(modeToggle.textContent).toContain("Denylist");
    expect(canvasElement.textContent).toContain("gpt-5.4-mini");
  },
};

export const EditableAvailableModelsCompact: Story = {
  tags: ["test"],
  render: () => (
    <EditableRoutingRuleDemo initialRule={strictRule} visibleRows={["availableModels"]} />
  ),
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const modeGroup = canvasElement.querySelector<HTMLElement>(
      '[role="radiogroup"][aria-label="Available models"]',
    );
    if (!modeGroup) {
      throw new Error("missing compact available-models mode group");
    }
    expect(modeGroup.querySelectorAll('[role="radio"]')).toHaveLength(2);
    expect(modeGroup.querySelector('[aria-checked="true"]')?.textContent).toContain("Allowlist");
    expect(canvasElement.textContent).toContain("gpt-5.4-mini");
  },
};

export const EditableAvailableModelsWithCatalog: Story = {
  tags: ["test"],
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={{ ...strictRule, availableModels: ["account-only", "missing-model"] }}
      visibleRows={["availableModels"]}
      availableModelCatalog={[
        { value: "gpt-5.5", sources: ["account"] },
        { value: "account-only", sources: ["account"] },
        { value: "gpt-5.4-mini", sources: ["account"] },
      ]}
      availableModelCatalogStatus="ready"
      onRefreshAvailableModelCatalog={() => undefined}
    />
  ),
  play: async ({ canvasElement }) => {
    const trigger = canvasElement.querySelector<HTMLButtonElement>('button[role="combobox"]');
    if (!trigger) throw new Error("missing available-models selector");
    await userEvent.click(trigger);
    const documentScope = within(canvasElement.ownerDocument.body);
    await expect(documentScope.getByRole("button", { name: "Project presets" })).toBeVisible();
    await expect(documentScope.getByRole("button", { name: "This account" })).toBeVisible();
    await expect(documentScope.getAllByText("Selected value").length).toBeGreaterThan(0);

    const searchInput = documentScope.getByPlaceholderText("Model id");
    await userEvent.type(searchInput, "gpt-5.5");
    expect(
      canvasElement.ownerDocument.querySelector(
        '[data-testid="available-models-source-filter-all"]',
      ),
    ).toBeNull();
    const searchHeading = canvasElement.ownerDocument.querySelector("[cmdk-group-heading]");
    expect(searchHeading?.textContent).toContain("Project presets + This account");
  },
};

export const EditableAvailableModelsCatalogFailure: Story = {
  tags: ["test"],
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={strictRule}
      visibleRows={["availableModels"]}
      availableModelCatalogStatus="failed"
      availableModelCatalogError="The account model catalog refresh failed."
      onRefreshAvailableModelCatalog={() => undefined}
    />
  ),
};

export const EditableAvailableModelsWithCatalogMobile: Story = {
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "mobile393" },
  },
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={{ ...strictRule, availableModels: ["account-only", "missing-model"] }}
      visibleRows={["availableModels"]}
      availableModelCatalog={[
        { value: "gpt-5.5", sources: ["account"] },
        { value: "account-only", sources: ["account"] },
        { value: "gpt-5.4-mini", sources: ["account"] },
      ]}
      availableModelCatalogStatus="ready"
      onRefreshAvailableModelCatalog={() => undefined}
    />
  ),
};

export const MobileFlatHierarchy: Story = {
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
  play: async ({ canvasElement }) => {
    const card = canvasElement.querySelector<HTMLElement>(
      '[data-testid="effective-routing-rule-card"]',
    );
    if (!card) throw new Error("missing effective routing rule card");

    const staticSections = Array.from(
      canvasElement.querySelectorAll<HTMLElement>('[data-testid$="-section"]'),
    );
    const rowTables = Array.from(
      canvasElement.querySelectorAll<HTMLElement>('[data-testid$="-table"]'),
    );
    const fieldRows = Array.from(
      canvasElement.querySelectorAll<HTMLElement>(
        '[data-testid="effective-routing-rule-field-row"]',
      ),
    );
    const timeoutRows = Array.from(
      canvasElement.querySelectorAll<HTMLElement>(
        '[data-testid="effective-routing-rule-timeout-row"]',
      ),
    );
    if (staticSections.length !== 4 || rowTables.length !== 2) {
      throw new Error("missing effective routing mobile hierarchy sections");
    }

    const modeToggle = canvasElement.querySelector('[data-testid="available-models-mode-toggle"]');
    expect(modeToggle?.parentElement?.parentElement?.classList).toContain("min-w-0");
    expect(modeToggle?.parentElement?.parentElement?.classList).toContain(
      "min-[769px]:min-w-[18rem]",
    );

    const cardRect = card.getBoundingClientRect();
    expect(card.classList).toContain("rounded-none");
    expect(card.classList).toContain("border-0");
    expect(card.classList).toContain("bg-transparent");
    expect(card.classList).toContain("sm:rounded-xl");
    for (const section of staticSections) {
      const rect = section.getBoundingClientRect();
      expect(section.classList).toContain("border-t");
      expect(section.classList).toContain("pt-5");
      expect(section.classList).toContain("first:border-t-0");
      expect(section.classList).toContain("sm:border");
      expect(section.classList).toContain("sm:rounded-xl");
      expect(rect.left).toBeGreaterThanOrEqual(cardRect.left);
      expect(rect.right).toBeLessThanOrEqual(cardRect.right);
      expect(section.scrollWidth).toBeLessThanOrEqual(section.clientWidth);
    }

    for (const table of rowTables) {
      expect(table.classList).toContain("border-0");
      expect(table.classList).toContain("mt-3");
      expect(table.classList).toContain("overflow-visible");
      expect(table.classList).toContain("sm:border");
      expect(table.classList).toContain("sm:overflow-hidden");
      expect(table.scrollWidth).toBeLessThanOrEqual(table.clientWidth);
    }

    for (const row of [...fieldRows, ...timeoutRows]) {
      expect(row.classList).toContain("grid-cols-[fit-content(40%)_minmax(0,1fr)_2.75rem]");
      expect(row.classList).toContain("py-3.5");
      expect(row.children.item(1)?.classList).not.toContain("col-span-2");
      expect(row.children.item(1)?.classList).toContain("justify-end");
    }

    expect(
      canvasElement.querySelector('button[aria-label="Clear account override: Priority"]')
        ?.classList,
    ).toContain("col-start-3");

    expect(
      canvasElement.querySelector('[data-testid="effective-routing-rule-status-change-grid"]')
        ?.classList,
    ).toContain("grid-cols-2");
  },
};

export const DesktopFramedHierarchy: Story = {
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
  play: async ({ canvasElement }) => {
    const card = canvasElement.querySelector<HTMLElement>(
      '[data-testid="effective-routing-rule-card"]',
    );
    const staticSections = Array.from(
      canvasElement.querySelectorAll<HTMLElement>('[data-testid$="-section"]'),
    );
    const rowTables = Array.from(
      canvasElement.querySelectorAll<HTMLElement>('[data-testid$="-table"]'),
    );

    if (!card) throw new Error("missing effective routing rule card");
    expect(card.classList).toContain("sm:border");
    expect(card.classList).toContain("sm:rounded-xl");

    for (const section of staticSections) {
      expect(section.classList).toContain("sm:border");
      expect(section.classList).toContain("sm:rounded-xl");
    }

    for (const table of rowTables) {
      expect(table.classList).toContain("sm:border");
      expect(table.classList).toContain("sm:overflow-hidden");
    }
  },
};

export const EditableMultipleAccountOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={multipleAccountOverridesRule} />,
};

export const EditableTimeoutOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={multipleAccountOverridesRule} />,
  parameters: {
    docs: {
      description: {
        story:
          "Account effective rule with mixed timeout inheritance: account overrides on first-byte and stream fields, inherited group/default values on the remaining fields.",
      },
    },
  },
};

export const EditableSavingAndError: Story = {
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={strictRule}
      saveStatusByField={{ priorityTier: "saving" }}
      errorByField={{
        allowCutIn: "Save failed. Check the account policy and retry.",
      }}
      onRetry={() => undefined}
      onRevert={() => undefined}
    />
  ),
};

export const EditableDenyAllModels: Story = {
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={{
        ...strictRule,
        availableModels: [],
        fieldSources: {
          allowCutOut: strictRule.fieldSources?.allowCutOut ?? "root",
          allowCutIn: strictRule.fieldSources?.allowCutIn ?? "root",
          priorityTier: strictRule.fieldSources?.priorityTier ?? "root",
          fastModeRewriteMode: strictRule.fieldSources?.fastModeRewriteMode ?? "root",
          imageToolRewriteMode: strictRule.fieldSources?.imageToolRewriteMode ?? "root",
          codexImagegenRewriteMode: strictRule.fieldSources?.codexImagegenRewriteMode ?? "root",
          concurrencyLimit: strictRule.fieldSources?.concurrencyLimit ?? "root",
          upstream429Retry: strictRule.fieldSources?.upstream429Retry ?? "root",
          ...strictRule.fieldSources,
          availableModels: "account",
        },
      }}
    />
  ),
};
