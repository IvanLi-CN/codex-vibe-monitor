import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
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
import { useCompactViewport } from "../hooks/useCompactViewport";
import { useForwardProxyLiveStats } from "../hooks/useForwardProxyLiveStats";
import { useInvocationStream } from "../hooks/useInvocations";
import { useModelRoutingLive } from "../hooks/useModelRoutingLive";
import { usePromptCacheConversations } from "../hooks/usePromptCacheConversations";
import { useSummary } from "../hooks/useStats";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import type { TranslationKey } from "../i18n";
import { useTranslation } from "../i18n";
import type { PromptCacheConversationSelection } from "../lib/api";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

const LIMIT_OPTIONS = [20, 50, 100];
const PROMPT_CACHE_SELECTION_STORAGE_KEY = "codex-vibe-monitor.live.prompt-cache-selection";
const LIVE_TAB_STORAGE_KEY = "codex-vibe-monitor.live.active-tab";
const LIVE_TABS = ["conversations", "records", "routing", "proxy"] as const;
type LiveTab = (typeof LIVE_TABS)[number];
const LIVE_TAB_IDS: Record<LiveTab, { tab: string; panel: string }> = {
  conversations: {
    tab: "live-workspace-tab-conversations",
    panel: "live-workspace-panel-conversations",
  },
  records: {
    tab: "live-workspace-tab-records",
    panel: "live-workspace-panel-records",
  },
  routing: {
    tab: "live-workspace-tab-routing",
    panel: "live-workspace-panel-routing",
  },
  proxy: {
    tab: "live-workspace-tab-proxy",
    panel: "live-workspace-panel-proxy",
  },
};
const DEFAULT_PROMPT_CACHE_SELECTION: PromptCacheConversationSelection = {
  mode: "count",
  limit: 50,
};
const DEFAULT_PROMPT_CACHE_SELECTION_VALUE = "count:50";
const PROMPT_CACHE_SELECTION_OPTIONS: Array<
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      labelKey: TranslationKey;
      count: number;
    }
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      labelKey: TranslationKey;
      hours: number;
    }
> = [
  {
    value: "count:20",
    selection: { mode: "count", limit: 20 },
    labelKey: "live.conversations.option.count",
    count: 20,
  },
  {
    value: "count:50",
    selection: { mode: "count", limit: 50 },
    labelKey: "live.conversations.option.count",
    count: 50,
  },
  {
    value: "count:100",
    selection: { mode: "count", limit: 100 },
    labelKey: "live.conversations.option.count",
    count: 100,
  },
  {
    value: "activityWindow:1",
    selection: { mode: "activityWindow", activityHours: 1 },
    labelKey: "live.conversations.option.activityHours",
    hours: 1,
  },
  {
    value: "activityWindow:3",
    selection: { mode: "activityWindow", activityHours: 3 },
    labelKey: "live.conversations.option.activityHours",
    hours: 3,
  },
  {
    value: "activityWindow:6",
    selection: { mode: "activityWindow", activityHours: 6 },
    labelKey: "live.conversations.option.activityHours",
    hours: 6,
  },
  {
    value: "activityWindow:12",
    selection: { mode: "activityWindow", activityHours: 12 },
    labelKey: "live.conversations.option.activityHours",
    hours: 12,
  },
  {
    value: "activityWindow:24",
    selection: { mode: "activityWindow", activityHours: 24 },
    labelKey: "live.conversations.option.activityHours",
    hours: 24,
  },
];
const PROMPT_CACHE_SELECTION_LOOKUP = new Map(
  PROMPT_CACHE_SELECTION_OPTIONS.map((option) => [option.value, option.selection]),
);
const SUMMARY_WINDOWS: { value: string; labelKey: TranslationKey }[] = [
  { value: "current", labelKey: "live.summary.current" },
  { value: "30m", labelKey: "live.summary.30m" },
  { value: "1h", labelKey: "live.summary.1h" },
  { value: "1d", labelKey: "live.summary.1d" },
];

function readPromptCacheSelectionValue() {
  if (typeof window === "undefined") {
    return DEFAULT_PROMPT_CACHE_SELECTION_VALUE;
  }
  try {
    const cached = window.localStorage.getItem(PROMPT_CACHE_SELECTION_STORAGE_KEY);
    if (cached && PROMPT_CACHE_SELECTION_LOOKUP.has(cached)) {
      return cached;
    }
  } catch {
    // Ignore storage access failures and fall back to the default option.
  }
  return DEFAULT_PROMPT_CACHE_SELECTION_VALUE;
}

function persistPromptCacheSelectionValue(value: string) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(PROMPT_CACHE_SELECTION_STORAGE_KEY, value);
  } catch {
    // Ignore storage write failures and keep the UI responsive.
  }
}

