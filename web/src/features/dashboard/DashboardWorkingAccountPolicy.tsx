import { Chip } from "../../components/ui/chip";
import type {
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpstreamAccountActivityAccount,
} from "../../lib/api";

export type AccountQuickPolicyDraft = {
  priorityTier: TagPriorityTier;
  allowCutOut: boolean;
  allowCutIn: boolean;
  fastModeRewriteMode: TagFastModeRewriteMode;
};

type AccountQuickPolicyTone = "neutral" | "success" | "warning" | "primary";

export function accountPolicyDraftFromRule(
  account: UpstreamAccountActivityAccount,
): AccountQuickPolicyDraft {
  return accountPolicyDraftFromRoutingRule(account.effectiveRoutingRule);
}

export function accountPolicyDraftFromRoutingRule(
  rule: UpstreamAccountActivityAccount["effectiveRoutingRule"],
): AccountQuickPolicyDraft {
  const effectiveRule = rule ?? {
    allowCutOut: true,
    allowCutIn: true,
    priorityTier: "normal" as TagPriorityTier,
    fastModeRewriteMode: "keep_original" as TagFastModeRewriteMode,
  };
  return {
    priorityTier: effectiveRule.priorityTier ?? "normal",
    allowCutOut: effectiveRule.allowCutOut !== false,
    allowCutIn: effectiveRule.allowCutIn !== false,
    fastModeRewriteMode: effectiveRule.fastModeRewriteMode ?? "keep_original",
  };
}

export function accountPolicyDraftsEqual(
  left: AccountQuickPolicyDraft,
  right: AccountQuickPolicyDraft,
): boolean {
  return (
    left.priorityTier === right.priorityTier &&
    left.allowCutOut === right.allowCutOut &&
    left.allowCutIn === right.allowCutIn &&
    left.fastModeRewriteMode === right.fastModeRewriteMode
  );
}

export function cycleAccountPriorityPolicy(
  draft: AccountQuickPolicyDraft,
): AccountQuickPolicyDraft {
  if (draft.priorityTier === "normal") return { ...draft, priorityTier: "fallback" };
  if (draft.priorityTier === "fallback") return { ...draft, priorityTier: "primary" };
  if (draft.priorityTier === "primary") return { ...draft, priorityTier: "no_new" };
  return { ...draft, priorityTier: "normal" };
}

export function cycleAccountFastModePolicy(
  draft: AccountQuickPolicyDraft,
): AccountQuickPolicyDraft {
  if (draft.fastModeRewriteMode === "keep_original") {
    return { ...draft, fastModeRewriteMode: "fill_missing" };
  }
  if (draft.fastModeRewriteMode === "fill_missing") {
    return { ...draft, fastModeRewriteMode: "force_add" };
  }
  if (draft.fastModeRewriteMode === "force_add") {
    return { ...draft, fastModeRewriteMode: "force_remove" };
  }
  return { ...draft, fastModeRewriteMode: "keep_original" };
}

function priorityPolicyLabel(draft: AccountQuickPolicyDraft, locale: "zh" | "en") {
  if (draft.priorityTier === "no_new") return locale === "zh" ? "禁新" : "No new";
  if (draft.priorityTier === "primary") return locale === "zh" ? "主力" : "Primary";
  if (draft.priorityTier === "fallback") return locale === "zh" ? "兜底" : "Fallback";
  return locale === "zh" ? "普通" : "Normal";
}

function fastModePolicyLabel(mode: TagFastModeRewriteMode, locale: "zh" | "en") {
  if (mode === "fill_missing") return locale === "zh" ? "补Fast" : "+Fast";
  if (mode === "force_add") return locale === "zh" ? "强制Fast" : "Force Fast";
  if (mode === "force_remove") return locale === "zh" ? "禁Fast" : "No Fast";
  return locale === "zh" ? "不改Fast" : "Leave Fast";
}

function fastModePolicyCycleTitle(currentLabel: string, locale: "zh" | "en") {
  if (locale === "zh") {
    return `Fast 改写策略：${currentLabel}。点击循环 不改Fast / 补Fast / 强制Fast / 禁Fast`;
  }
  return `Fast rewrite policy: ${currentLabel}. Cycle Leave Fast / +Fast / Force Fast / No Fast`;
}

function fastModePolicyAriaLabel(currentLabel: string, locale: "zh" | "en") {
  if (locale === "zh") return `Fast 改写策略：${currentLabel}，点击切换`;
  return `Fast rewrite policy: ${currentLabel}, click to cycle`;
}

