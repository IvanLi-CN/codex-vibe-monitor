import { useMemo, useState } from 'react'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { formatTokensShort } from '../lib/numberFormatters'

export type MetricKey = 'totalCount' | 'totalCost' | 'totalTokens'

interface MetricOption {
  key: MetricKey
  labelKey: TranslationKey
}

const METRIC_OPTIONS: MetricOption[] = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

// Match palettes used by WeeklyHourlyHeatmap for consistency
const LEVEL_COLORS_BY_METRIC: Record<MetricKey, string[]> = {
  totalCount: ['bg-base-300', 'bg-blue-200', 'bg-blue-300', 'bg-blue-400', 'bg-blue-500'],
  totalCost: ['bg-base-300', 'bg-amber-200', 'bg-amber-300', 'bg-amber-400', 'bg-amber-500'],
  totalTokens: ['bg-base-300', 'bg-violet-200', 'bg-violet-300', 'bg-violet-400', 'bg-violet-500'],
}

// eslint-disable-next-line react-refresh/only-export-components
export const ACCENT_BY_METRIC: Record<MetricKey, string> = {
  totalCount: '#3B82F6',
  totalCost: '#F59E0B',
  totalTokens: '#8B5CF6',
}

const COLUMN_COUNT = 25
const HOUR_MS = 3_600_000
const SLOT_MINUTES = 10
const SLOT_MS = SLOT_MINUTES * 60 * 1000

type Cell = { columnStart: number; columnEnd: number; slot: number; value: number }
type Column = { start: number; end: number }

function parseLocalParts(value: string) {
  if (value.includes('T')) {
    const d = new Date(value)
    return {
      y: d.getFullYear(),
      m: d.getMonth() + 1,
      d: d.getDate(),
      h: d.getHours(),
      min: d.getMinutes(),
    }
  }
  const [datePart, timePart] = value.split(' ')
  const [y, m, d] = (datePart ?? '').split('-').map(Number)
  const [h, min] = (timePart ?? '').split(':').map(Number)
  return { y, m, d, h: Number.isFinite(h) ? h : 0, min: Number.isFinite(min) ? min : 0 }
}

function compute24h6(points: TimeseriesPoint[] | undefined, metric: MetricKey) {
  const now = Date.now()
  const endAnchor = Math.ceil(now / HOUR_MS) * HOUR_MS
  const columnCount = COLUMN_COUNT
  const startAnchor = endAnchor - columnCount * HOUR_MS
  const columns: Column[] = Array.from({ length: columnCount }, (_, idx) => {
    const start = startAnchor + idx * HOUR_MS
    return { start, end: start + HOUR_MS }
  })

  // Always initialize a fixed grid (6 slots x `columnCount` hours)
  const rows: Cell[][] = Array.from({ length: 6 }, (_, slot) =>
    columns.map((col) => ({ columnStart: col.start, columnEnd: col.end, slot, value: 0 })),
  )

  let max = 0
  let hasData = false
  for (const p of points ?? []) {
    const bucketStartMs = Date.parse(p.bucketStart)
    if (Number.isNaN(bucketStartMs)) continue
    if (bucketStartMs < startAnchor || bucketStartMs >= endAnchor) continue
    const columnIndex = Math.floor((bucketStartMs - startAnchor) / HOUR_MS)
    if (columnIndex < 0 || columnIndex >= columns.length) continue
    const { min } = parseLocalParts(p.bucketStart)
    if (!Number.isFinite(min)) continue
    const slot = Math.floor(Math.max(0, Math.min(59, min)) / SLOT_MINUTES)
    const val = metric === 'totalCount' ? p.totalCount ?? 0 : metric === 'totalCost' ? p.totalCost ?? 0 : p.totalTokens ?? 0
    const cell = rows[slot][columnIndex]
    cell.value += val
    if (cell.value > max) max = cell.value
    hasData = true
  }

  return { rows, max, columns, hasData }
}

function levelFor(value: number, max: number) {
  if (max <= 0 || value <= 0) return 0
  const ratio = value / max
  if (ratio >= 0.85) return 4
  if (ratio >= 0.55) return 3
  if (ratio >= 0.25) return 2
  return 1
}

export interface Last24hTenMinuteHeatmapProps {
  metric?: MetricKey
  onChangeMetric?: (m: MetricKey) => void
  showHeader?: boolean
}