function readLiveTab(): LiveTab {
  if (typeof window === "undefined") return "routing";
  try {
    const value = window.localStorage.getItem(LIVE_TAB_STORAGE_KEY);
    return LIVE_TABS.includes(value as LiveTab) ? (value as LiveTab) : "routing";
  } catch {
    return "routing";
  }
}

function persistLiveTab(value: LiveTab) {
  try {
    window.localStorage.setItem(LIVE_TAB_STORAGE_KEY, value);
  } catch {
    // Local storage is optional; routing remains the stable default.
  }
}

export default function LivePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const isCompactViewport = useCompactViewport();
  const {
    upstreamAccountId,
    upstreamAccountTab,
    upstreamAccountModel,
    openUpstreamAccount,
    closeUpstreamAccount,
  } = useUpstreamAccountDetailRoute();
  const [limit, setLimit] = useState(50);
  const [activeTab, setActiveTab] = useState<LiveTab>(readLiveTab);
  const [routingWindow, setRoutingWindow] = useState<"15m" | "1h" | "6h" | "24h">("1h");
  const [routingModel, setRoutingModel] = useState("");
  const [routingState, setRoutingState] = useState("");
  const [conversationSelectionValue, setConversationSelectionValue] = useState(() =>
    readPromptCacheSelectionValue(),
  );
  const [expandedPromptCacheKeys, setExpandedPromptCacheKeys] = useState<string[]>([]);
  const [summaryWindow, setSummaryWindow] = useState("current");
  const {
    stats: forwardProxyStats,
    isLoading: forwardProxyLoading,
    error: forwardProxyError,
  } = useForwardProxyLiveStats(activeTab === "proxy");

  const summaryWindows = useMemo(
    () =>
      SUMMARY_WINDOWS.map((option) => ({
        value: option.value,
        label: t(option.labelKey),
      })),
    [t],
  );

  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useSummary(summaryWindow, summaryWindow === "current" ? { limit } : undefined);

  const { records, isLoading, error } = useInvocationStream(limit, undefined, undefined, {
    enableStream: activeTab === "records",
  });
  const chartRecords = useMemo(
    () =>
      records.filter((record) => {
        const status = resolveInvocationDisplayStatus(record)?.trim().toLowerCase() ?? "";
        return status !== "running" && status !== "pending";
      }),
    [records],
  );
  const conversationSelection =
    PROMPT_CACHE_SELECTION_LOOKUP.get(conversationSelectionValue) ?? DEFAULT_PROMPT_CACHE_SELECTION;
  const {
    stats: conversationStats,
    isLoading: conversationsLoading,
    error: conversationsError,
  } = usePromptCacheConversations(conversationSelection, activeTab === "conversations");
  const {
    data: modelRouting,
    isLoading: modelRoutingLoading,
    error: modelRoutingError,
    refresh: refreshModelRouting,
  } = useModelRoutingLive(
    {
      window: routingWindow,
      model: routingModel || undefined,
      state: routingState || undefined,
      limit: 100,
    },
    activeTab === "routing",
  );
  const promptCacheSelectionOptions = useMemo(
    () =>
      PROMPT_CACHE_SELECTION_OPTIONS.map((option) => ({
        value: option.value,
        label:
          "count" in option
            ? t(option.labelKey, { count: option.count })
            : t(option.labelKey, { hours: option.hours }),
      })),
    [t],
  );
  const visiblePromptCacheKeys = useMemo(
    () => conversationStats?.conversations.map((conversation) => conversation.promptCacheKey) ?? [],
    [conversationStats],
  );
  const hasVisiblePromptCacheConversations = visiblePromptCacheKeys.length > 0;
  const allVisiblePromptCacheKeysExpanded =
    hasVisiblePromptCacheConversations &&
    visiblePromptCacheKeys.every((promptCacheKey) =>
      expandedPromptCacheKeys.includes(promptCacheKey),
    );

  useEffect(() => {
    if (!conversationStats) return;

    const visiblePromptCacheKeySet = new Set(
      conversationStats.conversations.map((conversation) => conversation.promptCacheKey),
    );
    setExpandedPromptCacheKeys((current) => {
      const next = current.filter((promptCacheKey) => visiblePromptCacheKeySet.has(promptCacheKey));
      return next.length === current.length ? current : next;
    });
  }, [conversationStats]);

  const toggleExpandedPromptCacheKey = (promptCacheKey: string) => {
    setExpandedPromptCacheKeys((current) =>
      current.includes(promptCacheKey)
        ? current.filter((value) => value !== promptCacheKey)
        : [...current, promptCacheKey],
    );
  };

  const toggleAllVisiblePromptCacheKeys = () => {
    if (!hasVisiblePromptCacheConversations) return;

    setExpandedPromptCacheKeys((current) => {
      const allExpanded = visiblePromptCacheKeys.every((promptCacheKey) =>
        current.includes(promptCacheKey),
      );
      if (allExpanded) {
        return current.filter((promptCacheKey) => !visiblePromptCacheKeys.includes(promptCacheKey));
      }

      const preserved = current.filter(
        (promptCacheKey) => !visiblePromptCacheKeys.includes(promptCacheKey),
      );
      return [...preserved, ...visiblePromptCacheKeys];
    });
  };

  if (isCompactViewport && upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          initialExpandedModel={upstreamAccountModel}
          onClose={closeUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
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
                  active={summaryWindow === option.value}
                  role="tab"
                  aria-selected={summaryWindow === option.value}
                  aria-pressed={summaryWindow === option.value}
                  onClick={() => setSummaryWindow(option.value)}
                >
                  {option.label}
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>

      <nav aria-label={t("live.tabs.label")} data-testid="live-view-tabs">
        <SegmentedControl className="grid w-full grid-cols-4" role="tablist">
          {LIVE_TABS.map((tab) => (
            <SegmentedControlItem
              key={tab}
              id={LIVE_TAB_IDS[tab].tab}
              active={activeTab === tab}
              role="tab"
              aria-selected={activeTab === tab}
              aria-controls={LIVE_TAB_IDS[tab].panel}
              className="min-w-0 w-full px-2 sm:px-3.5"
              onClick={() => {
                setActiveTab(tab);
                persistLiveTab(tab);
              }}
            >
              {t(`live.tabs.${tab}`)}
            </SegmentedControlItem>
          ))}
        </SegmentedControl>
      </nav>

      {activeTab === "conversations" ? (
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
                  disabled={conversationsLoading || !hasVisiblePromptCacheConversations}
                  onClick={toggleAllVisiblePromptCacheKeys}
                >
                  <AppIcon
                    name={allVisiblePromptCacheKeysExpanded ? "chevron-up" : "chevron-down"}
                    className="h-4 w-4"
                    data-testid="live-prompt-cache-expand-all-icon"
                    data-icon-name={
                      allVisiblePromptCacheKeysExpanded ? "chevron-up" : "chevron-down"
                    }
                    aria-hidden
                  />
                  {allVisiblePromptCacheKeysExpanded
                    ? t("live.conversations.actions.collapseAllRecords")
                    : t("live.conversations.actions.expandAllRecords")}
                </Button>
                <SelectField
                  label={t("live.conversations.selectionLabel")}
                  className="w-40"
                  name="livePromptCacheSelection"
                  data-testid="live-prompt-cache-selection"
                  size="sm"
                  value={conversationSelectionValue}
                  options={promptCacheSelectionOptions}
                  onValueChange={(value) => {
                    if (!PROMPT_CACHE_SELECTION_LOOKUP.has(value)) return;
                    setConversationSelectionValue(value);
                    persistPromptCacheSelectionValue(value);
                  }}
                />
              </div>
            </div>
            <PromptCacheConversationTable
              stats={conversationStats}
              isLoading={conversationsLoading}
              error={conversationsError}
              expandedPromptCacheKeys={expandedPromptCacheKeys}
              onToggleExpandedPromptCacheKey={toggleExpandedPromptCacheKey}
              onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
            />
          </div>
        </section>
      ) : null}

      {activeTab === "records" ? (
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
                value={String(limit)}
                options={LIMIT_OPTIONS.map((value) => ({
                  value: String(value),
                  label: t("live.option.records", { count: value }),
                }))}
                onValueChange={(value) => setLimit(Number(value))}
              />
            </div>
            <InvocationChart records={chartRecords} isLoading={isLoading} />
            <div className="section-heading">
              <h2 className="section-title">{t("live.latest.title")}</h2>
            </div>
            <InvocationCardList
              records={records}
              isLoading={isLoading}
              error={error}
              onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
            />
          </div>
        </section>
      ) : null}

      {activeTab === "routing" ? (
        <div
          id={LIVE_TAB_IDS.routing.panel}
          role="tabpanel"
          aria-labelledby={LIVE_TAB_IDS.routing.tab}
        >
          <ModelRoutingLivePanel
            data={modelRouting}
            isLoading={modelRoutingLoading}
            error={modelRoutingError}
            window={routingWindow}
            model={routingModel}
            state={routingState}
            onWindowChange={setRoutingWindow}
            onModelChange={setRoutingModel}
            onStateChange={setRoutingState}
            onOpenAccount={(accountId, selectedModel) =>
              openUpstreamAccount(accountId, { tab: "healthEvents", model: selectedModel })
            }
            onOpenInvocation={(invokeId) =>
              navigate(`/records?invokeId=${encodeURIComponent(invokeId)}`)
            }
            onRefresh={refreshModelRouting}
          />
        </div>
      ) : null}

      {activeTab === "proxy" ? (
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
              stats={forwardProxyStats}
              isLoading={forwardProxyLoading}
              error={forwardProxyError}
            />
          </div>
        </section>
      ) : null}
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          initialExpandedModel={upstreamAccountModel}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
