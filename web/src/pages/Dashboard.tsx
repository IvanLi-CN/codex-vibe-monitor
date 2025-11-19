import { InvocationTable } from '../components/InvocationTable'
import { useState } from 'react'
import { QuotaOverview } from '../components/QuotaOverview'
import { StatsCards } from '../components/StatsCards'
import { UsageCalendar } from '../components/UsageCalendar'
import { WeeklyHourlyHeatmap } from '../components/WeeklyHourlyHeatmap'
import { Last24hTenMinuteHeatmap, type MetricKey, ACCENT_BY_METRIC } from '../components/Last24hTenMinuteHeatmap'
import { useInvocationStream } from '../hooks/useInvocations'
import { useQuotaSnapshot } from '../hooks/useQuotaSnapshot'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'

const RECENT_LIMIT = 20

export default function DashboardPage() {
  const { t } = useTranslation()
  // Metric selector moved to the card top-right
  const [metric, setMetric] = useState<MetricKey>('totalCount')
  const {
    snapshot,
    isLoading: snapshotLoading,
    error: snapshotError,
  } = useQuotaSnapshot()
  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useSummary('1d')
  const {
    records,
    isLoading: tableLoading,
    error: tableError,
  } = useInvocationStream(RECENT_LIMIT, undefined, undefined, { enableStream: true })

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_max-content] items-start">
        <QuotaOverview
          snapshot={snapshot}
          isLoading={snapshotLoading}
          error={snapshotError}
        />
        <UsageCalendar />
      </div>

      <WeeklyHourlyHeatmap />

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-6">
          <div className="flex items-center justify-between gap-3">
            <h2 className="card-title">{t('dashboard.section.summaryTitle')}</h2>
            <div className="tabs tabs-sm tabs-border" role="tablist" aria-label={t('heatmap.metricsToggleAria')}>
              {[
                { key: 'totalCount', label: t('metric.totalCount') },
                { key: 'totalCost', label: t('metric.totalCost') },
                { key: 'totalTokens', label: t('metric.totalTokens') },
              ].map((o) => {
                const active = o.key === metric
                return (
                  <button
                    key={o.key}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    className={`tab whitespace-nowrap px-2 sm:px-3 ${
                      active ? 'tab-active text-primary font-medium' : 'text-base-content/70 hover:text-base-content'
                    }`}
                    style={active ? { color: ACCENT_BY_METRIC[o.key as MetricKey] } : undefined}
                    onClick={() => setMetric(o.key as MetricKey)}
                  >
                    {o.label}
                  </button>
                )
              })}
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
          {/* 24x6 heatmap (each cell = 10 minutes) under last 24h stats */}
          <Last24hTenMinuteHeatmap metric={metric} onChangeMetric={setMetric} showHeader={false} />
        </div>
      </section>

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="flex items-center justify-between">
            <h2 className="card-title">{t('dashboard.section.recentLiveTitle', { count: RECENT_LIMIT })}</h2>
          </div>
          <InvocationTable records={records} isLoading={tableLoading} error={tableError} />
        </div>
      </section>
    </div>
  )
}
