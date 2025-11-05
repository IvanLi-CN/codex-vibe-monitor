import { useMemo } from 'react'
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

interface InvocationChartProps {
  records: ApiInvocation[]
  isLoading: boolean
}

const numberFormatter = new Intl.NumberFormat('en-US', {
  maximumFractionDigits: 2,
})

export function InvocationChart({ records, isLoading }: InvocationChartProps) {
  const data = useMemo(() => {
    const chronological = [...records].sort((a, b) => {
      const aEpoch = parseNaiveDateTime(a.occurredAt)
      const bEpoch = parseNaiveDateTime(b.occurredAt)
      if (aEpoch == null && bEpoch == null) return 0
      if (aEpoch == null) return -1
      if (bEpoch == null) return 1
      return aEpoch - bEpoch
    })

    return chronological.map((record, index) => {
      const occurredEpoch = parseNaiveDateTime(record.occurredAt)
      const occurred = occurredEpoch != null ? new Date(occurredEpoch * 1000) : null
      const timeLabel = occurred
        ? occurred.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
        : record.occurredAt

      return {
        i: index, // 用序号做等距数轴
        timeLabel,
        totalTokens: record.totalTokens ?? 0,
        cost: record.cost ?? 0,
      }
    })
  }, [records])

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label="Loading chart" />
      </div>
    )
  }

  if (data.length === 0) {
    return <div className="alert">No data points yet.</div>
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
          <YAxis yAxisId="tokens" orientation="left" tickFormatter={numberFormatter.format} />
          <YAxis
            yAxisId="cost"
            orientation="right"
            tickFormatter={(value) => `$${numberFormatter.format(value)}`}
            width={80}
          />
          <Tooltip
            labelFormatter={(value) => {
              const idx = Math.max(0, Math.min(data.length - 1, Math.round(Number(value))))
              return data[idx]?.timeLabel ?? String(idx)
            }}
            formatter={(value: number, key) =>
              key === 'cost'
                ? `$${numberFormatter.format(value)}`
                : numberFormatter.format(value)
            }
          />
          <Legend />
          <Area
            type="monotone"
            dataKey="totalTokens"
            name="Total Tokens"
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
            name="Cost (USD)"
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

function parseNaiveDateTime(value: string) {
  const [datePart, timePart] = value?.split(' ') ?? []
  if (!datePart || !timePart) {
    return null
  }

  const [year, month, day] = datePart.split('-').map(Number)
  const [hour, minute, second] = timePart.split(':').map(Number)
  if ([year, month, day, hour, minute, second].some((segment) => !Number.isFinite(segment))) {
    return null
  }

  const epochMilliseconds = Date.UTC(year, (month ?? 1) - 1, day ?? 1, hour ?? 0, minute ?? 0, second ?? 0)
  return Math.floor(epochMilliseconds / 1000)
}
