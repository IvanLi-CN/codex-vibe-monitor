export type UpstreamBaseUrlValidationCode = 'invalid_absolute_url' | 'query_or_fragment_not_allowed'

export function validateUpstreamBaseUrl(raw: string): UpstreamBaseUrlValidationCode | null {
  const trimmed = raw.trim()
  if (!trimmed) return null

  let parsed: URL
  try {
    parsed = new URL(trimmed)
  } catch {
    return 'invalid_absolute_url'
  }

  if (!matchesAllowedProtocol(parsed) || !parsed.host) {
    return 'invalid_absolute_url'
  }

  if (parsed.search || parsed.hash) {
    return 'query_or_fragment_not_allowed'
  }

  return null
}

function matchesAllowedProtocol(url: URL) {
  return url.protocol === 'http:' || url.protocol === 'https:'
}
