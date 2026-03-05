import { cloneElement, useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { ReactElement, CSSProperties, MouseEvent as ReactMouseEvent } from 'react'
import ActivityCalendar, { type Activity } from 'react-activity-calendar'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesResponse } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { formatTokensShort } from '../lib/numberFormatters'
import { getBrowserTimeZone } from '../lib/timeZone'
import { calendarPalette, metricAccent } from '../lib/chartTheme'
import { cn } from '../lib/utils'
import { useTheme } from '../theme'
import { Alert } from './ui/alert'

type MetricKey = 'totalCount' | 'totalCost' | 'totalTokens'

interface MetricOption {
  key: MetricKey
  labelKey: TranslationKey
}

type AccessibleBlock = ReactElement<{
  title?: string
  'aria-label'?: string
  className?: string
  style?: CSSProperties
  onMouseEnter?: (event: ReactMouseEvent<SVGElement>) => void
  onMouseLeave?: (event: ReactMouseEvent<SVGElement>) => void
}>

const METRIC_OPTIONS: MetricOption[] = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

const WEEKDAY_LABELS: Array<'mon' | 'wed' | 'fri' | 'sun'> = ['mon', 'wed', 'fri', 'sun']
const MAX_LEVEL = 4
// Keep visual spacing consistent with WeeklyHourlyHeatmap (uses 3px gaps)
const BLOCK_MARGIN = 3
const DEFAULT_BLOCK_SIZE = 18
const MIN_BLOCK_SIZE = 8
const MAX_BLOCK_SIZE = 20
const WEEKDAY_LABEL_SPACE = 16

interface CalendarTooltipState {
  x: number
  y: number
  dateLabel: string
  valueLabel: string
}

