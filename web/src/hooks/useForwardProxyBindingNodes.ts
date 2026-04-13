import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  fetchForwardProxyBindingNodes,
  type ForwardProxyBindingNode,
} from "../lib/api";
import type { ForwardProxyCatalogState } from "./useUpstreamAccounts";

type UseForwardProxyBindingNodesOptions = {
  enabled?: boolean;
  groupName?: string;
};

function normalizeBindingNodeKeys(keys?: string[]) {
  if (!Array.isArray(keys)) return [];
  return Array.from(
    new Set(
      keys.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  ).sort((left, right) => left.localeCompare(right));
}

function normalizeOptionalGroupName(groupName?: string) {
  const normalized = groupName?.trim();
  return normalized ? normalized : null;
}

function buildForwardProxyBindingNodesQueryKey(
  keys: string[],
  groupName: string | null,
) {
  return JSON.stringify({ keys, groupName });
}

export function useForwardProxyBindingNodes(
  keys?: string[],
  options?: UseForwardProxyBindingNodesOptions,
) {
  const enabled = options?.enabled === true;
  const normalizedGroupName = useMemo(
    () => normalizeOptionalGroupName(options?.groupName),
    [options?.groupName],
  );
  const normalizedKeys = useMemo(() => normalizeBindingNodeKeys(keys), [keys]);
  const currentQueryKey = useMemo(
    () =>
      enabled
        ? buildForwardProxyBindingNodesQueryKey(
            normalizedKeys,
            normalizedGroupName,
          )
        : null,
    [enabled, normalizedGroupName, normalizedKeys],
  );
  const [nodes, setNodes] = useState<ForwardProxyBindingNode[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [dataQueryKey, setDataQueryKey] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const currentQueryKeyRef = useRef<string | null>(currentQueryKey);
  const dataQueryKeyRef = useRef<string | null>(dataQueryKey);

  useEffect(() => {
    currentQueryKeyRef.current = currentQueryKey;
  }, [currentQueryKey]);

  useEffect(() => {
    dataQueryKeyRef.current = dataQueryKey;
  }, [dataQueryKey]);

  const refresh = useCallback(
    async (loadOptions?: { silent?: boolean }) => {
      if (!enabled) return;

      const requestQueryKey = currentQueryKeyRef.current;
      requestSeqRef.current += 1;
      const requestSeq = requestSeqRef.current;
      const shouldShowLoading = !(
        loadOptions?.silent &&
        requestQueryKey != null &&
        dataQueryKeyRef.current === requestQueryKey
      );
      if (shouldShowLoading) setIsLoading(true);
      setError(null);
      try {
        const response = await fetchForwardProxyBindingNodes(normalizedKeys, {
          includeCurrent: true,
          groupName: normalizedGroupName ?? undefined,
        });
        if (requestSeq !== requestSeqRef.current) {
          return;
        }
        setNodes(response);
        setDataQueryKey(requestQueryKey);
        setError(null);
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
      }
    },
    [enabled, normalizedGroupName, normalizedKeys],
  );

  useEffect(() => {
    if (!enabled) {
      setIsLoading(false);
      setError(null);
      return;
    }
    void refresh();
  }, [enabled, refresh]);

  const hasCurrentQueryData =
    currentQueryKey != null && dataQueryKey === currentQueryKey;
  const freshness: ForwardProxyCatalogState["freshness"] = !enabled
    ? "deferred"
    : hasCurrentQueryData
      ? "fresh"
      : dataQueryKey != null
        ? "stale"
        : "missing";
  const kind: ForwardProxyCatalogState["kind"] = !enabled
    ? "deferred"
    : isLoading && !hasCurrentQueryData
      ? "loading"
      : Array.isArray(nodes)
        ? nodes.length > 0
          ? "ready-with-data"
          : "ready-empty"
        : "missing";
  const catalogState: ForwardProxyCatalogState = {
    kind,
    freshness,
    isPending: isLoading,
    hasNodes: Array.isArray(nodes) && nodes.length > 0,
  };

  return {
    nodes: nodes ?? [],
    error,
    isLoading,
    refresh,
    catalogState,
  };
}
