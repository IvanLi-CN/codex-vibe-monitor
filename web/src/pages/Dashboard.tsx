import { DashboardActivityOverview } from '../components/DashboardActivityOverview'
import { DashboardWorkingConversationsSection } from '../components/DashboardWorkingConversationsSection'
import { TodayStatsOverview } from '../components/TodayStatsOverview'
import { UsageCalendar } from '../components/UsageCalendar'
import { useDashboardWorkingConversations } from '../hooks/useDashboardWorkingConversations'
import { useUpstreamAccountDetailRoute } from '../hooks/useUpstreamAccountDetailRoute'
import { useSummary } from '../hooks/useStats'
import { SharedUpstreamAccountDetailDrawer } from './account-pool/UpstreamAccounts'

export default function DashboardPage() {
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } = useUpstreamAccountDetailRoute()
  const {
    summary: todaySummary,
    isLoading: todaySummaryLoading,
    error: todaySummaryError,
  } = useSummary('today')
  const {
    cards,
    isLoading: workingCardsLoading,
    error: workingCardsError,
  } = useDashboardWorkingConversations()

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_max-content] items-start">
        <TodayStatsOverview stats={todaySummary} loading={todaySummaryLoading} error={todaySummaryError} />
        <UsageCalendar />
      </div>

      <DashboardActivityOverview />

      <DashboardWorkingConversationsSection
        cards={cards}
        isLoading={workingCardsLoading}
        error={workingCardsError}
        onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
      />
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
