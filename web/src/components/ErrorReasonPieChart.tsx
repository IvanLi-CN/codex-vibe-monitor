import { Pie, PieChart, Cell, Legend, ResponsiveContainer, Tooltip } from 'recharts'
import type { ErrorDistributionItem } from '../lib/api'
import { useTranslation } from '../i18n'

interface ErrorReasonPieChartProps {
  items: ErrorDistributionItem[]
  isLoading: boolean
}

const COLORS = ['#ef4444', '#f97316', '#f59e0b', '#eab308', '#22c55e', '#10b981', '#06b6d4', '#3b82f6', '#8b5cf6']

export function ErrorReasonPieChart({ items, isLoading }: ErrorReasonPieChartProps) {
  const { t } = useTranslation()

  const hasData = Array.isArray(items) && items.length > 0

  if (!hasData) {
    if (isLoading) {
      return (
        <div className="flex justify-center py-10">
          <span className="loading loading-bars loading-lg" aria-label={t('chart.loadingDetailed')} />
        </div>
      )
    }
    return <div className="alert">{t('chart.noDataRange')}</div>
  }

  const data = items.map((it) => ({ name: it.reason, value: it.count }))

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <PieChart>
          <Tooltip />
          <Legend />
          <Pie data={data} dataKey="value" nameKey="name" outerRadius={120} label>
            {data.map((_, index) => (
              <Cell key={`cell-${index}`} fill={COLORS[index % COLORS.length]} />
            ))}
          </Pie>
        </PieChart>
      </ResponsiveContainer>
    </div>
  )
}
