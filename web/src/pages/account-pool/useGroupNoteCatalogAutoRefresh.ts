import { useEffect, useRef } from "react";
import type { ForwardProxyCatalogState } from "../../hooks/useUpstreamAccounts";

type RefreshForwardProxyCatalog = (options?: { silent?: boolean }) => Promise<unknown>;

export function useGroupNoteCatalogAutoRefresh(options: {
  open: boolean;
  refresh?: RefreshForwardProxyCatalog | null;
  catalogState?: Pick<ForwardProxyCatalogState, "kind" | "freshness"> | null;
}) {
  const autoRefreshReasonRef = useRef<"missing" | "stale" | null>(null);
  const { catalogState, open, refresh } = options;

  useEffect(() => {
    if (!open) {
      autoRefreshReasonRef.current = null;
      return;
    }

    const autoRefreshReason =
      catalogState?.kind === "missing"
        ? "missing"
        : catalogState?.kind !== "loading" &&
            catalogState?.freshness === "stale"
          ? "stale"
          : null;

    if (autoRefreshReason == null) {
      if (catalogState?.kind !== "loading") {
        autoRefreshReasonRef.current = null;
      }
      return;
    }
    if (typeof refresh !== "function") return;
    if (autoRefreshReasonRef.current === autoRefreshReason) return;
    autoRefreshReasonRef.current = autoRefreshReason;
    void refresh({ silent: true });
  }, [catalogState?.freshness, catalogState?.kind, open, refresh]);
}
