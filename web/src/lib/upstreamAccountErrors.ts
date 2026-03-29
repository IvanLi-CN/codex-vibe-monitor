export function isUpstreamAccountNotFoundError(error: unknown) {
  const message =
    error instanceof Error ? error.message : typeof error === 'string' ? error : ''
  const normalized = message.trim().toLowerCase()
  return normalized.includes('request failed: 404') && normalized.includes('account not found')
}
