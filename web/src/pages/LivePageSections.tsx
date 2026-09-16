import { useEffect } from "react";
import { Button } from "../components/ui/button";
import { SegmentedControl, SegmentedControlItem } from "../components/ui/segmented-control";
import { SelectField } from "../components/ui/select-field";
import { ForwardProxyLiveTable } from "../features/forward-proxy/ForwardProxyLiveTable";
import { InvocationChart } from "../features/invocations/InvocationChart";
import { InvocationCardList } from "../features/invocations/InvocationTable";
import { ModelRoutingLivePanel } from "../features/live/ModelRoutingLivePanel";
import { PromptCacheConversationTable } from "../features/prompt-cache/PromptCacheConversationTable";
import { AppIcon } from "../features/shared/AppIcon";
import { StatsCards } from "../features/stats/StatsCards";
import type { UpstreamAccountDetailRouteTab } from "../hooks/useUpstreamAccountDetailRoute";
import { useTranslation } from "../i18n";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";
import type { useLivePageData } from "./useLivePageData";
import {
  LIMIT_OPTIONS,
  LIVE_TAB_IDS,
  LIVE_TABS,
  type LivePageState,
  type LiveTab,
  PROMPT_CACHE_SELECTION_OPTIONS,
  SUMMARY_WINDOWS,
} from "./useLivePageState";

type LivePageData = ReturnType<typeof useLivePageData>;

interface LivePageSectionsProps {
  state: LivePageState;
  data: LivePageData;
  upstreamAccountId: number | null;
  upstreamAccountTab: UpstreamAccountDetailRouteTab;
  upstreamAccountModel: string | null;
  onOpenUpstreamAccount: (accountId: number) => void;
  onCloseUpstreamAccount: () => void;
  onOpenRoutingAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
}

function SummarySection({ state, data }: Pick<LivePageSectionsProps, "state" | "data">) {
  const { t } = useTranslation();
  const summaryWindows = SUMMARY_WINDOWS.map((option) => ({
    value: option.value,
    label: t(option.labelKey),
  }));
  return (
    <section className="surface-panel">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t("live.summary.title")}</h2>
          </div>
          <SegmentedControl role="tablist" aria-label={t("live.summary.title")}>
            {summaryWindows.map((option) => (
              <SegmentedControlItem
                key={option.value}
                active={state.summaryWindow === option.value}
                role="tab"
                aria-selected={state.summaryWindow === option.value}
                aria-pressed={state.summaryWindow === option.value}
                onClick={() => state.setSummaryWindow(option.value)}
              >
                {option.label}
              </SegmentedControlItem>
            ))}
          </SegmentedControl>
        </div>
        <StatsCards
          stats={data.summary.summary}
          loading={data.summary.isLoading}
          error={data.summary.error}
        />
      </div>
    </section>
  );
}

function LiveTabs({
  activeTab,
  onTabChange,
}: {
  activeTab: LiveTab;
  onTabChange: (tab: LiveTab) => void;
}) {
  const { t } = useTranslation();
  return (
    <nav aria-label={t("live.tabs.label")} data-testid="live-view-tabs">
      <SegmentedControl className="w-fit max-w-full flex-wrap" role="tablist">
        {LIVE_TABS.map((tab) => (
          <SegmentedControlItem
            key={tab}
            id={LIVE_TAB_IDS[tab].tab}
            active={activeTab === tab}
            role="tab"
            aria-selected={activeTab === tab}
            aria-controls={LIVE_TAB_IDS[tab].panel}
            className="min-w-0 px-2.5 sm:px-3.5"
            onClick={() => onTabChange(tab)}
          >
            {t(`live.tabs.${tab}`)}
          </SegmentedControlItem>
        ))}
      </SegmentedControl>
    </nav>
  );
}

function ConversationsSection({
  state,
  data,
  onOpenUpstreamAccount,
}: Pick<LivePageSectionsProps, "state" | "data" | "onOpenUpstreamAccount">) {
  const { t } = useTranslation();
  const conversations = data.conversations.stats?.conversations ?? [];
  const allExpanded =
    conversations.length > 0 &&
    conversations.every((item) => state.expandedPromptCacheKeys.includes(item.promptCacheKey));
  useEffect(() => {
    state.syncExpandedPromptCacheKeys(conversations.map((item) => item.promptCacheKey));
  }, [conversations, state.syncExpandedPromptCacheKeys]);
  const options = PROMPT_CACHE_SELECTION_OPTIONS.map((option) => ({
    value: option.value,
    label:
      "count" in option
        ? t(option.labelKey, { count: option.count })
        : t(option.labelKey, { hours: option.hours }),
  }));
  return (
    <section
      id={LIVE_TAB_IDS.conversations.panel}
      className="surface-panel"
      role="tabpanel"
      aria-labelledby={LIVE_TAB_IDS.conversations.tab}
    >
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t("live.conversations.title")}</h2>
            <p className="section-description">{t("live.conversations.description")}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="gap-2"
              data-testid="live-prompt-cache-expand-all"
              disabled={data.conversations.isLoading || conversations.length === 0}
              onClick={() =>
                state.toggleAllVisiblePromptCacheKeys(
                  conversations.map((item) => item.promptCacheKey),
                )
              }
            >
              <AppIcon
                name={allExpanded ? "chevron-up" : "chevron-down"}
                className="h-4 w-4"
                data-testid="live-prompt-cache-expand-all-icon"
                data-icon-name={allExpanded ? "chevron-up" : "chevron-down"}
                aria-hidden
              />
              {allExpanded
                ? t("live.conversations.actions.collapseAllRecords")
                : t("live.conversations.actions.expandAllRecords")}
            </Button>
            <SelectField
              label={t("live.conversations.selectionLabel")}
              className="w-40"
              name="livePromptCacheSelection"
              data-testid="live-prompt-cache-selection"
              size="sm"
              value={state.conversationSelectionValue}
              options={options}
              onValueChange={state.setConversationSelectionValue}
            />
          </div>
        </div>
        <PromptCacheConversationTable
          stats={data.conversations.stats}
          isLoading={data.conversations.isLoading}
          error={data.conversations.error}
          expandedPromptCacheKeys={state.expandedPromptCacheKeys}
          onToggleExpandedPromptCacheKey={state.toggleExpandedPromptCacheKey}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
      </div>
    </section>
  );
}

