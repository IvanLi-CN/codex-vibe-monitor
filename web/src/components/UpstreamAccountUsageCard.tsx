import { Icon } from '@iconify/react'
import { Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import type { Formatter, NameType, ValueType } from 'recharts/types/component/DefaultTooltipContent'
import { Badge } from './ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import type { RateWindowSnapshot, UpstreamAccountHistoryPoint } from '../lib/api'

interface UpstreamAccountUsageCardProps {
  title: string
  description: string
  window?: RateWindowSnapshot | null
  history: UpstreamAccountHistoryPoint[]
  historyKey: 'primaryUsedPercent' | 'secondaryUsedPercent'
  emptyLabel: string
  noteLabel?: string
  accentClassName?: string
}

function historyLabel(value: string) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date)
}

export function UpstreamAccountUsageCard({
  title,
  description,
  window,
  history,
  historyKey,
  emptyLabel,
  noteLabel,
  accentClassName = 'text-primary',
}: UpstreamAccountUsageCardProps) {
  const chartData = history
    .slice(-14)
    .map((point) => ({
      label: historyLabel(point.capturedAt),
      value: point[historyKey] ?? null,
    }))
    .filter((point) => point.value != null)

  const chartEmpty = chartData.length === 0
  const usedPercent = Math.max(0, Math.min(window?.usedPercent ?? 0, 100))
  const resetLabel = window?.resetsAt
    ? historyLabel(window.resetsAt)
    : emptyLabel
  const tooltipFormatter: Formatter<ValueType, NameType> = (value) => {
    const rawValue = Array.isArray(value) ? value[0] : value
    const numericValue = typeof rawValue === 'number' ? rawValue : Number(rawValue ?? 0)
    return [`${Math.round(numericValue)}%`, title]
  }

  return (
    <Card className="border-base-300/80 bg-base-100/75">
      <CardHeader className="gap-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="text-base">{title}</CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          {noteLabel ? <Badge variant="secondary">{noteLabel}</Badge> : null}
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 lg:grid-cols-[auto,minmax(0,1fr)] lg:items-center">
        <div className="flex items-center gap-4">
          <div className="progress-ring" style={{ ['--value' as string]: usedPercent }}>
            <span className={`text-lg font-semibold ${accentClassName}`}>{Math.round(usedPercent)}%</span>
          </div>
          <div className="space-y-1 text-sm text-base-content/75">
            <p className="text-base font-semibold text-base-content">{window?.usedText ?? emptyLabel}</p>
            <p>{window?.limitText ?? emptyLabel}</p>
            <p className="inline-flex items-center gap-1">
              <Icon icon="mdi:timer-refresh-outline" className="h-4 w-4 text-base-content/50" aria-hidden />
              <span>{resetLabel}</span>
            </p>
          </div>
        </div>

        <div className="rounded-2xl border border-base-300/70 bg-base-100/65 p-3">
          {chartEmpty ? (
            <div className="flex h-28 items-center justify-center rounded-xl border border-dashed border-base-300/75 bg-base-200/35 text-sm text-base-content/55">
              {emptyLabel}
            </div>
          ) : (
            <div className="h-28 w-full">
              <ResponsiveContainer>
                <LineChart data={chartData} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
                  <XAxis dataKey="label" hide />
                  <YAxis hide domain={[0, 100]} />
                  <Tooltip
                    cursor={{ stroke: 'oklch(var(--color-base-content) / 0.14)', strokeWidth: 1 }}
                    formatter={tooltipFormatter}
                    labelFormatter={(value) => String(value)}
                    contentStyle={{
                      borderRadius: '0.9rem',
                      border: '1px solid color-mix(in oklab, oklch(var(--color-base-content)) 18%, transparent)',
                      background: 'oklch(var(--color-base-100) / 0.96)',
                    }}
                  />
                  <Line
                    type="monotone"
                    dataKey="value"
                    stroke="oklch(var(--color-primary))"
                    strokeWidth={2.5}
                    dot={{ r: 2.5, strokeWidth: 0, fill: 'oklch(var(--color-primary))' }}
                    activeDot={{ r: 4 }}
                    connectNulls
                  />
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
