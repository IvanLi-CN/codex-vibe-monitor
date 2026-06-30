export type DashboardActivityRangeKey = 'today' | 'yesterday' | '1d' | '7d' | 'usage'
export type DashboardWorkspaceView = 'conversations' | 'upstreamAccounts'

export const DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY = 'dashboard.activityOverview.activeRange.v1'
export const ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX = 'account.activityOverview.activeRange.v1'
export const DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY = 'dashboard.workspace.activeView.v1'

const DEFAULT_RANGE: DashboardActivityRangeKey = 'today'
const DEFAULT_WORKSPACE_VIEW: DashboardWorkspaceView = 'conversations'

function isRangeKey(value: string | null): value is DashboardActivityRangeKey {
  return value === 'today' || value === 'yesterday' || value === '1d' || value === '7d' || value === 'usage'
}

function isDashboardWorkspaceView(
  value: string | null,
): value is DashboardWorkspaceView {
  return value === 'conversations' || value === 'upstreamAccounts'
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

export function readPersistedDashboardWorkspaceView(
  storageKey: string,
): DashboardWorkspaceView {
  if (typeof window === 'undefined') return DEFAULT_WORKSPACE_VIEW
  try {
    const cached = window.localStorage.getItem(storageKey)
    return isDashboardWorkspaceView(cached) ? cached : DEFAULT_WORKSPACE_VIEW
  } catch {
    return DEFAULT_WORKSPACE_VIEW
  }
}

export function persistDashboardWorkspaceView(
  storageKey: string,
  view: DashboardWorkspaceView,
) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(storageKey, view)
  } catch {
    // Ignore storage write failures and keep the UI responsive.
  }
}
