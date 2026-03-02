const DEFAULT_FALLBACK = '—'

export function formatProxyWeightDelta(
  value: number | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback
  const normalized = Object.is(value, -0) ? 0 : value
  const sign = normalized >= 0 ? '+' : '-'
  return `${sign}${Math.abs(normalized).toFixed(2)}`
}
