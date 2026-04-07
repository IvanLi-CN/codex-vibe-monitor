import { useEffect, useState } from 'react'
import { DashboardInvocationDetailDrawer } from '../components/DashboardInvocationDetailDrawer'
import { DashboardActivityOverview } from '../components/DashboardActivityOverview'
import { DashboardWorkingConversationsSection } from '../components/DashboardWorkingConversationsSection'
import { TodayStatsOverview } from '../components/TodayStatsOverview'
import { useDashboardWorkingConversations } from '../hooks/useDashboardWorkingConversations'
import type { DashboardWorkingConversationInvocationSelection } from '../lib/dashboardWorkingConversations'
import { useUpstreamAccountDetailRoute } from '../hooks/useUpstreamAccountDetailRoute'
import { useSummary } from '../hooks/useStats'
import { SharedUpstreamAccountDetailDrawer } from './account-pool/UpstreamAccounts'

export default function DashboardPage() {
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null)
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

  useEffect(() => {
    if (upstreamAccountId != null) {
      setSelectedInvocation(null)
    }
  }, [upstreamAccountId])

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <TodayStatsOverview stats={todaySummary} loading={todaySummaryLoading} error={todaySummaryError} />

      <DashboardActivityOverview />

      <DashboardWorkingConversationsSection
        cards={cards}
        isLoading={workingCardsLoading}
        error={workingCardsError}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null)
          openUpstreamAccount(accountId)
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true })
          setSelectedInvocation(selection)
        }}
      />
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null)
          openUpstreamAccount(accountId)
        }}
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
