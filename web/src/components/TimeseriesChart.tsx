import { useMemo } from 'react'
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import type { TimeseriesPoint } from '../lib/api'
import { useTranslation } from '../i18n'
import { chartBaseTokens, metricAccent, withOpacity } from '../lib/chartTheme'
import { useTheme } from '../theme'
import {
  buildTimeseriesChartData,
  resolveTimeseriesChartMode,
} from './timeseriesChartModel'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface TimeseriesChartProps {
  points: TimeseriesPoint[]
  isLoading: boolean
  bucketSeconds?: number
  showDate?: boolean
}

export function TimeseriesChart({ points, isLoading, bucketSeconds, showDate = true }: TimeseriesChartProps) {
  const { t, locale } = useTranslation()
  const { themeMode } = useTheme()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'

  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }), [localeTag])
  const currencyFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD', maximumFractionDigits: 4 }),
    [localeTag],
  )
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode)
    const tokenColor = metricAccent('totalTokens', themeMode)
    const countColor = metricAccent('totalCount', themeMode)
    const costColor = metricAccent('totalCost', themeMode)
    return {
      ...base,
      tokenColor,
      tokenFill: withOpacity(tokenColor, 0.22),
      countColor,
      countFill: withOpacity(countColor, 0.22),
      costColor,
      costFill: withOpacity(costColor, 0.22),
    }
  }, [themeMode])

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (points.length === 0) {
    return <Alert>{t('chart.noDataRange')}</Alert>
  }

  const chartMode = resolveTimeseriesChartMode(points.length)
  const chartData = buildTimeseriesChartData(points, bucketSeconds, showDate)

  // Keep animations for normal point counts; auto-disable only for extreme cases to avoid UI lockups
  const animate = chartData.length <= 800

  const seriesNames = {
    totalTokens: t('chart.totalTokens'),
    totalCost: t('chart.totalCost'),
    totalCount: t('chart.totalCount'),
  }

  const formatValue = (value: number, key: keyof typeof seriesNames) => {
    if (key === 'totalCost') {
      return currencyFormatter.format(value)
    }
    return numberFormatter.format(value)
  }

  return (
    <div
      className="h-96 w-full"
      data-chart-kind="stats-timeseries-trend"
      data-chart-mode={chartMode}
    >
      <ResponsiveContainer>
        {chartMode === 'cumulative-area' ? (
          <AreaChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
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
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
            />
            <YAxis yAxisId="count" hide />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => currencyFormatter.format(value as number)}
              width={90}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
            />
            <Tooltip
              formatter={(value, key) => [formatValue(value as number, key as keyof typeof seriesNames), seriesNames[key as keyof typeof seriesNames]]}
              contentStyle={{
                backgroundColor: chartColors.tooltipBg,
                borderColor: chartColors.tooltipBorder,
                borderRadius: 10,
              }}
              labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
              itemStyle={{ color: chartColors.axisText }}
            />
            <Legend wrapperStyle={{ color: chartColors.axisText }} />
            <Area
              type="monotone"
              dataKey="totalTokens"
              name={seriesNames.totalTokens}
              yAxisId="tokens"
              stroke={chartColors.tokenColor}
              fill={chartColors.tokenFill}
              fillOpacity={1}
              strokeWidth={2}
              isAnimationActive={animate}
            />
            <Area
              type="monotone"
              dataKey="totalCount"
              name={seriesNames.totalCount}
              yAxisId="count"
              stroke={chartColors.countColor}
              fill={chartColors.countFill}
              fillOpacity={1}
              strokeWidth={2}
              isAnimationActive={animate}
            />
            <Area
              type="monotone"
              dataKey="totalCost"
              name={seriesNames.totalCost}
              yAxisId="cost"
              stroke={chartColors.costColor}
              fill={chartColors.costFill}
              fillOpacity={1}
              strokeWidth={2}
              isAnimationActive={animate}
            />
          </AreaChart>
        ) : (
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
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
            />
            <YAxis yAxisId="count" hide />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => currencyFormatter.format(value as number)}
              width={90}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
            />
            <Tooltip
              formatter={(value, key) => [formatValue(value as number, key as keyof typeof seriesNames), seriesNames[key as keyof typeof seriesNames]]}
              contentStyle={{
                backgroundColor: chartColors.tooltipBg,
                borderColor: chartColors.tooltipBorder,
                borderRadius: 10,
              }}
              labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
              itemStyle={{ color: chartColors.axisText }}
            />
            <Legend wrapperStyle={{ color: chartColors.axisText }} />
            <Bar yAxisId="tokens" dataKey="totalTokens" name={seriesNames.totalTokens} fill={chartColors.tokenColor} radius={[4, 4, 0, 0]} isAnimationActive={animate} />
            <Bar yAxisId="count" dataKey="totalCount" name={seriesNames.totalCount} fill={chartColors.countColor} radius={[4, 4, 0, 0]} isAnimationActive={animate} />
            <Bar yAxisId="cost" dataKey="totalCost" name={seriesNames.totalCost} fill={chartColors.costColor} radius={[4, 4, 0, 0]} isAnimationActive={animate} />
          </ComposedChart>
        )}
      </ResponsiveContainer>
    </div>
  )
}
