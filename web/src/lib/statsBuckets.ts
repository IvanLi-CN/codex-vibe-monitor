import type { TranslationKey } from '../i18n'

export interface StatsOption {
  value: string
  labelKey: TranslationKey
}

export const STATS_RANGE_OPTIONS = [
  { value: '1h', labelKey: 'stats.range.lastHour' },
  { value: 'today', labelKey: 'stats.range.today' },
  { value: '1d', labelKey: 'stats.range.lastDay' },
  { value: 'thisWeek', labelKey: 'stats.range.thisWeek' },
  { value: '7d', labelKey: 'stats.range.lastWeek' },
  { value: 'thisMonth', labelKey: 'stats.range.thisMonth' },
  { value: '1mo', labelKey: 'stats.range.lastMonth' },
] as const satisfies readonly StatsOption[]

export const STATS_BUCKET_OPTION_KEYS: Record<string, readonly StatsOption[]> = {
  '1h': [
    { value: '1m', labelKey: 'stats.bucket.eachMinute' },
    { value: '5m', labelKey: 'stats.bucket.each5Minutes' },
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
  ],
  '1d': [
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
    { value: '30m', labelKey: 'stats.bucket.each30Minutes' },
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
  ],
  today: [
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
    { value: '30m', labelKey: 'stats.bucket.each30Minutes' },
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
  ],
  '7d': [
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
  ],
  thisWeek: [
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
  '1mo': [
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
  thisMonth: [
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
}

export function resolveStatsBucketOptions(
  range: string,
  availableBuckets?: readonly string[] | null,
): StatsOption[] {
  const raw = STATS_BUCKET_OPTION_KEYS[range] ?? STATS_BUCKET_OPTION_KEYS['1d']
  if (!availableBuckets || availableBuckets.length === 0) {
    return [...raw]
  }

  const allowed = new Set(availableBuckets)
  const filtered = raw.filter((option) => allowed.has(option.value))
  if (filtered.length > 0) {
    return filtered
  }

  const fallbackDaily = raw.filter((option) => option.value === '1d')
  return fallbackDaily.length > 0 ? fallbackDaily : [...raw]
}

export function resolveStatsBucketValue(
  currentBucket: string,
  options: readonly Pick<StatsOption, 'value'>[],
): string {
  if (options.some((option) => option.value === currentBucket)) {
    return currentBucket
  }
  return options[0]?.value ?? '1d'
}
