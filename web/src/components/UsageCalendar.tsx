import { cloneElement, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { ReactElement, CSSProperties } from 'react'
import ActivityCalendar, { type Activity } from 'react-activity-calendar'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'

type MetricKey = 'totalCount' | 'totalCost' | 'totalTokens'

interface MetricOption {
  key: MetricKey
  label: string
  formatter: (value: number) => string
}

type AccessibleBlock = ReactElement<{ title?: string; 'aria-label'?: string; style?: CSSProperties }>

const METRIC_OPTIONS: MetricOption[] = [
  { key: 'totalCount', label: '次数', formatter: (v) => v.toLocaleString() },
  { key: 'totalCost', label: '金额', formatter: (v) => `$${v.toFixed(2)}` },
  { key: 'totalTokens', label: 'Tokens', formatter: (v) => v.toLocaleString() },
]

const WEEKDAY_LABELS: Array<'mon' | 'wed' | 'fri' | 'sun'> = ['mon', 'wed', 'fri', 'sun']
const MAX_LEVEL = 4
// Keep visual spacing consistent with WeeklyHourlyHeatmap (uses 3px gaps)
const BLOCK_MARGIN = 3
const DEFAULT_BLOCK_SIZE = 18
const MIN_BLOCK_SIZE = 8
const MAX_BLOCK_SIZE = 20
const WEEKDAY_LABEL_SPACE = 16

// Use DaisyUI/Tailwind aligned palette so it visually matches
// the 7-day heatmap (WeeklyHourlyHeatmap.tsx).
// Level0 uses base-300 token rendered through color function; DaisyUI exports
// `--b3` as OKLCH components, so we must wrap it with `oklch(...)` (directly
// using `var(--b3)` would resolve to "black").
// Use explicit HEX for zero level to avoid SVG var()/oklch() incompatibilities
// in presentation attributes. This matches DaisyUI base-300 in light theme.
const BASE_ZERO = { light: '#E5E7EB', dark: '#374151' }
const THEME_BY_METRIC: Record<MetricKey, { light: string[]; dark: string[] }> = {
  // 次数：蓝色系（200..500）
  totalCount: {
    light: [BASE_ZERO.light, '#BFDBFE', '#93C5FD', '#60A5FA', '#3B82F6'],
    dark: [BASE_ZERO.dark, '#93C5FD', '#60A5FA', '#3B82F6', '#1D4ED8'],
  },
  // 金额：琥珀/橙色系（200..500）
  totalCost: {
    light: [BASE_ZERO.light, '#FDE68A', '#FCD34D', '#F59E0B', '#D97706'],
    dark: [BASE_ZERO.dark, '#FCD34D', '#F59E0B', '#D97706', '#B45309'],
  },
  // Tokens：紫色系（200..500）
  totalTokens: {
    light: [BASE_ZERO.light, '#DDD6FE', '#C4B5FD', '#A78BFA', '#8B5CF6'],
    dark: [BASE_ZERO.dark, '#C4B5FD', '#A78BFA', '#8B5CF6', '#7C3AED'],
  },
}

const ACCENT_BY_METRIC: Record<MetricKey, string> = {
  totalCount: '#3B82F6',
  totalCost: '#F59E0B',
  totalTokens: '#8B5CF6',
}

export function UsageCalendar() {
  const [metric, setMetric] = useState<MetricKey>('totalCount')
  const { data, isLoading, error } = useTimeseries('90d', { bucket: '1d' })
  const [blockSize, setBlockSize] = useState(DEFAULT_BLOCK_SIZE)
  const containerRef = useRef<HTMLDivElement>(null)
  const tabsRef = useRef<HTMLDivElement>(null)
  const [tabsWidth, setTabsWidth] = useState(128)
  const [leftOffset, setLeftOffset] = useState(0) // svg.marginLeft introduced by weekday labels
  const colorProbeRef = useRef<HTMLSpanElement>(null)
  const [baseZeroColor, setBaseZeroColor] = useState<string>(BASE_ZERO.light)

  const metricDefinition = useMemo(
    () => METRIC_OPTIONS.find((o) => o.key === metric) ?? METRIC_OPTIONS[0],
    [metric],
  )

  const calendarData = useMemo(
    () => transformPointsToActivities(data?.points ?? [], metric),
    [data, metric],
  )

  // Probe actual theme color for base-300 to match 7-day heatmap exactly
  useLayoutEffect(() => {
    const el = colorProbeRef.current
    if (!el) return
    const read = () => {
      const bg = getComputedStyle(el).backgroundColor
      if (bg && bg !== baseZeroColor) setBaseZeroColor(bg)
    }
    read()
    const ro = new ResizeObserver(read)
    ro.observe(el)
    return () => ro.disconnect()
  }, [baseZeroColor])

  useLayoutEffect(() => {
    if (!containerRef.current || calendarData.weekCount === 0) return
    const node = containerRef.current

    const computeByContainer = (width: number) => {
      if (!Number.isFinite(width) || width <= 0) {
        setBlockSize(DEFAULT_BLOCK_SIZE)
        return
      }
      // First pass: approximate by container width minus tabs width, weekday label offset and a small gap.
      const GAP = 6
      const approxWidth = Math.max(0, width - leftOffset - GAP)
      const cols = Math.max(1, calendarData.weekCount)
      const candidate = Math.floor(((approxWidth + BLOCK_MARGIN) / cols) - BLOCK_MARGIN)
      const next = Math.max(MIN_BLOCK_SIZE, Math.min(MAX_BLOCK_SIZE, candidate))
      setBlockSize((prev) => {
        const base = next || DEFAULT_BLOCK_SIZE
        return prev ? Math.min(prev, base) : base
      })
    }

    // Second pass: refine using actual rendered width of the content box.
    const refineByMeasurements = () => {
      const svg = node.querySelector('article svg') as SVGElement | null
      if (!svg) return
      const styles = getComputedStyle(node)
      const paddingLeft = parseFloat(styles.paddingLeft || '0')
      const paddingRight = parseFloat(styles.paddingRight || '0')
      const contentWidth = Math.max(0, node.clientWidth - paddingLeft - paddingRight)
      const desiredGap = 0
      const available = Math.max(0, Math.floor(contentWidth - leftOffset - desiredGap))
      const gList = Array.from(svg.querySelectorAll('g'))
      const measuredCols = gList.filter(g => (g as SVGGElement).children.length === 7).length
      const cols = Math.max(1, measuredCols || calendarData.weekCount)
      const candidate = Math.floor(((available + BLOCK_MARGIN) / cols) - BLOCK_MARGIN)
      const clamped = Math.max(MIN_BLOCK_SIZE, Math.min(MAX_BLOCK_SIZE, candidate))
      if (Number.isFinite(clamped) && clamped > 0) {
        setBlockSize((prev) => (Math.abs(prev - clamped) >= 1 ? clamped : prev))
      }
    }

    // initial and when deps change
    computeByContainer(node.getBoundingClientRect().width)
    // observe container size changes
    const observer = new ResizeObserver((entries) => {
      const entry = entries.at(0)
      if (!entry) return
      computeByContainer(entry.contentRect.width)
      // defer refine to the next frame to ensure the SVG updated
      requestAnimationFrame(refineByMeasurements)
    })
    observer.observe(node)
    // also refine shortly after mount/change to catch initial SVG paint
    const raf = requestAnimationFrame(refineByMeasurements)
    const t = window.setTimeout(refineByMeasurements, 120)
    return () => { observer.disconnect(); cancelAnimationFrame(raf); window.clearTimeout(t) }
  }, [calendarData.weekCount, tabsWidth, leftOffset])

  // Measure tabs width (kept for possible future responsive tweaks)
  useLayoutEffect(() => {
    const tabsEl = tabsRef.current
    const contEl = containerRef.current
    if (!tabsEl || !contEl) return
    const update = () => {
      const w = Math.ceil(tabsEl.getBoundingClientRect().width)
      setTabsWidth(Math.max(0, w))
    }
    update()
    const ro1 = new ResizeObserver(update)
    const ro2 = new ResizeObserver(update)
    ro1.observe(tabsEl)
    ro2.observe(contEl)
    window.addEventListener('resize', update)
    return () => {
      ro1.disconnect()
      ro2.disconnect()
      window.removeEventListener('resize', update)
    }
  }, [metric])

  // Measure svg margin-left (weekday label offset) and track changes
  useLayoutEffect(() => {
    const contEl = containerRef.current
    if (!contEl) return
    const queryAndSet = () => {
      const svg = contEl.querySelector('svg') as SVGElement | null
      if (!svg) return
      const ml = parseFloat(getComputedStyle(svg).marginLeft || '0')
      if (Number.isFinite(ml)) setLeftOffset(Math.max(0, Math.round(ml)))
    }
    // initial read and on resize
    queryAndSet()
    const ro = new ResizeObserver(queryAndSet)
    ro.observe(contEl)
    window.addEventListener('resize', queryAndSet)
    // guard for first render of ActivityCalendar
    const id = window.setInterval(queryAndSet, 600)
    return () => { ro.disconnect(); window.removeEventListener('resize', queryAndSet); window.clearInterval(id) }
  }, [metric, calendarData.weekCount])

  const calendarLoading = isLoading || calendarData.activities.length === 0

  // Build theme palette with runtime-resolved zero level color
  const themeForMetric = useMemo(() => {
    const base = THEME_BY_METRIC[metric]
    return {
      light: [baseZeroColor, ...base.light.slice(1)],
      dark: [baseZeroColor, ...base.dark.slice(1)],
    }
  }, [metric, baseZeroColor])

  return (
    <section
      className="card h-full w-full max-w-full overflow-hidden bg-base-100 shadow-sm lg:w-fit lg:max-w-none"
      data-testid="usage-calendar-card"
    >
      <div className="card-body gap-4 lg:w-auto">
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-3">
            <h2 className="card-title">使用活动</h2>
            <div
              ref={tabsRef}
              className="tabs tabs-sm tabs-border"
              role="tablist"
              aria-label="统计指标切换"
            >
              {METRIC_OPTIONS.map((option) => {
                const active = metric === option.key
                return (
                  <button
                    key={option.key}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    aria-current={active ? 'true' : undefined}
                    className={`tab whitespace-nowrap px-2 sm:px-3 ${
                      active ? 'tab-active text-primary font-medium' : 'text-base-content/70 hover:text-base-content'
                    }`}
                    style={active ? { color: ACCENT_BY_METRIC[option.key] } : undefined}
                    onClick={() => setMetric(option.key)}
                  >
                    {option.label}
                  </button>
                )
              })}
            </div>
          </div>

          <div className="divider my-1 opacity-40" />

          {error ? (
            <div className="alert alert-error">{error}</div>
          ) : (
          <div className="grid gap-3">
            <div className="min-w-0">
              <div
                ref={containerRef}
                className="relative inline-block w-full overflow-hidden pt-4 [&>svg]:h-auto [&>svg]:w-full lg:w-fit"
                data-testid="usage-calendar-wrapper"
              >
                {/* Theme color probe to fetch exact bg-base-300 value */}
                <span ref={colorProbeRef} className="invisible absolute h-0 w-0 bg-base-300" aria-hidden />
                <MonthLabelOverlay
                  markers={calendarData.monthMarkers}
                  blockSize={blockSize}
                  blockMargin={BLOCK_MARGIN}
                  offset={leftOffset || WEEKDAY_LABEL_SPACE}
                />
                <ActivityCalendar
                  data={calendarData.activities}
                  loading={calendarLoading}
                  blockSize={blockSize}
                  // Match the subtle rounding used by the 7-day heatmap
                  blockRadius={2}
                  blockMargin={BLOCK_MARGIN}
                  weekStart={1}
                  maxLevel={MAX_LEVEL}
                  theme={themeForMetric}
                  colorScheme="light"
                  hideTotalCount
                  hideColorLegend
                  hideMonthLabels
                  labels={{ legend: { less: '低', more: '高' }, weekdays: ['日', '一', '二', '三', '四', '五', '六'] }}
                  showWeekdayLabels={WEEKDAY_LABELS}
                  renderBlock={(block, activity) => {
                    const accessibleBlock = block as AccessibleBlock
                    const formatted = metricDefinition.formatter(activity.count)
                    const title = `${activity.date}：${formatted}`
                    return cloneElement(accessibleBlock, {
                      title,
                      'aria-label': title,
                      // Remove default stroke from react-activity-calendar to
                      // match WeeklyHourlyHeatmap appearance exactly
                      style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                    })
                  }}
                  renderColorLegend={(block, level) => {
                    const accessibleBlock = block as AccessibleBlock
                    if (level === 0)
                      return cloneElement(accessibleBlock, {
                        title: '低',
                        'aria-label': '低',
                        style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                      })
                    const threshold = calendarData.thresholds[level] ?? calendarData.maxValue
                    const formatted = metricDefinition.formatter(threshold)
                    const title = `≤ ${formatted}`
                    return cloneElement(accessibleBlock, {
                      title,
                      'aria-label': title,
                      style: { ...(accessibleBlock.props?.style ?? {}), stroke: 'none', strokeWidth: 0 },
                    })
                  }}
                />
              </div>
            </div>

            {/* tabs moved to header */}
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

function transformPointsToActivities(points: TimeseriesPoint[], metric: MetricKey): CalendarComputation {
  if (!points || points.length === 0) {
    return { activities: [], maxValue: 0, totalValue: 0, thresholds: [], weekCount: 0, monthMarkers: [] }
  }

  const sortedPoints = [...points].sort((a, b) => parseNaiveDate(a.bucketStart) - parseNaiveDate(b.bucketStart))

  const valuesByDate = new Map<string, number>()
  for (const point of sortedPoints) {
    const iso = toISODate(point.bucketStart)
    const current = valuesByDate.get(iso) ?? 0
    valuesByDate.set(iso, current + (point[metric] ?? 0))
  }

  const startDate = new Date(parseNaiveDate(sortedPoints[0].bucketStart))
  const endDate = new Date(parseNaiveDate(sortedPoints[sortedPoints.length - 1].bucketStart))

  const activities: Activity[] = []
  const values: number[] = []
  const cursor = new Date(startDate)
  while (cursor <= endDate) {
    const iso = formatISODate(cursor)
    const value = valuesByDate.get(iso) ?? 0
    values.push(value)
    activities.push({ date: iso, count: value, level: 0 })
    cursor.setUTCDate(cursor.getUTCDate() + 1)
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

function parseNaiveDate(value: string) {
  const [datePart] = value.split(' ')
  if (!datePart) return 0
  const [year, month, day] = datePart.split('-').map(Number)
  const date = Date.UTC(year, (month ?? 1) - 1, day ?? 1)
  return date
}

function toISODate(value: string) {
  const [datePart] = value.split(' ')
  return datePart ?? ''
}

function formatISODate(date: Date) {
  const pad = (num: number) => num.toString().padStart(2, '0')
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`
}

interface MonthMarker { weekIndex: number; label: string }

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
    markers.push({ weekIndex, label: `${year}年${month}月` })
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
}: {
  markers: MonthMarker[]
  blockSize: number
  blockMargin: number
  offset: number
}) {
  if (!markers.length) return null
  return (
    <div className="pointer-events-none absolute left-0 right-0 top-0 h-6" aria-hidden>
      {markers.map((marker) => {
        const columnWidth = blockSize + blockMargin * 2
        const position = offset + marker.weekIndex * columnWidth + columnWidth / 2
        return (
          <span
            key={`${marker.label}-${marker.weekIndex}`}
            className="absolute top-0 -translate-x-1/2 transform text-xs font-medium text-base-content/70 whitespace-nowrap"
            style={{ left: `${position}px` }}
          >
            {marker.label}
          </span>
        )
      })}
    </div>
  )
}
