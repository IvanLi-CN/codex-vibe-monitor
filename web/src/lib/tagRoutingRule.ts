import type { TagFastModeRewriteMode, TagPriorityTier, TagRoutingRule } from "./api";

export type RoutingRuleChipTone = "primary" | "info" | "accent" | "secondary" | "warning";

type PriorityTierLabels = {
  priorityPrimary: string;
  priorityNormal: string;
  priorityFallback: string;
  priorityNoNew?: string;
};

type FastModeRewriteLabels = {
  fastModeKeepOriginal: string;
  fastModeFillMissing: string;
  fastModeForceAdd: string;
  fastModeForceRemove: string;
};

export function priorityTierChipTone(priorityTier?: TagPriorityTier): RoutingRuleChipTone {
  if (priorityTier === "primary") return "primary";
  if (priorityTier === "no_new") return "warning";
  if (priorityTier === "fallback") return "warning";
  return "secondary";
}

export function priorityTierChipLabel(
  priorityTier: TagPriorityTier | undefined,
  labels: PriorityTierLabels,
): string {
  if (priorityTier === "primary") return labels.priorityPrimary;
  if (priorityTier === "no_new") return labels.priorityNoNew ?? "No new";
  if (priorityTier === "fallback") return labels.priorityFallback;
  return labels.priorityNormal;
}

export function fastModeRewriteChipTone(
  fastModeRewriteMode?: TagFastModeRewriteMode,
): RoutingRuleChipTone {
  if (fastModeRewriteMode === "fill_missing") return "info";
  if (fastModeRewriteMode === "force_add") return "primary";
  if (fastModeRewriteMode === "force_remove") return "warning";
  return "secondary";
}

export function fastModeRewriteChipLabel(
  fastModeRewriteMode: TagFastModeRewriteMode | undefined,
  labels: FastModeRewriteLabels,
): string {
  if (fastModeRewriteMode === "fill_missing") return labels.fastModeFillMissing;
  if (fastModeRewriteMode === "force_add") return labels.fastModeForceAdd;
  if (fastModeRewriteMode === "force_remove") return labels.fastModeForceRemove;
  return labels.fastModeKeepOriginal;
}

export type ActiveRoutingPolicyChipLabels = {
  policyPriorityPrimary?: string;
  policyPriorityFallback?: string;
  policyFastFillMissing?: string;
  policyFastForceAdd?: string;
  policyFastForceRemove?: string;
  policyForbidCutOut?: string;
  policyForbidCutIn?: string;
  policyForbidNewConversation?: string;
  policyConcurrency?: (count: number) => string;
  policyRetry?: (count: number) => string;
};

export type ActiveRoutingPolicyChip = {
  key: string;
  label: string;
  title?: string;
  variant: RoutingRuleChipTone;
};

export function resolveActiveRoutingPolicyChips(
  rule: TagRoutingRule | null | undefined,
  labels: ActiveRoutingPolicyChipLabels = {},
): ActiveRoutingPolicyChip[] {
  if (!rule) return [];
  const badges: ActiveRoutingPolicyChip[] = [];

  if (rule.priorityTier === "primary") {
    badges.push({
      key: "priority-primary",
      label: labels.policyPriorityPrimary ?? "主力",
      variant: "primary",
    });
  } else if (rule.priorityTier === "fallback") {
    badges.push({
      key: "priority-fallback",
      label: labels.policyPriorityFallback ?? "兜底",
      variant: "warning",
    });
  }

  if (rule.fastModeRewriteMode === "fill_missing") {
    badges.push({
      key: "fast-fill",
      label: labels.policyFastFillMissing ?? "补Fast",
      variant: "info",
    });
  } else if (rule.fastModeRewriteMode === "force_add") {
    badges.push({
      key: "fast-add",
      label: labels.policyFastForceAdd ?? "Fast",
      variant: "primary",
    });
  } else if (rule.fastModeRewriteMode === "force_remove") {
    badges.push({
      key: "fast-remove",
      label: labels.policyFastForceRemove ?? "禁Fast",
      variant: "warning",
    });
  }
  if (rule.priorityTier === "no_new") {
    badges.push({
      key: "forbid-new",
      label: labels.policyForbidNewConversation ?? "禁新",
      variant: "warning",
    });
  }

  if (rule.allowCutOut === false) {
    badges.push({
      key: "forbid-cut-out",
      label: labels.policyForbidCutOut ?? "禁出",
      variant: "warning",
    });
  }

  if (rule.allowCutIn === false) {
    badges.push({
      key: "forbid-cut-in",
      label: labels.policyForbidCutIn ?? "禁入",
      variant: "warning",
    });
  }

  const concurrencyLimit = rule.concurrencyLimit ?? 0;
  if (concurrencyLimit > 0) {
    badges.push({
      key: "concurrency",
      label: labels.policyConcurrency?.(concurrencyLimit) ?? `并发${concurrencyLimit}`,
      variant: "secondary",
    });
  }

  if (rule.upstream429RetryEnabled) {
    const retryCount = Math.max(1, rule.upstream429MaxRetries ?? 1);
    badges.push({
      key: "retry",
      label: labels.policyRetry?.(retryCount) ?? `重试${retryCount}`,
      variant: "warning",
    });
  }

  return badges;
}
