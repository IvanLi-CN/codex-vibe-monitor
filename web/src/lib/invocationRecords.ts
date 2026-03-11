import type { InvocationRangePreset, InvocationRecordsQuery, InvocationSortBy, InvocationSortOrder } from './api'

export const RECORDS_PAGE_SIZE_OPTIONS = [20, 50, 100] as const
export const RECORDS_NEW_COUNT_POLL_INTERVAL_MS = 15_000
export const DEFAULT_RECORDS_FOCUS = 'token' as const
export const DEFAULT_RECORDS_SORT_BY: InvocationSortBy = 'occurredAt'
export const DEFAULT_RECORDS_SORT_ORDER: InvocationSortOrder = 'desc'
export const DEFAULT_RECORDS_PAGE_SIZE = RECORDS_PAGE_SIZE_OPTIONS[0]

export interface InvocationRecordsDraftFilters {
  rangePreset: InvocationRangePreset
  customFrom: string
  customTo: string
  status: string
  model: string
  proxy: string
  endpoint: string
  failureClass: string
  failureKind: string
  promptCacheKey: string
  requesterIp: string
  keyword: string
  minTotalTokens: string
  maxTotalTokens: string
  minTotalMs: string
  maxTotalMs: string
}

export function createDefaultInvocationRecordsDraft(): InvocationRecordsDraftFilters {
  return {
    rangePreset: 'today',
    customFrom: '',
    customTo: '',
    status: '',
    model: '',
    proxy: '',
    endpoint: '',
    failureClass: '',
    failureKind: '',
    promptCacheKey: '',
    requesterIp: '',
    keyword: '',
    minTotalTokens: '',
    maxTotalTokens: '',
    minTotalMs: '',
    maxTotalMs: '',
  }
}

function toIsoString(date: Date) {
  return date.toISOString()
}

function isMinutePrecisionLocalDateTimeValue(value: string) {
  // `datetime-local` defaults to "YYYY-MM-DDTHH:mm" (minute precision).
  // When we send that as an exclusive `< to` bound, it unintentionally excludes the whole minute.
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value)
}

function resolveCustomToUpperBound(value: string) {
  const parsed = new Date(value)
  if (isMinutePrecisionLocalDateTimeValue(value)) {
    return new Date(parsed.getTime() + 60_000)
  }
  return parsed
}

