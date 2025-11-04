import { InvocationTable } from '../components/InvocationTable'
import { QuotaOverview } from '../components/QuotaOverview'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { UsageCalendar } from '../components/UsageCalendar'
import { WeeklyHourlyHeatmap } from '../components/WeeklyHourlyHeatmap'
import { useInvocationStream } from '../hooks/useInvocations'
import { useQuotaSnapshot } from '../hooks/useQuotaSnapshot'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'

const RECENT_LIMIT = 20

export default function DashboardPage() {
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
    data: timeseries,
    isLoading: timeseriesLoading,
    error: timeseriesError,
  } = useTimeseries('1d', { bucket: '30m' })
  const {
    records,
    isLoading: tableLoading,
    error: tableError,
  } = useInvocationStream(RECENT_LIMIT, undefined, undefined, { enableStream: false })

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
          <div className="flex items-center justify-between">
            <h2 className="card-title">最近 24 小时统计</h2>
            <span className="text-sm text-base-content/60">实时刷新</span>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
          {timeseriesError ? (
            <div className="alert alert-error">{timeseriesError}</div>
          ) : (
            <TimeseriesChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="flex items-center justify-between">
            <h2 className="card-title">最近 {RECENT_LIMIT} 条实况</h2>
          </div>
          <InvocationTable records={records} isLoading={tableLoading} error={tableError} />
        </div>
      </section>
    </div>
  )
}
