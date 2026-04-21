import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ApiInvocation, InvocationRecordsQuery } from "../lib/api";
import { fetchInvocationRecords } from "../lib/api";
import { mergeInvocationRecordCollections } from "../lib/invocationLiveMerge";
import { invocationStableKey } from "../lib/invocation";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";
import { InvocationTable } from "./InvocationTable";
import { Alert } from "./ui/alert";

export const PROMPT_CACHE_HISTORY_PAGE_SIZE = 200;
const PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS = 1_000;

export type PromptCacheConversationHistoryQueryBuilder = (
  conversationKey: string,
) => Partial<InvocationRecordsQuery>;

export type PromptCacheConversationHistoryRecordMatcher = (
  record: ApiInvocation,
  conversationKey: string,
) => boolean;

export function PromptCacheConversationInvocationTable({
  records,
  isLoading,
  error,
  emptyLabel,
  onOpenUpstreamAccount,
}: {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const hasLoadedRecords = records.length > 0;

  if (hasLoadedRecords) {
    return (
      <div className="space-y-3">
        {error ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}
        <InvocationTable
          records={records}
          isLoading={false}
          error={null}
          emptyLabel={emptyLabel}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <InvocationTable
      records={records}
      isLoading={isLoading}
      error={error}
      emptyLabel={emptyLabel}
      onOpenUpstreamAccount={onOpenUpstreamAccount}
    />
  );
}

export function usePromptCacheConversationHistory({
  open,
  conversationKey,
  historyQueryForConversationKey,
  historyRecordMatchesConversationKey,
}: {
  open: boolean;
  conversationKey: string | null;
  historyQueryForConversationKey?: PromptCacheConversationHistoryQueryBuilder;
  historyRecordMatchesConversationKey?: PromptCacheConversationHistoryRecordMatcher;
}) {
  const requestSeqRef = useRef(0);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const pendingLoadRef = useRef<{ silent?: boolean } | null>(null);
  const pendingOpenResyncRef = useRef(false);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [liveRecords, setLiveRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasHydrated, setHasHydrated] = useState(false);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const runLoad = useCallback(
    async ({ silent = false }: { silent?: boolean } = {}) => {
      if (!open || !conversationKey) return;

      inFlightRef.current = true;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      const shouldShowLoading = !(silent && hasHydratedRef.current);
      if (shouldShowLoading) setIsLoading(true);

      try {
        let page = 1;
        let snapshotId: number | undefined;
        let loaded: ApiInvocation[] = [];
        let totalRecords = 0;

        while (true) {
          const historyFilters = historyQueryForConversationKey?.(
            conversationKey,
          ) ?? {
            promptCacheKey: conversationKey,
          };
          const response = await fetchInvocationRecords({
            ...historyFilters,
            page,
            pageSize: PROMPT_CACHE_HISTORY_PAGE_SIZE,
            sortBy: "occurredAt",
            sortOrder: "desc",
            ...(snapshotId != null ? { snapshotId } : {}),
          });
          if (requestSeq !== requestSeqRef.current) return;

          snapshotId = response.snapshotId;
          totalRecords = response.total;
          loaded = [...loaded, ...response.records];
          setRecords(loaded);
          setTotal(totalRecords);

        if (loaded.length >= totalRecords || response.records.length === 0) {
          break;
        }
        page += 1;
      }

      if (requestSeq !== requestSeqRef.current) return;
      hasHydratedRef.current = true;
      setHasHydrated(true);
      const loadedStableKeys = new Set(loaded.map(invocationStableKey));
        setLiveRecords((current) =>
          current.filter(
            (record) => !loadedStableKeys.has(invocationStableKey(record)),
          ),
        );
        setError(null);
        if (pendingOpenResyncRef.current) {
          pendingOpenResyncRef.current = false;
          const pendingSilent = pendingLoadRef.current?.silent ?? true;
          pendingLoadRef.current = { silent: pendingSilent };
        }
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
        if (requestSeq === requestSeqRef.current) {
          inFlightRef.current = false;
        }
        const pendingLoad = pendingLoadRef.current;
        if (requestSeq === requestSeqRef.current && pendingLoad) {
          pendingLoadRef.current = null;
          void runLoad(pendingLoad);
        }
      }
    },
    [conversationKey, historyQueryForConversationKey, open],
  );

  const load = useCallback(
    async (options: { silent?: boolean } = {}) => {
      const silent = options.silent ?? false;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = { silent: pendingSilent && silent };
        return;
      }
      await runLoad({ silent });
    },
    [runLoad],
  );

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now();
    const delay = Math.max(
      0,
      PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS -
        (now - lastRefreshAtRef.current),
    );
    const run = () => {
      refreshTimerRef.current = null;
      lastRefreshAtRef.current = Date.now();
      void load({ silent: true });
    };
    if (delay === 0) {
      clearPendingRefreshTimer();
      run();
      return;
    }
    if (refreshTimerRef.current) return;
    refreshTimerRef.current = setTimeout(run, delay);
  }, [clearPendingRefreshTimer, load]);

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true;
        return;
      }
      const now = Date.now();
      if (
        !force &&
        now - lastRefreshAtRef.current < PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS
      ) {
        return;
      }
      lastRefreshAtRef.current = now;
      void load({ silent: true });
    },
    [load],
  );

  useEffect(() => {
    requestSeqRef.current += 1;
    hasHydratedRef.current = false;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    pendingOpenResyncRef.current = false;
    lastRefreshAtRef.current = 0;
    clearPendingRefreshTimer();

    if (!open || !conversationKey) {
      setRecords([]);
      setLiveRecords([]);
      setTotal(0);
      setIsLoading(false);
      setError(null);
      setHasHydrated(false);
      return;
    }

    setRecords([]);
    setLiveRecords([]);
    setTotal(0);
    setIsLoading(false);
    setError(null);
    setHasHydrated(false);
    void load();
  }, [clearPendingRefreshTimer, conversationKey, load, open]);

  useEffect(() => {
    if (!open || !conversationKey) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const matching = payload.records.filter(
        (record) =>
          historyRecordMatchesConversationKey?.(record, conversationKey) ??
          record.promptCacheKey?.trim() === conversationKey,
      );
      if (matching.length === 0) return;
      setLiveRecords((current) =>
        mergeInvocationRecordCollections(matching, current).slice(
          0,
          PROMPT_CACHE_HISTORY_PAGE_SIZE,
        ),
      );
      triggerSseRefresh();
    });
    return unsubscribe;
  }, [
    conversationKey,
    historyRecordMatchesConversationKey,
    open,
    triggerSseRefresh,
  ]);

  useEffect(() => {
    if (!open) return;
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync(true);
    });
    return unsubscribe;
  }, [open, triggerOpenResync]);

  useEffect(
    () => () => {
      clearPendingRefreshTimer();
      pendingLoadRef.current = null;
      pendingOpenResyncRef.current = false;
    },
    [clearPendingRefreshTimer],
  );

  const visibleRecords = useMemo(
    () => mergeInvocationRecordCollections(liveRecords, records),
    [liveRecords, records],
  );
  const effectiveTotal = useMemo(() => {
    const loadedStableKeys = new Set(records.map(invocationStableKey));
    const optimisticCount = liveRecords.reduce(
      (count, record) =>
        count + (loadedStableKeys.has(invocationStableKey(record)) ? 0 : 1),
      0,
    );
    return total + optimisticCount;
  }, [liveRecords, records, total]);
  const loadedCount = visibleRecords.length;

  return {
    visibleRecords,
    effectiveTotal,
    loadedCount,
    isLoading,
    error,
    hasHydrated,
  };
}
