import { useMemo, useState } from 'react'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'
import { metricAccent } from '../lib/chartTheme'
import { cn } from '../lib/utils'
import { useTheme } from '../theme'
import { Last24hTenMinuteHeatmap, type MetricKey } from './Last24hTenMinuteHeatmap'
import { StatsCards } from './StatsCards'
import { SegmentedControl, SegmentedControlItem } from './ui/segmented-control'
import { WeeklyHourlyHeatmap } from './WeeklyHourlyHeatmap'

type RangeKey = '1d' | '7d'

const RANGE_OPTIONS: Array<{ key: RangeKey; labelKey: string }> = [
  { key: '1d', labelKey: 'dashboard.activityOverview.range24h' },
  { key: '7d', labelKey: 'dashboard.activityOverview.range7d' },
]

const METRIC_OPTIONS: Array<{ key: MetricKey; labelKey: string }> = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

export function DashboardActivityOverview() {
  const { t } = useTranslation()
  const { themeMode } = useTheme()
  const [activeRange, setActiveRange] = useState<RangeKey>('1d')
  const [metric24h, setMetric24h] = useState<MetricKey>('totalCount')
  const [metric7d, setMetric7d] = useState<MetricKey>('totalCount')
  const {
    summary: summary24h,
    isLoading: summary24hLoading,
    error: summary24hError,
  } = useSummary('1d')
  const {
    summary: summary7d,
    isLoading: summary7dLoading,
    error: summary7dError,
  } = useSummary('7d')

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )
  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const activeMetric = activeRange === '1d' ? metric24h : metric7d
  const activeSummary = activeRange === '1d' ? summary24h : summary7d
  const activeSummaryLoading = activeRange === '1d' ? summary24hLoading : summary7dLoading
  const activeSummaryError = activeRange === '1d' ? summary24hError : summary7dError

  const setActiveMetric = (metric: MetricKey) => {
    if (activeRange === '1d') {
      setMetric24h(metric)
      return
    }
    setMetric7d(metric)
  }

  return (
    <section className="surface-panel overflow-visible" data-testid="dashboard-activity-overview">
      <div className="surface-panel-body gap-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('dashboard.activityOverview.title')}</h2>
            </div>
            <SegmentedControl role="tablist" aria-label={t('dashboard.activityOverview.rangeToggleAria')}>
              {rangeOptions.map((option) => {
                const active = option.key === activeRange
                return (
                  <SegmentedControlItem
                    key={option.key}
                    active={active}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveRange(option.key)}
                  >
                    {option.label}
                  </SegmentedControlItem>
                )
              })}
            </SegmentedControl>
          </div>
          <SegmentedControl size="compact" role="tablist" aria-label={t('heatmap.metricsToggleAria')}>
            {metricOptions.map((option) => {
              const active = option.key === activeMetric
              return (
                <SegmentedControlItem
                  key={option.key}
                  active={active}
                  role="tab"
                  aria-selected={active}
                  style={active ? { color: metricAccent(option.key, themeMode) } : undefined}
                  onClick={() => setActiveMetric(option.key)}
                >
                  {option.label}
                </SegmentedControlItem>
              )
            })}
          </SegmentedControl>
        </div>

        <StatsCards stats={activeSummary} loading={activeSummaryLoading} error={activeSummaryError} />

        <div className="grid">
          <div
            data-testid="dashboard-activity-range-1d"
            aria-hidden={activeRange !== '1d'}
            data-active={activeRange === '1d'}
            className={cn(
              'col-start-1 row-start-1',
              activeRange !== '1d' && 'invisible pointer-events-none',
            )}
          >
            <Last24hTenMinuteHeatmap
              metric={metric24h}
              onChangeMetric={setMetric24h}
              showHeader={false}
            />
          </div>

          <div
            data-testid="dashboard-activity-range-7d"
            aria-hidden={activeRange !== '7d'}
            data-active={activeRange === '7d'}
            className={cn(
              'col-start-1 row-start-1',
              activeRange !== '7d' && 'invisible pointer-events-none',
            )}
          >
            <WeeklyHourlyHeatmap
              metric={metric7d}
              onChangeMetric={setMetric7d}
              showHeader={false}
              showSurface={false}
            />
          </div>
        </div>
      </div>
    </section>
  )
}

export default DashboardActivityOverview
