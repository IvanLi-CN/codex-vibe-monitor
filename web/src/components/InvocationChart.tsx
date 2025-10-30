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
    const chronological = [...records].sort(
      (a, b) => new Date(a.occurredAt).getTime() - new Date(b.occurredAt).getTime(),
    )
    return chronological.map((record) => {
      const occurred = new Date(record.occurredAt)
      const label = isNaN(occurred.getTime())
        ? record.occurredAt
        : occurred.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
      return {
        label,
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
          <XAxis dataKey="label" minTickGap={24} />
          <YAxis yAxisId="tokens" orientation="left" tickFormatter={numberFormatter.format} />
          <YAxis
            yAxisId="cost"
            orientation="right"
            tickFormatter={(value) => `$${numberFormatter.format(value)}`}
            width={80}
          />
          <Tooltip
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
            stroke="#2563eb"
            fill="#3b82f6"
            fillOpacity={0.25}
            strokeWidth={2}
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
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}
