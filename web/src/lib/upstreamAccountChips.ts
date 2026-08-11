export type UpstreamPlanChipTone =
  | "primary"
  | "accent"
  | "secondary"
  | "success"
  | "info"
  | "warning"
  | "error";

type UpstreamPlanChipRecipe = {
  tone: UpstreamPlanChipTone;
  dataPlan: string;
};

const PLAN_VARIANTS: Record<string, UpstreamPlanChipTone> = {
  local: "secondary",
  free: "warning",
  k12: "success",
  plus: "primary",
  pro: "primary",
  team: "info",
  enterprise: "accent",
};

const COMPACT_PLAN_LABELS: Record<string, string> = {
  free: "Free",
  k12: "K12",
  plus: "Plus",
  pro: "Pro",
  team: "Team",
  enterprise: "Ent",
};

function normalizePlanType(planType?: string | null) {
  const normalized = planType?.trim().toLowerCase();
  return normalized ? normalized : null;
}

export function upstreamPlanChipRecipe(planType?: string | null): UpstreamPlanChipRecipe | null {
  const normalized = normalizePlanType(planType);
  if (!normalized) return null;

  return {
    tone: PLAN_VARIANTS[normalized] ?? "secondary",
    dataPlan: normalized,
  };
}

export function shouldShowUpstreamPlanChip(planType?: string | null) {
  const normalized = normalizePlanType(planType);
  return Boolean(normalized && normalized !== "local");
}

export function compactUpstreamPlanLabel(planType?: string | null) {
  const normalized = normalizePlanType(planType);
  if (!normalized) return null;
  return COMPACT_PLAN_LABELS[normalized] ?? planType?.trim() ?? normalized;
}
