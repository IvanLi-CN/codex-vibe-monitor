import { useEffect, useLayoutEffect, useState } from "react";
import { DashboardConversationDetailDrawer } from "../components/DashboardConversationDetailDrawer";
import { DashboardInvocationDetailDrawer } from "../components/DashboardInvocationDetailDrawer";
import { DashboardActivityOverview } from "../components/DashboardActivityOverview";
import { DashboardPerformanceDiagnostics } from "../components/DashboardPerformanceDiagnostics";
import { DashboardWorkingConversationsSection } from "../components/DashboardWorkingConversationsSection";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { resetDashboardPerformanceDiagnostics } from "../lib/dashboardPerformanceDiagnostics";
import type {
  DashboardWorkingConversationInvocationSelection,
  DashboardWorkingConversationSelection,
} from "../lib/dashboardWorkingConversations";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

export default function DashboardPage() {
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null);
  const [selectedConversation, setSelectedConversation] =
    useState<DashboardWorkingConversationSelection | null>(null);
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const {
    cards,
    totalMatched,
    hasMore,
    isLoading: workingCardsLoading,
    isLoadingMore: workingCardsLoadingMore,
    error: workingCardsError,
    loadMore,
    setRefreshTargetCount,
  } = useDashboardWorkingConversations();

  useEffect(() => {
    if (upstreamAccountId != null) {
      setSelectedInvocation(null);
      setSelectedConversation(null);
    }
  }, [upstreamAccountId]);

  useLayoutEffect(() => {
    resetDashboardPerformanceDiagnostics();
  }, []);

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <DashboardActivityOverview />
      <DashboardPerformanceDiagnostics />

      <DashboardWorkingConversationsSection
        cards={cards}
        totalMatched={totalMatched}
        hasMore={hasMore}
        isLoading={workingCardsLoading}
        isLoadingMore={workingCardsLoadingMore}
        error={workingCardsError}
        onLoadMore={loadMore}
        setRefreshTargetCount={setRefreshTargetCount}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true });
          setSelectedConversation(null);
          setSelectedInvocation(selection);
        }}
        onOpenConversation={(selection) => {
          closeUpstreamAccount({ replace: true });
          setSelectedInvocation(null);
          setSelectedConversation(selection);
        }}
      />
      <DashboardConversationDetailDrawer
        open={selectedConversation != null}
        selection={selectedConversation}
        onClose={() => setSelectedConversation(null)}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
        }}
      />
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
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
  );
}
