/* eslint-disable react-hooks/exhaustive-deps */
import { useCallback, useEffect } from "react";
import type { ChangeEvent, ClipboardEvent } from "react";
import {
  createImportedOauthValidationJobEventSource,
  normalizeImportedOauthValidationFailedEventPayload,
  normalizeImportedOauthValidationRowEventPayload,
  normalizeImportedOauthValidationSnapshotEventPayload,
} from "../../lib/api";
import { isExistingGroup, normalizeGroupName } from "../../lib/upstreamAccountGroups";
import type {
  ImportOauthCredentialFilePayload,
  ImportedOauthValidationFailedEventPayload,
  ImportedOauthValidationRow,
  ImportedOauthValidationSnapshotEventPayload,
} from "../../lib/api";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import type { ImportedOauthValidationDialogState } from "../../components/ImportedOauthValidationDialog";
type ImportOauthAccountResult = {
  sourceId: string;
  status: string;
  detail?: string | null;
};

import {
  buildImportedOauthPendingState,
  buildImportedOauthStateFromRows,
  buildImportedOauthStateFromSnapshot,
  chunkImportedOauthItems,
  createImportedOauthPastedFileName,
  createImportedOauthPastedSourceId,
  createImportedOauthSourceId,
  getImportedOauthPasteValidationError,
  markImportedOauthRowsAsError,
  mergeImportedOauthValidationRow,
  mergeImportedOauthValidationRows,
  parseImportedOauthPasteDraft,
  replaceImportedOauthValidationRows,
  summarizeImportedOauthBatchErrors,
} from "./UpstreamAccountCreate.shared";

