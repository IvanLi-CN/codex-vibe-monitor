import { useMemo } from 'react'
import { Bar, CartesianGrid, ComposedChart, Legend, Line, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import { chartBaseTokens, chartStatusTokens } from '../lib/chartTheme'
import { useTheme } from '../theme'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface SuccessFailureChartProps {
  points: TimeseriesPoint[]
  isLoading: boolean
  bucketSeconds?: number
}

interface ChartDatum {
  label: string
  success: number
  failure: number
  successRate: number | null
  firstByteAvgMs: number | null
  firstByteP95Ms: number | null
  firstByteSampleCount: number
}

interface SuccessFailureTooltipLabels {
  failure: string
  success: string
  successRate: string
  firstByteAvg: string
  firstByteP95: string
}

interface SuccessFailureTooltipContentProps {
  label: string
  datum: ChartDatum
  labels: SuccessFailureTooltipLabels
  noValueLabel: string
  numberFormatter: Intl.NumberFormat
  percentFormatter: Intl.NumberFormat
  latencyFormatter: Intl.NumberFormat
  tooltipBg: string
  tooltipBorder: string
  axisText: string
}

function formatSuccessRate(value: number | null, formatter: Intl.NumberFormat, noValueLabel: string) {
  if (value == null || !Number.isFinite(value)) return noValueLabel
  return `${formatter.format(value * 100)}%`
}

function formatLatencyMs(value: number | null, formatter: Intl.NumberFormat, noValueLabel: string) {
  if (value == null || !Number.isFinite(value)) return noValueLabel
  return `${formatter.format(value)} ms`
}

export function SuccessFailureTooltipContent({
  label,
  datum,
  labels,
  noValueLabel,
  numberFormatter,
  percentFormatter,
  latencyFormatter,
  tooltipBg,
  tooltipBorder,
  axisText,
}: SuccessFailureTooltipContentProps) {
  const metrics = [
    {
      label: labels.failure,
      value: numberFormatter.format(datum.failure),
    },
    {
      label: labels.success,
      value: numberFormatter.format(datum.success),
    },
    {
      label: labels.successRate,
      value: formatSuccessRate(datum.successRate, percentFormatter, noValueLabel),
    },
    {
      label: labels.firstByteAvg,
      value:
        datum.firstByteSampleCount > 0
          ? formatLatencyMs(datum.firstByteAvgMs, latencyFormatter, noValueLabel)
          : noValueLabel,
    },
    {
      label: labels.firstByteP95,
      value:
        datum.firstByteSampleCount > 0
          ? formatLatencyMs(datum.firstByteP95Ms, latencyFormatter, noValueLabel)
          : noValueLabel,
    },
  ]

  return (
    <div
      className="min-w-[11rem] rounded-lg border px-3 py-2 text-xs"
      style={{
        backgroundColor: tooltipBg,
        borderColor: tooltipBorder,
        color: axisText,
      }}
    >
      <div className="mb-2 text-sm font-semibold">{label}</div>
      <div className="space-y-1.5">
        {metrics.map((item) => (
          <div key={item.label} className="flex items-center justify-between gap-3">
            <span>{item.label}</span>
            <span className="font-semibold">{item.value}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

export function SuccessFailureChart({ points, isLoading, bucketSeconds }: SuccessFailureChartProps) {
  const { t, locale } = useTranslation()
  const { themeMode } = useTheme()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const noValueLabel = '—'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }), [localeTag])
  const percentFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 1 }), [localeTag])
  const latencyFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 1 }), [localeTag])
  const chartColors = useMemo(
    () => ({
      ...chartBaseTokens(themeMode),
      ...chartStatusTokens(themeMode),
      firstByteAvg: themeMode === 'dark' ? '#60a5fa' : '#1d4ed8',
    }),
    [themeMode],
  )

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (!points || points.length === 0) {
    return <Alert>{t('chart.noDataRange')}</Alert>
  }

  const chartData: ChartDatum[] = points.map((point) => {
    const success = point.successCount
    const failure = point.failureCount
    const total = success + failure
    return {
      label: formatLocalLabel(new Date(point.bucketStart), bucketSeconds),
      success,
      failure,
      successRate: total > 0 ? success / total : null,
      firstByteAvgMs: point.firstByteAvgMs ?? null,
      firstByteP95Ms: point.firstByteP95Ms ?? null,
      firstByteSampleCount: point.firstByteSampleCount ?? 0,
    }
  })

  const animate = chartData.length <= 800

  const tooltipLabels: SuccessFailureTooltipLabels = {
    failure: t('stats.cards.failures'),
    success: t('stats.cards.success'),
    successRate: t('stats.successFailure.tooltip.successRate'),
    firstByteAvg: t('stats.successFailure.tooltip.firstByteAvg'),
    firstByteP95: t('stats.successFailure.tooltip.firstByteP95'),
  }

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <ComposedChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
          <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
          <XAxis
            dataKey="label"
            minTickGap={32}
            angle={-15}
            dy={8}
            height={60}
            interval="preserveStartEnd"
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <YAxis
            yAxisId="count"
            orientation="left"
            tickFormatter={(v) => numberFormatter.format(v as number)}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <YAxis
            yAxisId="latency"
            orientation="right"
            tickFormatter={(v) => formatLatencyMs(v as number, latencyFormatter, noValueLabel)}
            width={90}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <Tooltip
            content={({ active, payload, label }) => {
              const datum = payload?.[0]?.payload as ChartDatum | undefined
              if (!active || !datum || typeof label !== 'string') return null
              return (
                <SuccessFailureTooltipContent
                  label={label}
                  datum={datum}
                  labels={tooltipLabels}
                  noValueLabel={noValueLabel}
                  numberFormatter={numberFormatter}
                  percentFormatter={percentFormatter}
                  latencyFormatter={latencyFormatter}
                  tooltipBg={chartColors.tooltipBg}
                  tooltipBorder={chartColors.tooltipBorder}
                  axisText={chartColors.axisText}
                />
              )
            }}
          />
          <Legend wrapperStyle={{ color: chartColors.axisText }} />
          <Bar
            yAxisId="count"
            dataKey="success"
            name={t('stats.cards.success')}
            stackId="count"
            fill={chartColors.success}
            radius={[0, 0, 4, 4]}
            isAnimationActive={animate}
          />
          <Bar
            yAxisId="count"
            dataKey="failure"
            name={t('stats.cards.failures')}
            stackId="count"
            fill={chartColors.failure}
            radius={[4, 4, 0, 0]}
            isAnimationActive={animate}
          />
          <Line
            yAxisId="latency"
            type="monotone"
            dataKey="firstByteAvgMs"
            name={t('stats.successFailure.legend.firstByteAvg')}
            stroke={chartColors.firstByteAvg}
            strokeWidth={2}
            dot={false}
            connectNulls={false}
            isAnimationActive={animate}
          />
        </ComposedChart>
      </ResponsiveContainer>
    </div>
  )
}

function pad2(n: number) {
  return n.toString().padStart(2, '0')
}

function formatLocalLabel(date: Date, bucketSeconds?: number) {
  const y = date.getFullYear()
  const m = pad2(date.getMonth() + 1)
  const d = pad2(date.getDate())
  const hh = pad2(date.getHours())
  const mm = pad2(date.getMinutes())
  if (!bucketSeconds || bucketSeconds >= 3600) {
    return `${y}-${m}-${d} ${hh}:00`
  }
  return `${y}-${m}-${d} ${hh}:${mm}`
}
