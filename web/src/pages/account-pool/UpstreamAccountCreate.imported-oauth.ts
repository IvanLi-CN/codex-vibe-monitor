// biome-ignore-all lint/correctness/useExhaustiveDependencies: synchronization effects deliberately depend on mutable refs and preserve request ordering

import type { ChangeEvent, ClipboardEvent } from "react";
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
import { isExistingGroup, normalizeGroupName } from "../../lib/upstreamAccountGroups";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";

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
  convertImportedWebSessionDocumentLocally,
  createImportedOauthPastedFileName,
  createImportedOauthPastedSourceId,
  createImportedOauthSourceId,
  createImportedSessionPastedFileName,
  markImportedOauthRowsAsError,
  mergeImportedOauthValidationRow,
  mergeImportedOauthValidationRows,
  parseImportedOauthPasteDraft,
  replaceImportedOauthValidationRows,
  summarizeImportedOauthBatchErrors,
  validateImportedOauthCredentialLocally,
} from "./UpstreamAccountCreate.shared";

type LocalImportedOauthCandidate = {
  fileName: string;
  payload: ImportOauthCredentialFilePayload;
  matchKey: string;
};

export function useUpstreamAccountCreateImportedOauth(ctx: UpstreamAccountCreateControllerContext) {
  const {
    groupDraftConcurrencyLimits,
    groupDraftNotes,
    groups,
    importFiles,
    importFilesRef,
    importFilesRevisionRef,
    importFileSourceSequenceRef,
    importGroupName,
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
    importOauthAccounts,
    resolveRequiredGroupProxyState,
    resolveGroupSingleAccountRotationEnabledForName,
    activeTab,
    setActionError,
    setImportFiles,
    setImportInputKey,
    setImportPasteBusy,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteError,
    setImportSelectionFeedback,
    setImportValidationDialogOpen,
    setImportValidationState,
    startImportedOauthValidationJob,
    stopImportedOauthValidationJob,
    t,
    writesEnabled,
  } = ctx;
  const isImportingWebSession = activeTab === "importSession";

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
          const baselineRows = current?.rows ?? buildImportedOauthPendingState(allItems).rows;
          const mergedRows = merge
            ? nextRows.length === 1
              ? mergeImportedOauthValidationRow(baselineRows, nextRows[0]!, retriedSourceIds)
              : mergeImportedOauthValidationRows(baselineRows, nextRows, retriedSourceIds)
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
          const payload = normalizeImportedOauthValidationRowEventPayload(JSON.parse(message.data));
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
      eventSource.addEventListener("completed", handleCompleted as EventListener);
      eventSource.addEventListener("failed", handleFailed as EventListener);
      eventSource.addEventListener("cancelled", handleCancelled as EventListener);

      importValidationEventCleanupRef.current = () => {
        eventSource.removeEventListener("snapshot", handleSnapshot as EventListener);
        eventSource.removeEventListener("row", handleRow as EventListener);
        eventSource.removeEventListener("completed", handleCompleted as EventListener);
        eventSource.removeEventListener("failed", handleFailed as EventListener);
        eventSource.removeEventListener("cancelled", handleCancelled as EventListener);
      };
    },
    [closeImportValidationEventSource],
  );

  const runImportValidation = useCallback(
    async (items: ImportOauthCredentialFilePayload[], options?: { merge?: boolean }) => {
      if (items.length === 0) return;
      const currentImportGroupProxyState = resolveRequiredGroupProxyState(importGroupName);
      if (currentImportGroupProxyState.error) {
        setActionError(currentImportGroupProxyState.error);
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
          groupName: currentImportGroupProxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: currentImportGroupProxyState.boundProxyKeys,
          groupNodeShuntEnabled: currentImportGroupProxyState.nodeShuntEnabled,
          groupSingleAccountRotationEnabled:
            resolveGroupSingleAccountRotationEnabledForName(importGroupName),
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
      importGroupName,
      resolveGroupSingleAccountRotationEnabledForName,
      resolveRequiredGroupProxyState,
      startImportedOauthValidationJob,
    ],
  );

  const summarizeRejectedImportedOauthItems = useCallback(
    (
      rejected: Array<{
        fileName: string;
        reason: string;
        duplicate?: boolean;
      }>,
    ) => {
      if (rejected.length === 0) {
        setImportSelectionFeedback(null);
        return;
      }
      setImportSelectionFeedback({
        variant: rejected.some((item) => item.duplicate !== true) ? "error" : "warning",
        messages: rejected.map((item) =>
          item.duplicate
            ? t("accountPool.upstreamAccounts.import.local.duplicateSkipped", {
                fileName: item.fileName,
              })
            : t("accountPool.upstreamAccounts.import.local.fileRejected", {
                fileName: item.fileName,
                reason: item.reason,
              }),
        ),
      });
    },
    [t],
  );

  const collectQueuedImportedOauthMatchKeys = useCallback(() => {
    const matchKeys = new Set<string>();
    for (const item of importFilesRef.current as ImportOauthCredentialFilePayload[]) {
      const parsed = validateImportedOauthCredentialLocally(item.content, t);
      if (!parsed.ok || !parsed.matchKey) continue;
      matchKeys.add(parsed.matchKey);
    }
    return matchKeys;
  }, [importFilesRef, t]);

  const buildWebSessionCandidates = useCallback(
    ({
      content,
      fileName,
      createSourceId,
    }: {
      content: string;
      fileName: string;
      createSourceId: (index: number) => string;
    }) => {
      const parsed = convertImportedWebSessionDocumentLocally(content, t);
      if (!parsed.ok) {
        return {
          ok: false as const,
          error: parsed.error,
        };
      }
      return {
        ok: true as const,
        candidates: parsed.items.map((item, index) => ({
          fileName:
            parsed.items.length === 1
              ? fileName
              : `${fileName.replace(/\.json$/i, "")} session ${index + 1}.json`,
          matchKey: item.matchKey,
          payload: {
            sourceId: createSourceId(index),
            fileName:
              parsed.items.length === 1
                ? fileName
                : `${fileName.replace(/\.json$/i, "")} session ${index + 1}.json`,
            content: item.content,
          },
        })),
      };
    },
    [t],
  );

  const validateAndQueueImportedOauthPaste = useCallback(
    async (
      draftContent: string,
      options?: {
        serial?: number | null;
      },
    ) => {
      const normalizedDraft = draftContent.trim();
      const parsedDraft = isImportingWebSession
        ? null
        : parseImportedOauthPasteDraft(draftContent, t);
      const parsedSessionDraft = isImportingWebSession
        ? buildWebSessionCandidates({
            content: draftContent,
            fileName: createImportedSessionPastedFileName(
              options?.serial && options.serial > 0
                ? options.serial
                : importPasteSequenceRef.current + 1,
            ),
            createSourceId: (index) =>
              `${createImportedOauthPastedSourceId(
                options?.serial && options.serial > 0
                  ? options.serial
                  : importPasteSequenceRef.current + 1,
              )}:session:${index}`,
          })
        : null;
      if (parsedDraft && !parsedDraft.ok) {
        setImportPasteError(parsedDraft.error);
        return;
      }
      if (parsedSessionDraft && !parsedSessionDraft.ok) {
        setImportPasteError(parsedSessionDraft.error);
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
      setImportPasteDraftSerial(serial);
      setImportPasteBusy(true);
      setImportPasteError(null);
      setActionError(null);
      setImportSelectionFeedback(null);

      const items: Array<ImportOauthCredentialFilePayload & { matchKey: string }> = [];
      if (parsedSessionDraft?.ok) {
        items.push(
          ...parsedSessionDraft.candidates.map((candidate, index) => ({
            ...candidate.payload,
            sourceId: `${createImportedOauthPastedSourceId(serial)}:session:${index}`,
            fileName: createImportedSessionPastedFileName(
              serial,
              parsedSessionDraft.candidates.length === 1 ? undefined : index,
            ),
            matchKey: candidate.matchKey,
          })),
        );
      } else if (parsedDraft?.ok) {
        if (!parsedDraft.matchKey) {
          setImportPasteError(
            t("accountPool.upstreamAccounts.import.local.requiredField", {
              fieldName: "account_id",
            }),
          );
          return;
        }
        items.push({
          sourceId: createImportedOauthPastedSourceId(serial),
          fileName: createImportedOauthPastedFileName(serial),
          content: parsedDraft.normalizedContent,
          matchKey: parsedDraft.matchKey,
        });
      }

      try {
        const seenKeys = collectQueuedImportedOauthMatchKeys();
        for (const item of items) {
          if (item.matchKey && seenKeys.has(item.matchKey)) {
            setImportPasteError(t("accountPool.upstreamAccounts.import.local.pasteDuplicate"));
            return;
          }
          if (item.matchKey) seenKeys.add(item.matchKey);
        }
        await resetImportValidationForSelectionChange();
        if (
          validationToken !== importPasteValidationTokenRef.current ||
          importPasteDraftRef.current.trim() !== normalizedDraft
        ) {
          return;
        }
        const refreshedSeenKeys = collectQueuedImportedOauthMatchKeys();
        for (const item of items) {
          if (item.matchKey && refreshedSeenKeys.has(item.matchKey)) {
            setImportPasteError(t("accountPool.upstreamAccounts.import.local.pasteDuplicate"));
            return;
          }
          if (item.matchKey) refreshedSeenKeys.add(item.matchKey);
        }
        importFilesRevisionRef.current += 1;
        const nextItems = [
          ...(importFilesRef.current as ImportOauthCredentialFilePayload[]),
          ...items.map((item) => {
            const { matchKey, ...queuedItem } = item;
            void matchKey;
            return queuedItem;
          }),
        ];
        importFilesRef.current = nextItems;
        setImportFiles(nextItems);
        importPasteDraftRef.current = "";
        setImportPasteDraft("");
        setImportPasteDraftSerial(null);
        setImportPasteError(null);
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
      collectQueuedImportedOauthMatchKeys,
      buildWebSessionCandidates,
      importFilesRef,
      isImportingWebSession,
      resetImportValidationForSelectionChange,
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
        event.clipboardData.getData("text/plain") || event.clipboardData.getData("text");
      importPasteValidationTokenRef.current += 1;
      importPasteSequenceRef.current += 1;
      const serial = importPasteSequenceRef.current;
      importPasteDraftRef.current = nextDraft;
      setImportPasteDraft(nextDraft);
      setImportPasteDraftSerial(serial);
      setImportPasteBusy(false);
      setImportPasteError(null);
      setActionError(null);
      setImportSelectionFeedback(null);
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
  }, [importPasteDraft, importPasteDraftSerial, validateAndQueueImportedOauthPaste, writesEnabled]);

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
        const sourceIdOffset = importFileSourceSequenceRef.current;
        importFileSourceSequenceRef.current += selectedFiles.length;
        const parsedItems = await Promise.all(
          selectedFiles.map(async (file, index) => {
            const content = await file.text();
            const sourceIdBase = createImportedOauthSourceId(file, sourceIdOffset + index);
            if (isImportingWebSession) {
              const parsed = buildWebSessionCandidates({
                content,
                fileName: file.name,
                createSourceId: (sessionIndex) => `${sourceIdBase}:session:${sessionIndex}`,
              });
              return {
                fileName: file.name,
                parsed,
              };
            }
            const parsed = validateImportedOauthCredentialLocally(content, t);
            if (parsed.ok && !parsed.matchKey) {
              return {
                fileName: file.name,
                parsed: {
                  ok: false as const,
                  error: t("accountPool.upstreamAccounts.import.local.requiredField", {
                    fieldName: "account_id",
                  }),
                },
              };
            }
            if (parsed.ok) {
              const matchKey = parsed.matchKey as string;
              return {
                fileName: file.name,
                parsed: {
                  ok: true as const,
                  candidates: [
                    {
                      fileName: file.name,
                      matchKey,
                      payload: {
                        sourceId: sourceIdBase,
                        fileName: file.name,
                        content: parsed.normalizedContent,
                      },
                    },
                  ],
                },
              };
            }
            return {
              fileName: file.name,
              parsed,
            };
          }),
        );
        const validCandidates: LocalImportedOauthCandidate[] = [];
        const rejectedItems: Array<{
          fileName: string;
          reason: string;
          duplicate?: boolean;
        }> = [];

        for (const item of parsedItems) {
          if (!item.parsed.ok) {
            rejectedItems.push({
              fileName: item.fileName,
              reason: item.parsed.error,
            });
            continue;
          }
          validCandidates.push(...item.parsed.candidates);
        }

        const seenKeys = collectQueuedImportedOauthMatchKeys();
        const acceptedCandidates: LocalImportedOauthCandidate[] = [];
        for (const candidate of validCandidates) {
          if (seenKeys.has(candidate.matchKey)) {
            rejectedItems.push({
              fileName: candidate.fileName,
              reason: "",
              duplicate: true,
            });
            continue;
          }
          seenKeys.add(candidate.matchKey);
          acceptedCandidates.push(candidate);
        }
        if (acceptedCandidates.length > 0) {
          await resetImportValidationForSelectionChange();
          const refreshedSeenKeys = collectQueuedImportedOauthMatchKeys();
          const acceptedItems: ImportOauthCredentialFilePayload[] = [];
          for (const candidate of acceptedCandidates) {
            if (refreshedSeenKeys.has(candidate.matchKey)) {
              rejectedItems.push({
                fileName: candidate.fileName,
                reason: "",
                duplicate: true,
              });
              continue;
            }
            refreshedSeenKeys.add(candidate.matchKey);
            acceptedItems.push(candidate.payload);
          }
          if (acceptedItems.length === 0) {
            summarizeRejectedImportedOauthItems(rejectedItems);
            setImportInputKey((current: number) => current + 1);
            return;
          }
          importFilesRevisionRef.current += 1;
          const nextItems = [
            ...(importFilesRef.current as ImportOauthCredentialFilePayload[]),
            ...acceptedItems,
          ];
          importFilesRef.current = nextItems;
          setImportFiles(nextItems);
        }

        summarizeRejectedImportedOauthItems(rejectedItems);
        setImportInputKey((current: number) => current + 1);
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [
      collectQueuedImportedOauthMatchKeys,
      buildWebSessionCandidates,
      importFileSourceSequenceRef,
      importFilesRef,
      isImportingWebSession,
      resetImportValidationForSelectionChange,
      summarizeRejectedImportedOauthItems,
      t,
    ],
  );

  const handleClearImportSelection = useCallback(() => {
    void (async () => {
      importPasteValidationTokenRef.current += 1;
      setImportPasteBusy(false);
      await resetImportValidationForSelectionChange();
      importFilesRevisionRef.current += 1;
      importFilesRef.current = [];
      setImportFiles([]);
      setImportSelectionFeedback(null);
      setImportInputKey((current: number) => current + 1);
    })();
  }, [importFilesRef, resetImportValidationForSelectionChange]);

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
        .filter(
          (row: ImportedOauthValidationRow) => row.status === "invalid" || row.status === "error",
        )
        .map((row: ImportedOauthValidationRow) => row.sourceId),
    );
    if (failedSourceIds.size === 0) return;
    await runImportValidation(
      importFiles.filter((item: ImportOauthCredentialFilePayload) =>
        failedSourceIds.has(item.sourceId),
      ),
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
    const currentImportGroupProxyState = resolveRequiredGroupProxyState(importGroupName);
    if (currentImportGroupProxyState.error) {
      setActionError(currentImportGroupProxyState.error);
      setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
        current
          ? {
              ...current,
              importError: currentImportGroupProxyState.error,
            }
          : current,
      );
      return;
    }
    const currentRows = importValidationState?.rows ?? [];
    const validSourceIds = currentRows
      .filter(
        (row: ImportedOauthValidationRow) => row.status === "ok" || row.status === "ok_exhausted",
      )
      .map((row: ImportedOauthValidationRow) => row.sourceId);
    if (validSourceIds.length === 0) return;
    const validSourceIdSet = new Set(validSourceIds);
    const selectedItems = importFiles.filter((item: ImportOauthCredentialFilePayload) =>
      validSourceIdSet.has(item.sourceId),
    );
    const batches = chunkImportedOauthItems(selectedItems);
    const normalizedImportGroupName = normalizeGroupName(importGroupName);
    const importGroupNote =
      normalizedImportGroupName && !isExistingGroup(groups, normalizedImportGroupName)
        ? groupDraftNotes[normalizedImportGroupName]?.trim() || undefined
        : undefined;
    const importGroupConcurrencyLimit =
      normalizedImportGroupName && !isExistingGroup(groups, normalizedImportGroupName)
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
          groupName: currentImportGroupProxyState.normalizedGroupName || undefined,
          groupBoundProxyKeys: currentImportGroupProxyState.boundProxyKeys,
          groupNodeShuntEnabled: currentImportGroupProxyState.nodeShuntEnabled,
          groupSingleAccountRotationEnabled:
            resolveGroupSingleAccountRotationEnabledForName(importGroupName),
          groupNote: importGroupNote,
          concurrencyLimit: importGroupConcurrencyLimit,
          tagIds: importTagIds,
        });
        const importedSourceIds = new Set(
          response.results
            .filter(
              (result: ImportOauthAccountResult) =>
                result.status === "created" || result.status === "updated_existing",
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
        workingItems = workingItems.filter((item) => !importedSourceIds.has(item.sourceId));
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
        importFilesRef.current = workingItems;
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
        workingRows = markImportedOauthRowsAsError(workingRows, batchSourceIds, message);
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
    importFilesRef,
    importGroupName,
    importOauthAccounts,
    importTagIds,
    importValidationState?.rows,
    resolveRequiredGroupProxyState,
    resolveGroupSingleAccountRotationEnabledForName,
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