export function useUpstreamAccountCreateImportedOauth(
  ctx: UpstreamAccountCreateControllerContext,
) {
  const {
    groupDraftConcurrencyLimits,
    groupDraftNotes,
    groups,
    importFiles,
    importFilesRevisionRef,
    importFileSourceSequenceRef,
    importGroupName,
    importGroupProxyState,
    importPasteDraft,
    importPasteDraftRef,
    importPasteDraftSerial,
    importPasteSequenceRef,
    importPasteValidationTokenRef,
    importTagIds,
    importValidationEventCleanupRef,
    importValidationEventSourceRef,
    importValidationJobIdRef,
    importValidationState,
    persistDraftGroupSettings,
    runImportedOauthValidation,
    importOauthAccounts,
    setActionError,
    setImportFiles,
    setImportInputKey,
    setImportPasteBusy,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteError,
    setImportValidationDialogOpen,
    setImportValidationState,
    startImportedOauthValidationJob,
    stopImportedOauthValidationJob,
    t,
    writesEnabled,
  } = ctx;

  const closeImportValidationEventSource = useCallback(() => {
    importValidationEventCleanupRef.current?.();
    importValidationEventCleanupRef.current = null;
    importValidationEventSourceRef.current?.close();
    importValidationEventSourceRef.current = null;
  }, []);

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

  const attachImportedOauthValidationJob = useCallback(
    ({
      jobId,
      allItems,
      merge,
      retriedSourceIds,
    }: {
      jobId: string;
      allItems: ImportOauthCredentialFilePayload[];
      merge: boolean;
      retriedSourceIds: Set<string>;
    }) => {
      closeImportValidationEventSource();
      importValidationJobIdRef.current = jobId;
      const eventSource = createImportedOauthValidationJobEventSource(jobId);
      importValidationEventSourceRef.current = eventSource;

      const updateRows = (
        nextRows: ImportedOauthValidationRow[],
        options?: {
          checking?: boolean;
          importError?: string | null;
        },
      ) => {
        setImportValidationState((current: ImportedOauthValidationDialogState | null) => {
          const baselineRows =
            current?.rows ?? buildImportedOauthPendingState(allItems).rows;
          const mergedRows = merge
            ? nextRows.length === 1
              ? mergeImportedOauthValidationRow(
                  baselineRows,
                  nextRows[0]!,
                  retriedSourceIds,
                )
              : mergeImportedOauthValidationRows(
                  baselineRows,
                  nextRows,
                  retriedSourceIds,
                )
            : mergeImportedOauthValidationRows(
                baselineRows,
                nextRows,
                new Set(nextRows.map((row: ImportedOauthValidationRow) => row.sourceId)),
              );
          return {
            ...buildImportedOauthStateFromRows(mergedRows, allItems),
            checking: options?.checking ?? true,
            importing: false,
            importError: options?.importError ?? null,
          };
        });
      };

      const handleSnapshot = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationSnapshotEventPayload(
            JSON.parse(message.data),
          );
          if (merge) {
            setImportValidationState((current: ImportedOauthValidationDialogState | null) => {
              const baselineRows =
                current?.rows ?? buildImportedOauthPendingState(allItems).rows;
              return {
                ...buildImportedOauthStateFromRows(
                  replaceImportedOauthValidationRows(
                    baselineRows,
                    payload.snapshot.rows,
                  ),
                  allItems,
                ),
                checking: true,
                importing: false,
                importError: null,
              };
            });
            return;
          }
          setImportValidationState({
            ...buildImportedOauthStateFromSnapshot(payload.snapshot),
            checking: true,
            importing: false,
            importError: null,
          });
        } catch (err) {
          setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
            current
              ? {
                  ...current,
                  checking: false,
                  importing: false,
                  importError: err instanceof Error ? err.message : String(err),
                }
              : current,
          );
        }
      };

      const handleRow = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationRowEventPayload(
            JSON.parse(message.data),
          );
          updateRows([payload.row], { checking: true, importError: null });
        } catch (err) {
          setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
            current
              ? {
                  ...current,
                  checking: false,
                  importing: false,
                  importError: err instanceof Error ? err.message : String(err),
                }
              : current,
          );
        }
      };

      const finalizeValidation = (
        payload:
          | ImportedOauthValidationSnapshotEventPayload
          | ImportedOauthValidationFailedEventPayload,
        importError: string | null,
      ) => {
        closeImportValidationEventSource();
        if (merge) {
          updateRows(payload.snapshot.rows, {
            checking: false,
            importError,
          });
          return;
        }
        setImportValidationState({
          ...buildImportedOauthStateFromRows(payload.snapshot.rows, allItems),
          checking: false,
          importing: false,
          importError,
        });
      };

      const handleCompleted = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationSnapshotEventPayload(
            JSON.parse(message.data),
          );
          finalizeValidation(payload, null);
        } catch (err) {
          closeImportValidationEventSource();
          setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
            current
              ? {
                  ...current,
                  checking: false,
                  importing: false,
                  importError: err instanceof Error ? err.message : String(err),
                }
              : current,
          );
        }
      };

      const handleFailed = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationFailedEventPayload(
            JSON.parse(message.data),
          );
          finalizeValidation(payload, payload.error);
        } catch (err) {
          setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
            current
              ? {
                  ...current,
                  checking: false,
                  importing: false,
                  importError: err instanceof Error ? err.message : String(err),
                }
              : current,
          );
        }
      };

      const handleCancelled = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationSnapshotEventPayload(
            JSON.parse(message.data),
          );
          importValidationJobIdRef.current = null;
          finalizeValidation(payload, null);
        } catch {
          closeImportValidationEventSource();
          importValidationJobIdRef.current = null;
        }
      };

      eventSource.addEventListener("snapshot", handleSnapshot as EventListener);
      eventSource.addEventListener("row", handleRow as EventListener);
      eventSource.addEventListener(
        "completed",
        handleCompleted as EventListener,
      );
      eventSource.addEventListener("failed", handleFailed as EventListener);
      eventSource.addEventListener(
        "cancelled",
        handleCancelled as EventListener,
      );

      importValidationEventCleanupRef.current = () => {
        eventSource.removeEventListener(
          "snapshot",
          handleSnapshot as EventListener,
        );
        eventSource.removeEventListener("row", handleRow as EventListener);
        eventSource.removeEventListener(
          "completed",
          handleCompleted as EventListener,
        );
        eventSource.removeEventListener(
          "failed",
          handleFailed as EventListener,
        );
        eventSource.removeEventListener(
          "cancelled",
          handleCancelled as EventListener,
        );
      };
    },
    [closeImportValidationEventSource],
  );

  const runImportValidation = useCallback(
    async (
      items: ImportOauthCredentialFilePayload[],
      options?: { merge?: boolean },
    ) => {
      if (items.length === 0) return;
      if (importGroupProxyState.error) {
        setActionError(importGroupProxyState.error);
        return;
      }
      const merge = options?.merge === true;
      const retriedSourceIds = new Set(items.map((item) => item.sourceId));
      const allItems = merge ? importFiles : items;
      setImportValidationDialogOpen(true);
      try {
        if (importValidationJobIdRef.current) {
          await cancelActiveImportedOauthValidation({ closeDialog: false });
        } else {
          closeImportValidationEventSource();
        }
        const response = await startImportedOauthValidationJob({
          items,
          groupName: importGroupProxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: importGroupProxyState.boundProxyKeys,
          groupNodeShuntEnabled: importGroupProxyState.nodeShuntEnabled,
        });
        setImportValidationState((current: ImportedOauthValidationDialogState | null) => {
          if (merge && current) {
            return {
              ...buildImportedOauthStateFromRows(
                mergeImportedOauthValidationRows(
                  current.rows.map((row) =>
                    retriedSourceIds.has(row.sourceId)
                      ? {
                          ...row,
                          status: "pending",
                          detail: null,
                        }
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
        attachImportedOauthValidationJob({
          jobId: response.jobId,
          allItems,
          merge,
          retriedSourceIds,
        });
      } catch (err) {
        setImportValidationState((current: ImportedOauthValidationDialogState | null) => {
          const baseline = current ?? buildImportedOauthPendingState(allItems);
          const nextRows = merge
            ? markImportedOauthRowsAsError(
                baseline.rows,
                retriedSourceIds,
                err instanceof Error ? err.message : String(err),
              )
            : baseline.rows;
          return {
            ...buildImportedOauthStateFromRows(nextRows, allItems),
            checking: false,
            importing: false,
            importError: err instanceof Error ? err.message : String(err),
          };
        });
      }
    },
    [
      attachImportedOauthValidationJob,
      cancelActiveImportedOauthValidation,
      closeImportValidationEventSource,
      importFiles,
      importGroupProxyState.boundProxyKeys,
      importGroupProxyState.error,
      importGroupProxyState.nodeShuntEnabled,
      importGroupProxyState.normalizedGroupName,
      startImportedOauthValidationJob,
    ],
  );

  const validateAndQueueImportedOauthPaste = useCallback(
    async (
      draftContent: string,
      options?: {
        serial?: number | null;
      },
    ) => {
      const parsedDraft = parseImportedOauthPasteDraft(draftContent, t);
      if (!parsedDraft.ok) {
        setImportPasteError(parsedDraft.error);
        return;
      }
      if (importGroupProxyState.error) {
        setImportPasteError(importGroupProxyState.error);
        return;
      }
      const serial =
        options?.serial && options.serial > 0
          ? options.serial
          : (() => {
              importPasteSequenceRef.current += 1;
              return importPasteSequenceRef.current;
            })();
      const validationToken = importPasteValidationTokenRef.current + 1;
      importPasteValidationTokenRef.current = validationToken;
      const importFilesRevision = importFilesRevisionRef.current;
      setImportPasteDraftSerial(serial);
      setImportPasteBusy(true);
      setImportPasteError(null);
      setActionError(null);

      const item: ImportOauthCredentialFilePayload = {
        sourceId: createImportedOauthPastedSourceId(serial),
        fileName: createImportedOauthPastedFileName(serial),
        content: parsedDraft.normalizedContent,
      };

      try {
        const response = await runImportedOauthValidation({
          items: [item],
          groupName: importGroupProxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: importGroupProxyState.boundProxyKeys,
        });
        if (
          validationToken !== importPasteValidationTokenRef.current ||
          importFilesRevision !== importFilesRevisionRef.current ||
          importPasteDraftRef.current.trim() !== parsedDraft.normalizedContent
        ) {
          return;
        }
        const row = response.rows[0];
        if (!row || response.rows.length !== 1) {
          setImportPasteError(
            t("accountPool.upstreamAccounts.import.paste.unexpectedResponse"),
          );
          return;
        }
        if (row.status === "ok" || row.status === "ok_exhausted") {
          await resetImportValidationForSelectionChange();
          if (
            validationToken !== importPasteValidationTokenRef.current ||
            importFilesRevision !== importFilesRevisionRef.current ||
            importPasteDraftRef.current.trim() !== parsedDraft.normalizedContent
          ) {
            return;
          }
          importFilesRevisionRef.current += 1;
          setImportFiles((current: ImportOauthCredentialFilePayload[]) => [...current, item]);
          importPasteDraftRef.current = "";
          setImportPasteDraft("");
          setImportPasteDraftSerial(null);
          setImportPasteError(null);
          return;
        }
        setImportPasteError(getImportedOauthPasteValidationError(row, t));
      } catch (err) {
        if (validationToken === importPasteValidationTokenRef.current) {
          setImportPasteError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (validationToken === importPasteValidationTokenRef.current) {
          setImportPasteBusy(false);
        }
      }
    },
    [
      importGroupProxyState.boundProxyKeys,
      importGroupProxyState.error,
      importGroupProxyState.normalizedGroupName,
      resetImportValidationForSelectionChange,
      runImportedOauthValidation,
      t,
    ],
  );

  const handleImportedOauthPasteDraftChange = useCallback(
    (event: ChangeEvent<HTMLTextAreaElement>) => {
      importPasteValidationTokenRef.current += 1;
      importPasteDraftRef.current = event.target.value;
      setImportPasteDraft(event.target.value);
      setImportPasteBusy(false);
      setImportPasteError(null);
      setActionError(null);
    },
    [],
  );

  const handleImportedOauthPaste = useCallback(
    (event: ClipboardEvent<HTMLTextAreaElement>) => {
      event.preventDefault();
      const nextDraft =
        event.clipboardData.getData("text/plain") ||
        event.clipboardData.getData("text");
      importPasteValidationTokenRef.current += 1;
      importPasteSequenceRef.current += 1;
      const serial = importPasteSequenceRef.current;
      importPasteDraftRef.current = nextDraft;
      setImportPasteDraft(nextDraft);
      setImportPasteDraftSerial(serial);
      setImportPasteBusy(false);
      setImportPasteError(null);
      setActionError(null);
      void validateAndQueueImportedOauthPaste(nextDraft, {
        serial,
      });
    },
    [validateAndQueueImportedOauthPaste],
  );

  const handleValidateImportedOauthPasteDraft = useCallback(async () => {
    if (!writesEnabled) return;
    await validateAndQueueImportedOauthPaste(importPasteDraft, {
      serial: importPasteDraftSerial,
    });
  }, [
    importPasteDraft,
    importPasteDraftSerial,
    validateAndQueueImportedOauthPaste,
    writesEnabled,
  ]);

  const handleImportFilesChange = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const selectedFiles = Array.from(event.target.files ?? []);
      setActionError(null);
      if (selectedFiles.length === 0) {
        return;
      }
      try {
        importPasteValidationTokenRef.current += 1;
        setImportPasteBusy(false);
        await resetImportValidationForSelectionChange();
        const sourceIdOffset = importFileSourceSequenceRef.current;
        importFileSourceSequenceRef.current += selectedFiles.length;
        const items = await Promise.all(
          selectedFiles.map(async (file, index) => ({
            sourceId: createImportedOauthSourceId(file, sourceIdOffset + index),
            fileName: file.name,
            content: await file.text(),
          })),
        );
        importFilesRevisionRef.current += 1;
        setImportFiles((current: ImportOauthCredentialFilePayload[]) => [...current, ...items]);
        setImportInputKey((current: number) => current + 1);
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [resetImportValidationForSelectionChange],
  );

  const handleClearImportSelection = useCallback(() => {
    void (async () => {
      importPasteValidationTokenRef.current += 1;
      setImportPasteBusy(false);
      await resetImportValidationForSelectionChange();
      importFilesRevisionRef.current += 1;
      setImportFiles([]);
      setImportInputKey((current: number) => current + 1);
    })();
  }, [resetImportValidationForSelectionChange]);

  const handleValidateImportedOauth = useCallback(async () => {
    if (!writesEnabled || importFiles.length === 0) return;
    setActionError(null);
    await runImportValidation(importFiles);
  }, [importFiles, runImportValidation, writesEnabled]);

  const handleRetryImportedOauthOne = useCallback(
    async (sourceId: string) => {
      const item = importFiles.find(
        (candidate: ImportOauthCredentialFilePayload) => candidate.sourceId === sourceId,
      );
      if (!item) return;
      await runImportValidation([item], { merge: true });
    },
    [importFiles, runImportValidation],
  );

  const handleRetryImportedOauthFailed = useCallback(async () => {
    const failedSourceIds = new Set(
      (importValidationState?.rows ?? [])
        .filter((row: ImportedOauthValidationRow) => row.status === "invalid" || row.status === "error")
        .map((row: ImportedOauthValidationRow) => row.sourceId),
    );
    if (failedSourceIds.size === 0) return;
    await runImportValidation(
      importFiles.filter((item: ImportOauthCredentialFilePayload) => failedSourceIds.has(item.sourceId)),
      { merge: true },
    );
  }, [importFiles, importValidationState?.rows, runImportValidation]);

  const handleCloseImportedOauthValidationDialog = useCallback(() => {
    if (importValidationState?.importing) return;
    if (importValidationState?.checking) {
      void cancelActiveImportedOauthValidation({ closeDialog: true });
      return;
    }
    closeImportValidationEventSource();
    setImportValidationDialogOpen(false);
    setImportValidationState(null);
  }, [
    cancelActiveImportedOauthValidation,
    closeImportValidationEventSource,
    importValidationState?.checking,
    importValidationState?.importing,
  ]);

  useEffect(() => {
    importPasteDraftRef.current = importPasteDraft;
  }, [importPasteDraft]);

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

  const handleImportValidatedOauth = useCallback(async () => {
    if (importGroupProxyState.error) {
      setActionError(importGroupProxyState.error);
      setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
        current
          ? {
              ...current,
              importError: importGroupProxyState.error,
            }
          : current,
      );
      return;
    }
    const currentRows = importValidationState?.rows ?? [];
    const validSourceIds = currentRows
      .filter((row: ImportedOauthValidationRow) => row.status === "ok" || row.status === "ok_exhausted")
      .map((row: ImportedOauthValidationRow) => row.sourceId);
    if (validSourceIds.length === 0) return;
    const validSourceIdSet = new Set(validSourceIds);
    const selectedItems = importFiles.filter((item: ImportOauthCredentialFilePayload) =>
      validSourceIdSet.has(item.sourceId),
    );
    const batches = chunkImportedOauthItems(selectedItems);
    const normalizedImportGroupName = normalizeGroupName(importGroupName);
    const importGroupNote =
      normalizedImportGroupName &&
      !isExistingGroup(groups, normalizedImportGroupName)
        ? groupDraftNotes[normalizedImportGroupName]?.trim() || undefined
        : undefined;
    const importGroupConcurrencyLimit =
      normalizedImportGroupName &&
      !isExistingGroup(groups, normalizedImportGroupName)
        ? (groupDraftConcurrencyLimits[normalizedImportGroupName] ?? 0)
        : 0;
    const validationJobId = importValidationJobIdRef.current ?? undefined;
    let workingItems = [...importFiles];
    let workingRows = [...currentRows];
    let importedAny = false;
    const batchErrors: string[] = [];

    setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
      current
        ? {
            ...current,
            importing: true,
            importError: null,
          }
        : current,
    );
    for (const batch of batches) {
      const batchSourceIds = new Set(batch.map((item) => item.sourceId));
      try {
        const response = await importOauthAccounts({
          items: batch,
          selectedSourceIds: batch.map((item) => item.sourceId),
          validationJobId,
          groupName: importGroupProxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: importGroupProxyState.boundProxyKeys,
          groupNodeShuntEnabled: importGroupProxyState.nodeShuntEnabled,
          groupNote: importGroupNote,
          concurrencyLimit: importGroupConcurrencyLimit,
          tagIds: importTagIds,
        });
        const importedSourceIds = new Set(
          response.results
            .filter(
              (result: ImportOauthAccountResult) =>
                result.status === "created" ||
                result.status === "updated_existing",
            )
            .map((result: ImportOauthAccountResult) => result.sourceId),
        );
        const failedResultsBySourceId = new Map<string, ImportOauthAccountResult>(
          response.results
            .filter((result: ImportOauthAccountResult) => result.status === "failed")
            .map((result: ImportOauthAccountResult) => [result.sourceId, result] as const),
        );

        if (importedSourceIds.size > 0) {
          importedAny = true;
        }
        workingItems = workingItems.filter(
          (item) => !importedSourceIds.has(item.sourceId),
        );
        workingRows = workingRows
          .filter((row: ImportedOauthValidationRow) => !importedSourceIds.has(row.sourceId))
          .map((row: ImportedOauthValidationRow) => {
            const failedResult = failedResultsBySourceId.get(row.sourceId);
            if (!failedResult) return row;
            return {
              ...row,
              status: "error",
              detail: failedResult.detail ?? row.detail,
            };
          });

        importFilesRevisionRef.current += 1;
        setImportFiles(workingItems);
        setImportValidationState(() => {
          if (workingRows.length === 0) {
            return null;
          }
          return {
            ...buildImportedOauthStateFromRows(workingRows, workingItems),
            importing: true,
            importError: summarizeImportedOauthBatchErrors(batchErrors),
          };
        });
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        batchErrors.push(message);
        workingRows = markImportedOauthRowsAsError(
          workingRows,
          batchSourceIds,
          message,
        );
        setImportValidationState(() => {
          if (workingRows.length === 0) {
            return null;
          }
          return {
            ...buildImportedOauthStateFromRows(workingRows, workingItems),
            importing: true,
            importError: summarizeImportedOauthBatchErrors(batchErrors),
          };
        });
      }
    }
    if (importedAny) {
      try {
        await persistDraftGroupSettings(importGroupName);
      } catch (err) {
        batchErrors.push(err instanceof Error ? err.message : String(err));
      }
    }
    if (importedAny) {
      setImportInputKey((current: number) => current + 1);
    }
    setImportValidationState(() => {
      if (workingRows.length === 0) {
        return null;
      }
      return {
        ...buildImportedOauthStateFromRows(workingRows, workingItems),
        importing: false,
        importError: summarizeImportedOauthBatchErrors(batchErrors),
      };
    });
    if (workingRows.length === 0) {
      setImportValidationDialogOpen(false);
    }
  }, [
    groupDraftNotes,
    groupDraftConcurrencyLimits,
    groups,
    importFiles,
    importGroupName,
    importGroupProxyState.boundProxyKeys,
    importGroupProxyState.error,
    importGroupProxyState.normalizedGroupName,
    importOauthAccounts,
    importTagIds,
    importValidationState?.rows,
    persistDraftGroupSettings,
    t,
  ]);


  return {
    closeImportValidationEventSource,
    cancelActiveImportedOauthValidation,
    resetImportValidationForSelectionChange,
    attachImportedOauthValidationJob,
    runImportValidation,
    validateAndQueueImportedOauthPaste,
    handleImportedOauthPasteDraftChange,
    handleImportedOauthPaste,
    handleValidateImportedOauthPasteDraft,
    handleImportFilesChange,
    handleClearImportSelection,
    handleValidateImportedOauth,
    handleRetryImportedOauthOne,
    handleRetryImportedOauthFailed,
    handleCloseImportedOauthValidationDialog,
    handleImportValidatedOauth,
  };
}
