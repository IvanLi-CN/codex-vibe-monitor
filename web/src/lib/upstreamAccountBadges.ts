export type UpstreamPlanBadgeVariant =
  | 'default'
  | 'accent'
  | 'secondary'
  | 'success'
  | 'info'
  | 'warning'
  | 'error'

type UpstreamPlanBadgeRecipe = {
  variant: UpstreamPlanBadgeVariant
  className: string
  dataPlan: string
}

const PLAN_VARIANTS: Record<string, UpstreamPlanBadgeVariant> = {
  local: 'secondary',
  free: 'warning',
  pro: 'default',
  team: 'info',
  enterprise: 'accent',
}

function normalizePlanType(planType?: string | null) {
  const normalized = planType?.trim().toLowerCase()
  return normalized ? normalized : null
}

export function upstreamPlanBadgeRecipe(planType?: string | null): UpstreamPlanBadgeRecipe | null {
  const normalized = normalizePlanType(planType)
  if (!normalized) return null

  return {
    variant: PLAN_VARIANTS[normalized] ?? 'secondary',
    className: 'upstream-plan-badge',
    dataPlan: normalized,
  }
}
