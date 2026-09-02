import { useCallback, useEffect, useState } from "react";
import {
  fetchLiveRequestStreamingEvaluation,
  fetchPerfStats,
  type LiveRequestStreamingEvaluation,
  type LiveRequestStreamingPerf,
} from "../lib/api";

export function useLiveRequestStreamingPerf(range: string) {
  const [data, setData] = useState<LiveRequestStreamingPerf | null>(null);
  const [evaluation, setEvaluation] = useState<LiveRequestStreamingEvaluation | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setIsLoading(true);
    try {
      const [diagnostics, canonicalEvaluation] = await Promise.all([
        fetchPerfStats({ range, endpoint: "/v1/responses" }),
        fetchLiveRequestStreamingEvaluation(),
      ]);
      setData(diagnostics.liveRequestStreaming);
      setEvaluation(canonicalEvaluation);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsLoading(false);
    }
  }, [range]);

  useEffect(() => {
    void load();
  }, [load]);

  return { data, evaluation, isLoading, error, refresh: load };
}
