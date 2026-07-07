export type AdaptiveMetricValueKind = 'number' | 'integer' | 'currency'
export type AdaptiveCurrencyProfile = 'default' | 'rate'

const COMPACT_SUFFIX_LOCALE = 'en-US'
const COMPACT_UNITS = [
  { level: 1, divisor: 1_000, suffix: 'K' },
  { level: 2, divisor: 1_000_000, suffix: 'M' },
  { level: 3, divisor: 1_000_000_000, suffix: 'B' },
  { level: 4, divisor: 1_000_000_000_000, suffix: 'T' },
] as const

export interface AdaptiveDisplayMeasureCandidate {
  key: string
  value: string
  compact: boolean
  precisionLabel: string
  priority: number
}

export interface AdaptiveDisplayValueSpec {
  fullValue: string
  candidates: AdaptiveDisplayMeasureCandidate[]
}

interface AdaptiveMetricSpecOptions {
  currencyProfile?: AdaptiveCurrencyProfile
}

interface StandardFormatterOptions {
  maximumFractionDigits: number
  minimumFractionDigits?: number
}

function compactNumberLocale(localeTag: string) {
  return localeTag.toLowerCase().startsWith('zh') ? COMPACT_SUFFIX_LOCALE : localeTag
}

function createStandardMetricFormatter(
  localeTag: string,
  kind: AdaptiveMetricValueKind,
  { maximumFractionDigits, minimumFractionDigits = 0 }: StandardFormatterOptions,
) {
  if (kind === 'currency') {
    return new Intl.NumberFormat(localeTag, {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits,
      maximumFractionDigits,
    })
  }

  return new Intl.NumberFormat(localeTag, {
    minimumFractionDigits,
    maximumFractionDigits,
  })
}

function createCurrencyFormatter(
  localeTag: string,
  maximumFractionDigits: number,
  minimumFractionDigits = 0,
) {
  return new Intl.NumberFormat(localeTag, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits,
    maximumFractionDigits,
  })
}

function createDecimalFormatter(
  localeTag: string,
  maximumFractionDigits: number,
  minimumFractionDigits = 0,
) {
  return new Intl.NumberFormat(localeTag, {
    minimumFractionDigits,
    maximumFractionDigits,
  })
}

function standardPrecisionCandidates(kind: AdaptiveMetricValueKind) {
  if (kind === 'integer') return [0]
  return [2, 1, 0]
}

function compactPrecisionCandidates(scaledAbsValue: number) {
  if (scaledAbsValue < 10) return [4, 3, 2, 1, 0]
  if (scaledAbsValue < 100) return [3, 2, 1, 0]
  if (scaledAbsValue < 1_000) return [2, 1, 0]
  return [1, 0]
}

function minimumPrimaryCompactPrecision(scaledAbsValue: number) {
  if (scaledAbsValue <= 0 || !Number.isFinite(scaledAbsValue)) return 0
  const integerDigits = Math.max(1, Math.floor(Math.log10(scaledAbsValue)) + 1)
  return Math.max(0, 3 - integerDigits)
}

function createCompactMetricValue(
  value: number,
  localeTag: string,
  kind: AdaptiveMetricValueKind,
  unitLevel: number,
  precision: number,
  minimumFractionDigits = 0,
) {
  const unit = COMPACT_UNITS.find((candidate) => candidate.level === unitLevel)
  if (!unit) {
    return createStandardMetricFormatter(localeTag, kind, {
      maximumFractionDigits: precision,
      minimumFractionDigits: Math.min(precision, minimumFractionDigits),
    }).format(value)
  }

  const scaledValue = value / unit.divisor
  const numberFormatter = new Intl.NumberFormat(
    kind === 'currency' ? localeTag : compactNumberLocale(localeTag),
    {
      minimumFractionDigits,
      maximumFractionDigits: precision,
    },
  )
  const formattedScaledValue =
    kind === 'currency'
      ? createStandardMetricFormatter(localeTag, kind, {
          maximumFractionDigits: precision,
          minimumFractionDigits,
        }).format(scaledValue)
      : numberFormatter.format(scaledValue)

  return `${formattedScaledValue}${unit.suffix}`
}

function sortAndDedupeCandidates(
  fullValue: string,
  candidates: AdaptiveDisplayMeasureCandidate[],
): AdaptiveDisplayValueSpec {
  const uniqueCandidates = new Map<string, AdaptiveDisplayMeasureCandidate>()

  for (const candidate of candidates) {
    const existing = uniqueCandidates.get(candidate.value)
    if (!existing || candidate.priority < existing.priority) {
      uniqueCandidates.set(candidate.value, candidate)
    }
  }

  return {
    fullValue,
    candidates: [...uniqueCandidates.values()].sort((left, right) => {
      if (left.priority !== right.priority) return left.priority - right.priority
      return left.value.length - right.value.length
    }),
  }
}

