const DEFAULT_FALLBACK = '—'
const PRIORITY_SERVICE_TIER = 'priority'

export type ProxyWeightDeltaDirection = 'up' | 'down' | 'flat' | 'missing'

export interface ProxyWeightDeltaView {
  direction: ProxyWeightDeltaDirection
  value: string
}

export function normalizeServiceTier(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null
  const normalized = value.trim().toLowerCase()
  return normalized.length > 0 ? normalized : null
}

export function formatServiceTier(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): string {
  return normalizeServiceTier(value) ?? fallback
}

export function isPriorityServiceTier(value: string | null | undefined): boolean {
  return normalizeServiceTier(value) === PRIORITY_SERVICE_TIER
}

export function formatProxyWeightDelta(
  value: number | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): ProxyWeightDeltaView {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return { direction: 'missing', value: fallback }
  }
  const normalized = Object.is(value, -0) ? 0 : value
  const rounded = Number(normalized.toFixed(2))
  if (rounded > 0) return { direction: 'up', value: Math.abs(rounded).toFixed(2) }
  if (rounded < 0) return { direction: 'down', value: Math.abs(rounded).toFixed(2) }
  return { direction: 'flat', value: Math.abs(rounded).toFixed(2) }
}
