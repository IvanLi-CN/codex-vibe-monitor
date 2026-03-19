import type { TranslationKey } from '../i18n'

export const RANGE_OPTIONS = [
  { value: '1h', labelKey: 'stats.range.lastHour' },
  { value: 'today', labelKey: 'stats.range.today' },
  { value: '1d', labelKey: 'stats.range.lastDay' },
  { value: 'thisWeek', labelKey: 'stats.range.thisWeek' },
  { value: '7d', labelKey: 'stats.range.lastWeek' },
  { value: 'thisMonth', labelKey: 'stats.range.thisMonth' },
  { value: '1mo', labelKey: 'stats.range.lastMonth' },
] as const satisfies readonly { value: string; labelKey: TranslationKey }[]

export const BUCKET_OPTION_KEYS: Record<string, { value: string; labelKey: TranslationKey }[]> = {
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
    { value: '1d', labelKey: 'stats.bucket.each24Hours' },
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
