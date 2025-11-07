import { useMemo, useState } from 'react'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'

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

type Cell = { hour: number; slot: number; value: number }

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
  // Always initialize a 24x6 grid to keep layout stable even without data
  const rows: Cell[][] = Array.from({ length: 6 }, (_, slot) =>
    Array.from({ length: 24 }, (_, h) => ({ hour: h, slot, value: 0 })),
  )

  let max = 0
  for (const p of points ?? []) {
    const { h, min } = parseLocalParts(p.bucketStart)
    if (!Number.isFinite(h) || !Number.isFinite(min)) continue
    const slot = Math.floor(Math.max(0, Math.min(59, min)) / 10) // 0..5
    const val = metric === 'totalCount' ? p.totalCount ?? 0 : metric === 'totalCost' ? p.totalCost ?? 0 : p.totalTokens ?? 0
    const cell = rows[slot][h]
    cell.value += val
    if (cell.value > max) max = cell.value
  }

  return { rows, max }
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

  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((o) => ({ ...o, label: t(o.labelKey) })),
    [t],
  )

  const grid = useMemo(() => compute24h6(data?.points ?? [], metric), [data?.points, metric])

  const formatValue = (value: number) => (metric === 'totalCost' ? currencyFormatter.format(value) : numberFormatter.format(value))

  const noDataText = t('heatmap.noData')

  const setMetric = (m: MetricKey) => {
    if (onChangeMetric) onChangeMetric(m)
    else setUncontrolledMetric(m)
  }

  const minuteSlotLabel = (slot: number) => String(slot * 10).padStart(2, '0')

  return (
    <section className="card bg-base-100 shadow-sm" data-testid="last24h-10m-heatmap">
      <div className="card-body gap-4">
        {showHeader && (
          <div className="flex items-center justify-between gap-3">
            <h3 className="card-title">{t('heatmap24h.title')}</h3>
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
        ) : grid.rows.length > 0 ? (
          <div className="w-full overflow-x-auto">
            <div className="flex justify-center">
              <div className="inline-block">
                {/* Hour labels */}
                <div className="ml-14 grid gap-[3px] pl-[3px]" style={{ gridTemplateColumns: 'repeat(24, minmax(0, 1fr))' }}>
                  {Array.from({ length: 24 }, (_, h) => (
                    <div key={`h-${h}`} className="text-center text-[10px] leading-3 text-base-content/60">
                      {h}
                    </div>
                  ))}
                </div>

                {/* Grid 6 rows (10-minute slots), 24 columns (hours) */}
                <div className="mt-2 flex flex-col gap-[3px]">
                  {grid.rows.map((row, slotIdx) => {
                    const label = `${minuteSlotLabel(slotIdx)}m`
                    return (
                      <div key={`r-${slotIdx}`} className="flex items-center gap-3">
                        <div className="w-14 shrink-0 text-right text-xs text-base-content/70">{label}</div>
                        <div className="grid gap-[3px]" style={{ gridTemplateColumns: 'repeat(24, minmax(0, 1fr))' }}>
                          {row.map((cell, ci) => {
                            const lvl = levelFor(cell.value, grid.max)
                            const palette = LEVEL_COLORS_BY_METRIC[metric]
                            const cls = palette[lvl] ?? palette[0]
                            const formatted = formatValue(cell.value)
                            const hLabel = String(ci).padStart(2, '0')
                            const mStart = String(cell.slot * 10).padStart(2, '0')
                            const mEnd = String(cell.slot * 10 + 9).padStart(2, '0')
                            const title = `${hLabel}:${mStart}-${hLabel}:${mEnd} ${formatted}`
                            return (
                              <div key={`c-${slotIdx}-${ci}`} className={`${cls} h-5 w-5 sm:h-6 sm:w-6 rounded-sm`} title={title} aria-label={title} />
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
    </section>
  )
}

export default Last24hTenMinuteHeatmap
