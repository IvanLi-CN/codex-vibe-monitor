// biome-ignore-all lint/correctness/useExhaustiveDependencies: synchronization effects deliberately depend on mutable refs and preserve request ordering

import type { Dispatch, SetStateAction } from "react";
import { useCallback, useEffect } from "react";
import type { ImportedOauthValidationDialogState } from "../../features/account-pool/ImportedOauthValidationDialog";
import type {
  ImportedOauthValidationFailedEventPayload,
  ImportedOauthValidationRow,
  ImportedOauthValidationSnapshotEventPayload,
  ImportOauthCredentialFilePayload,
} from "../../lib/api";
import {
  createImportedOauthValidationJobEventSource,
  normalizeImportedOauthValidationFailedEventPayload,
  normalizeImportedOauthValidationRowEventPayload,
  normalizeImportedOauthValidationSnapshotEventPayload,
} from "../../lib/api";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import {
  buildImportedOauthPendingState,
  buildImportedOauthStateFromRows,
  buildImportedOauthStateFromSnapshot,
  markImportedOauthRowsAsError,
  mergeImportedOauthValidationRows,
  replaceImportedOauthValidationRows,
} from "./UpstreamAccountCreate.shared";

type SetImportValidationState = Dispatch<SetStateAction<ImportedOauthValidationDialogState | null>>;

type ValidationContext = Pick<
  UpstreamAccountCreateControllerContext,
  | "importValidationEventCleanupRef"
  | "importValidationEventSourceRef"
  | "importValidationJobIdRef"
  | "setImportValidationDialogOpen"
  | "setImportValidationState"
  | "stopImportedOauthValidationJob"
>;

type AttachValidationJobOptions = {
  jobId: string;
  allItems: ImportOauthCredentialFilePayload[];
  merge: boolean;
  retriedSourceIds: Set<string>;
};

type ValidationHandlerOptions = AttachValidationJobOptions & {
  eventSource: EventSource;
  closeEventSource: () => void;
  setValidationState: SetImportValidationState;
  setEventSourceCleanup: (cleanup: () => void) => void;
  isCurrentJob: () => boolean;
  clearJobId: () => void;
};

type ValidationPayload =
  | ImportedOauthValidationSnapshotEventPayload
  | ImportedOauthValidationFailedEventPayload;

function importedOauthValidationErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function setImportedOauthValidationError(
  setValidationState: SetImportValidationState,
  error: unknown,
): void {
  setValidationState((current) =>
    current
      ? {
          ...current,
          checking: false,
          importing: false,
          importError: importedOauthValidationErrorMessage(error),
        }
      : current,
  );
}

function parseImportedOauthValidationEvent<T>(event: Event, normalize: (payload: unknown) => T): T {
  const message = event as MessageEvent<string>;
  return normalize(JSON.parse(message.data));
}

function createImportedOauthValidationRowUpdater({
  allItems,
  merge,
  retriedSourceIds,
  setValidationState,
}: Pick<
  ValidationHandlerOptions,
  "allItems" | "merge" | "retriedSourceIds" | "setValidationState"
>) {
  return (
    nextRows: ImportedOauthValidationRow[],
    options?: { checking?: boolean; importError?: string | null },
  ): void => {
    setValidationState((current) => {
      const baselineRows = current?.rows ?? buildImportedOauthPendingState(allItems).rows;
      const mergedRows = merge
        ? mergeImportedOauthValidationRows(baselineRows, nextRows, retriedSourceIds)
        : mergeImportedOauthValidationRows(
            baselineRows,
            nextRows,
            new Set(nextRows.map((row) => row.sourceId)),
          );
      return {
        ...buildImportedOauthStateFromRows(mergedRows, allItems),
        checking: options?.checking ?? true,
        importing: false,
        importError: options?.importError ?? null,
      };
    });
  };
}

function createSnapshotHandler({
  allItems,
  merge,
  isCurrentJob,
  setValidationState,
}: Pick<ValidationHandlerOptions, "allItems" | "merge" | "setValidationState"> & {
  isCurrentJob: () => boolean;
}) {
  return (event: Event): void => {
    if (!isCurrentJob()) return;
    try {
      const payload = parseImportedOauthValidationEvent(
        event,
        normalizeImportedOauthValidationSnapshotEventPayload,
      );
      if (merge) {
        setValidationState((current) => {
          const baselineRows = current?.rows ?? buildImportedOauthPendingState(allItems).rows;
          return {
            ...buildImportedOauthStateFromRows(
              replaceImportedOauthValidationRows(baselineRows, payload.snapshot.rows),
              allItems,
            ),
            checking: true,
            importing: false,
            importError: null,
          };
        });
        return;
      }
      setValidationState({
        ...buildImportedOauthStateFromSnapshot(payload.snapshot),
        checking: true,
        importing: false,
        importError: null,
      });
    } catch (error) {
      setImportedOauthValidationError(setValidationState, error);
    }
  };
}

