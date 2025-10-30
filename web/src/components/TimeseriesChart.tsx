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

interface TimeseriesChartProps {
  points: TimeseriesPoint[]
  isLoading: boolean
  bucketSeconds?: number
}

const numberFormatter = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 })

export function TimeseriesChart({ points, isLoading, bucketSeconds }: TimeseriesChartProps) {
  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label="Loading" />
      </div>
    )
  }

  if (points.length === 0) {
    return <div className="alert">No data for selected range.</div>
  }

  const chartData = points.map((point) => ({
    label: point.bucketStart,
    totalTokens: point.totalTokens,
    totalCost: point.totalCost,
    totalCount: point.totalCount,
  }))

  const useLine = (bucketSeconds ?? 0) >= 3600

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        {useLine ? (
          <AreaChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="label" minTickGap={32} angle={-15} dy={8} height={60} />
            <YAxis
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
            />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => `$${numberFormatter.format(value as number)}`}
              width={80}
            />
            <Tooltip
              formatter={(value: number, key) =>
                key === 'totalCost'
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
              fillOpacity={0.2}
              strokeWidth={2}
            />
            <Area
              type="monotone"
              dataKey="totalCost"
              name="Cost (USD)"
              yAxisId="cost"
              stroke="#f97316"
              fill="#fb923c"
              fillOpacity={0.2}
              strokeWidth={2}
            />
          </AreaChart>
        ) : (
          <ComposedChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="label" minTickGap={32} angle={-15} dy={8} height={60} />
            <YAxis
              yAxisId="tokens"
              orientation="left"
              tickFormatter={(value) => numberFormatter.format(value as number)}
            />
            <YAxis
              yAxisId="cost"
              orientation="right"
              tickFormatter={(value) => `$${numberFormatter.format(value as number)}`}
              width={80}
            />
            <Tooltip
              formatter={(value: number, key) =>
                key === 'totalCost'
                  ? `$${numberFormatter.format(value)}`
                  : numberFormatter.format(value)
              }
            />
            <Legend />
            <Bar
              yAxisId="tokens"
              dataKey="totalTokens"
              name="Total Tokens"
              fill="#3b82f6"
              radius={[4, 4, 0, 0]}
            />
            <Bar
              yAxisId="cost"
              dataKey="totalCost"
              name="Cost (USD)"
              fill="#fb923c"
              radius={[4, 4, 0, 0]}
            />
          </ComposedChart>
        )}
      </ResponsiveContainer>
    </div>
  )
}
