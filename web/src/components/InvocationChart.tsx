import { useCallback, useMemo } from 'react'
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import type { ApiInvocation } from '../lib/api'
import { useTranslation } from '../i18n'
import { chartBaseTokens, metricAccent, withOpacity } from '../lib/chartTheme'
import { useTheme } from '../theme'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface InvocationChartProps {
  records: ApiInvocation[]
  isLoading: boolean
}

export function InvocationChart({ records, isLoading }: InvocationChartProps) {
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

  const data = useMemo(() => {
    const chronological = [...records].sort((a, b) => {
      const aEpoch = parseIsoEpoch(a.occurredAt)
      const bEpoch = parseIsoEpoch(b.occurredAt)
      if (aEpoch == null && bEpoch == null) return 0
      if (aEpoch == null) return -1
      if (bEpoch == null) return 1
      return aEpoch - bEpoch
    })

    return chronological.map((record, index) => {
      const occurredEpoch = parseIsoEpoch(record.occurredAt)
      const occurred = occurredEpoch != null ? new Date(occurredEpoch * 1000) : null
      const timeLabel = occurred
        ? occurred.toLocaleTimeString(localeTag, {
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
            hour12: false,
          })
        : record.occurredAt

      return {
        i: index, // use index as evenly spaced axis
        timeLabel,
        totalTokens: record.totalTokens ?? 0,
        cost: record.cost ?? 0,
      }
    })
  }, [records, localeTag])

  const seriesNames = useMemo(
    () => ({
      totalTokens: t('chart.totalTokens'),
      cost: t('chart.totalCost'),
    }),
    [t],
  )

  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode)
    const tokenColor = metricAccent('totalTokens', themeMode)
    const costColor = metricAccent('totalCost', themeMode)
    return {
      ...base,
      tokenColor,
      tokenFill: withOpacity(tokenColor, 0.24),
      costColor,
      costFill: withOpacity(costColor, 0.2),
    }
  }, [themeMode])

  const tooltipFormatter = useCallback(
    (value: number, key: string | number) => {
      if (key === 'cost') {
        return [currencyFormatter.format(value), seriesNames.cost]
      }
      if (key === 'totalTokens') {
        return [numberFormatter.format(value), seriesNames.totalTokens]
      }
      return [numberFormatter.format(value), String(key)]
    },
    [currencyFormatter, numberFormatter, seriesNames],
  )

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (data.length === 0) {
    return <Alert>{t('chart.noDataPoints')}</Alert>
  }

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <AreaChart data={data} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
          <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
          <XAxis
            dataKey="i"
            type="number"
            domain={[0, Math.max(0, data.length - 1)]}
            minTickGap={24}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
            tickFormatter={(value: number) => {
              const idx = Math.max(0, Math.min(data.length - 1, Math.round(value)))
              const label = data[idx]?.timeLabel
              return label ?? String(idx)
            }}
          />
          <YAxis
            yAxisId="tokens"
            orientation="left"
            tickFormatter={(value) => numberFormatter.format(value as number)}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <YAxis
            yAxisId="cost"
            orientation="right"
            tickFormatter={(value) => currencyFormatter.format(value as number)}
            width={80}
            axisLine={{ stroke: chartColors.gridLine }}
            tickLine={{ stroke: chartColors.gridLine }}
            tick={{ fill: chartColors.axisText, fontSize: 12 }}
          />
          <Tooltip
            labelFormatter={(value) => {
              const idx = Math.max(0, Math.min(data.length - 1, Math.round(Number(value))))
              return data[idx]?.timeLabel ?? String(idx)
            }}
            formatter={tooltipFormatter}
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
            isAnimationActive={false}
          />
          <Area
            type="monotone"
            dataKey="cost"
            name={seriesNames.cost}
            yAxisId="cost"
            stroke={chartColors.costColor}
            fill={chartColors.costFill}
            fillOpacity={1}
            strokeWidth={2}
            isAnimationActive={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}

function parseIsoEpoch(value?: string | null) {
  if (!value) return null
  const t = Date.parse(value)
  if (Number.isNaN(t)) return null
  return Math.floor(t / 1000)
}
