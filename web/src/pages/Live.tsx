import { useMemo, useState } from "react";
import { ForwardProxyLiveTable } from "../components/ForwardProxyLiveTable";
import { InvocationChart } from "../components/InvocationChart";
import { InvocationTable } from "../components/InvocationTable";
import { PromptCacheConversationTable } from "../components/PromptCacheConversationTable";
import { StatsCards } from "../components/StatsCards";
import { useForwardProxyLiveStats } from "../hooks/useForwardProxyLiveStats";
import { useInvocationStream } from "../hooks/useInvocations";
import { usePromptCacheConversations } from "../hooks/usePromptCacheConversations";
import { useSummary } from "../hooks/useStats";
import { useTranslation } from "../i18n";
import type { TranslationKey } from "../i18n";
import type { PromptCacheConversationSelection } from "../lib/api";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import { cn } from "../lib/utils";

const LIMIT_OPTIONS = [20, 50, 100];
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
const SUMMARY_WINDOWS: { value: string; labelKey: TranslationKey }[] = [
  { value: "current", labelKey: "live.summary.current" },
  { value: "30m", labelKey: "live.summary.30m" },
  { value: "1h", labelKey: "live.summary.1h" },
  { value: "1d", labelKey: "live.summary.1d" },
];

export default function LivePage() {
  const { t } = useTranslation();
  const [limit, setLimit] = useState(50);
  const [conversationSelection, setConversationSelection] =
    useState<PromptCacheConversationSelection>({
      mode: "count",
      limit: 50,
    });
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
  const {
    stats: conversationStats,
    isLoading: conversationsLoading,
    error: conversationsError,
  } = usePromptCacheConversations(conversationSelection);
  const conversationSelectionValue =
    conversationSelection.mode === "count"
      ? `count:${conversationSelection.limit}`
      : `activityWindow:${conversationSelection.activityHours}`;
  const promptCacheSelectionOptions = useMemo(
    () =>
      PROMPT_CACHE_SELECTION_OPTIONS.map((option) => ({
        value: option.value,
        label:
          "count" in option
            ? t(option.labelKey, { count: option.count })
            : t(option.labelKey, { hours: option.hours }),
        selection: option.selection,
      })),
    [t],
  );

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("live.summary.title")}</h2>
            </div>
            <div
              className="segment-group"
              role="tablist"
              aria-label={t("live.summary.title")}
            >
              {summaryWindows.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="tab"
                  aria-selected={summaryWindow === option.value}
                  aria-pressed={summaryWindow === option.value}
                  onClick={() => setSummaryWindow(option.value)}
                  className={cn(
                    "segment-button px-3",
                    summaryWindow === option.value && "font-semibold",
                  )}
                  data-active={summaryWindow === option.value}
                >
                  {option.label}
                </button>
              ))}
            </div>
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
            <label className="field w-40">
              <span className="field-label">
                {t("live.conversations.selectionLabel")}
              </span>
              <select
                data-testid="live-prompt-cache-selection"
                className="field-select field-select-sm"
                value={conversationSelectionValue}
                onChange={(event) => {
                  const next = promptCacheSelectionOptions.find(
                    (option) => option.value === event.target.value,
                  );
                  if (next) setConversationSelection(next.selection);
                }}
              >
                {promptCacheSelectionOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <PromptCacheConversationTable
            stats={conversationStats}
            isLoading={conversationsLoading}
            error={conversationsError}
          />
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-6">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("live.chart.title")}</h2>
            </div>
            <label className="field w-36">
              <span className="field-label">{t("live.window.label")}</span>
              <select
                className="field-select field-select-sm"
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
              >
                {LIMIT_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {t("live.option.records", { count: value })}
                  </option>
                ))}
              </select>
            </label>
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
          />
        </div>
      </section>
    </div>
  );
}
