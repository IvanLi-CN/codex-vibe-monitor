import { useMemo } from 'react'
import { Bar, BarChart, CartesianGrid, Legend, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationValues } from '../i18n/translations'
import { chartBaseTokens, chartStatusTokens } from '../lib/chartTheme'
import { useTheme } from '../theme'

interface SuccessFailureChartProps {
  points: TimeseriesPoint[]
  isLoading: boolean
  bucketSeconds?: number
}

interface ChartDatum {
  label: string
  success: number
  failure: number
}

export function SuccessFailureChart({ points, isLoading, bucketSeconds }: SuccessFailureChartProps) {
  const { t, locale } = useTranslation()
  const { themeMode } = useTheme()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }), [localeTag])
  const chartColors = useMemo(
    () => ({
      ...chartBaseTokens(themeMode),
      ...chartStatusTokens(themeMode),
    }),
    [themeMode],
  )

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (!points || points.length === 0) {
    return <div className="alert">{t('chart.noDataRange')}</div>
  }

  const chartData: ChartDatum[] = points.map((p) => ({
    label: formatLocalLabel(new Date(p.bucketStart), bucketSeconds),
    success: p.successCount,
    failure: p.failureCount,
  }))
  const animate = chartData.length <= 800

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <BarChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
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
            tickFormatter={(v) => numberFormatter.format(v as number)}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <Tooltip
            formatter={(v, k) => [numberFormatter.format(v as number), legendName(k as string, t)]}
            contentStyle={{
              backgroundColor: chartColors.tooltipBg,
              borderColor: chartColors.tooltipBorder,
              borderRadius: 10,
            }}
            labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
            itemStyle={{ color: chartColors.axisText }}
          />
          <Legend wrapperStyle={{ color: chartColors.axisText }} />
          {/* Success at bottom, Failure stacked above */}
          <Bar dataKey="success" name={t('stats.cards.success')} stackId="count" fill={chartColors.success} radius={[4, 4, 0, 0]} isAnimationActive={animate} />
          <Bar dataKey="failure" name={t('stats.cards.failures')} stackId="count" fill={chartColors.failure} radius={[4, 4, 0, 0]} isAnimationActive={animate} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}

function legendName(key: string, t: (k: string, values?: TranslationValues) => string) {
  if (key === 'success') return t('stats.cards.success')
  if (key === 'failure') return t('stats.cards.failures')
  return key
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
