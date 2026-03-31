export const CONCURRENCY_LIMIT_MIN = 1
export const CONCURRENCY_LIMIT_MAX = 30
export const CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE = 31

function normalizeStoredConcurrencyLimit(value?: number | null): number {
  if (!Number.isFinite(value)) return 0
  const normalized = Math.trunc(value as number)
  if (normalized <= 0) return 0
  return Math.min(normalized, CONCURRENCY_LIMIT_MAX)
}

export function apiConcurrencyLimitToSliderValue(value?: number | null): number {
  const normalized = normalizeStoredConcurrencyLimit(value)
  return normalized === 0 ? CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE : normalized
}

export function sliderConcurrencyLimitToApiValue(value: number): number {
  if (!Number.isFinite(value)) return 0
  const normalized = Math.trunc(value)
  if (normalized >= CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE) return 0
  if (normalized <= CONCURRENCY_LIMIT_MIN) return CONCURRENCY_LIMIT_MIN
  return Math.min(normalized, CONCURRENCY_LIMIT_MAX)
}

export function isFiniteConcurrencyLimit(value?: number | null): boolean {
  return normalizeStoredConcurrencyLimit(value) > 0
}

export function formatConcurrencyLimitValue(
  value: number,
  unlimitedLabel: string,
): string {
  const normalized = normalizeStoredConcurrencyLimit(value)
  return normalized === 0 ? unlimitedLabel : String(normalized)
}
