import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { AnimatedDigits } from './AnimatedDigits'

export type AdaptiveMetricValueKind = 'number' | 'currency'

const ADAPTIVE_METRIC_COMPACT_GUTTER_PX = 12
const COMPACT_SUFFIX_LOCALE = 'en-US'

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
  compact: boolean,
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
      maximumFractionDigits: 2,
    })
  }

  return new Intl.NumberFormat(compactLocale, {
    notation: compact ? 'compact' : 'standard',
    maximumFractionDigits: 2,
  })
}

export function AdaptiveMetricValue({
  value,
  localeTag,
  kind = 'number',
  className,
  'data-testid': dataTestId,
}: AdaptiveMetricValueProps) {
  const containerRef = useRef<HTMLSpanElement | null>(null)
  const measureRef = useRef<HTMLSpanElement | null>(null)
  const [useCompactValue, setUseCompactValue] = useState(false)

  const fullValue = useMemo(
    () => createMetricFormatter(localeTag, kind, false).format(value),
    [kind, localeTag, value],
  )
  const compactValue = useMemo(
    () => createMetricFormatter(localeTag, kind, true).format(value),
    [kind, localeTag, value],
  )

  const evaluateOverflow = useCallback(() => {
    const container = containerRef.current
    const measure = measureRef.current
    if (!container || !measure) return

    const availableWidth = container.clientWidth
    const requiredWidth = measure.scrollWidth
    if (availableWidth <= 0 || requiredWidth <= 0) return

    const nextUseCompactValue =
      requiredWidth + ADAPTIVE_METRIC_COMPACT_GUTTER_PX > availableWidth
    setUseCompactValue((current) =>
      current === nextUseCompactValue ? current : nextUseCompactValue,
    )
  }, [])

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
    const measure = measureRef.current
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
    if (measure) {
      observer.observe(measure)
    }

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', evaluateOverflow)
    }
  }, [evaluateOverflow])

  const visibleValue = useCompactValue ? compactValue : fullValue
  const shouldAnimateDigits = kind === 'number' && !useCompactValue

  return (
    <span
      ref={containerRef}
      data-adaptive-metric-container="true"
      data-compact={useCompactValue ? 'true' : 'false'}
      data-testid={dataTestId}
      title={useCompactValue ? fullValue : undefined}
      className={`relative block min-w-0 max-w-full overflow-hidden whitespace-nowrap tabular-nums ${className ?? ''}`}
    >
      <span
        ref={measureRef}
        aria-hidden
        data-adaptive-metric-measure="true"
        className="pointer-events-none invisible absolute left-0 top-0 whitespace-nowrap tabular-nums"
      >
        {fullValue}
      </span>
      <span className="block max-w-full overflow-hidden whitespace-nowrap">
        {shouldAnimateDigits ? <AnimatedDigits value={visibleValue} /> : visibleValue}
      </span>
    </span>
  )
}
