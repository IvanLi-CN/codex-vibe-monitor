import { useMemo } from 'react'
import { useTimeseries } from '../hooks/useTimeseries'
import type { TimeseriesPoint } from '../lib/api'

type Cell = { date: string; hour: number; value: number }

const LEVEL_COLORS = [
  'bg-base-300', // 0
  'bg-blue-200', // 1
  'bg-blue-300', // 2
  'bg-blue-400', // 3
  'bg-blue-500', // 4
]

function parseDateTimeParts(naive: string) {
  const [datePart, timePart] = naive.split(' ')
  const [year, month, day] = (datePart ?? '').split('-').map(Number)
  const [hour] = (timePart ?? '').split(':').map(Number)
  return { year, month, day, hour: Number.isFinite(hour) ? hour : 0 }
}

function toIsoDate(y: number, m: number, d: number) {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${y}-${pad(m)}-${pad(d)}`
}

function compute7x24(points: TimeseriesPoint[]) {
  if (!points || points.length === 0) {
    return { days: [] as string[], rows: [] as Cell[][], max: 0 }
  }

  const sorted = [...points].sort((a, b) => a.bucketStart.localeCompare(b.bucketStart))
  // Collect unique ISO dates in order
  const dateSet: string[] = []
  for (const p of sorted) {
    const { year, month, day } = parseDateTimeParts(p.bucketStart)
    if (!Number.isFinite(year) || !Number.isFinite(month) || !Number.isFinite(day)) continue
    const iso = toIsoDate(year, month, day)
    if (dateSet.length === 0 || dateSet[dateSet.length - 1] !== iso) {
      if (dateSet.length === 0 || dateSet[dateSet.length - 1] !== iso) dateSet.push(iso)
    }
  }
  const last7 = dateSet.slice(-7)
  const indexByDate = new Map<string, number>()
  last7.forEach((d, i) => indexByDate.set(d, i))

  const rows: Cell[][] = Array.from({ length: last7.length }, () =>
    Array.from({ length: 24 }, (_, h) => ({ date: '', hour: h, value: 0 })),
  )

  let max = 0
  for (const p of sorted) {
    const { year, month, day, hour } = parseDateTimeParts(p.bucketStart)
    const iso = toIsoDate(year, month, day)
    const rowIndex = indexByDate.get(iso)
    if (rowIndex == null) continue
    const value = p.totalCount ?? 0
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
  const { data, isLoading, error } = useTimeseries('7d', { bucket: '1h' })

  const grid = useMemo(() => compute7x24(data?.points ?? []), [data?.points])

  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  return (
    <section className="card bg-base-100 shadow-sm">
      <div className="card-body gap-4">
        <div className="flex items-center justify-between">
          <h2 className="card-title">最近 7 天活动图</h2>
          <span className="text-sm text-base-content/60">按小时聚合</span>
        </div>

        {isLoading ? (
          <div className="skeleton h-40 w-full" />
        ) : grid.days.length === 0 ? (
          <div className="text-base-content/70">暂无数据</div>
        ) : (
          <div className="overflow-x-auto">
            <div className="inline-block">
              {/* Column labels */}
              <div className="ml-12 grid grid-cols-24 gap-[2px] pl-[2px]">
                {Array.from({ length: 24 }, (_, h) => (
                  <div key={`h-${h}`} className="text-center text-[10px] leading-3 text-base-content/60">
                    {h}
                  </div>
                ))}
              </div>

              {/* Grid rows */}
              <div className="mt-2 flex flex-col gap-[2px]">
                {grid.rows.map((row, idx) => {
                  const dateLabel = grid.days[idx]?.slice(5) ?? '' // MM-DD
                  return (
                    <div key={`r-${idx}`} className="flex items-center gap-2">
                      <div className="w-12 shrink-0 text-right text-xs text-base-content/70">{dateLabel}</div>
                      <div className="grid grid-cols-24 gap-[2px]">
                        {row.map((cell, ci) => {
                          const lvl = levelFor(cell.value, grid.max)
                          const cls = LEVEL_COLORS[lvl] ?? LEVEL_COLORS[0]
                          const title = `${cell.date || grid.days[idx]} ${String(ci).padStart(2, '0')}:00：${cell.value.toLocaleString()} 次`
                          return (
                            <div
                              key={`c-${idx}-${ci}`}
                              className={`${cls} h-4 w-4 rounded-sm`}
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
        )}
      </div>
    </section>
  )
}

export default WeeklyHourlyHeatmap
