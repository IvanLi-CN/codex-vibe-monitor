import { useEffect, useState } from "react";
import {
  fetchUpstreamAccountModelRouting,
  type ModelRoutingState,
  type UpstreamAccountDetail,
} from "../../lib/api/core-upstream";

export function useAccountModelRouting(detail: UpstreamAccountDetail | null) {
  const [result, setResult] = useState<{
    accountId: number;
    states: ModelRoutingState[];
    loading: boolean;
    error: string | null;
  } | null>(null);
  useEffect(() => {
    if (!detail) return;
    const controller = new AbortController();
    const accountId = detail.id;
    setResult({ accountId, states: [], loading: true, error: null });
    void fetchUpstreamAccountModelRouting(accountId, controller.signal)
      .then((states) => {
        if (!controller.signal.aborted)
          setResult({ accountId, states, loading: false, error: null });
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted)
          setResult({
            accountId,
            states: [],
            loading: false,
            error: cause instanceof Error ? cause.message : String(cause),
          });
      });
    return () => controller.abort();
  }, [detail]);
  return result && result.accountId === detail?.id
    ? result
    : { states: [], loading: detail != null, error: null };
}
