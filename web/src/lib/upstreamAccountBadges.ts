export type UpstreamPlanBadgeVariant =
  | "default"
  | "accent"
  | "secondary"
  | "success"
  | "info"
  | "warning"
  | "error";

type UpstreamPlanBadgeRecipe = {
  variant: UpstreamPlanBadgeVariant;
  className: string;
  dataPlan: string;
};

const PLAN_VARIANTS: Record<string, UpstreamPlanBadgeVariant> = {
  local: "secondary",
  free: "warning",
  plus: "default",
  pro: "default",
  team: "info",
  enterprise: "accent",
};

const COMPACT_PLAN_LABELS: Record<string, string> = {
  free: "Free",
  plus: "Plus",
  pro: "Pro",
  team: "Team",
  enterprise: "Ent",
};

function normalizePlanType(planType?: string | null) {
  const normalized = planType?.trim().toLowerCase();
  return normalized ? normalized : null;
}

export function upstreamPlanBadgeRecipe(planType?: string | null): UpstreamPlanBadgeRecipe | null {
  const normalized = normalizePlanType(planType);
  if (!normalized) return null;

  return {
    variant: PLAN_VARIANTS[normalized] ?? "secondary",
    className: "upstream-plan-badge",
    dataPlan: normalized,
  };
}

export function shouldShowUpstreamPlanBadge(planType?: string | null) {
  const normalized = normalizePlanType(planType);
  return Boolean(normalized && normalized !== "local");
}

export function compactUpstreamPlanLabel(planType?: string | null) {
  const normalized = normalizePlanType(planType);
  if (!normalized) return null;
  return COMPACT_PLAN_LABELS[normalized] ?? planType?.trim() ?? normalized;
}
