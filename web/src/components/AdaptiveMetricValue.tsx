import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { AnimatedDigits } from './AnimatedDigits'

export type AdaptiveMetricValueKind = 'number' | 'integer' | 'currency'

const ADAPTIVE_METRIC_COMPACT_GUTTER_PX = 12
const COMPACT_SUFFIX_LOCALE = 'en-US'

interface MetricFormatterOptions {
  compact: boolean
  maximumFractionDigits?: number
}

interface CompactCandidate {
  value: string
  precision: number
}

interface AdaptiveMetricValueProps {
  value: number
  localeTag: string
  kind?: AdaptiveMetricValueKind
  className?: string
  'data-testid'?: string
}

function createMetricFormatter(
  localeTag: string,
  kind: AdaptiveMetricValueKind,
  { compact, maximumFractionDigits }: MetricFormatterOptions,
) {
  // Product choice: zh dashboards use short-scale suffixes like `1.31B`
  // for overflow fallback because they are materially shorter than localized compact units.
  const compactLocale =
    compact && localeTag.toLowerCase().startsWith('zh')
      ? COMPACT_SUFFIX_LOCALE
      : localeTag

  if (kind === 'currency') {
    return new Intl.NumberFormat(compactLocale, {
      style: 'currency',
      currency: 'USD',
      notation: compact ? 'compact' : 'standard',
      maximumFractionDigits: maximumFractionDigits ?? 2,
    })
  }

  return new Intl.NumberFormat(compactLocale, {
    notation: compact ? 'compact' : 'standard',
    maximumFractionDigits: maximumFractionDigits ?? (kind === 'integer' ? 0 : 2),
  })
}

function buildCompactCandidates(
  value: number,
  localeTag: string,
  kind: AdaptiveMetricValueKind,
): CompactCandidate[] {
  const precisionCandidates = kind === 'integer' ? [0] : [2, 1, 0]
  const uniqueValues = new Set<string>()
  const candidates: CompactCandidate[] = []

  for (const precision of precisionCandidates) {
    const compactValue = createMetricFormatter(localeTag, kind, {
      compact: true,
      maximumFractionDigits: precision,
    }).format(value)
    if (uniqueValues.has(compactValue)) continue
    uniqueValues.add(compactValue)
    candidates.push({ value: compactValue, precision })
  }

  return candidates
}

export function AdaptiveMetricValue({
  value,
  localeTag,
  kind = 'number',
  className,
  'data-testid': dataTestId,
}: AdaptiveMetricValueProps) {
  const containerRef = useRef<HTMLSpanElement | null>(null)
  const measureRefs = useRef<Array<HTMLSpanElement | null>>([])
  const [compactPrecision, setCompactPrecision] = useState<number | null>(null)

  const fullValue = useMemo(
    () => createMetricFormatter(localeTag, kind, { compact: false }).format(value),
    [kind, localeTag, value],
  )
  const compactCandidates = useMemo(
    () => buildCompactCandidates(value, localeTag, kind),
    [kind, localeTag, value],
  )

  const evaluateOverflow = useCallback(() => {
    const container = containerRef.current
    if (!container) return

    const availableWidth = container.clientWidth
    if (availableWidth <= 0) return

    const measures = measureRefs.current
    const fullMeasure = measures[0]
    const fullRequiredWidth = fullMeasure?.scrollWidth ?? 0
    if (fullRequiredWidth <= 0) return

    if (fullRequiredWidth + ADAPTIVE_METRIC_COMPACT_GUTTER_PX <= availableWidth) {
      setCompactPrecision((current) => (current === null ? current : null))
      return
    }

    let nextCompactPrecision = compactCandidates.at(-1)?.precision ?? null

    for (let index = 0; index < compactCandidates.length; index += 1) {
      const candidate = compactCandidates[index]
      const measure = measures[index + 1]
      const requiredWidth = measure?.scrollWidth ?? 0
      if (requiredWidth <= 0) continue
      if (requiredWidth + ADAPTIVE_METRIC_COMPACT_GUTTER_PX <= availableWidth) {
        nextCompactPrecision = candidate.precision
        break
      }
    }

    setCompactPrecision((current) =>
      current === nextCompactPrecision ? current : nextCompactPrecision,
    )
  }, [compactCandidates])

  useLayoutEffect(() => {
    evaluateOverflow()
    const frame = window.requestAnimationFrame(() => {
      evaluateOverflow()
    })

    return () => {
      window.cancelAnimationFrame(frame)
    }
  }, [evaluateOverflow, fullValue])

  useLayoutEffect(() => {
    const container = containerRef.current
    if (!container) return undefined

    window.addEventListener('resize', evaluateOverflow)

    if (typeof ResizeObserver === 'undefined') {
      return () => {
        window.removeEventListener('resize', evaluateOverflow)
      }
    }

    const observer = new ResizeObserver(() => {
      evaluateOverflow()
    })
    observer.observe(container)
    for (const measure of measureRefs.current) {
      if (measure) observer.observe(measure)
    }

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', evaluateOverflow)
    }
  }, [evaluateOverflow])

  const visibleValue =
    compactPrecision == null
      ? fullValue
      : compactCandidates.find((candidate) => candidate.precision === compactPrecision)?.value ?? fullValue
  const shouldAnimateDigits = (kind === 'number' || kind === 'integer') && compactPrecision == null

  return (
    <span
      ref={containerRef}
      data-adaptive-metric-container="true"
      data-compact={compactPrecision == null ? 'false' : 'true'}
      data-compact-precision={compactPrecision == null ? 'full' : String(compactPrecision)}
      data-testid={dataTestId}
      title={compactPrecision == null ? undefined : fullValue}
      className={`relative block min-w-0 max-w-full overflow-hidden whitespace-nowrap tabular-nums ${className ?? ''}`}
    >
      {[fullValue, ...compactCandidates.map((candidate) => candidate.value)].map((candidateValue, index) => (
        <span
          key={`${index}-${candidateValue}`}
          ref={(node) => {
            measureRefs.current[index] = node
          }}
          aria-hidden
          data-adaptive-metric-measure="true"
          data-adaptive-metric-measure-kind={index === 0 ? 'full' : 'compact'}
          data-adaptive-metric-measure-index={String(index)}
          className="pointer-events-none invisible absolute left-0 top-0 whitespace-nowrap tabular-nums"
        >
          {candidateValue}
        </span>
      ))}
      <span
        data-adaptive-metric-visible="true"
        className="block max-w-full overflow-hidden whitespace-nowrap"
      >
        {shouldAnimateDigits ? <AnimatedDigits value={visibleValue} /> : visibleValue}
      </span>
    </span>
  )
}
