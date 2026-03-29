function decodeBase64Maybe(raw: string): string | null {
  const compact = raw.trim().replace(/\s+/g, "")
  if (!compact) return null
  const normalized = compact.replace(/-/g, "+").replace(/_/g, "/")
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4)
  try {
    return atob(padded)
  } catch {
    return null
  }
}

function labelFromUrl(url: URL): string | null {
  const fragment = decodeURIComponent(url.hash.replace(/^#/, "")).trim()
  if (fragment) return fragment

  const host = url.hostname || url.host
  if (!host) return null
  const defaultPort = url.protocol === "https:" ? "443" : url.protocol === "http:" ? "80" : ""
  const port = url.port || defaultPort
  return port ? `${host}:${port}` : host
}

const FORWARD_PROXY_PROTOCOL_LABELS: Record<string, string> = {
  direct: "DIRECT",
  http: "HTTP",
  https: "HTTPS",
  socks: "SOCKS",
  socks5: "SOCKS5",
  socks5h: "SOCKS5H",
  vmess: "VMESS",
  vless: "VLESS",
  trojan: "TROJAN",
  ss: "SS",
}

export function normalizeForwardProxyProtocolLabel(raw: string | null | undefined): string {
  const candidate = raw?.trim()
  if (!candidate) return "UNKNOWN"
  return FORWARD_PROXY_PROTOCOL_LABELS[candidate.toLowerCase()] ?? candidate.toUpperCase()
}

export function extractProxyDisplayName(raw: string): string | null {
  const candidate = raw.trim()
  if (!candidate) return null

  if (candidate.startsWith("vmess://")) {
    const payload = candidate.slice("vmess://".length).split("#")[0].split("?")[0]
    const decoded = decodeBase64Maybe(payload)
    if (!decoded) return null
    try {
      const parsed = JSON.parse(decoded) as { ps?: string; add?: string; port?: string | number }
      const display = (parsed.ps || "").trim()
      if (display) return display
      if (parsed.add) return parsed.port ? `${parsed.add}:${parsed.port}` : parsed.add
    } catch {
      return null
    }
    return null
  }

  if (candidate.startsWith("ss://")) {
    const fragment = candidate.split("#")[1]
    if (fragment) {
      const decoded = decodeURIComponent(fragment).trim()
      if (decoded) return decoded
    }
  }

  try {
    return labelFromUrl(new URL(candidate))
  } catch {
    return null
  }
}

export function extractProxyProtocolName(raw: string): string | null {
  const candidate = raw.trim()
  if (!candidate) return null

  const schemeFromUrl = (() => {
    try {
      return new URL(candidate).protocol.replace(/:$/, "").toLowerCase()
    } catch {
      return null
    }
  })()
  const scheme =
    schemeFromUrl ??
    candidate.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//)?.[1]?.toLowerCase() ??
    null
  if (!scheme) return null
  return normalizeForwardProxyProtocolLabel(scheme)
}
