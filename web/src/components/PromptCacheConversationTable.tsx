import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { KeyedConversationTable } from "./KeyedConversationTable";

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
}

const PROMPT_CACHE_NOW_TICK_MS = 30_000;

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

export function PromptCacheConversationTable({
  stats,
  isLoading,
  error,
}: PromptCacheConversationTableProps) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, PROMPT_CACHE_NOW_TICK_MS);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!stats) return;
    setNow(Date.now());
  }, [stats]);

  const chartRangeOverride = useMemo(() => {
    if (!stats || stats.conversations.length === 0) return null;
    const earliestCreatedAt = stats.conversations.reduce<number | null>(
      (earliest, conversation) => {
        const createdAt = parseEpoch(conversation.createdAt);
        if (createdAt == null) return earliest;
        return earliest == null ? createdAt : Math.min(earliest, createdAt);
      },
      null,
    );
    if (earliestCreatedAt == null) return null;
    return {
      rangeStart: new Date(earliestCreatedAt).toISOString(),
      rangeEnd: new Date(now).toISOString(),
    };
  }, [now, stats]);

  const chartHours = useMemo(() => {
    const rangeStartEpoch = parseEpoch(
      chartRangeOverride?.rangeStart ?? stats?.rangeStart ?? "",
    );
    const rangeEndEpoch = parseEpoch(
      chartRangeOverride?.rangeEnd ?? stats?.rangeEnd ?? "",
    );
    if (
      rangeStartEpoch == null ||
      rangeEndEpoch == null ||
      rangeEndEpoch <= rangeStartEpoch
    )
      return 24;
    return Math.max(
      1,
      Math.ceil((rangeEndEpoch - rangeStartEpoch) / 3_600_000),
    );
  }, [
    chartRangeOverride?.rangeEnd,
    chartRangeOverride?.rangeStart,
    stats?.rangeEnd,
    stats?.rangeStart,
  ]);

  const footerNote = useMemo(() => {
    if (
      !stats ||
      stats.implicitFilter.filteredCount <= 0 ||
      stats.implicitFilter.kind == null
    )
      return null;
    if (stats.implicitFilter.kind === "inactiveOutside24h") {
      return t("live.conversations.implicitFilter.inactiveOutside24h", {
        count: stats.implicitFilter.filteredCount,
      });
    }
    return t("live.conversations.implicitFilter.cappedTo50", {
      count: stats.implicitFilter.filteredCount,
    });
  }, [stats, t]);

  return (
    <KeyedConversationTable<PromptCacheConversation>
      stats={stats}
      isLoading={isLoading}
      error={error}
      getConversationKey={(conversation) => conversation.promptCacheKey}
      keyColumnLabel={t("live.conversations.table.promptCacheKey")}
      emptyLabel={t("live.conversations.empty")}
      chartAriaLabel={t("live.conversations.chartAria", { hours: chartHours })}
      chartColumnLabel={t("live.conversations.table.chartWindow", {
        hours: chartHours,
      })}
      footerNote={footerNote}
      chartRangeOverride={chartRangeOverride}
    />
  );
}
