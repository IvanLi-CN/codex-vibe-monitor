import { useEffect, useLayoutEffect, useState } from "react";
import { useMatch, useNavigate } from "react-router-dom";
import { DashboardActivityOverview } from "../features/dashboard/DashboardActivityOverview";
import { DashboardInvocationDetailDrawer } from "../features/dashboard/DashboardInvocationDetailDrawer";
import { DashboardPerformanceDiagnostics } from "../features/dashboard/DashboardPerformanceDiagnostics";
import type { DashboardOpenUpstreamAccountOptions } from "../features/dashboard/DashboardWorkingConversationsSection";
import { DashboardWorkingConversationsSection } from "../features/dashboard/DashboardWorkingConversationsSection";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  type DashboardActivityRangeKey,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
} from "../features/dashboard/dashboardActivityRange";
import { PromptCacheConversationHistoryDrawer } from "../features/prompt-cache/PromptCacheConversationTable";
import { useCompactViewport } from "../hooks/useCompactViewport";
import useDashboardOverviewSnapshotRuntime from "../hooks/useDashboardOverviewSnapshotRuntime";
import { useDashboardActivitySnapshot } from "../hooks/useDashboardUpstreamAccountActivity";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { usePromptCacheConversationRoute } from "../hooks/usePromptCacheConversationRoute";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { useTranslation } from "../i18n";
import { usePageObservation } from "../lib/browserObservability";
import { resetDashboardPerformanceDiagnostics } from "../lib/dashboardPerformanceDiagnostics";
import type { DashboardWorkingConversationInvocationSelection } from "../lib/dashboardWorkingConversations";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts.page-local-shared";

