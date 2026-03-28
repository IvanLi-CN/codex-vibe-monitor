import { InvocationTable } from '../components/InvocationTable'
import { DashboardActivityOverview } from '../components/DashboardActivityOverview'
import { TodayStatsOverview } from '../components/TodayStatsOverview'
import { UsageCalendar } from '../components/UsageCalendar'
import { useUpstreamAccountDetailRoute } from '../hooks/useUpstreamAccountDetailRoute'
import { useInvocationStream } from '../hooks/useInvocations'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'
import { SharedUpstreamAccountDetailDrawer } from './account-pool/UpstreamAccounts'

const RECENT_LIMIT = 20

export default function DashboardPage() {
  const { t } = useTranslation()
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } = useUpstreamAccountDetailRoute()
  const {
    summary: todaySummary,
    isLoading: todaySummaryLoading,
    error: todaySummaryError,
  } = useSummary('today')
  const {
    records,
    isLoading: tableLoading,
    error: tableError,
  } = useInvocationStream(RECENT_LIMIT, undefined, undefined, { enableStream: true })

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_max-content] items-start">
        <TodayStatsOverview stats={todaySummary} loading={todaySummaryLoading} error={todaySummaryError} />
        <UsageCalendar />
      </div>

      <DashboardActivityOverview />

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex items-center justify-between">
            <div className="section-heading">
              <h2 className="section-title">{t('dashboard.section.recentLiveTitle', { count: RECENT_LIMIT })}</h2>
            </div>
          </div>
          <InvocationTable
            records={records}
            isLoading={tableLoading}
            error={tableError}
            onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
          />
        </div>
      </section>
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  )
}
