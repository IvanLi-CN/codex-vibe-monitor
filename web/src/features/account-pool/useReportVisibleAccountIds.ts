import { useEffect, useRef } from "react";
import type { UpstreamAccountSummary } from "../../lib/api";

export function useReportVisibleAccountIds(
  items: UpstreamAccountSummary[],
  onVisibleAccountIdsChange: ((accountIds: number[]) => void) | undefined,
) {
  const visibleAccountIds = items.map((item) => item.id);
  const visibleAccountIdsKey = visibleAccountIds.join(",");
  const lastReportedVisibleAccountIdsKeyRef = useRef<string | null>(null);
  const onVisibleAccountIdsChangeRef = useRef(onVisibleAccountIdsChange);

  useEffect(() => {
    onVisibleAccountIdsChangeRef.current = onVisibleAccountIdsChange;
  }, [onVisibleAccountIdsChange]);

  useEffect(() => {
    if (lastReportedVisibleAccountIdsKeyRef.current === visibleAccountIdsKey) return;
    lastReportedVisibleAccountIdsKeyRef.current = visibleAccountIdsKey;
    onVisibleAccountIdsChangeRef.current?.(visibleAccountIds);
  }, [visibleAccountIds, visibleAccountIdsKey]);

  useEffect(
    () => () => {
      lastReportedVisibleAccountIdsKeyRef.current = null;
      onVisibleAccountIdsChangeRef.current?.([]);
    },
    [],
  );
}