export function buildAdaptiveMetricSpec(
  value: number,
  localeTag: string,
  kind: AdaptiveMetricValueKind,
  options: AdaptiveMetricSpecOptions = {},
): AdaptiveDisplayValueSpec {
  const currencyProfile = options.currencyProfile ?? 'default'
  const standardPrecisions =
    kind === 'currency' && currencyProfile === 'rate' ? [2, 1, 0] : standardPrecisionCandidates(kind)
  const defaultPrecision = standardPrecisions[0] ?? 0
  const defaultMinimumFractionDigits =
    kind === 'currency' && currencyProfile === 'rate' ? defaultPrecision : 0
  const fullValue = createStandardMetricFormatter(localeTag, kind, {
    maximumFractionDigits: defaultPrecision,
    minimumFractionDigits: defaultMinimumFractionDigits,
  }).format(value)
  const candidates: AdaptiveDisplayMeasureCandidate[] = []

  for (const [index, precision] of standardPrecisions.entries()) {
    const minimumFractionDigits =
      kind === 'currency' && currencyProfile === 'rate' ? precision : 0
    candidates.push({
      key: `standard-${precision}`,
      value: createStandardMetricFormatter(localeTag, kind, {
        maximumFractionDigits: precision,
        minimumFractionDigits,
      }).format(value),
      compact: false,
      precisionLabel: index === 0 ? 'full' : `standard-${precision}`,
      priority: index,
    })
  }

  const absValue = Math.abs(value)
  if (!Number.isFinite(absValue) || absValue < 1_000) {
    return sortAndDedupeCandidates(fullValue, candidates)
  }

  const primaryUnit =
    [...COMPACT_UNITS].reverse().find((unit) => absValue >= unit.divisor) ?? COMPACT_UNITS[0]
  const primaryScaledAbsValue = absValue / primaryUnit.divisor
  const preferredCompactPrecision = minimumPrimaryCompactPrecision(primaryScaledAbsValue)

  for (const precision of compactPrecisionCandidates(primaryScaledAbsValue)) {
    const minimumFractionDigits = Math.min(precision, preferredCompactPrecision)
    candidates.push({
      key: `compact-${primaryUnit.suffix}-${precision}`,
      value: createCompactMetricValue(
        value,
        localeTag,
        kind,
        primaryUnit.level,
        precision,
        minimumFractionDigits,
      ),
      compact: true,
      precisionLabel: String(precision),
      priority:
        precision >= preferredCompactPrecision
          ? 20 + (preferredCompactPrecision === precision ? 0 : 4 - precision)
          : 60 + (4 - precision),
    })
  }

  if (primaryScaledAbsValue < 10 && primaryUnit.level > 1) {
    const fallbackUnit = COMPACT_UNITS.find((unit) => unit.level === primaryUnit.level - 1)
    if (fallbackUnit) {
      const fallbackScaledAbsValue = absValue / fallbackUnit.divisor
      for (const precision of compactPrecisionCandidates(fallbackScaledAbsValue)) {
        candidates.push({
          key: `compact-${fallbackUnit.suffix}-${precision}`,
          value: createCompactMetricValue(value, localeTag, kind, fallbackUnit.level, precision),
          compact: true,
          precisionLabel: `${fallbackUnit.suffix}-${precision}`,
          priority: 40 + (4 - precision),
        })
      }
    }
  }

  return sortAndDedupeCandidates(fullValue, candidates)
}

export function buildAdaptiveTextSpec(
  fullValue: string,
  candidates: Array<{ key: string; value: string; priority: number }>,
): AdaptiveDisplayValueSpec {
  return sortAndDedupeCandidates(
    fullValue,
    candidates.map((candidate) => ({
      key: candidate.key,
      value: candidate.value,
      compact: candidate.key.includes('compact'),
      precisionLabel: candidate.key,
      priority: candidate.priority,
    })),
  )
}

export function buildAdaptiveNumberTextSpec(
  value: number | null,
  localeTag: string,
  maximumFractionDigits = 2,
) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  const kind: AdaptiveMetricValueKind = maximumFractionDigits === 0 ? 'integer' : 'number'
  return buildAdaptiveMetricSpec(value, localeTag, kind)
}

export function buildAdaptiveCurrencyTextSpec(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  const fullValue = createCurrencyFormatter(localeTag, 2).format(value)

  return buildAdaptiveTextSpec(fullValue, [
    {
      key: 'full',
      value: fullValue,
      priority: 0,
    },
    {
      key: 'standard-1',
      value: createCurrencyFormatter(localeTag, 1).format(value),
      priority: 1,
    },
    {
      key: 'standard-0',
      value: createCurrencyFormatter(localeTag, 0).format(value),
      priority: 2,
    },
    ...buildAdaptiveMetricSpec(value, localeTag, 'currency').candidates.map((candidate, index) => ({
      key: candidate.key,
      value: candidate.value,
      priority: 20 + index,
    })),
  ])
}

