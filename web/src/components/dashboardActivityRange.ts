export type DashboardActivityRangeKey = 'today' | 'yesterday' | '1d' | '7d' | 'usage'

export const DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY = 'dashboard.activityOverview.activeRange.v1'
export const ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX = 'account.activityOverview.activeRange.v1'

const DEFAULT_RANGE: DashboardActivityRangeKey = 'today'

function isRangeKey(value: string | null): value is DashboardActivityRangeKey {
  return value === 'today' || value === 'yesterday' || value === '1d' || value === '7d' || value === 'usage'
}

export function readPersistedDashboardActivityRange(storageKey: string): DashboardActivityRangeKey {
  if (typeof window === 'undefined') return DEFAULT_RANGE
  try {
    const cached = window.localStorage.getItem(storageKey)
    return isRangeKey(cached) ? cached : DEFAULT_RANGE
  } catch {
    return DEFAULT_RANGE
  }
}

export function persistDashboardActivityRange(
  storageKey: string,
  range: DashboardActivityRangeKey,
) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(storageKey, range)
  } catch {
    // Ignore storage write failures and keep the UI responsive.
  }
}
