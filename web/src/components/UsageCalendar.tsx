import { cloneElement, useLayoutEffect, useMemo, useRef, useState } from 'react'
import ActivityCalendar, { type Activity } from 'react-activity-calendar'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'

type MetricKey = 'totalCount' | 'totalCost' | 'totalTokens'

interface MetricOption {
  key: MetricKey
  label: string
  formatter: (value: number) => string
}

const METRIC_OPTIONS: MetricOption[] = [
  { key: 'totalCount', label: '次数', formatter: (v) => v.toLocaleString() },
  { key: 'totalCost', label: '金额', formatter: (v) => `$${v.toFixed(2)}` },
  { key: 'totalTokens', label: 'Tokens', formatter: (v) => v.toLocaleString() },
]

const WEEKDAY_LABELS: Array<'mon' | 'wed' | 'fri' | 'sun'> = ['mon', 'wed', 'fri', 'sun']
const MAX_LEVEL = 4
const BLOCK_MARGIN = 2
const DEFAULT_BLOCK_SIZE = 18
const MIN_BLOCK_SIZE = 8
const MAX_BLOCK_SIZE = 20
const WEEKDAY_LABEL_SPACE = 16

const CALENDAR_THEME: { light: string[]; dark: string[] } = {
  light: ['#E2E8F0', '#C7D2FE', '#93C5FD', '#60A5FA', '#2563EB'],
  dark: ['#1F2937', '#4338CA', '#2563EB', '#1D4ED8', '#1E40AF'],
}

export function UsageCalendar() {
  const [metric, setMetric] = useState<MetricKey>('totalCount')
  const { data, isLoading, error } = useTimeseries('90d', { bucket: '1d' })
  const [blockSize, setBlockSize] = useState(DEFAULT_BLOCK_SIZE)
  const containerRef = useRef<HTMLDivElement>(null)

  const metricDefinition = useMemo(
    () => METRIC_OPTIONS.find((o) => o.key === metric) ?? METRIC_OPTIONS[0],
    [metric],
  )

  const calendarData = useMemo(
    () => transformPointsToActivities(data?.points ?? [], metric),
    [data, metric],
  )

  useLayoutEffect(() => {
    if (!containerRef.current || calendarData.weekCount === 0) return
    const node = containerRef.current

    const updateSize = (width: number) => {
      if (!Number.isFinite(width) || width <= 0) {
        setBlockSize(DEFAULT_BLOCK_SIZE)
        return
      }
      const effectiveWidth = Math.max(0, width - WEEKDAY_LABEL_SPACE)
      const columns = Math.max(1, calendarData.weekCount)
      const candidate = Math.floor(effectiveWidth / columns - BLOCK_MARGIN * 2)
      const next = Math.max(MIN_BLOCK_SIZE, Math.min(MAX_BLOCK_SIZE, candidate))
      setBlockSize(next || DEFAULT_BLOCK_SIZE)
    }

    updateSize(node.getBoundingClientRect().width)
    const observer = new ResizeObserver((entries) => {
      const entry = entries.at(0)
      if (!entry) return
      updateSize(entry.contentRect.width)
    })
    observer.observe(node)
    return () => observer.disconnect()
  }, [calendarData.weekCount])

  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  const calendarLoading = isLoading || calendarData.activities.length === 0

  return (
    <section className="card h-full w-full max-w-full overflow-hidden bg-base-100 shadow-sm lg:w-fit lg:max-w-none">
      <div className="card-body gap-4 lg:w-auto">
        <div className="flex flex-col gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <h2 className="card-title">使用活动</h2>
          </div>

          <div className="flex flex-col gap-4 md:flex-row md:items-start md:gap-6">
            <div className="order-1 md:order-1 md:flex-1 md:min-w-0">
              <div
                ref={containerRef}
                className="relative w-full overflow-hidden pt-4 [&>svg]:h-auto [&>svg]:w-full lg:w-fit"
              >
                <MonthLabelOverlay
                  markers={calendarData.monthMarkers}
                  blockSize={blockSize}
                  blockMargin={BLOCK_MARGIN}
                  offset={WEEKDAY_LABEL_SPACE}
                />
                <ActivityCalendar
                  data={calendarData.activities}
                  loading={calendarLoading}
                  blockSize={blockSize}
                  blockRadius={4}
                  blockMargin={BLOCK_MARGIN}
                  weekStart={1}
                  maxLevel={MAX_LEVEL}
                  theme={CALENDAR_THEME}
                  colorScheme="light"
                  style={{ width: '100%', height: 'auto' }}
                  hideTotalCount
                  hideColorLegend
                  hideMonthLabels
                  labels={{ legend: { less: '低', more: '高' }, weekdays: ['日', '一', '二', '三', '四', '五', '六'] }}
                  showWeekdayLabels={WEEKDAY_LABELS}
                  renderBlock={(block, activity) => {
                    const formatted = metricDefinition.formatter(activity.count)
                    const title = `${activity.date}：${formatted}`
                    return cloneElement(block, { title, 'aria-label': title })
                  }}
                  renderColorLegend={(block, level) => {
                    if (level === 0) return cloneElement(block, { title: '低', 'aria-label': '低' })
                    const threshold = calendarData.thresholds[level] ?? calendarData.maxValue
                    const formatted = metricDefinition.formatter(threshold)
                    const title = `≤ ${formatted}`
                    return cloneElement(block, { title, 'aria-label': title })
                  }}
                />
              </div>
            </div>

            <div className="order-2 flex flex-col items-end gap-2 md:order-2 md:ml-auto md:items-end">
              <span className="text-xs font-medium text-base-content/60">指标切换</span>
              <div className="tabs tabs-lifted tabs-sm tabs-vertical self-end gap-1 w-28" role="tablist" aria-label="统计指标切换">
                {METRIC_OPTIONS.map((option) => (
                  <button
                    key={option.key}
                    type="button"
                    role="tab"
                    aria-selected={metric === option.key}
                    className={`tab tab-sm text-xs whitespace-nowrap cursor-pointer ${
                      metric === option.key ? 'tab-active' : 'opacity-80 hover:opacity-100'
                    }`}
                    onClick={() => setMetric(option.key)}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </div>
          </div>
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