function toLocalDateTimeValue(date: Date) {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function createDefaultCustomRange(now = new Date()) {
  const from = new Date(now)
  from.setHours(0, 0, 0, 0)
  return {
    customFrom: toLocalDateTimeValue(from),
    customTo: toLocalDateTimeValue(now),
  }
}

function normalizeText(value: string) {
  const normalized = value.trim()
  return normalized ? normalized : undefined
}

function normalizeNumber(value: string) {
  const normalized = value.trim()
  if (!normalized) return undefined
  const parsed = Number(normalized)
  return Number.isFinite(parsed) ? parsed : undefined
}

function normalizeInteger(value: string, fieldName: string) {
  const normalized = value.trim()
  if (!normalized) return undefined
  const parsed = Number(normalized)
  if (!Number.isFinite(parsed)) return undefined
  if (!Number.isInteger(parsed)) {
    throw new Error(`${fieldName} must be a whole number`)
  }
  return parsed
}


function normalizeIntegerSafely(value: string, fieldName: string) {
  try {
    return normalizeInteger(value, fieldName)
  } catch {
    return undefined
  }
}

function resolveRangeBoundsSafely(
  rangePreset: InvocationRangePreset,
  draft: InvocationRecordsDraftFilters,
  now = new Date(),
) {
  try {
    return resolveRangeBounds(rangePreset, draft, now)
  } catch {
    return { from: undefined, to: undefined }
  }
}

export function resolveRangeBoundsFromValues(
  rangePreset: InvocationRangePreset,
  customFrom: string,
  customTo: string,
  now = new Date(),
) {
  if (rangePreset === 'custom') {
    return {
      from: customFrom ? toIsoString(new Date(customFrom)) : undefined,
      // Treat minute-based inputs as inclusive-of-minute for UX, while keeping server-side `< to` bounds.
      to: customTo ? toIsoString(resolveCustomToUpperBound(customTo)) : undefined,
    }
  }

  const end = new Date(now)
  const start = new Date(now)
  switch (rangePreset) {
    case 'today':
      start.setHours(0, 0, 0, 0)
      end.setDate(end.getDate() + 1)
      end.setHours(0, 0, 0, 0)
      break
    case '1d':
      start.setDate(start.getDate() - 1)
      break
    case '7d':
      start.setDate(start.getDate() - 7)
      break
    case '30d':
      start.setDate(start.getDate() - 30)
      break
    default:
      break
  }

  return {
    from: toIsoString(start),
    to: toIsoString(end),
  }
}

export function resolveRangeBounds(rangePreset: InvocationRangePreset, draft: InvocationRecordsDraftFilters, now = new Date()) {
  return resolveRangeBoundsFromValues(rangePreset, draft.customFrom, draft.customTo, now)
}

export function buildAppliedInvocationFilters(
  draft: InvocationRecordsDraftFilters,
  now = new Date(),
): Omit<InvocationRecordsQuery, 'page' | 'pageSize' | 'sortBy' | 'sortOrder' | 'snapshotId'> {
  const bounds = resolveRangeBounds(draft.rangePreset, draft, now)
  return {
    rangePreset: draft.rangePreset,
    from: bounds.from,
    to: bounds.to,
    status: normalizeText(draft.status),
    model: normalizeText(draft.model),
    proxy: normalizeText(draft.proxy),
    endpoint: normalizeText(draft.endpoint),
    failureClass: normalizeText(draft.failureClass),
    failureKind: normalizeText(draft.failureKind),
    promptCacheKey: normalizeText(draft.promptCacheKey),
    requesterIp: normalizeText(draft.requesterIp),
    keyword: normalizeText(draft.keyword),
    minTotalTokens: normalizeInteger(draft.minTotalTokens, 'minTotalTokens'),
    maxTotalTokens: normalizeInteger(draft.maxTotalTokens, 'maxTotalTokens'),
    minTotalMs: normalizeNumber(draft.minTotalMs),
    maxTotalMs: normalizeNumber(draft.maxTotalMs),
  }
}

export function buildInvocationSuggestionsQuery(
  draft: InvocationRecordsDraftFilters,
  snapshotId?: number,
  now = new Date(),
): Omit<InvocationRecordsQuery, 'page' | 'pageSize' | 'sortBy' | 'sortOrder'> {
  const bounds = resolveRangeBoundsSafely(draft.rangePreset, draft, now)
  return {
    rangePreset: draft.rangePreset,
    from: bounds.from,
    to: bounds.to,
    status: normalizeText(draft.status),
    model: normalizeText(draft.model),
    proxy: normalizeText(draft.proxy),
    endpoint: normalizeText(draft.endpoint),
    failureClass: normalizeText(draft.failureClass),
    failureKind: normalizeText(draft.failureKind),
    promptCacheKey: normalizeText(draft.promptCacheKey),
    requesterIp: normalizeText(draft.requesterIp),
    keyword: normalizeText(draft.keyword),
    minTotalTokens: normalizeIntegerSafely(draft.minTotalTokens, 'minTotalTokens'),
    maxTotalTokens: normalizeIntegerSafely(draft.maxTotalTokens, 'maxTotalTokens'),
    minTotalMs: normalizeNumber(draft.minTotalMs),
    maxTotalMs: normalizeNumber(draft.maxTotalMs),
    snapshotId,
  }
}

export function buildInvocationRecordsQuery(
  base: Omit<InvocationRecordsQuery, 'page' | 'pageSize' | 'sortBy' | 'sortOrder' | 'snapshotId'>,
  options: {
    page: number
    pageSize: number
    sortBy: InvocationSortBy
    sortOrder: InvocationSortOrder
    snapshotId?: number
  },
): InvocationRecordsQuery {
  return {
    ...base,
    page: options.page,
    pageSize: options.pageSize,
    sortBy: options.sortBy,
    sortOrder: options.sortOrder,
    snapshotId: options.snapshotId,
  }
}
