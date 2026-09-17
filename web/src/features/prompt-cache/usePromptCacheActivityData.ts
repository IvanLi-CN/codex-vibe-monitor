import { useEffect, useRef, useState } from "react";
import type { InvocationHistoryOverviewTopicPayload } from "../../hooks/useConversationDetailTopics";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationRecordsSummaryResponse,
} from "../../lib/api";
import { fetchInvocationRecords, fetchInvocationRecordsSummary } from "../../lib/api";

const MAX_CHART_RECORDS = 1_000;

type FallbackResult = {
  nextSummary: InvocationRecordsSummaryResponse;
  records: ApiInvocation[];
  chartRangeStartMs: number | null;
  chartRangeEndMs: number | null;
  chartTotal: number;
};

async function loadActivityFallback(
  baseQuery: Partial<InvocationRecordsQuery>,
  signal: AbortSignal,
): Promise<FallbackResult | undefined> {
  const {
    page,
    pageSize,
    snapshotId,
    sortBy,
    sortOrder,
    signal: ignoredSignal,
    ...filters
  } = baseQuery;
  void page;
  void pageSize;
  void snapshotId;
  void sortBy;
  void sortOrder;
  void ignoredSignal;
  const latestFirstPage = await fetchInvocationRecords({
    ...filters,
    page: 1,
    pageSize: MAX_CHART_RECORDS,
    sortBy: "occurredAt",
    sortOrder: "desc",
    signal,
  });
  const firstPage = await fetchInvocationRecords({
    ...filters,
    page: 1,
    pageSize: latestFirstPage.pageSize,
    snapshotId: latestFirstPage.snapshotId,
    sortBy: "occurredAt",
    sortOrder: "desc",
    signal,
  });
  const nextSummary = await fetchInvocationRecordsSummary({
    ...filters,
    snapshotId: firstPage.snapshotId,
    signal,
  });
  const records = firstPage.records.slice(0, MAX_CHART_RECORDS);
  const safePageSize = Math.max(1, firstPage.pageSize);
  const targetCount = Math.min(MAX_CHART_RECORDS, Math.max(0, firstPage.total));
  let pageNumber = 2;
  let previousPageCount = firstPage.records.length;
  while (records.length < targetCount && previousPageCount >= safePageSize) {
    if (signal.aborted) return undefined;
    const nextPage = await fetchInvocationRecords({
      ...filters,
      page: pageNumber,
      pageSize: safePageSize,
      snapshotId: firstPage.snapshotId,
      sortBy: "occurredAt",
      sortOrder: "desc",
      signal,
    });
    previousPageCount = nextPage.records.length;
    records.push(...nextPage.records.slice(0, MAX_CHART_RECORDS - records.length));
    pageNumber += 1;
  }
  const rangeRecords = [...records];
  if (firstPage.total > records.length) {
    const oldestPage = Math.max(1, Math.ceil(firstPage.total / safePageSize));
    const oldestResponse = await fetchInvocationRecords({
      ...filters,
      page: oldestPage,
      pageSize: safePageSize,
      snapshotId: firstPage.snapshotId,
      sortBy: "occurredAt",
      sortOrder: "desc",
      signal,
    });
    rangeRecords.push(...oldestResponse.records);
  }
  const occurredAt = rangeRecords
    .map((record) => Date.parse(record.occurredAt))
    .filter(Number.isFinite);
  return {
    nextSummary,
    records,
    chartRangeStartMs: occurredAt.length ? Math.min(...occurredAt) : null,
    chartRangeEndMs: occurredAt.length ? Math.max(...occurredAt) : null,
    chartTotal: firstPage.total,
  };
}

export function usePromptCacheActivityData({
  open,
  conversationKey,
  historyQueryForConversationKey,
  realtimePayload,
  isRealtimeLoading,
  allowHttpFallback,
}: {
  open: boolean;
  conversationKey: string | null;
  historyQueryForConversationKey?: (conversationKey: string) => Partial<InvocationRecordsQuery>;
  realtimePayload: InvocationHistoryOverviewTopicPayload | null;
  isRealtimeLoading: boolean;
  allowHttpFallback: boolean;
}) {
  const [summary, setSummary] = useState<InvocationRecordsSummaryResponse | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [chartRangeStartMs, setChartRangeStartMs] = useState<number | null>(null);
  const [chartRangeEndMs, setChartRangeEndMs] = useState<number | null>(null);
  const [chartTotal, setChartTotal] = useState(0);
  const [chartIsSampled, setChartIsSampled] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);

  useEffect(() => {
    setSummary(open && conversationKey ? null : null);
    setRecords([]);
    setChartRangeStartMs(null);
    setChartRangeEndMs(null);
    setChartTotal(0);
    setChartIsSampled(false);
    setError(null);
    setIsLoading(Boolean(open && conversationKey && isRealtimeLoading));
  }, [conversationKey, isRealtimeLoading, open]);

  useEffect(() => {
    if (!realtimePayload || allowHttpFallback) return;
    requestSeqRef.current += 1;
    setSummary(realtimePayload.summary);
    setRecords(realtimePayload.records);
    setChartRangeStartMs(
      realtimePayload.chartRangeStart ? Date.parse(realtimePayload.chartRangeStart) : null,
    );
    setChartRangeEndMs(
      realtimePayload.chartRangeEnd ? Date.parse(realtimePayload.chartRangeEnd) : null,
    );
    setChartTotal(realtimePayload.chartTotal);
    setChartIsSampled(realtimePayload.chartIsSampled);
    setIsLoading(false);
    setError(null);
  }, [allowHttpFallback, realtimePayload]);

  useEffect(() => {
    if (!open || !conversationKey || !allowHttpFallback) return;
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    const controller = new AbortController();
    setIsLoading(true);
    void loadActivityFallback(
      historyQueryForConversationKey?.(conversationKey) ?? { promptCacheKey: conversationKey },
      controller.signal,
    )
      .then((response) => {
        if (!response || controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        setSummary(response.nextSummary);
        setRecords(response.records);
        setChartRangeStartMs(response.chartRangeStartMs);
        setChartRangeEndMs(response.chartRangeEndMs);
        setChartTotal(response.chartTotal);
        setChartIsSampled(response.records.length < response.chartTotal);
        setIsLoading(false);
      })
      .catch((err) => {
        if (
          controller.signal.aborted ||
          requestSeq !== requestSeqRef.current ||
          (err instanceof Error && err.name === "AbortError")
        )
          return;
        setError(err instanceof Error ? err.message : String(err));
        setIsLoading(false);
      });
    return () => controller.abort();
  }, [allowHttpFallback, conversationKey, historyQueryForConversationKey, open]);

  return {
    summary,
    records,
    chartRangeStartMs,
    chartRangeEndMs,
    chartTotal,
    chartIsSampled,
    isLoading,
    error,
  };
}
