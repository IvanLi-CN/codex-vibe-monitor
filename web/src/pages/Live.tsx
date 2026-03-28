import { useEffect, useMemo, useState } from "react";
import { ForwardProxyLiveTable } from "../components/ForwardProxyLiveTable";
import { AppIcon } from "../components/AppIcon";
import { InvocationChart } from "../components/InvocationChart";
import { InvocationTable } from "../components/InvocationTable";
import { PromptCacheConversationTable } from "../components/PromptCacheConversationTable";
import { StatsCards } from "../components/StatsCards";
import { Button } from "../components/ui/button";
import { useForwardProxyLiveStats } from "../hooks/useForwardProxyLiveStats";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { useInvocationStream } from "../hooks/useInvocations";
import { usePromptCacheConversations } from "../hooks/usePromptCacheConversations";
import { useSummary } from "../hooks/useStats";
import { useTranslation } from "../i18n";
import type { TranslationKey } from "../i18n";
import type { PromptCacheConversationSelection } from "../lib/api";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import { SegmentedControl, SegmentedControlItem } from "../components/ui/segmented-control";
import { SelectField } from "../components/ui/select-field";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

const LIMIT_OPTIONS = [20, 50, 100];
const PROMPT_CACHE_SELECTION_STORAGE_KEY =
  "codex-vibe-monitor.live.prompt-cache-selection";
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
    const cached = window.localStorage.getItem(
      PROMPT_CACHE_SELECTION_STORAGE_KEY,
    );
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

export default function LivePage() {
  const { t } = useTranslation();
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const [limit, setLimit] = useState(50);
  const [conversationSelectionValue, setConversationSelectionValue] = useState(
    () => readPromptCacheSelectionValue(),
  );
  const [expandedPromptCacheKeys, setExpandedPromptCacheKeys] = useState<
    string[]
  >([]);
  const [summaryWindow, setSummaryWindow] = useState("current");
  const {
    stats: forwardProxyStats,
    isLoading: forwardProxyLoading,
    error: forwardProxyError,
  } = useForwardProxyLiveStats();

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
  } = useSummary(
    summaryWindow,
    summaryWindow === "current" ? { limit } : undefined,
  );

  const { records, isLoading, error } = useInvocationStream(
    limit,
    undefined,
    undefined,
    { enableStream: true },
  );
  const chartRecords = useMemo(
    () =>
      records.filter((record) => {
        const status =
          resolveInvocationDisplayStatus(record)?.trim().toLowerCase() ?? "";
        return status !== "running" && status !== "pending";
      }),
    [records],
  );
  const conversationSelection =
    PROMPT_CACHE_SELECTION_LOOKUP.get(conversationSelectionValue) ??
    DEFAULT_PROMPT_CACHE_SELECTION;
  const {
    stats: conversationStats,
    isLoading: conversationsLoading,
    error: conversationsError,
  } = usePromptCacheConversations(conversationSelection);
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
    () =>
      conversationStats?.conversations.map(
        (conversation) => conversation.promptCacheKey,
      ) ?? [],
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
      conversationStats.conversations.map(
        (conversation) => conversation.promptCacheKey,
      ),
    );
    setExpandedPromptCacheKeys((current) => {
      const next = current.filter((promptCacheKey) =>
        visiblePromptCacheKeySet.has(promptCacheKey),
      );
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
        return current.filter(
          (promptCacheKey) => !visiblePromptCacheKeys.includes(promptCacheKey),
        );
      }

      const preserved = current.filter(
        (promptCacheKey) => !visiblePromptCacheKeys.includes(promptCacheKey),
      );
      return [...preserved, ...visiblePromptCacheKeys];
    });
  };

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("live.summary.title")}</h2>
            </div>
            <SegmentedControl
              role="tablist"
              aria-label={t("live.summary.title")}
            >
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
          <StatsCards
            stats={summary}
            loading={summaryLoading}
            error={summaryError}
          />
        </div>
      </section>

      <section className="surface-panel">
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

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("live.conversations.title")}</h2>
              <p className="section-description">
                {t("live.conversations.description")}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="gap-2"
                data-testid="live-prompt-cache-expand-all"
                disabled={
                  conversationsLoading || !hasVisiblePromptCacheConversations
                }
                onClick={toggleAllVisiblePromptCacheKeys}
              >
                <AppIcon
                  name={
                    allVisiblePromptCacheKeysExpanded
                      ? "chevron-up"
                      : "chevron-down"
                  }
                  className="h-4 w-4"
                  data-testid="live-prompt-cache-expand-all-icon"
                  data-icon-name={
                    allVisiblePromptCacheKeysExpanded
                      ? "chevron-up"
                      : "chevron-down"
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

      <section className="surface-panel">
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
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h2 className="section-title">{t("live.latest.title")}</h2>
          </div>
          <InvocationTable
            records={records}
            isLoading={isLoading}
            error={error}
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
  );
}
