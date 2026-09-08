import { useCallback, useEffect, useRef, useState } from "react";

export const DEFAULT_DEBOUNCED_MUTATION_DELAY_MS = 600;

export type DebouncedMutationStatus = "idle" | "pending" | "saving" | "error";

export interface UseLatestDebouncedMutationOptions<TPayload, TResult> {
  delayMs?: number;
  resourceKey?: string | number | null;
  mutate: (payload: TPayload) => Promise<TResult>;
  onSuccess?: (result: TResult, payload: TPayload) => void;
  onError?: (error: unknown, payload: TPayload) => void;
  onRevert?: (result: TResult | undefined) => void;
}

export interface UseLatestDebouncedMutationResult<TPayload, TResult> {
  status: DebouncedMutationStatus;
  error: unknown;
  hasPending: boolean;
  schedule: (payload: TPayload) => void;
  flush: () => Promise<void>;
  retry: () => void;
  revert: () => void;
  reconcile: (result: TResult) => void;
}

type QueueEntry<TPayload> = {
  resourceKey: string;
  payload: TPayload;
  sequence: number;
};

function resourceKeyId(resourceKey: string | number | null | undefined): string {
  return `${typeof resourceKey}:${String(resourceKey ?? "")}`;
}

/**
 * Serializes rapid edits for one resource while keeping the UI responsive.
 * A newer payload never starts a concurrent request; it is sent once the active
 * request settles. Failures keep the latest payload available for retry.
 */
export function useLatestDebouncedMutation<TPayload, TResult>({
  delayMs = DEFAULT_DEBOUNCED_MUTATION_DELAY_MS,
  resourceKey = null,
  mutate,
  onSuccess,
  onError,
  onRevert,
}: UseLatestDebouncedMutationOptions<TPayload, TResult>): UseLatestDebouncedMutationResult<
  TPayload,
  TResult
