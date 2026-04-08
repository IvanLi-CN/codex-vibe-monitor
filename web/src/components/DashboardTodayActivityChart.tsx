import { useMemo } from 'react'
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import type { TimeseriesResponse } from '../lib/api'
import { useTranslation } from '../i18n'
import { chartBaseTokens, chartStatusTokens, metricAccent, withOpacity } from '../lib/chartTheme'
import { formatTokensShort } from '../lib/numberFormatters'
import { useTheme } from '../theme'
import type { MetricKey } from './Last24hTenMinuteHeatmap'
import { Alert } from './ui/alert'

const MINUTE_MS = 60_000

export interface DashboardTodayActivityChartProps {
  response: TimeseriesResponse | null
  loading: boolean
  error?: string | null
  metric: MetricKey
}

export interface DashboardTodayMinuteDatum {
  index: number
  epochMs: number
  label: string
  tooltipLabel: string
  successCount: number
  failureCount: number
  failureCountNegative: number
  totalCount: number
  totalCost: number
  totalTokens: number
  cumulativeCost: number
  cumulativeTokens: number
}

export function buildTodayMinuteChartData(
  response: TimeseriesResponse | null,
  options?: { now?: Date; localeTag?: string },
): DashboardTodayMinuteDatum[] {
  const localeTag = options?.localeTag ?? 'en-US'
  const fallbackNow = options?.now ?? new Date()
  const anchor = floorToMinute(parseDateInput(response?.rangeEnd) ?? fallbackNow)
  const start = startOfLocalDay(anchor)

  const startMs = start.getTime()
  const endMs = anchor.getTime()
  if (endMs < startMs) return []

  const timeFormatter = new Intl.DateTimeFormat(localeTag, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
  const tooltipFormatter = new Intl.DateTimeFormat(localeTag, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })

  const pointMap = new Map<number, {
    successCount: number
    failureCount: number
    totalCount: number
    totalCost: number
    totalTokens: number
  }>()

  for (const point of response?.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart)
    if (!bucketStart) continue
    const bucketEpoch = floorToMinute(bucketStart).getTime()
    if (bucketEpoch < startMs || bucketEpoch > endMs) continue
    const current = pointMap.get(bucketEpoch) ?? {
      successCount: 0,
      failureCount: 0,
      totalCount: 0,
      totalCost: 0,
      totalTokens: 0,
    }
    current.successCount += point.successCount ?? 0
    current.failureCount += point.failureCount ?? 0
    current.totalCount += point.totalCount ?? 0
    current.totalCost += point.totalCost ?? 0
    current.totalTokens += point.totalTokens ?? 0
    pointMap.set(bucketEpoch, current)
  }

  const data: DashboardTodayMinuteDatum[] = []
  let cumulativeCost = 0
  let cumulativeTokens = 0

  for (let epochMs = startMs, index = 0; epochMs <= endMs; epochMs += MINUTE_MS, index += 1) {
    const point = pointMap.get(epochMs)
    const successCount = point?.successCount ?? 0
    const failureCount = point?.failureCount ?? 0
    const totalCount = point?.totalCount ?? successCount + failureCount
    const totalCost = point?.totalCost ?? 0
    const totalTokens = point?.totalTokens ?? 0
    cumulativeCost += totalCost
    cumulativeTokens += totalTokens

    const currentDate = new Date(epochMs)
    data.push({
      index,
      epochMs,
      label: timeFormatter.format(currentDate),
      tooltipLabel: tooltipFormatter.format(currentDate),
      successCount,
      failureCount,
      failureCountNegative: failureCount > 0 ? -failureCount : 0,
      totalCount,
      totalCost,
      totalTokens,
      cumulativeCost,
      cumulativeTokens,
    })
  }

  return data
}

function startOfLocalDay(date: Date) {
  const next = new Date(date)
  next.setHours(0, 0, 0, 0)
  return next
}

function floorToMinute(date: Date) {
  const next = new Date(date)
  next.setSeconds(0, 0)
  return next
}

