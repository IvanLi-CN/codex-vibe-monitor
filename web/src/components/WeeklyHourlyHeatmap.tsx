import { useMemo, useState } from 'react'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'

type Cell = { date: string; hour: number; value: number }

type MetricKey = 'totalCount' | 'totalCost' | 'totalTokens'

interface MetricOption {
  key: MetricKey
  labelKey: TranslationKey
}

const METRIC_OPTIONS: MetricOption[] = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

const LEVEL_COLORS_BY_METRIC: Record<MetricKey, string[]> = {
  totalCount: ['bg-base-300', 'bg-blue-200', 'bg-blue-300', 'bg-blue-400', 'bg-blue-500'],
  totalCost: ['bg-base-300', 'bg-amber-200', 'bg-amber-300', 'bg-amber-400', 'bg-amber-500'],
  totalTokens: ['bg-base-300', 'bg-violet-200', 'bg-violet-300', 'bg-violet-400', 'bg-violet-500'],
}

const ACCENT_BY_METRIC: Record<MetricKey, string> = {
  totalCount: '#3B82F6',
  totalCost: '#F59E0B',
  totalTokens: '#8B5CF6',
}

function parseDateTimeParts(value: string) {
  if (value.includes('T')) {
    const d = new Date(value)
    return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate(), hour: d.getHours() }
  }
  const [datePart, timePart] = value.split(' ')
  const [year, month, day] = (datePart ?? '').split('-').map(Number)
  const [hour] = (timePart ?? '').split(':').map(Number)
  return { year, month, day, hour: Number.isFinite(hour) ? hour : 0 }
}

function toIsoDate(y: number, m: number, d: number) {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${y}-${pad(m)}-${pad(d)}`
}

function compute7x24(points: TimeseriesPoint[], metric: MetricKey) {
  if (!points || points.length === 0) {
    return { days: [] as string[], rows: [] as Cell[][], max: 0 }
  }

  const sorted = [...points].sort((a, b) => a.bucketStart.localeCompare(b.bucketStart))
  const dates: string[] = []
  for (const p of sorted) {
    const { year, month, day } = parseDateTimeParts(p.bucketStart)
    if (!Number.isFinite(year) || !Number.isFinite(month) || !Number.isFinite(day)) continue
    const iso = toIsoDate(year, month, day)
    if (!dates.includes(iso)) {
      dates.push(iso)
    }
  }
  const last7 = dates.slice(-7)
  const indexByDate = new Map<string, number>()
  last7.forEach((d, idx) => indexByDate.set(d, idx))

  const rows: Cell[][] = Array.from({ length: last7.length }, () =>
    Array.from({ length: 24 }, (_, h) => ({ date: '', hour: h, value: 0 })),
  )

  let max = 0
  for (const p of sorted) {
    const { year, month, day, hour } = parseDateTimeParts(p.bucketStart)
    const iso = toIsoDate(year, month, day)
    const rowIndex = indexByDate.get(iso)
    if (rowIndex == null) continue
    const value =
      metric === 'totalCount' ? p.totalCount ?? 0 : metric === 'totalCost' ? p.totalCost ?? 0 : p.totalTokens ?? 0
    rows[rowIndex][hour] = { date: iso, hour, value }
    if (value > max) max = value
  }

  return { days: last7, rows, max }
}

function levelFor(value: number, max: number) {
  if (max <= 0 || value <= 0) return 0
  const ratio = value / max
  if (ratio >= 0.85) return 4
  if (ratio >= 0.55) return 3
  if (ratio >= 0.25) return 2
  return 1
}

export function WeeklyHourlyHeatmap() {
  const { t, locale } = useTranslation()
  const [metric, setMetric] = useState<MetricKey>('totalCount')
  const { data, isLoading, error } = useTimeseries('7d', { bucket: '1h' })
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD' }), [localeTag])

  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const grid = useMemo(() => compute7x24(data?.points ?? [], metric), [data?.points, metric])

  const formatValue = (value: number) => (metric === 'totalCost' ? currencyFormatter.format(value) : numberFormatter.format(value))

  const noDataText = t('heatmap.noData')

  return (
    <section className="card bg-base-100 shadow-sm" data-testid="weekly-hourly-heatmap">
      <div className="card-body gap-4">
        <div className="flex items-center justify-between gap-3">
          <h2 className="card-title">{t('heatmap.title')}</h2>
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

        {error ? (
          <div className="alert alert-error">{error}</div>
        ) : grid.days.length > 0 ? (
          <div className="w-full overflow-x-auto">
            <div className="flex justify-center">
              <div className="inline-block">
                <div className="ml-14 grid gap-[3px] pl-[3px]" style={{ gridTemplateColumns: 'repeat(24, minmax(0, 1fr))' }}>
                  {Array.from({ length: 24 }, (_, h) => (
                    <div key={`h-${h}`} className="text-center text-[10px] leading-3 text-base-content/60">
                      {h}
                    </div>
                  ))}
                </div>

                <div className="mt-2 flex flex-col gap-[3px]">
                  {grid.rows.map((row, idx) => {
                    const dateLabel = grid.days[idx]?.slice(5) ?? ''
                    return (
                      <div key={`r-${idx}`} className="flex items-center gap-3">
                        <div className="w-14 shrink-0 text-right text-xs text-base-content/70">{dateLabel}</div>
                        <div className="grid gap-[3px]" style={{ gridTemplateColumns: 'repeat(24, minmax(0, 1fr))' }}>
                          {row.map((cell, ci) => {
                            const lvl = levelFor(cell.value, grid.max)
                            const palette = LEVEL_COLORS_BY_METRIC[metric]
                            const cls = palette[lvl] ?? palette[0]
                            const formatted = formatValue(cell.value)
                            const hourLabel = String(ci).padStart(2, '0')
                            const title = `${cell.date || grid.days[idx]} ${hourLabel}:00 ${formatted}`
                            return (
                              <div
                                key={`c-${idx}-${ci}`}
                                className={`${cls} h-5 w-5 sm:h-6 sm:w-6 rounded-sm`}
                                title={title}
                                aria-label={title}
                              />
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

export default WeeklyHourlyHeatmap
