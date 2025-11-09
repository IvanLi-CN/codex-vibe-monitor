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

interface TimeseriesChartProps {
  points: TimeseriesPoint[]
  isLoading: boolean
  bucketSeconds?: number
  showDate?: boolean
}

interface ChartDatum {
  label: string
  totalTokens: number
  totalCost: number
  totalCount: number
}

export function TimeseriesChart({ points, isLoading, bucketSeconds, showDate = true }: TimeseriesChartProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'

  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }), [localeTag])
  const currencyFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD', maximumFractionDigits: 4 }),
    [localeTag],
  )

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (points.length === 0) {
    return <div className="alert">{t('chart.noDataRange')}</div>
  }

  const chartData: ChartDatum[] = points.map((point) => {
    const start = new Date(point.bucketStart)
    const label = formatLocalLabel(start, bucketSeconds, showDate)
    return {
      label,
      totalTokens: point.totalTokens,
      totalCost: point.totalCost,
      totalCount: point.totalCount,
    }
  })

  // Keep animations for normal point counts; auto-disable only for extreme cases to avoid UI lockups
  const animate = chartData.length <= 800

  const seriesNames = {
    totalTokens: t('chart.totalTokens'),
    totalCost: t('chart.totalCost'),
    totalCount: t('chart.totalCount'),
  }

  const useLine = (bucketSeconds ?? 0) >= 3600

  const formatValue = (value: number, key: keyof typeof seriesNames) => {
    if (key === 'totalCost') {
      return currencyFormatter.format(value)
    }
    return numberFormatter.format(value)
  }

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        {useLine ? (
          <AreaChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="label" minTickGap={32} angle={-15} dy={8} height={60} interval="preserveStartEnd" />
            <YAxis
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
            />
            <YAxis yAxisId="count" hide />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => currencyFormatter.format(value as number)}
              width={90}
            />
            <Tooltip
              formatter={(value, key) => [formatValue(value as number, key as keyof typeof seriesNames), seriesNames[key as keyof typeof seriesNames]]}
            />
            <Legend />
            <Area
              type="monotone"
              dataKey="totalTokens"
              name={seriesNames.totalTokens}
              yAxisId="tokens"
              stroke="#8b5cf6"
              fill="#a78bfa"
              fillOpacity={0.2}
              strokeWidth={2}
              isAnimationActive={animate}
            />
            <Area
              type="monotone"
              dataKey="totalCount"
              name={seriesNames.totalCount}
              yAxisId="count"
              stroke="#2563eb"
              fill="#3b82f6"
              fillOpacity={0.2}
              strokeWidth={2}
              isAnimationActive={animate}
            />
            <Area
              type="monotone"
              dataKey="totalCost"
              name={seriesNames.totalCost}
              yAxisId="cost"
              stroke="#f97316"
              fill="#fb923c"
              fillOpacity={0.2}
              strokeWidth={2}
              isAnimationActive={animate}
            />
          </AreaChart>
        ) : (
          <ComposedChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="label" minTickGap={32} angle={-15} dy={8} height={60} interval="preserveStartEnd" />
            <YAxis
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
            />
            <YAxis yAxisId="count" hide />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => currencyFormatter.format(value as number)}
              width={90}
            />
            <Tooltip
              formatter={(value, key) => [formatValue(value as number, key as keyof typeof seriesNames), seriesNames[key as keyof typeof seriesNames]]}
            />
            <Legend />
            <Bar yAxisId="tokens" dataKey="totalTokens" name={seriesNames.totalTokens} fill="#a78bfa" radius={[4, 4, 0, 0]} isAnimationActive={animate} />
            <Bar yAxisId="count" dataKey="totalCount" name={seriesNames.totalCount} fill="#3b82f6" radius={[4, 4, 0, 0]} isAnimationActive={animate} />
            <Bar yAxisId="cost" dataKey="totalCost" name={seriesNames.totalCost} fill="#fb923c" radius={[4, 4, 0, 0]} isAnimationActive={animate} />
          </ComposedChart>
        )}
      </ResponsiveContainer>
    </div>
  )
}

function pad2(n: number) {
  return n.toString().padStart(2, '0')
}

function formatLocalLabel(date: Date, bucketSeconds: number | undefined, showDate: boolean) {
  const y = date.getFullYear()
  const m = pad2(date.getMonth() + 1)
  const d = pad2(date.getDate())
  const hh = pad2(date.getHours())
  const mm = pad2(date.getMinutes())
  if (!bucketSeconds || bucketSeconds >= 3600) {
    return showDate ? `${y}-${m}-${d} ${hh}:00` : `${hh}:00`
  }
  return showDate ? `${y}-${m}-${d} ${hh}:${mm}` : `${hh}:${mm}`
}