function parseDateInput(value?: string | null) {
  if (!value) return null
  if (value.includes('T')) {
    const parsed = new Date(value)
    return Number.isNaN(parsed.getTime()) ? null : parsed
  }

  const [datePart, timePart] = value.split(' ')
  const [year, month, day] = (datePart ?? '').split('-').map(Number)
  const [hour, minute, second] = (timePart ?? '').split(':').map(Number)
  if (![year, month, day].every(Number.isFinite)) return null
  const parsed = new Date(
    year,
    Math.max(0, month - 1),
    day,
    Number.isFinite(hour) ? hour : 0,
    Number.isFinite(minute) ? minute : 0,
    Number.isFinite(second) ? second : 0,
    0,
  )
  return Number.isNaN(parsed.getTime()) ? null : parsed
}

function formatCountValue(value: number, unitLabel: string, formatter: Intl.NumberFormat) {
  return `${formatter.format(value)} ${unitLabel}`
}

interface TooltipPayloadEntry {
  payload?: DashboardTodayMinuteDatum
}

interface ChartTooltipContentProps {
  active?: boolean
  label?: string | number
  payload?: TooltipPayloadEntry[]
  theme: {
    tooltipBg: string
    tooltipBorder: string
    axisText: string
    success: string
    failure: string
    accent: string
  }
  renderValue: (point: DashboardTodayMinuteDatum) => Array<{ label: string; value: string; color: string }>
}