export function buildAdaptiveCurrencyAmountTextSpec(
  value: number | null,
  localeTag: string,
  {
    maximumFractionDigits = 2,
    minimumFractionDigits = maximumFractionDigits,
  }: {
    maximumFractionDigits?: number
    minimumFractionDigits?: number
  } = {},
) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  const precisionCandidates = Array.from(
    { length: maximumFractionDigits + 1 },
    (_, index) => maximumFractionDigits - index,
  )
  const fullValue = createDecimalFormatter(
    localeTag,
    maximumFractionDigits,
    minimumFractionDigits,
  ).format(value)

  return buildAdaptiveTextSpec(fullValue, [
    {
      key: 'full',
      value: fullValue,
      priority: 0,
    },
    ...precisionCandidates
      .filter((precision) => precision !== maximumFractionDigits)
      .map((precision, index) => ({
        key: `standard-${precision}`,
        value: createDecimalFormatter(localeTag, precision, Math.min(precision, minimumFractionDigits)).format(value),
        priority: index + 1,
      })),
    ...buildAdaptiveMetricSpec(value, localeTag, 'number').candidates.map((candidate, index) => ({
      key: candidate.key,
      value: candidate.value,
      priority: 20 + index,
    })),
  ])
}

export function buildAdaptiveRateCurrencyTextSpec(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  return buildAdaptiveMetricSpec(value, localeTag, 'currency', { currencyProfile: 'rate' })
}

export function buildAdaptivePercentTextSpec(
  value: number | null,
  localeTag: string,
  {
    maximumFractionDigits = 1,
    signDisplay,
  }: {
    maximumFractionDigits?: number
    signDisplay?: Intl.NumberFormatOptions['signDisplay']
  } = {},
) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  const precisions = [...new Set([maximumFractionDigits, maximumFractionDigits - 1, 0])].filter(
    (precision) => precision >= 0,
  )
  const fullValue = new Intl.NumberFormat(localeTag, {
    style: 'percent',
    maximumFractionDigits,
    signDisplay,
  }).format(value)

  return buildAdaptiveTextSpec(
    fullValue,
    precisions.map((precision, index) => ({
      key: index === 0 ? 'full' : `percent-${precision}`,
      value: new Intl.NumberFormat(localeTag, {
        style: 'percent',
        maximumFractionDigits: precision,
        signDisplay,
      }).format(value),
      priority: index,
    })),
  )
}

export function buildAdaptiveDurationTextSpec(valueMs: number | null, localeTag: string) {
  if (valueMs == null || !Number.isFinite(valueMs)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  if (valueMs < 1_000) {
    const fullValue = `${new Intl.NumberFormat(localeTag, { maximumFractionDigits: 1 }).format(valueMs)} ms`
    return buildAdaptiveTextSpec(fullValue, [
      { key: 'full', value: fullValue, priority: 0 },
      {
        key: 'ms-0',
        value: `${new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }).format(valueMs)} ms`,
        priority: 1,
      },
      {
        key: 'ms-0-compact',
        value: `${new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }).format(valueMs)}ms`,
        priority: 2,
      },
    ])
  }

  const seconds = valueMs / 1_000
  const maximumFractionDigits =
    Math.abs(seconds) >= 100 ? 1 : Math.abs(seconds) >= 1 ? 2 : 3
  const roundedValue = Number(seconds.toFixed(maximumFractionDigits))
  const precisions = Array.from(
    { length: maximumFractionDigits + 1 },
    (_, index) => maximumFractionDigits - index,
  )
  const fullValue = `${roundedValue.toLocaleString(localeTag, {
    minimumFractionDigits: 0,
    maximumFractionDigits,
  })} s`
  const candidates: Array<{ key: string; value: string; priority: number }> = []
  let priority = 0

  for (const precision of precisions) {
    const formattedValue = roundedValue.toLocaleString(localeTag, {
      minimumFractionDigits: 0,
      maximumFractionDigits: precision,
    })
    candidates.push({
      key: precision === maximumFractionDigits ? 'full' : `seconds-${precision}`,
      value: `${formattedValue} s`,
      priority,
    })
    priority += 1
    if (precision < maximumFractionDigits) {
      candidates.push({
        key: `seconds-${precision}-compact`,
        value: `${formattedValue}s`,
        priority,
      })
      priority += 1
    }
  }

  return buildAdaptiveTextSpec(fullValue, candidates)
}
