import { useMemo } from 'react'
import { Pie, PieChart, Cell, Legend, ResponsiveContainer, Tooltip } from 'recharts'
import type { ErrorDistributionItem } from '../lib/api'
import { useTranslation } from '../i18n'
import { chartBaseTokens, piePalette } from '../lib/chartTheme'
import { useTheme } from '../theme'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface ErrorReasonPieChartProps {
  items: ErrorDistributionItem[]
  isLoading: boolean
}

export function ErrorReasonPieChart({ items, isLoading }: ErrorReasonPieChartProps) {
  const { t } = useTranslation()
  const { themeMode } = useTheme()
  const chartColors = useMemo(() => chartBaseTokens(themeMode), [themeMode])
  const colors = useMemo(() => piePalette(themeMode), [themeMode])

  const hasData = Array.isArray(items) && items.length > 0

  if (!hasData) {
    if (isLoading) {
      return (
        <div className="flex justify-center py-10">
          <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
        </div>
      )
    }
    return <Alert>{t('chart.noDataRange')}</Alert>
  }

  const data = items.map((it) => ({ name: it.reason, value: it.count }))

  return (
    <div className="h-96 w-full">
      <ResponsiveContainer>
        <PieChart>
          <Tooltip
            contentStyle={{
              backgroundColor: chartColors.tooltipBg,
              borderColor: chartColors.tooltipBorder,
              borderRadius: 10,
            }}
            labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
            itemStyle={{ color: chartColors.axisText }}
          />
          <Legend wrapperStyle={{ color: chartColors.axisText }} />
          <Pie data={data} dataKey="value" nameKey="name" outerRadius={120} label>
            {data.map((_, index) => (
              <Cell key={`cell-${index}`} fill={colors[index % colors.length]} />
            ))}
          </Pie>
        </PieChart>
      </ResponsiveContainer>
    </div>
  )
}
