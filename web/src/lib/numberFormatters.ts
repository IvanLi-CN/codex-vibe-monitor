export function formatTokensShort(value: number, localeTag: string): string {
  const abs = Math.abs(value)
  const sign = value < 0 ? '-' : ''

  const format = (n: number) =>
    new Intl.NumberFormat(localeTag, {
      maximumFractionDigits: n >= 10 ? 0 : 1,
      minimumFractionDigits: 0,
    }).format(n)

  if (abs >= 1_000_000_000) {
    return `${sign}${format(abs / 1_000_000_000)}B`
  }
  if (abs >= 1_000_000) {
    return `${sign}${format(abs / 1_000_000)}M`
  }
  if (abs >= 1_000) {
    return `${sign}${format(abs / 1_000)}K`
  }

  // For smaller values, keep full precision but with locale-aware grouping
  return new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }).format(value)
}