export default function DashboardPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const invocationRouteMatch = useMatch("/dashboard/invocations/:invokeId");
  const routeInvokeId = invocationRouteMatch?.params.invokeId;
  const isCompactViewport = useCompactViewport();
  const [activeRange, setActiveRange] = useState<DashboardActivityRangeKey>(() =>
    readPersistedDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY),
  );
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null);
  const [selectedConversation, setSelectedConversation] = useState<{
    key: string;
    label: string | null;
  } | null>(null);
  const [verifiedConversationRoute, setVerifiedConversationRoute] = useState<{
    key: string;
    conversationId: string;
  } | null>(null);
  const [pendingConversationSelection, setPendingConversationSelection] = useState<{
    key: string;
    conversationId: string | null;
  } | null>(null);
  const [includeUpstreamAccountActivity, setIncludeUpstreamAccountActivity] = useState(false);
  const { upstreamAccountId, upstreamAccountTab, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const {
    promptCacheConversationKey,
    promptCacheConversationId,
    promptCacheConversationTab,
    blockedBindingFilter,
    openPromptCacheConversation,
    closePromptCacheConversation,
    clearBlockedBindingFilter,
  } = usePromptCacheConversationRoute();
  const {
    cards,
    hasDelayedStatistics,
    totalMatched,
    hasMore,
    canLoadMore: canLoadMoreFromHook,
    isLoading: workingCardsLoading,
    isLoadingMore: workingCardsLoadingMore,
    error: workingCardsError,
    loadMore,
    recentPreviewLimit,
    setRefreshTargetCount,
    refresh: refreshWorkingConversations,
  } = useDashboardWorkingConversations(blockedBindingFilter);
  const canLoadMore = canLoadMoreFromHook ?? hasMore;
  const dashboardActivityEnabled = activeRange !== "usage";
  const overviewSnapshotRuntime = useDashboardOverviewSnapshotRuntime(activeRange);
  const {
    data: dashboardActivity,
    isLoading: dashboardActivityLoading,
    isRefreshing: dashboardActivityRefreshing,
    recentLoading: dashboardActivityRecentLoading,
    recentError: dashboardActivityRecentError,
    error: dashboardActivityError,
    recentInvocationLimit: upstreamAccountRecentPreviewLimit,
    reload: reloadDashboardActivity,
    retryRecent: retryDashboardActivityRecent,
  } = useDashboardActivitySnapshot(
    activeRange,
    dashboardActivityEnabled,
    true,
    includeUpstreamAccountActivity,
  );
  usePageObservation("dashboard", overviewSnapshotRuntime.bundle);
  const conversationDataIsComplete =
    !workingCardsLoading && !workingCardsLoadingMore && !hasMore && workingCardsError == null;
  useEffect(() => {
    if (
      promptCacheConversationKey == null ||
      conversationDataIsComplete ||
      workingCardsError != null
    ) {
      return;
    }
    setPendingConversationSelection((current) =>
      current?.key === promptCacheConversationKey &&
      current.conversationId === promptCacheConversationId
        ? current
        : {
            key: promptCacheConversationKey,
            conversationId: promptCacheConversationId,
          },
    );
  }, [
    conversationDataIsComplete,
    promptCacheConversationId,
    promptCacheConversationKey,
    workingCardsError,
  ]);
  useEffect(() => {
    if (
      selectedInvocation != null &&
      routeInvokeId != null &&
      selectedInvocation.invocation.record.invokeId !== routeInvokeId
    ) {
      setSelectedInvocation(null);
    }
  }, [routeInvokeId, selectedInvocation]);

  useEffect(() => {
    if (upstreamAccountId != null) {
      setSelectedInvocation(null);
      setSelectedConversation(null);
      setVerifiedConversationRoute(null);
      setPendingConversationSelection(null);
    }
  }, [upstreamAccountId]);

  useEffect(() => {
    if (pendingConversationSelection == null) return;
    if (
      promptCacheConversationKey !== pendingConversationSelection.key ||
      promptCacheConversationId !== pendingConversationSelection.conversationId
    ) {
      setPendingConversationSelection((current) =>
        current?.key === pendingConversationSelection.key &&
        current.conversationId === pendingConversationSelection.conversationId
          ? null
          : current,
      );
      return;
    }
    if (workingCardsError != null) {
      setPendingConversationSelection(null);
      return;
    }
    if (conversationDataIsComplete) {
      setPendingConversationSelection(null);
      return;
    }
    if (!workingCardsLoading && !workingCardsLoadingMore && canLoadMore) {
      loadMore();
    } else if (!workingCardsLoading && !workingCardsLoadingMore && !canLoadMore) {
      setPendingConversationSelection(null);
    }
  }, [
    conversationDataIsComplete,
    canLoadMore,
    loadMore,
    pendingConversationSelection,
    promptCacheConversationId,
    promptCacheConversationKey,
    workingCardsError,
    workingCardsLoading,
    workingCardsLoadingMore,
  ]);

  useEffect(() => {
    if (promptCacheConversationKey == null) {
      setSelectedConversation(null);
      setVerifiedConversationRoute(null);
      setPendingConversationSelection(null);
      return;
    }
    const matchingConversationCards = cards.filter(
      (card) => card.promptCacheKey === promptCacheConversationKey,
    );
    const conversationIds = new Set(
      matchingConversationCards
        .map((card) => card.conversationId?.trim())
        .filter((conversationId): conversationId is string => Boolean(conversationId)),
    );
    const candidateConversationId = conversationIds.values().next().value ?? null;
    const singleConversationId =
      candidateConversationId != null &&
      matchingConversationCards.length > 0 &&
      matchingConversationCards.every(
        (card) => card.conversationId?.trim() === candidateConversationId,
      )
        ? candidateConversationId
        : null;
    const previouslyVerifiedRoute =
      promptCacheConversationId != null &&
      verifiedConversationRoute?.key === promptCacheConversationKey &&
      verifiedConversationRoute?.conversationId === promptCacheConversationId;
    const conversationLabel = conversationDataIsComplete
      ? promptCacheConversationId != null
        ? singleConversationId === promptCacheConversationId
          ? promptCacheConversationId
          : null
        : singleConversationId
      : previouslyVerifiedRoute
        ? promptCacheConversationId
        : null;
    if (
      promptCacheConversationId == null &&
      conversationDataIsComplete &&
      singleConversationId != null
    ) {
      openPromptCacheConversation(promptCacheConversationKey, {
        conversationId: singleConversationId,
        replace: true,
        tab: promptCacheConversationTab,
      });
    }
    setSelectedConversation((current) => {
      if (conversationLabel == null) {
        if (current?.key !== promptCacheConversationKey) return null;
        if (current.label === null) return current;
      } else if (
        current?.key === promptCacheConversationKey &&
        current.label === conversationLabel
      ) {
        return current;
      }
      return { key: promptCacheConversationKey, label: conversationLabel };
    });
    if (workingCardsError != null) {
      if (!previouslyVerifiedRoute) {
        setVerifiedConversationRoute(null);
      }
    } else if (promptCacheConversationId != null && conversationLabel == null) {
      setVerifiedConversationRoute(null);
    } else if (
      conversationDataIsComplete &&
      promptCacheConversationId != null &&
      conversationLabel === promptCacheConversationId
    ) {
      if (
        verifiedConversationRoute?.key !== promptCacheConversationKey ||
        verifiedConversationRoute?.conversationId !== promptCacheConversationId
      ) {
        setVerifiedConversationRoute({
          key: promptCacheConversationKey,
          conversationId: promptCacheConversationId,
        });
      }
    }
  }, [
    cards,
    conversationDataIsComplete,
    openPromptCacheConversation,
    promptCacheConversationId,
    promptCacheConversationKey,
    promptCacheConversationTab,
    verifiedConversationRoute,
    workingCardsError,
  ]);

  const conversationCardsForRoute = cards.filter(
    (card) => card.promptCacheKey === promptCacheConversationKey,
  );
  const conversationIdsForRoute = new Set(
    conversationCardsForRoute
      .map((card) => card.conversationId?.trim())
      .filter((conversationId): conversationId is string => Boolean(conversationId)),
  );
  const routeConversationId = conversationIdsForRoute.values().next().value ?? null;
  const visibleRouteIdentityIsUnique =
    routeConversationId != null &&
    conversationCardsForRoute.length > 0 &&
    conversationCardsForRoute.every((card) => card.conversationId?.trim() === routeConversationId);
  const verifiedRouteMatchesCurrentCards = conversationDataIsComplete
    ? visibleRouteIdentityIsUnique && routeConversationId === promptCacheConversationId
    : conversationCardsForRoute.length === 0 ||
      (visibleRouteIdentityIsUnique && routeConversationId === promptCacheConversationId);
  // History and settings are queried by prompt-cache key, so an ID-only deep link is not enough
  // to authorize a route after the working-set identity has disappeared.
  const verifiedConversationRouteIsActive =
    pendingConversationSelection == null &&
    promptCacheConversationId != null &&
    verifiedConversationRoute?.key === promptCacheConversationKey &&
    verifiedConversationRoute?.conversationId === promptCacheConversationId &&
    verifiedRouteMatchesCurrentCards;
  const conversationRouteIsSafe =
    promptCacheConversationKey == null ||
    verifiedConversationRouteIsActive ||
    (conversationDataIsComplete &&
      visibleRouteIdentityIsUnique &&
      (promptCacheConversationId == null || routeConversationId === promptCacheConversationId));

  useLayoutEffect(() => {
    resetDashboardPerformanceDiagnostics();
  }, []);

  useEffect(() => {
    persistDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, activeRange);
  }, [activeRange]);

  const handleOpenUpstreamAccount = (
    accountId: number,
    _accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => {
    setSelectedInvocation(null);
    setSelectedConversation(null);
    if (routeInvokeId != null) {
      const search = new URLSearchParams({
        upstreamAccountId: String(Math.trunc(accountId)),
      });
      if (options?.tab && options.tab !== "overview") {
        search.set("upstreamAccountTab", options.tab);
      }
      navigate({ pathname: "/dashboard", search: `?${search.toString()}` }, { replace: true });
      return;
    }
    openUpstreamAccount(accountId, {
      tab: options?.tab,
      clearPromptCacheConversation: true,
    });
  };

  if (isCompactViewport && promptCacheConversationKey != null && conversationRouteIsSafe) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <PromptCacheConversationHistoryDrawer
          open
          presentation="page"
          conversationKey={promptCacheConversationKey}
          conversationLabel={selectedConversation?.label ?? null}
          initialTab={promptCacheConversationTab}
          onTabChange={(tab) =>
            openPromptCacheConversation(promptCacheConversationKey, {
              replace: true,
              tab,
            })
          }
          onClose={() => {
            setPendingConversationSelection(null);
            closePromptCacheConversation();
          }}
          t={t}
          onOpenUpstreamAccount={handleOpenUpstreamAccount}
        />
      </div>
    );
  }

  if (isCompactViewport && upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          onClose={closeUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <DashboardActivityOverview
        activeRange={activeRange}
        onActiveRangeChange={setActiveRange}
        dashboardActivity={dashboardActivity}
        dashboardActivityLoading={dashboardActivityLoading}
        dashboardActivityError={dashboardActivityError}
        snapshotStatus={overviewSnapshotRuntime.status}
        snapshotBundle={overviewSnapshotRuntime.bundle}
      />
      <DashboardPerformanceDiagnostics />

      <DashboardWorkingConversationsSection
        activeRange={activeRange}
        cards={cards}
        hasDelayedStatistics={hasDelayedStatistics}
        totalMatched={totalMatched}
        hasMore={hasMore}
        canLoadMore={canLoadMore}
        recentPreviewLimit={recentPreviewLimit}
        isLoading={workingCardsLoading}
        isLoadingMore={workingCardsLoadingMore}
        error={workingCardsError}
        onLoadMore={loadMore}
        setRefreshTargetCount={setRefreshTargetCount}
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
        onOpenConversation={(selection) => {
          closeUpstreamAccount({ replace: true });
          setSelectedInvocation(null);
          setVerifiedConversationRoute(null);
          setPendingConversationSelection(
            hasMore
              ? {
                  key: selection.promptCacheKey,
                  conversationId: selection.conversationId,
                }
              : null,
          );
          setSelectedConversation({
            key: selection.promptCacheKey,
            label: selection.conversationId,
          });
          if (routeInvokeId != null) {
            const search = new URLSearchParams({
              promptCacheConversationKey: selection.promptCacheKey,
              promptCacheConversationId: selection.conversationId,
            });
            navigate(
              { pathname: "/dashboard", search: `?${search.toString()}` },
              { replace: true },
            );
            return;
          }
          openPromptCacheConversation(selection.promptCacheKey, {
            conversationId: selection.conversationId,
            tab: selection.tab,
            clearUpstreamAccount: true,
          });
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true });
          closePromptCacheConversation({ replace: true });
          setSelectedConversation(null);
          setPendingConversationSelection(null);
          setSelectedInvocation(selection);
          navigate(
            `/dashboard/invocations/${encodeURIComponent(selection.invocation.record.invokeId)}`,
          );
        }}
        upstreamAccountActivity={
          dashboardActivity?.accounts
            ? {
                range: dashboardActivity.range,
                rangeStart: dashboardActivity.rangeStart,
                rangeEnd: dashboardActivity.rangeEnd,
                networkLiveBucket: dashboardActivity.networkLiveBucket,
                networkRealtimeRate: dashboardActivity.networkRealtimeRate,
                accounts: dashboardActivity.accounts,
              }
            : null
        }
        upstreamAccountActivityLoading={dashboardActivityLoading}
        upstreamAccountActivityRefreshing={dashboardActivityRefreshing}
        upstreamAccountActivityError={dashboardActivityError}
        upstreamAccountRecentLoading={dashboardActivityRecentLoading}
        upstreamAccountRecentError={dashboardActivityRecentError}
        onRetryUpstreamAccountRecent={retryDashboardActivityRecent}
        upstreamAccountRecentPreviewLimit={upstreamAccountRecentPreviewLimit}
        onUpstreamAccountActivityEnabledChange={setIncludeUpstreamAccountActivity}
        onUpstreamAccountPolicyChanged={() => {
          reloadDashboardActivity();
        }}
        onConversationsChanged={() => {
          refreshWorkingConversations();
        }}
        activeBlockedBindingFilter={blockedBindingFilter}
        onClearBlockedBindingFilter={() => {
          clearBlockedBindingFilter();
        }}
      />
      <DashboardInvocationDetailDrawer
        open={routeInvokeId != null}
        invocationId={routeInvokeId ?? null}
        selection={selectedInvocation}
        onClose={() => {
          setSelectedInvocation(null);
          navigate("/dashboard");
        }}
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
      />
      <PromptCacheConversationHistoryDrawer
        open={
          promptCacheConversationKey != null && upstreamAccountId == null && conversationRouteIsSafe
        }
        conversationKey={promptCacheConversationKey}
        conversationLabel={selectedConversation?.label ?? null}
        initialTab={promptCacheConversationTab}
        onTabChange={(tab) => {
          if (promptCacheConversationKey == null) return;
          openPromptCacheConversation(promptCacheConversationKey, {
            replace: true,
            tab,
          });
        }}
        onClose={() => {
          setPendingConversationSelection(null);
          closePromptCacheConversation();
        }}
        t={t}
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
      />
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