function priorityPolicyTone(draft: AccountQuickPolicyDraft): AccountQuickPolicyTone {
  if (draft.priorityTier === "no_new") return "warning";
  if (draft.priorityTier === "primary") return "primary";
  if (draft.priorityTier === "fallback") return "success";
  return "neutral";
}

function fastModePolicyTone(mode: TagFastModeRewriteMode): AccountQuickPolicyTone {
  if (mode === "force_remove") return "warning";
  if (mode === "force_add") return "primary";
  if (mode === "fill_missing") return "success";
  return "neutral";
}

function booleanBlockPolicyTone(isBlocked: boolean): AccountQuickPolicyTone {
  return isBlocked ? "warning" : "neutral";
}

type AccountPolicyChip = {
  key: string;
  tone: AccountQuickPolicyTone;
  label: string;
  title: string;
  ariaLabel: string;
  onClick: () => void;
};

function buildAccountPolicyChips(
  draft: AccountQuickPolicyDraft,
  locale: "zh" | "en",
  actions: Pick<AccountPolicyChip, "onClick">[],
): AccountPolicyChip[] {
  const fastModeLabel = fastModePolicyLabel(draft.fastModeRewriteMode, locale);
  return [
    {
      key: "priority-new-conversations",
      tone: priorityPolicyTone(draft),
      label: priorityPolicyLabel(draft, locale),
      title:
        locale === "zh"
          ? "点击切换 普通 / 兜底 / 主力 / 禁新"
          : "Cycle normal / fallback / primary / no new",
      ariaLabel: locale === "zh" ? "切换账号优先级" : "Cycle account priority",
      onClick: actions[0].onClick,
    },
    {
      key: "fast-mode-rewrite",
      tone: fastModePolicyTone(draft.fastModeRewriteMode),
      label: fastModeLabel,
      title: fastModePolicyCycleTitle(fastModeLabel, locale),
      ariaLabel: fastModePolicyAriaLabel(fastModeLabel, locale),
      onClick: actions[1].onClick,
    },
    {
      key: "allow-cut-out",
      tone: booleanBlockPolicyTone(!draft.allowCutOut),
      label: locale === "zh" ? "禁出" : "No out",
      title: locale === "zh" ? "点击切换账号级禁出" : "Toggle account-level cut out",
      ariaLabel: locale === "zh" ? "切换禁出" : "Toggle cut out",
      onClick: actions[2].onClick,
    },
    {
      key: "allow-cut-in",
      tone: booleanBlockPolicyTone(!draft.allowCutIn),
      label: locale === "zh" ? "禁入" : "No in",
      title: locale === "zh" ? "点击切换账号级禁入" : "Toggle account-level cut in",
      ariaLabel: locale === "zh" ? "切换禁入" : "Toggle cut in",
      onClick: actions[3].onClick,
    },
  ];
}

function AccountPolicyChipButton({
  chip,
  disabled,
}: {
  chip: AccountPolicyChip;
  disabled: boolean;
}) {
  return (
    <Chip
      asChild
      size="header"
      tone={chip.tone}
      className="shrink-0 font-semibold transition-opacity"
    >
      <button
        type="button"
        data-testid="dashboard-upstream-account-policy-badge"
        data-policy-key={chip.key}
        data-policy-tone={chip.tone}
        disabled={disabled}
        title={chip.title}
        aria-label={chip.ariaLabel}
        onClick={(event) => {
          event.stopPropagation();
          chip.onClick();
        }}
        onKeyDown={(event) => event.stopPropagation()}
      >
        {chip.label}
      </button>
    </Chip>
  );
}

export function AccountQuickPolicyChips({
  draft,
  locale,
  disabled,
  isSaving,
  onCyclePriority,
  onCycleFastMode,
  onToggleCutOut,
  onToggleCutIn,
}: {
  draft: AccountQuickPolicyDraft;
  locale: "zh" | "en";
  disabled: boolean;
  isSaving: boolean;
  onCyclePriority: () => void;
  onCycleFastMode: () => void;
  onToggleCutOut: () => void;
  onToggleCutIn: () => void;
}) {
  const chips = buildAccountPolicyChips(draft, locale, [
    { onClick: onCyclePriority },
    { onClick: onCycleFastMode },
    { onClick: onToggleCutOut },
    { onClick: onToggleCutIn },
  ]);
  return (
    <div
      data-testid="dashboard-upstream-account-policy-badges"
      data-saving={isSaving ? "true" : "false"}
      className="flex min-w-0 flex-wrap items-center gap-1.5 overflow-hidden"
    >
      {chips.map((chip) => (
        <AccountPolicyChipButton key={chip.key} chip={chip} disabled={disabled} />
      ))}
    </div>
  );
}