function ChartTooltipContent({
  active,
  label,
  payload,
  theme,
  renderValue,
}: ChartTooltipContentProps) {
  const point = payload?.find((entry) => entry.payload)?.payload
  if (!active || !point) return null

  const rows = renderValue(point)

  return (
    <div
      className="min-w-[180px] rounded-lg border px-3 py-2 shadow-lg"
      style={{
        backgroundColor: theme.tooltipBg,
        borderColor: theme.tooltipBorder,
        color: theme.axisText,
      }}
    >
      <div className="text-sm font-semibold">{typeof label === 'string' ? label : point.tooltipLabel}</div>
      <div className="mt-2 space-y-1 text-xs">
        {rows.map((row) => (
          <div key={row.label} className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: row.color }}
                aria-hidden="true"
              />
              <span>{row.label}</span>
            </div>
            <span className="font-medium">{row.value}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

export function DashboardTodayActivityChart({ response, loading, error, metric }: DashboardTodayActivityChartProps) {
  const { t, locale } = useTranslation()
  const { themeMode } = useTheme()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  )
  const currencyFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD', maximumFractionDigits: 4 }),
    [localeTag],
  )
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode)
    const status = chartStatusTokens(themeMode)
    const accent = metricAccent(metric, themeMode)
    return {
      ...base,
      success: status.success,
      successFill: withOpacity(status.success, 0.24),
      failure: status.failure,
      failureFill: withOpacity(status.failure, 0.24),
      accent,
      accentFill: withOpacity(accent, 0.22),
    }
  }, [metric, themeMode])

  const data = useMemo(
    () => buildTodayMinuteChartData(response, { localeTag }),
    [localeTag, response],
  )

  const countUnit = t('unit.calls')
  const countSeriesNames = useMemo(
    () => ({
      success: t('stats.cards.success'),
      failures: t('stats.cards.failures'),
      total: t('chart.totalCount'),
    }),
    [t],
  )
  const areaSeriesName = metric === 'totalCost' ? t('chart.totalCost') : t('chart.totalTokens')
  const countAxisBound = useMemo(() => {
    const maxValue = data.reduce(
      (current, item) => Math.max(current, item.successCount, item.failureCount),
      0,
    )
    return Math.max(1, maxValue)
  }, [data])

  if (error) {
    return <Alert variant="error">{error}</Alert>
  }

  if (loading && !response) {
    return <div className="h-80 w-full animate-pulse rounded-xl border border-base-300/70 bg-base-200/55" />
  }

  if (!loading && data.length === 0) {
    return <Alert>{t('chart.noDataRange')}</Alert>
  }

  const chartData = data.length > 0 ? data : buildTodayMinuteChartData(response, { localeTag })
  const animate = chartData.length <= 800
  const chartMode = metric === 'totalCount' ? 'count-bars' : 'cumulative-area'
  const renderCountTooltip = (point: DashboardTodayMinuteDatum) => [
    {
      label: countSeriesNames.success,
      value: formatCountValue(point.successCount, countUnit, numberFormatter),
      color: chartColors.success,
    },
    {
      label: countSeriesNames.failures,
      value: formatCountValue(point.failureCount, countUnit, numberFormatter),
      color: chartColors.failure,
    },
    {
      label: countSeriesNames.total,
      value: formatCountValue(point.totalCount, countUnit, numberFormatter),
      color: chartColors.accent,
    },
  ]
  const renderAreaTooltip = (point: DashboardTodayMinuteDatum) => [
    {
      label: areaSeriesName,
      value:
        metric === 'totalCost'
          ? currencyFormatter.format(point.cumulativeCost)
          : formatTokensShort(point.cumulativeTokens, localeTag),
      color: chartColors.accent,
    },
  ]

  return (
    <section
      className="rounded-xl border border-base-300/75 bg-base-200/40 p-4"
      data-testid="dashboard-today-activity-chart"
      data-chart-mode={chartMode}
      data-chart-metric={metric}
    >
      <div className="h-80 w-full" data-chart-kind="dashboard-today-activity">
        <ResponsiveContainer>
          {metric === 'totalCount' ? (
            <ComposedChart data={chartData} margin={{ top: 12, right: 24, left: 0, bottom: 8 }}>
              <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
              <XAxis
                dataKey="index"
                type="number"
                domain={[0, Math.max(0, chartData.length - 1)]}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item = chartData[Math.max(0, Math.min(chartData.length - 1, Math.round(value)))]
                  return item?.label ?? String(value)
                }}
              />
              <YAxis
                domain={[-countAxisBound, countAxisBound]}
                allowDecimals={false}
                tickFormatter={(value) => numberFormatter.format(Math.abs(Number(value)))}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item = chartData[Math.max(0, Math.min(chartData.length - 1, Math.round(Number(value))))]
                  return item?.tooltipLabel ?? String(value)
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={props.payload as unknown as TooltipPayloadEntry[] | undefined}
                    theme={chartColors}
                    renderValue={renderCountTooltip}
                  />
                )}
              />
              <Legend wrapperStyle={{ color: chartColors.axisText }} />
              <ReferenceLine y={0} stroke={chartColors.gridLine} />
              <Bar
                dataKey="successCount"
                name={countSeriesNames.success}
                fill={chartColors.success}
                radius={[3, 3, 0, 0]}
                isAnimationActive={animate}
              />
              <Bar
                dataKey="failureCountNegative"
                name={countSeriesNames.failures}
                fill={chartColors.failure}
                radius={[0, 0, 3, 3]}
                isAnimationActive={animate}
              />
            </ComposedChart>
          ) : (
            <AreaChart data={chartData} margin={{ top: 12, right: 24, left: 0, bottom: 8 }}>
              <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
              <XAxis
                dataKey="index"
                type="number"
                domain={[0, Math.max(0, chartData.length - 1)]}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item = chartData[Math.max(0, Math.min(chartData.length - 1, Math.round(value)))]
                  return item?.label ?? String(value)
                }}
              />
              <YAxis
                tickFormatter={(value) =>
                  metric === 'totalCost'
                    ? currencyFormatter.format(Number(value))
                    : formatTokensShort(Number(value), localeTag)
                }
                width={metric === 'totalCost' ? 90 : 80}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item = chartData[Math.max(0, Math.min(chartData.length - 1, Math.round(Number(value))))]
                  return item?.tooltipLabel ?? String(value)
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={props.payload as unknown as TooltipPayloadEntry[] | undefined}
                    theme={chartColors}
                    renderValue={renderAreaTooltip}
                  />
                )}
              />
              <Area
                type="monotone"
                dataKey={metric === 'totalCost' ? 'cumulativeCost' : 'cumulativeTokens'}
                name={areaSeriesName}
                stroke={chartColors.accent}
                fill={chartColors.accentFill}
                fillOpacity={1}
                strokeWidth={2}
                isAnimationActive={animate}
              />
            </AreaChart>
          )}
        </ResponsiveContainer>
      </div>
    </section>
  )
}
