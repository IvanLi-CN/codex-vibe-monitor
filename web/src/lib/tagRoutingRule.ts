import type {
  TagFastModeRewriteMode,
  TagPriorityTier,
} from "./api";

export type RoutingRuleBadgeVariant =
  | "default"
  | "info"
  | "accent"
  | "secondary"
  | "warning";

type PriorityTierLabels = {
  priorityPrimary: string;
  priorityNormal: string;
  priorityFallback: string;
};

type FastModeRewriteLabels = {
  fastModeKeepOriginal: string;
  fastModeFillMissing: string;
  fastModeForceAdd: string;
  fastModeForceRemove: string;
};

export function priorityTierBadgeVariant(
  priorityTier?: TagPriorityTier,
): RoutingRuleBadgeVariant {
  if (priorityTier === "primary") return "default";
  if (priorityTier === "fallback") return "warning";
  return "secondary";
}

export function priorityTierBadgeLabel(
  priorityTier: TagPriorityTier | undefined,
  labels: PriorityTierLabels,
): string {
  if (priorityTier === "primary") return labels.priorityPrimary;
  if (priorityTier === "fallback") return labels.priorityFallback;
  return labels.priorityNormal;
}

export function fastModeRewriteBadgeVariant(
  fastModeRewriteMode?: TagFastModeRewriteMode,
): RoutingRuleBadgeVariant {
  if (fastModeRewriteMode === "fill_missing") return "info";
  if (fastModeRewriteMode === "force_add") return "default";
  if (fastModeRewriteMode === "force_remove") return "warning";
  return "secondary";
}

export function fastModeRewriteBadgeLabel(
  fastModeRewriteMode: TagFastModeRewriteMode | undefined,
  labels: FastModeRewriteLabels,
): string {
  if (fastModeRewriteMode === "fill_missing") return labels.fastModeFillMissing;
  if (fastModeRewriteMode === "force_add") return labels.fastModeForceAdd;
  if (fastModeRewriteMode === "force_remove") return labels.fastModeForceRemove;
  return labels.fastModeKeepOriginal;
}