function RecordsSection({
  state,
  data,
  onOpenUpstreamAccount,
}: Pick<LivePageSectionsProps, "state" | "data" | "onOpenUpstreamAccount">) {
  const { t } = useTranslation();
  return (
    <section
      id={LIVE_TAB_IDS.records.panel}
      className="surface-panel"
      role="tabpanel"
      aria-labelledby={LIVE_TAB_IDS.records.tab}
    >
      <div className="surface-panel-body gap-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t("live.chart.title")}</h2>
          </div>
          <SelectField
            label={t("live.window.label")}
            className="w-36"
            name="liveWindowSize"
            size="sm"
            value={String(state.limit)}
            options={LIMIT_OPTIONS.map((value) => ({
              value: String(value),
              label: t("live.option.records", { count: value }),
            }))}
            onValueChange={(value) => state.setLimit(Number(value))}
          />
        </div>
        <InvocationChart records={data.chartRecords} isLoading={data.stream.isLoading} />
        <div className="section-heading">
          <h2 className="section-title">{t("live.latest.title")}</h2>
        </div>
        <InvocationCardList
          records={data.stream.records}
          isLoading={data.stream.isLoading}
          error={data.stream.error}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
      </div>
    </section>
  );
}

function RoutingSection({
  state,
  data,
  onOpenRoutingAccount,
  onOpenInvocation,
}: Pick<LivePageSectionsProps, "state" | "data" | "onOpenRoutingAccount" | "onOpenInvocation">) {
  return (
    <div id={LIVE_TAB_IDS.routing.panel} role="tabpanel" aria-labelledby={LIVE_TAB_IDS.routing.tab}>
      <ModelRoutingLivePanel
        data={data.routing.data}
        isLoading={data.routing.isLoading}
        error={data.routing.error}
        window={state.routingWindow}
        onWindowChange={state.setRoutingWindow}
        onOpenAccount={onOpenRoutingAccount}
        onOpenInvocation={onOpenInvocation}
        onRefresh={data.routing.refresh}
      />
    </div>
  );
}

function ProxySection({ data }: Pick<LivePageSectionsProps, "data">) {
  const { t } = useTranslation();
  return (
    <section
      id={LIVE_TAB_IDS.proxy.panel}
      className="surface-panel"
      role="tabpanel"
      aria-labelledby={LIVE_TAB_IDS.proxy.tab}
    >
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h2 className="section-title">{t("live.proxy.title")}</h2>
          <p className="section-description">{t("live.proxy.description")}</p>
        </div>
        <ForwardProxyLiveTable
          stats={data.proxy.stats}
          isLoading={data.proxy.isLoading}
          error={data.proxy.error}
        />
      </div>
    </section>
  );
}

export function LivePageSections({
  state,
  data,
  upstreamAccountId,
  upstreamAccountTab,
  upstreamAccountModel,
  onOpenUpstreamAccount,
  onCloseUpstreamAccount,
  onOpenRoutingAccount,
  onOpenInvocation,
}: LivePageSectionsProps) {
  return (
    <>
      <SummarySection state={state} data={data} />
      <LiveTabs activeTab={state.activeTab} onTabChange={state.setActiveTab} />
      {state.activeTab === "conversations" ? (
        <ConversationsSection
          state={state}
          data={data}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
      ) : null}
      {state.activeTab === "records" ? (
        <RecordsSection state={state} data={data} onOpenUpstreamAccount={onOpenUpstreamAccount} />
      ) : null}
      {state.activeTab === "routing" ? (
        <RoutingSection
          state={state}
          data={data}
          onOpenRoutingAccount={onOpenRoutingAccount}
          onOpenInvocation={onOpenInvocation}
        />
      ) : null}
      {state.activeTab === "proxy" ? <ProxySection data={data} /> : null}
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          initialExpandedModel={upstreamAccountModel}
          onClose={onCloseUpstreamAccount}
        />
      ) : null}
    </>
  );
}