function createRowHandler({
  isCurrentJob,
  updateRows,
  setValidationState,
}: {
  isCurrentJob: () => boolean;
  updateRows: ReturnType<typeof createImportedOauthValidationRowUpdater>;
  setValidationState: SetImportValidationState;
}) {
  return (event: Event): void => {
    if (!isCurrentJob()) return;
    try {
      const payload = parseImportedOauthValidationEvent(
        event,
        normalizeImportedOauthValidationRowEventPayload,
      );
      updateRows([payload.row], { checking: true, importError: null });
    } catch (error) {
      setImportedOauthValidationError(setValidationState, error);
    }
  };
}

function createFinalizer({
  allItems,
  merge,
  closeEventSource,
  setValidationState,
  updateRows,
}: Pick<
  ValidationHandlerOptions,
  "allItems" | "merge" | "closeEventSource" | "setValidationState"
> & {
  updateRows: ReturnType<typeof createImportedOauthValidationRowUpdater>;
}) {
  return (payload: ValidationPayload, importError: string | null): void => {
    closeEventSource();
    if (merge) {
      updateRows(payload.snapshot.rows, { checking: false, importError });
      return;
    }
    setValidationState({
      ...buildImportedOauthStateFromRows(payload.snapshot.rows, allItems),
      checking: false,
      importing: false,
      importError,
    });
  };
}

function createCompletedHandler({
  isCurrentJob,
  finalize,
  closeEventSource,
  setValidationState,
}: {
  isCurrentJob: () => boolean;
  finalize: (payload: ValidationPayload, importError: string | null) => void;
  closeEventSource: () => void;
  setValidationState: SetImportValidationState;
}) {
  return (event: Event): void => {
    if (!isCurrentJob()) return;
    try {
      const payload = parseImportedOauthValidationEvent(
        event,
        normalizeImportedOauthValidationSnapshotEventPayload,
      );
      finalize(payload, null);
    } catch (error) {
      closeEventSource();
      setImportedOauthValidationError(setValidationState, error);
    }
  };
}

function createFailedHandler({
  isCurrentJob,
  finalize,
  setValidationState,
}: {
  isCurrentJob: () => boolean;
  finalize: (payload: ValidationPayload, importError: string | null) => void;
  setValidationState: SetImportValidationState;
}) {
  return (event: Event): void => {
    if (!isCurrentJob()) return;
    try {
      const payload = parseImportedOauthValidationEvent(
        event,
        normalizeImportedOauthValidationFailedEventPayload,
      );
      finalize(payload, payload.error);
    } catch (error) {
      setImportedOauthValidationError(setValidationState, error);
    }
  };
}

function createCancelledHandler({
  isCurrentJob,
  finalize,
  closeEventSource,
  clearJobId,
}: {
  isCurrentJob: () => boolean;
  finalize: (payload: ValidationPayload, importError: string | null) => void;
  closeEventSource: () => void;
  clearJobId: () => void;
}) {
  return (event: Event): void => {
    if (!isCurrentJob()) return;
    try {
      const payload = parseImportedOauthValidationEvent(
        event,
        normalizeImportedOauthValidationSnapshotEventPayload,
      );
      clearJobId();
      finalize(payload, null);
    } catch {
      closeEventSource();
      clearJobId();
    }
  };
}