export function UsageCalendar() {
  const { t, locale } = useTranslation()
  const { themeMode } = useTheme()
  const timeZone = getBrowserTimeZone()
  const [metric, setMetric] = useState<MetricKey>('totalCount')
  const { data, isLoading, error } = useTimeseries('90d', { bucket: '1d' })
  const skeletonMode = isLoading && !data
  const [blockSize, setBlockSize] = useState(DEFAULT_BLOCK_SIZE)
  const containerRef = useRef<HTMLDivElement>(null)
  const [tooltip, setTooltip] = useState<CalendarTooltipState | null>(null)
  // tabs width measurement removed (no longer needed for sizing)
  const [leftOffset, setLeftOffset] = useState(0) // svg.marginLeft introduced by weekday labels

  const legendLabels = useMemo(
    () => ({
      low: t('legend.low'),
      high: t('legend.high'),
    }),
    [t],
  )

  const weekdayLabels = useMemo(
    () => [
      t('calendar.weekday.sun'),
      t('calendar.weekday.mon'),
      t('calendar.weekday.tue'),
      t('calendar.weekday.wed'),
      t('calendar.weekday.thu'),
      t('calendar.weekday.fri'),
      t('calendar.weekday.sat'),
    ],
    [t],
  )

  const valueSeparator = t('calendar.valueSeparator')
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'

  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD' }), [localeTag])
  const countUnit = t('unit.calls')

  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const formatMetricValue = useCallback(
    (value: number) => {
      if (metric === 'totalCost') return currencyFormatter.format(value)
      if (metric === 'totalTokens') return formatTokensShort(value, localeTag)
      if (metric === 'totalCount') {
        const base = numberFormatter.format(value)
        return `${base} ${countUnit}`
      }
      return numberFormatter.format(value)
    },
    [countUnit, currencyFormatter, metric, numberFormatter, localeTag],
  )

  const calendarData = useMemo(
    () => transformTimeseriesToActivities(data, metric),
    [data, metric],
  )

  // Minimum width to keep blocks at least DEFAULT_BLOCK_SIZE so vertical size feels balanced
  const minContainerWidth = useMemo(() => {
    const cols = Math.max(1, calendarData.weekCount || 0)
    if (!cols) return undefined
    const offset = Math.max(leftOffset || WEEKDAY_LABEL_SPACE, WEEKDAY_LABEL_SPACE)
    // Width needed so that per-column block size is at least DEFAULT_BLOCK_SIZE
    // Derivation from sizing formula: A = L*(S + M) - M
    const required = cols * (DEFAULT_BLOCK_SIZE + BLOCK_MARGIN) - BLOCK_MARGIN
    return Math.ceil(offset + required)
  }, [calendarData.weekCount, leftOffset])

  const formatMonthLabel = useCallback(
    (marker: MonthMarker) => {
      const monthValue = locale === 'zh' ? marker.month.toString() : marker.month.toString().padStart(2, '0')
      return t('calendar.monthLabel', { year: marker.year, month: monthValue })
    },
    [locale, t],
  )

  useLayoutEffect(() => {
    if (!containerRef.current || calendarData.weekCount === 0) return
    const node = containerRef.current

    const computeByContainer = (width: number) => {
      if (!Number.isFinite(width) || width <= 0) return
      const GAP = 6
      const approxWidth = Math.max(0, width - leftOffset - GAP)
      const cols = Math.max(1, calendarData.weekCount)
      const candidate = Math.floor(((approxWidth + BLOCK_MARGIN) / cols) - BLOCK_MARGIN)
      const next = Math.max(MIN_BLOCK_SIZE, Math.min(MAX_BLOCK_SIZE, candidate))
      setBlockSize((prev) => (Math.abs(prev - next) >= 1 ? next : prev))
    }

    // initial and when deps change
    computeByContainer(node.getBoundingClientRect().width)
    // observe container size changes
    let raf = 0
    let lastWidth = node.getBoundingClientRect().width
    const schedule = (width: number) => {
      lastWidth = width
      if (raf) cancelAnimationFrame(raf)
      raf = requestAnimationFrame(() => computeByContainer(lastWidth))
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries.at(0)
      if (!entry) return
      schedule(entry.contentRect.width)
    })
    observer.observe(node)
    return () => { observer.disconnect(); if (raf) cancelAnimationFrame(raf) }
  }, [calendarData.weekCount, leftOffset])

  // Measure tabs width (kept for possible future responsive tweaks)
  // no-op: tab width no longer influences calendar sizing

  // Measure left offset from container to SVG (includes weekday label margin + centering gap)
  // Avoid periodic polling that could cause layout thrashing/jitter.
  useLayoutEffect(() => {
    const contEl = containerRef.current
    if (!contEl) return
    const queryAndSet = () => {
      const svg = contEl.querySelector('article svg') as SVGElement | null
      if (!svg) return
      const svgRect = svg.getBoundingClientRect()
      const contRect = contEl.getBoundingClientRect()
      const next = Math.max(0, Math.round(svgRect.left - contRect.left))
      setLeftOffset((prev) => (prev !== next ? next : prev))
    }
    // initial read and re-read once after paint
    queryAndSet()
    const ro = new ResizeObserver(queryAndSet)
    ro.observe(contEl)
    window.addEventListener('resize', queryAndSet)
    const raf = requestAnimationFrame(queryAndSet)
    const t = window.setTimeout(queryAndSet, 300)
    return () => {
      ro.disconnect()
      window.removeEventListener('resize', queryAndSet)
      cancelAnimationFrame(raf)
      window.clearTimeout(t)
    }
  }, [metric, calendarData.weekCount])

  const themeForMetric = useMemo(
    () => ({
      light: calendarPalette(metric, 'light'),
      dark: calendarPalette(metric, 'dark'),
    }),
    [metric],
  )

  return (
    <section
      className="surface-panel h-full w-full max-w-full overflow-visible lg:w-fit"
      data-testid="usage-calendar-card"
    >
      <div className="surface-panel-body gap-4 lg:w-auto">
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('calendar.title')}</h2>
              <p className="section-description">
                {t('calendar.timeZoneLabel')}{valueSeparator}{timeZone}
              </p>
            </div>
            <div className="segment-group" role="tablist" aria-label={t('calendar.metricsToggleAria')}>
              {metricOptions.map((option) => {
                const active = metric === option.key
                return (
                  <button
                    key={option.key}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    aria-current={active ? 'true' : undefined}
                    className={cn('segment-button px-2 sm:px-3', active && 'font-semibold')}
                    data-active={active}
                    style={active ? { color: metricAccent(option.key, themeMode) } : undefined}
                    onClick={() => setMetric(option.key)}
                  >
                    {option.label}
                  </button>
                )
              })}
            </div>
          </div>

          <div className="panel-divider my-1 opacity-40" />

          {error ? (
            <Alert variant="error">{error}</Alert>
          ) : (
            <div className="grid gap-3">
              <div className="min-w-0">
                <div
                  ref={containerRef}
                  className="relative flex w-full justify-center overflow-visible pt-4 [&>svg]:h-auto"
                  style={minContainerWidth ? { minWidth: `${minContainerWidth}px` } : undefined}
                  data-testid="usage-calendar-wrapper"
                >
                  <MonthLabelOverlay
                    markers={calendarData.monthMarkers}
                    blockSize={blockSize}
                    blockMargin={BLOCK_MARGIN}
                    offset={leftOffset || WEEKDAY_LABEL_SPACE}
                    formatLabel={formatMonthLabel}
                  />
                  <ActivityCalendar
                    data={calendarData.activities}
                    blockSize={blockSize}
                    // Match the subtle rounding used by the 7-day heatmap.
                    blockRadius={2}
                    blockMargin={BLOCK_MARGIN}
                    weekStart={1}
                    maxLevel={MAX_LEVEL}
                    theme={themeForMetric}
                    colorScheme={themeMode}
                    hideTotalCount
                    hideColorLegend
                    hideMonthLabels
                    labels={{ legend: { less: legendLabels.low, more: legendLabels.high }, weekdays: weekdayLabels }}
                    showWeekdayLabels={WEEKDAY_LABELS}
                    renderBlock={(block, activity) => {
                      const accessibleBlock = block as AccessibleBlock
                      const formatted = formatMetricValue(activity.count)
                      const title = `${activity.date}${valueSeparator}${formatted}`
                      const handleEnter = (event: ReactMouseEvent<SVGElement>) => {
                        if (!containerRef.current) return
                        const target = event.currentTarget as Element
                        const rect = target.getBoundingClientRect()
                        const containerRect = containerRef.current.getBoundingClientRect()
                        const centerXRaw = rect.left + rect.width / 2 - containerRect.left
                        const y = rect.top - containerRect.top
                        // Clamp the tooltip center so that even on the first/last column
                        // the bubble stays fully inside the card.
                        const margin = 80
                        const minCenter = margin
                        const maxCenter = Math.max(margin, containerRect.width - margin)
                        const x = Math.max(minCenter, Math.min(maxCenter, centerXRaw))
                        setTooltip({
                          x,
                          y,
                          dateLabel: activity.date,
                          valueLabel: formatted,
                        })
                      }
                      const handleLeave = () => setTooltip(null)
                      return cloneElement(accessibleBlock, {
                        title,
                        'aria-label': title,
                        onMouseEnter: skeletonMode ? undefined : handleEnter,
                        onMouseLeave: skeletonMode ? undefined : handleLeave,
                        className: cn(accessibleBlock.props?.className, skeletonMode && 'animate-pulse'),
                        // Remove default stroke from react-activity-calendar to align
                        // with the weekly heatmap appearance.
                        style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                      })
                    }}
                    renderColorLegend={(block, level) => {
                      const accessibleBlock = block as AccessibleBlock
                      if (level === 0)
                        return cloneElement(accessibleBlock, {
                          title: legendLabels.low,
                          'aria-label': legendLabels.low,
                          style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                        })
                      const threshold = calendarData.thresholds[level] ?? calendarData.maxValue
                      const formatted = formatMetricValue(threshold ?? 0)
                      const title = `≤ ${formatted}`
                      return cloneElement(accessibleBlock, {
                        title,
                        'aria-label': title,
                        style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                      })
                    }}
                  />
                  {tooltip && (
                    <div
                      className="pointer-events-none absolute z-30 -translate-x-1/2 whitespace-nowrap rounded-md bg-base-100 px-2 py-1 text-[11px] leading-tight text-base-content shadow-md sm:text-xs"
                      style={{ left: tooltip.x, top: tooltip.y - 8 }}
                    >
                      <div className="text-[10px] text-base-content/80 sm:text-xs">
                        {tooltip.dateLabel}
                      </div>
                      <div className="mt-0.5 text-center font-mono text-sm font-semibold tracking-tight sm:text-base">
                        {tooltip.valueLabel}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  )
}

interface CalendarComputation {
  activities: Activity[]
  maxValue: number
  totalValue: number
  thresholds: number[]
  weekCount: number
  monthMarkers: MonthMarker[]
}

function transformTimeseriesToActivities(response: TimeseriesResponse | null, metric: MetricKey): CalendarComputation {
  const points = response?.points ?? []
  const range = resolveLocalDateRange(response)

  const valuesByDate = new Map<string, number>()
  for (const point of points) {
    const iso = toLocalISODate(point.bucketStart)
    const current = valuesByDate.get(iso) ?? 0
    valuesByDate.set(iso, current + (point[metric] ?? 0))
  }

  const activities: Activity[] = []
  const values: number[] = []
  const cursor = new Date(range.start)
  while (cursor <= range.endInclusive) {
    const iso = formatLocalISODate(cursor)
    const value = valuesByDate.get(iso) ?? 0
    values.push(value)
    activities.push({ date: iso, count: value, level: 0 })
    cursor.setDate(cursor.getDate() + 1)
  }

  const maxValue = values.reduce((max, v) => (v > max ? v : max), 0)
  const thresholds = createThresholds(maxValue, MAX_LEVEL)
  const leveledActivities = activities.map((a) => ({ ...a, level: computeLevel(a.count, maxValue, MAX_LEVEL) }))
  const totalValue = values.reduce((acc, v) => acc + v, 0)
  const weekCount = Math.max(1, Math.ceil(activities.length / 7))
  const monthMarkers = createMonthMarkers(activities, weekCount)

  return { activities: leveledActivities, maxValue, totalValue, thresholds, weekCount, monthMarkers }
}

function computeLevel(value: number, maxValue: number, maxLevel: number) {
  if (maxValue <= 0 || value <= 0) return 0
  const ratio = value / maxValue
  return Math.max(1, Math.min(maxLevel, Math.ceil(ratio * maxLevel)))
}

function createThresholds(maxValue: number, maxLevel: number) {
  if (maxValue <= 0) return []
  const step = maxValue / maxLevel
  const thresholds: number[] = []
  for (let level = 1; level <= maxLevel; level += 1) {
    thresholds[level] = step * level
  }
  return thresholds
}

function formatLocalISODate(date: Date) {
  const pad = (num: number) => num.toString().padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

function toLocalISODate(value: string) {
  if (value.includes('T')) {
    // bucketStart/bucketEnd are RFC3339 UTC timestamps; convert to local day.
    return formatLocalISODate(new Date(value))
  }
  const [datePart] = value.split(' ')
  return datePart ?? ''
}

function resolveLocalDateRange(response: TimeseriesResponse | null) {
  // Daily time-series endpoints use [rangeStart, rangeEnd) where rangeEnd is the next local midnight.
  // We keep the UI stable by always rendering a 90-day calendar, even during the initial load.
  const today = startOfLocalDay(new Date())

  const rangeStart = response?.rangeStart
  const rangeEnd = response?.rangeEnd
  if (rangeStart && rangeEnd) {
    const start = startOfLocalDay(new Date(rangeStart))
    const endExclusive = startOfLocalDay(new Date(rangeEnd))
    if (Number.isFinite(start.getTime()) && Number.isFinite(endExclusive.getTime())) {
      const endInclusive = addLocalDays(endExclusive, -1)
      if (start <= endInclusive) {
        return { start, endInclusive }
      }
    }
  }

  const endInclusive = today
  const start = addLocalDays(endInclusive, -89)
  return { start, endInclusive }
}

function startOfLocalDay(date: Date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate())
}

function addLocalDays(date: Date, days: number) {
  const next = new Date(date)
  next.setDate(next.getDate() + days)
  return startOfLocalDay(next)
}

interface MonthMarker { weekIndex: number; year: number; month: number }

function createMonthMarkers(activities: Activity[], weekCount: number): MonthMarker[] {
  if (!activities.length || weekCount <= 0) return []
  const markers: MonthMarker[] = []
  let lastMonth = -1
  let lastYear = -1

  activities.forEach((activity, index) => {
    const [yearStr, monthStr, dayStr] = activity.date.split('-')
    const year = Number(yearStr)
    const month = Number(monthStr)
    const day = Number(dayStr)
    if (!Number.isFinite(year) || !Number.isFinite(month) || !Number.isFinite(day)) return
    if (day !== 1) return
    if (year === lastYear && month === lastMonth) return
    const weekIndex = Math.floor(index / 7)
    if (weekIndex >= weekCount) return
    markers.push({ weekIndex, year, month })
    lastYear = year
    lastMonth = month
  })

  return markers
}

function MonthLabelOverlay({
  markers,
  blockSize,
  blockMargin,
  offset,
  formatLabel,
}: {
  markers: MonthMarker[]
  blockSize: number
  blockMargin: number
  offset: number
  formatLabel: (marker: MonthMarker) => string
}) {
  if (!markers.length) return null
  return (
    <div className="pointer-events-none absolute left-0 right-0 top-0 h-6" aria-hidden>
      {markers.map((marker) => {
        const columnWidth = blockSize + blockMargin * 2
        const position = offset + marker.weekIndex * columnWidth + columnWidth / 2
        return (
          <span
            key={`${marker.year}-${marker.month}-${marker.weekIndex}`}
            className="absolute top-0 -translate-x-1/2 transform text-xs font-medium text-base-content/70 whitespace-nowrap"
            style={{ left: `${position}px` }}
          >
            {formatLabel(marker)}
          </span>
        )
      })}
    </div>
  )
}
