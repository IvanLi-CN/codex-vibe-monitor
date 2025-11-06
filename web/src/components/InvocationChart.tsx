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

interface InvocationChartProps {
  records: ApiInvocation[]
  isLoading: boolean
}

export function InvocationChart({ records, isLoading }: InvocationChartProps) {
  const { t, locale } = useTranslation()
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
        <span className="loading loading-bars loading-lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (data.length === 0) {
    return <div className="alert">{t('chart.noDataPoints')}</div>
  }

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <AreaChart data={data} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
          <CartesianGrid strokeDasharray="3 3" />
          <XAxis
            dataKey="i"
            type="number"
            domain={[0, Math.max(0, data.length - 1)]}
            minTickGap={24}
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
          />
          <YAxis
            yAxisId="cost"
            orientation="right"
            tickFormatter={(value) => currencyFormatter.format(value as number)}
            width={80}
          />
          <Tooltip
            labelFormatter={(value) => {
              const idx = Math.max(0, Math.min(data.length - 1, Math.round(Number(value))))
              return data[idx]?.timeLabel ?? String(idx)
            }}
            formatter={tooltipFormatter}
          />
          <Legend />
          <Area
            type="monotone"
            dataKey="totalTokens"
            name={seriesNames.totalTokens}
            yAxisId="tokens"
            stroke="#8b5cf6"
            fill="#a78bfa"
            fillOpacity={0.25}
            strokeWidth={2}
            isAnimationActive={false}
          />
          <Area
            type="monotone"
            dataKey="cost"
            name={seriesNames.cost}
            yAxisId="cost"
            stroke="#f97316"
            fill="#fb923c"
            fillOpacity={0.2}
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