export function Last24hTenMinuteHeatmap({ metric: controlledMetric, onChangeMetric, showHeader = true }: Last24hTenMinuteHeatmapProps) {
  const { t, locale } = useTranslation()
  const [uncontrolledMetric, setUncontrolledMetric] = useState<MetricKey>('totalCount')
  const metric = controlledMetric ?? uncontrolledMetric
  // Force 1-day range with 1-minute buckets, aggregate to 10-minute cells
  const { data, isLoading, error } = useTimeseries('1d', { bucket: '1m' })

  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD' }),
    [localeTag],
  )
  const countUnit = t('unit.calls')

  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((o) => ({ ...o, label: t(o.labelKey) })),
    [t],
  )

  const grid = useMemo(() => compute24h6(data?.points ?? [], metric), [data?.points, metric])

  const formatValue = (value: number) => {
    if (metric === 'totalCost') return currencyFormatter.format(value)
    if (metric === 'totalTokens') return formatTokensShort(value, localeTag)
    if (metric === 'totalCount') {
      const base = numberFormatter.format(value)
      return `${base} ${countUnit}`
    }
    return numberFormatter.format(value)
  }

  const noDataText = t('heatmap.noData')

  const setMetric = (m: MetricKey) => {
    if (onChangeMetric) onChangeMetric(m)
    else setUncontrolledMetric(m)
  }

  const minuteSlotLabel = (slot: number) => String(slot * 10).padStart(2, '0')

  return (
    <div data-testid="last24h-10m-heatmap">
      <div className="gap-4">
        {showHeader && (
          <div className="flex items-center justify-between gap-3">
            <div className="card-heading">
              <h3 className="card-title">{t('heatmap24h.title')}</h3>
            </div>
            <div className="tabs tabs-sm tabs-border" role="tablist" aria-label={t('heatmap.metricsToggleAria')}>
              {metricOptions.map((o) => {
                const active = o.key === metric
                return (
                  <button
                    key={o.key}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    className={`tab whitespace-nowrap px-2 sm:px-3 ${
                      active ? 'tab-active text-primary font-medium' : 'text-base-content/70 hover:text-base-content'
                    }`}
                    style={active ? { color: ACCENT_BY_METRIC[o.key] } : undefined}
                    onClick={() => setMetric(o.key)}
                  >
                    {o.label}
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {error ? (
          <div className="alert alert-error">{error}</div>
        ) : grid.hasData ? (
          <div className="w-full overflow-x-auto no-scrollbar">
            <div className="flex justify-center">
              <div className="inline-block">
                {/* Hour labels */}
                <div
                  className="ml-14 grid gap-[3px] pl-[3px]"
                  style={{ gridTemplateColumns: `repeat(${grid.columns.length}, minmax(0, 1fr))` }}
                >
                  {grid.columns.map((col, idx) => {
                    const colDate = new Date(col.start)
                    const hour = String(colDate.getHours()).padStart(2, '0')
                    return (
                      <div key={`h-${idx}`} className="text-center text-[10px] leading-3 text-base-content/60">
                        {hour}
                      </div>
                    )
                  })}
                </div>

                {/* Grid 6 rows (10-minute slots), rolling columns */}
                <div className="mt-2 flex flex-col gap-[3px]">
                  {grid.rows.map((row, slotIdx) => {
                    const label = `${minuteSlotLabel(slotIdx)}m`
                    // Treat the last two rows as bottom edge to avoid clipping by card border
                    const isBottomBand = slotIdx >= grid.rows.length - 2
                    return (
                      <div key={`r-${slotIdx}`} className="flex items-center gap-3">
                        <div className="w-14 shrink-0 text-right text-xs text-base-content/70">{label}</div>
                        <div
                          className="grid gap-[3px]"
                          style={{ gridTemplateColumns: `repeat(${grid.columns.length}, minmax(0, 1fr))` }}
                        >
                          {row.map((cell, ci) => {
                            const lvl = levelFor(cell.value, grid.max)
                            const palette = LEVEL_COLORS_BY_METRIC[metric]
                            const cls = palette[lvl] ?? palette[0]
                            const formatted = formatValue(cell.value)
                            const slotStart = cell.columnStart + cell.slot * SLOT_MS
                            const slotEnd = Math.min(cell.columnEnd, slotStart + SLOT_MS - 1)
                            const startLabel = new Date(slotStart).toLocaleString(localeTag, {
                              month: '2-digit',
                              day: '2-digit',
                              hour: '2-digit',
                              minute: '2-digit',
                            })
                            const endLabel = new Date(slotEnd).toLocaleString(localeTag, {
                              hour: '2-digit',
                              minute: '2-digit',
                            })
                            const rangeLabel = `${startLabel} - ${endLabel}`
                            const title = `${rangeLabel} ${formatted}`
                            const verticalClass = isBottomBand ? 'bottom-full mb-1' : 'top-full mt-1'
                            return (
                              <div
                                key={`c-${slotIdx}-${ci}`}
                                className="group relative"
                                aria-label={title}
                                title={title}
                              >
                                <div className={`${cls} h-5 w-5 sm:h-6 sm:w-6 rounded-sm`} />
                                <div
                                  className={`pointer-events-none absolute left-1/2 z-20 -translate-x-1/2 whitespace-nowrap rounded-md bg-base-100 px-2 py-1 text-[11px] sm:text-xs leading-tight text-base-content shadow-md opacity-0 group-hover:opacity-100 ${verticalClass}`}
                                >
                                  <div className="text-[10px] sm:text-xs text-base-content/80">{rangeLabel}</div>
                                  <div className="mt-0.5 font-mono font-semibold text-sm sm:text-base tracking-tight text-center">
                                    {formatted}
                                  </div>
                                </div>
                              </div>
                            )
                          })}
                        </div>
                      </div>
                    )
                  })}
                </div>
              </div>
            </div>
          </div>
        ) : isLoading ? (
          <div className="skeleton h-40 w-full" />
        ) : (
          <div className="text-base-content/70">{noDataText}</div>
        )}
      </div>
    </div>
  )
}

export default Last24hTenMinuteHeatmap