function createValidationEventHandlers(options: ValidationHandlerOptions) {
  const { closeEventSource, eventSource, setValidationState, setEventSourceCleanup } = options;
  const { isCurrentJob } = options;
  const updateRows = createImportedOauthValidationRowUpdater(options);
  const snapshot = createSnapshotHandler({
    allItems: options.allItems,
    merge: options.merge,
    isCurrentJob,
    setValidationState,
  });
  const row = createRowHandler({ isCurrentJob, updateRows, setValidationState });
  const finalize = createFinalizer({
    allItems: options.allItems,
    merge: options.merge,
    closeEventSource,
    setValidationState,
    updateRows,
  });
  const completed = createCompletedHandler({
    isCurrentJob,
    finalize,
    closeEventSource,
    setValidationState,
  });
  const failed = createFailedHandler({ isCurrentJob, finalize, setValidationState });
  const cancelled = createCancelledHandler({
    isCurrentJob,
    finalize,
    closeEventSource,
    clearJobId: options.clearJobId,
  });
  eventSource.addEventListener("snapshot", snapshot as EventListener);
  eventSource.addEventListener("row", row as EventListener);
  eventSource.addEventListener("completed", completed as EventListener);
  eventSource.addEventListener("failed", failed as EventListener);
  eventSource.addEventListener("cancelled", cancelled as EventListener);
  setEventSourceCleanup(() => {
    eventSource.removeEventListener("snapshot", snapshot as EventListener);
    eventSource.removeEventListener("row", row as EventListener);
    eventSource.removeEventListener("completed", completed as EventListener);
    eventSource.removeEventListener("failed", failed as EventListener);
    eventSource.removeEventListener("cancelled", cancelled as EventListener);
  });
}

function attachImportedOauthValidationJob(
  options: ValidationHandlerOptions & { setJobId: (jobId: string) => void },
): void {
  options.setJobId(options.jobId);
  createValidationEventHandlers(options);
}

function closeValidationEventSource(
  ctx: Pick<
    ValidationContext,
    "importValidationEventCleanupRef" | "importValidationEventSourceRef"
  >,
): void {
  ctx.importValidationEventCleanupRef.current?.();
  ctx.importValidationEventCleanupRef.current = null;
  ctx.importValidationEventSourceRef.current?.close();
  ctx.importValidationEventSourceRef.current = null;
}

function setInitialValidationState(
  setValidationState: SetImportValidationState,
  response: { snapshot: ImportedOauthValidationSnapshotEventPayload["snapshot"] },
  allItems: ImportOauthCredentialFilePayload[],
  merge: boolean,
  retriedSourceIds: Set<string>,
): void {
  setValidationState((current) => {
    if (merge && current) {
      return {
        ...buildImportedOauthStateFromRows(
          mergeImportedOauthValidationRows(
            current.rows.map((row) =>
              retriedSourceIds.has(row.sourceId)
                ? { ...row, status: "pending", detail: null }
                : row,
            ),
            response.snapshot.rows,
            retriedSourceIds,
          ),
          allItems,
        ),
        checking: true,
        importing: false,
        importError: null,
      };
    }
    return {
      ...buildImportedOauthStateFromSnapshot(response.snapshot),
      checking: true,
      importing: false,
      importError: null,
    };
  });
}

function setValidationRequestError(
  setValidationState: SetImportValidationState,
  error: unknown,
  allItems: ImportOauthCredentialFilePayload[],
  merge: boolean,
  retriedSourceIds: Set<string>,
): void {
  setValidationState((current) => {
    const baseline = current ?? buildImportedOauthPendingState(allItems);
    const message = importedOauthValidationErrorMessage(error);
    const nextRows = merge
      ? markImportedOauthRowsAsError(baseline.rows, retriedSourceIds, message)
      : baseline.rows;
    return {
      ...buildImportedOauthStateFromRows(nextRows, allItems),
      checking: false,
      importing: false,
      importError: message,
    };
  });
}

type ValidationCancellation = (options: { closeDialog: boolean }) => Promise<void>;
type ValidationAttachment = (options: AttachValidationJobOptions) => void;