> {
  const [status, setStatus] = useState<DebouncedMutationStatus>("idle");
  const [error, setError] = useState<unknown>(null);
  const [hasPending, setHasPending] = useState(false);
  const mountedRef = useRef(true);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queueRef = useRef(new Map<string, QueueEntry<TPayload>>());
  const failedEntryRef = useRef<QueueEntry<TPayload> | null>(null);
  const latestSequenceRef = useRef(0);
  const confirmedResultRef = useRef<TResult | undefined>(undefined);
  const inFlightRef = useRef<Promise<void> | null>(null);
  const resourceKeyRef = useRef(resourceKey);
  const mutateRef = useRef(mutate);
  const onSuccessRef = useRef(onSuccess);
  const onErrorRef = useRef(onError);
  const onRevertRef = useRef(onRevert);

  useEffect(() => {
    mutateRef.current = mutate;
    onSuccessRef.current = onSuccess;
    onErrorRef.current = onError;
    onRevertRef.current = onRevert;
  }, [mutate, onError, onRevert, onSuccess]);

  const clearTimer = useCallback(() => {
    if (timerRef.current == null) return;
    clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const updatePendingState = useCallback(() => {
    if (!mountedRef.current) return;
    setHasPending(queueRef.current.size > 0 || inFlightRef.current != null);
  }, []);

  const drain = useCallback(async (): Promise<void> => {
    if (inFlightRef.current != null || queueRef.current.size === 0) {
      updatePendingState();
      return inFlightRef.current ?? Promise.resolve();
    }

    const [entryKey, entry] = queueRef.current.entries().next().value as [
      string,
      QueueEntry<TPayload>,
    ];
    queueRef.current.delete(entryKey);
    updatePendingState();
    if (mountedRef.current) {
      setStatus("saving");
      setError(null);
    }

    const request = (async () => {
      let succeeded = false;
      try {
        const result = await mutateRef.current(entry.payload);
        const isLatest = entry.sequence === latestSequenceRef.current;
        if (isLatest) {
          confirmedResultRef.current = result;
          failedEntryRef.current = null;
          if (mountedRef.current) onSuccessRef.current?.(result, entry.payload);
        }
        if (mountedRef.current) {
          setError(null);
        }
        succeeded = true;
      } catch (nextError) {
        failedEntryRef.current = queueRef.current.get(entry.resourceKey) ?? entry;
        if (mountedRef.current) {
          setStatus("error");
          setError(nextError);
        }
        if (mountedRef.current) onErrorRef.current?.(nextError, failedEntryRef.current.payload);
      } finally {
        inFlightRef.current = null;
        updatePendingState();
        if (succeeded && queueRef.current.size > 0) {
          if (mountedRef.current) {
            setStatus("pending");
          }
          void drain();
        } else if (succeeded && failedEntryRef.current == null && mountedRef.current) {
          setStatus("idle");
        }
      }
    })();
    inFlightRef.current = request;
    updatePendingState();
    await request;
  }, [updatePendingState]);

  const schedule = useCallback(
    (payload: TPayload) => {
      latestSequenceRef.current += 1;
      queueRef.current.set(resourceKeyId(resourceKeyRef.current), {
        resourceKey: resourceKeyId(resourceKeyRef.current),
        payload,
        sequence: latestSequenceRef.current,
      });
      failedEntryRef.current = null;
      clearTimer();
      if (mountedRef.current) {
        setStatus("pending");
        setError(null);
      }
      updatePendingState();
      if (inFlightRef.current != null) return;
      timerRef.current = setTimeout(
        () => {
          timerRef.current = null;
          void drain();
        },
        Math.max(0, delayMs),
      );
    },
    [clearTimer, delayMs, drain, updatePendingState],
  );

  const flush = useCallback(async (): Promise<void> => {
    clearTimer();
    while (inFlightRef.current != null || queueRef.current.size > 0) {
      if (inFlightRef.current != null) {
        await inFlightRef.current;
      } else if (queueRef.current.size > 0) {
        await drain();
      }
      if (failedEntryRef.current != null) break;
    }
  }, [clearTimer, drain]);

  const retry = useCallback(() => {
    const failedEntry = failedEntryRef.current;
    if (failedEntry == null || inFlightRef.current != null) return;
    failedEntryRef.current = null;
    queueRef.current.set(failedEntry.resourceKey, {
      ...failedEntry,
      payload: failedEntry.payload,
      sequence: ++latestSequenceRef.current,
    });
    if (mountedRef.current) {
      setStatus("pending");
      setError(null);
    }
    updatePendingState();
    timerRef.current = setTimeout(
      () => {
        timerRef.current = null;
        void drain();
      },
      Math.max(0, delayMs),
    );
  }, [delayMs, drain, updatePendingState]);

  const revert = useCallback(() => {
    if (inFlightRef.current != null) return;
    clearTimer();
    queueRef.current.clear();
    failedEntryRef.current = null;
    latestSequenceRef.current += 1;
    if (mountedRef.current) {
      setStatus("idle");
      setError(null);
      setHasPending(false);
    }
    onRevertRef.current?.(confirmedResultRef.current);
  }, [clearTimer]);

  const reconcile = useCallback((result: TResult) => {
    confirmedResultRef.current = result;
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      clearTimer();
      if (queueRef.current.size > 0 && inFlightRef.current == null) {
        void drain();
      }
    };
  }, [clearTimer, drain]);

  useEffect(() => {
    if (resourceKeyRef.current === resourceKey) return;
    const previousResourceKey = resourceKeyId(resourceKeyRef.current);
    resourceKeyRef.current = resourceKey;
    latestSequenceRef.current += 1;
    void flush().then(() => {
      queueRef.current.delete(previousResourceKey);
      if (failedEntryRef.current?.resourceKey === previousResourceKey) {
        failedEntryRef.current = null;
      }
      if (mountedRef.current) {
        setStatus("idle");
        setError(null);
        updatePendingState();
      }
    });
  }, [flush, resourceKey, updatePendingState]);

  useEffect(() => {
    const flushOnPageExit = () => {
      void flush();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "hidden") flushOnPageExit();
    };
    window.addEventListener("pagehide", flushOnPageExit);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("pagehide", flushOnPageExit);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [flush]);

  return { status, error, hasPending, schedule, flush, retry, revert, reconcile };
}
