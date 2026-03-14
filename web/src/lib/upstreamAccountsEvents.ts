export const UPSTREAM_ACCOUNTS_CHANGED_EVENT = 'upstream-accounts:changed'

export function emitUpstreamAccountsChanged() {
  if (typeof window === 'undefined') return
  window.dispatchEvent(new CustomEvent(UPSTREAM_ACCOUNTS_CHANGED_EVENT))
}
