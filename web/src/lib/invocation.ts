const DEFAULT_FALLBACK = '—'

export function formatProxyWeightDelta(
  value: number | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback
  const normalized = Object.is(value, -0) ? 0 : value
  const rounded = Number(normalized.toFixed(2))
  const sign = rounded >= 0 ? '+' : '-'
  const direction = rounded >= 0 ? '↑' : '↓'
  return `${direction} ${sign}${Math.abs(rounded).toFixed(2)}`
}
