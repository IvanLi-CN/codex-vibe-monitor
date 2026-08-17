import { useCallback, useEffect, useState } from "react";
import { fetchPerfStats, type LiveRequestStreamingPerf } from "../lib/api";

export function useLiveRequestStreamingPerf(range: string) {
  const [data, setData] = useState<LiveRequestStreamingPerf | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setIsLoading(true);
    try {
      const result = await fetchPerfStats({ range, endpoint: "/v1/responses" });
      setData(result.liveRequestStreaming);
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

  return { data, isLoading, error, refresh: load };
}