function useImportedOauthValidationRunner(
  ctx: UpstreamAccountCreateControllerContext,
  closeEventSource: () => void,
  cancelValidation: ValidationCancellation,
  attachJob: ValidationAttachment,
) {
  const {
    importFiles,
    importGroupName,
    importValidationJobIdRef,
    setActionError,
    setImportValidationDialogOpen,
    setImportValidationState,
    startImportedOauthValidationJob,
    resolveRequiredGroupProxyState,
    resolveGroupSingleAccountRotationEnabledForName,
  } = ctx;
  return useCallback(
    async (items: ImportOauthCredentialFilePayload[], options?: { merge?: boolean }) => {
      if (items.length === 0) return;
      const proxyState = resolveRequiredGroupProxyState(importGroupName);
      if (proxyState.error) {
        setActionError(proxyState.error);
        return;
      }
      const merge = options?.merge === true;
      const retriedSourceIds = new Set(items.map((item) => item.sourceId));
      const allItems = merge ? importFiles : items;
      setImportValidationDialogOpen(true);
      try {
        if (importValidationJobIdRef.current) {
          await cancelValidation({ closeDialog: false });
        } else {
          closeEventSource();
        }
        const response = await startImportedOauthValidationJob({
          items,
          groupName: proxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: proxyState.boundProxyKeys,
          groupNodeShuntEnabled: proxyState.nodeShuntEnabled,
          groupSingleAccountRotationEnabled:
            resolveGroupSingleAccountRotationEnabledForName(importGroupName),
        });
        setInitialValidationState(
          setImportValidationState,
          response,
          allItems,
          merge,
          retriedSourceIds,
        );
        attachJob({ jobId: response.jobId, allItems, merge, retriedSourceIds });
      } catch (error) {
        setValidationRequestError(
          setImportValidationState,
          error,
          allItems,
          merge,
          retriedSourceIds,
        );
      }
    },
    [
      attachJob,
      cancelValidation,
      closeEventSource,
      importFiles,
      importGroupName,
      resolveGroupSingleAccountRotationEnabledForName,
      resolveRequiredGroupProxyState,
      startImportedOauthValidationJob,
    ],
  );
}

export function useImportedOauthValidation(ctx: UpstreamAccountCreateControllerContext) {
  const {
    importValidationEventCleanupRef,
    importValidationEventSourceRef,
    importValidationJobIdRef,
    setImportValidationDialogOpen,
    setImportValidationState,
    stopImportedOauthValidationJob,
  } = ctx;
  const closeImportValidationEventSource = useCallback(
    () =>
      closeValidationEventSource({
        importValidationEventCleanupRef,
        importValidationEventSourceRef,
      }),
    [importValidationEventCleanupRef, importValidationEventSourceRef],
  );
  const cancelActiveImportedOauthValidation = useCallback(
    async ({ closeDialog }: { closeDialog: boolean }) => {
      const jobId = importValidationJobIdRef.current;
      importValidationJobIdRef.current = null;
      closeImportValidationEventSource();
      if (closeDialog) {
        setImportValidationDialogOpen(false);
        setImportValidationState(null);
      }
      if (!jobId) return;
      try {
        await stopImportedOauthValidationJob(jobId);
      } catch {
        // Ignore cancellation failures; the local UI state is already closed.
      }
    },
    [closeImportValidationEventSource, stopImportedOauthValidationJob],
  );
  const resetImportValidationForSelectionChange = useCallback(async () => {
    if (importValidationJobIdRef.current) {
      await cancelActiveImportedOauthValidation({ closeDialog: true });
      return;
    }
    closeImportValidationEventSource();
    setImportValidationDialogOpen(false);
    setImportValidationState(null);
  }, [cancelActiveImportedOauthValidation, closeImportValidationEventSource]);
  const attachJob = useCallback(
    (options: AttachValidationJobOptions) => {
      closeImportValidationEventSource();
      const eventSource = createImportedOauthValidationJobEventSource(options.jobId);
      importValidationEventSourceRef.current = eventSource;
      attachImportedOauthValidationJob({
        ...options,
        eventSource,
        closeEventSource: closeImportValidationEventSource,
        setValidationState: setImportValidationState,
        setEventSourceCleanup: (cleanup) => {
          importValidationEventCleanupRef.current = cleanup;
        },
        setJobId: (jobId) => {
          importValidationJobIdRef.current = jobId;
        },
        isCurrentJob: () => importValidationJobIdRef.current === options.jobId,
        clearJobId: () => {
          importValidationJobIdRef.current = null;
        },
      });
    },
    [closeImportValidationEventSource],
  );
  const runImportValidation = useImportedOauthValidationRunner(
    ctx,
    closeImportValidationEventSource,
    cancelActiveImportedOauthValidation,
    attachJob,
  );
  useEffect(() => {
    return () => {
      const jobId = importValidationJobIdRef.current;
      importValidationJobIdRef.current = null;
      closeImportValidationEventSource();
      if (jobId) {
        void stopImportedOauthValidationJob(jobId).catch(() => {
          // Best-effort cleanup during unmount.
        });
      }
    };
  }, [closeImportValidationEventSource, stopImportedOauthValidationJob]);
  return {
    closeImportValidationEventSource,
    cancelActiveImportedOauthValidation,
    resetImportValidationForSelectionChange,
    attachImportedOauthValidationJob: attachJob,
    runImportValidation,
  };
}
