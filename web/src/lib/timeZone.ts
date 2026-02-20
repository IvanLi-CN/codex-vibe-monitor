export function getBrowserTimeZone(): string {
  try {
    // Returns an IANA timezone name, e.g. "Asia/Shanghai" or "America/Los_Angeles".
    return Intl.DateTimeFormat().resolvedOptions().timeZone ?? 'UTC'
  } catch {
    return 'UTC'
  }
}

