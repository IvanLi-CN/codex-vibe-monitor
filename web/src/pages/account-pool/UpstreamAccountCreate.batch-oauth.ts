/* eslint-disable react-hooks/exhaustive-deps */
import { useCallback, useEffect } from "react";
import type { KeyboardEvent } from "react";
import type { UpdateUpstreamAccountPayload } from "../../lib/api";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import {
  type BatchOauthPersistedMetadata,
  type BatchOauthRow,
  type CreateTab,
  MAX_SHARED_TAG_SYNC_ATTEMPTS,
  batchTagIdsEqual,
  buildBatchOauthPersistedMetadata,
  buildCompletedBatchOauthSharedTagBaselineSignature,
  canEditCompletedBatchOauthRowMetadata,
  createBatchOauthRow,
  didCompletedBatchOauthCommittedFieldsChange,
  enforceBatchMotherDraftUniqueness,
  findDisplayNameConflict,
  normalizeBatchTagIds,
  reconcileBatchOauthMotherRowsAfterSave,
  resolveCompletedBatchOauthRowBaselineTagIds,
  resolveCompletedBatchOauthRowPersistedTagIds,
  applyBatchMotherDraftRules,
} from "./UpstreamAccountCreate.shared";

export function useUpstreamAccountCreateBatchOauth(
  ctx: UpstreamAccountCreateControllerContext,
) {
  const {
    batchDefaultGroupName,
    batchRowIdRef,
    batchRows,
    batchRowsRef,
    batchMailboxToneResetRef,
    batchTagIds,
    batchSharedTagSyncEnabledRef,
    importValidationEventCleanupRef,
    importValidationEventSourceRef,
    importValidationJobIdRef,
    importValidationState,
    isRelinking,
    items,
    location,
    navigate,
    notifyMotherChange,
    previousBatchTagIdsRef,
    previousCompletedSharedTagBaselineRef,
    removeOauthMailboxSession,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    resolveRequiredGroupProxyState,
    saveAccount,
    setActiveTab,
    setBatchDefaultGroupName,
    setBatchManualCopyRowId,
    setBatchRows,
    setImportValidationDialogOpen,
    setImportValidationState,
    setOauthMailboxTone,
    oauthMailboxToneResetRef,
    stopImportedOauthValidationJob,
    t,
  } = ctx;

  const appendBatchRow = () => {
    const nextId = `row-${batchRowIdRef.current++}`;
    setBatchRows((current: BatchOauthRow[]) => [
      ...current,
      createBatchOauthRow(nextId, batchDefaultGroupName.trim()),
    ]);
  };

  const scheduleSingleMailboxToneReset = useCallback(() => {
    if (oauthMailboxToneResetRef.current != null) {
      window.clearTimeout(oauthMailboxToneResetRef.current);
    }
    oauthMailboxToneResetRef.current = window.setTimeout(() => {
      setOauthMailboxTone("idle");
      oauthMailboxToneResetRef.current = null;
    }, 1600);
  }, []);

  const updateBatchRow = (
    rowId: string,
    updater: (row: BatchOauthRow) => BatchOauthRow,
  ) => {
    setBatchRows((current: BatchOauthRow[]) =>
      enforceBatchMotherDraftUniqueness(
        applyBatchMotherDraftRules(
          current.map((row: BatchOauthRow) => (row.id === rowId ? updater(row) : row)),
          rowId,
        ),
      ),
    );
  };

  const scheduleBatchMailboxToneReset = useCallback(
    (rowId: string) => {
      const currentTimer = batchMailboxToneResetRef.current[rowId];
      if (currentTimer != null) {
        window.clearTimeout(currentTimer);
      }
      batchMailboxToneResetRef.current[rowId] = window.setTimeout(() => {
        updateBatchRow(rowId, (current: BatchOauthRow) => ({
          ...current,
          mailboxTone: "idle",
        }));
        delete batchMailboxToneResetRef.current[rowId];
      }, 1600);
    },
    [updateBatchRow],
  );

  const removeBatchRow = (rowId: string) => {
    const mailboxSessionId = batchRows.find((row: BatchOauthRow) => row.id === rowId)
      ?.mailboxSession?.sessionId;
    setBatchRows((current: BatchOauthRow[]) => {
      const remaining = current.filter((row: BatchOauthRow) => row.id !== rowId);
      return remaining.length > 0
        ? remaining
        : [
            createBatchOauthRow(
              `row-${batchRowIdRef.current++}`,
              batchDefaultGroupName.trim(),
            ),
          ];
    });
    setBatchManualCopyRowId((current: string | null) => (current === rowId ? null : current));
    if (mailboxSessionId) {
      void removeOauthMailboxSession(mailboxSessionId).catch(() => undefined);
    }
  };

  const toggleBatchNoteExpanded = (rowId: string) => {
    updateBatchRow(rowId, (row: BatchOauthRow) => ({
      ...row,
      noteExpanded: !row.noteExpanded,
    }));
  };

  async function persistCompletedBatchRowMetadata(
    rowId: string,
    overrides: Partial<BatchOauthPersistedMetadata>,
    committedFields: Array<keyof BatchOauthPersistedMetadata>,
  ) {
    const sourceRow = batchRowsRef.current.find((item: BatchOauthRow) => item.id === rowId);
    if (
      !sourceRow ||
      !canEditCompletedBatchOauthRowMetadata(sourceRow) ||
      sourceRow.metadataBusy
    ) {
      return;
    }
    const accountId = sourceRow.session?.accountId;
    if (accountId == null) return;

    const nextMetadata: BatchOauthPersistedMetadata = {
      displayName: overrides.displayName ?? sourceRow.displayName.trim(),
      groupName: overrides.groupName ?? sourceRow.groupName.trim(),
      note: overrides.note ?? sourceRow.note.trim(),
      isMother: overrides.isMother ?? sourceRow.isMother,
      tagIds: normalizeBatchTagIds(
        overrides.tagIds ??
          sourceRow.pendingSharedTagIds ??
          resolveCompletedBatchOauthRowBaselineTagIds(
            sourceRow,
            items,
            batchTagIds,
          ),
      ),
    };
    const isPendingSharedTagSyncAttempt =
      sourceRow.pendingSharedTagIds != null &&
      batchTagIdsEqual(sourceRow.pendingSharedTagIds, nextMetadata.tagIds);

    if (
      committedFields.includes("displayName") &&
      nextMetadata.displayName &&
      findDisplayNameConflict(items, nextMetadata.displayName, accountId)
    ) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        metadataError: t(
          "accountPool.upstreamAccounts.validation.displayNameDuplicate",
        ),
      }));
      return;
    }

    if (
      !didCompletedBatchOauthCommittedFieldsChange(
        sourceRow,
        nextMetadata,
        committedFields,
        items,
      )
    ) {
      updateBatchRow(rowId, (current: BatchOauthRow) =>
        current.metadataError
          ? {
              ...current,
              metadataError: null,
            }
          : current,
      );
      return;
    }

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      metadataBusy: true,
      metadataError: null,
      sharedTagSyncAttempts:
        committedFields.includes("tagIds") && isPendingSharedTagSyncAttempt
          ? current.sharedTagSyncAttempts + 1
          : current.sharedTagSyncAttempts,
    }));

    try {
      const payload: UpdateUpstreamAccountPayload = {};
      if (committedFields.includes("displayName")) {
        payload.displayName = nextMetadata.displayName || undefined;
      }
      if (committedFields.includes("groupName")) {
        const groupProxyState = resolveRequiredGroupProxyState(
          nextMetadata.groupName,
        );
        if (groupProxyState.error) {
          throw new Error(groupProxyState.error);
        }
        payload.groupName = groupProxyState.normalizedGroupName;
        payload.groupBoundProxyKeys = groupProxyState.boundProxyKeys;
        payload.concurrencyLimit = resolvePendingGroupConcurrencyLimitForName(
          nextMetadata.groupName,
        );
        payload.groupNodeShuntEnabled = groupProxyState.nodeShuntEnabled;
        payload.groupNote =
          resolvePendingGroupNoteForName(nextMetadata.groupName) || undefined;
      }
      if (committedFields.includes("note")) {
        payload.note = nextMetadata.note;
      }
      if (committedFields.includes("isMother")) {
        payload.isMother = nextMetadata.isMother;
      }
      if (committedFields.includes("tagIds")) {
        payload.tagIds = nextMetadata.tagIds;
      }
      const detail = await saveAccount(accountId, payload);
      notifyMotherChange(detail);
      setBatchRows((currentRows: BatchOauthRow[]) => {
        const nextRows = currentRows.map((current: BatchOauthRow) => {
          if (current.id !== rowId) return current;
          const nextPersisted = buildBatchOauthPersistedMetadata(
            {
              displayName: detail.displayName,
              groupName: detail.groupName ?? "",
              note: detail.note ?? "",
              isMother: detail.isMother === true,
            },
            (detail.tags ?? []).map((tag: { id: number }) => tag.id),
          );
          const pendingSharedTagIds =
            current.pendingSharedTagIds &&
            batchTagIdsEqual(current.pendingSharedTagIds, nextPersisted.tagIds)
              ? null
              : current.pendingSharedTagIds;
          return {
            ...current,
            displayName: committedFields.includes("displayName")
              ? detail.displayName
              : current.displayName,
            groupName: committedFields.includes("groupName")
              ? (detail.groupName ?? "")
              : current.groupName,
            note: committedFields.includes("note")
              ? (detail.note ?? "")
              : current.note,
            isMother: detail.isMother === true,
            metadataBusy: false,
            metadataError: null,
            metadataPersisted: nextPersisted,
            pendingSharedTagIds,
            sharedTagSyncAttempts: pendingSharedTagIds
              ? current.sharedTagSyncAttempts
              : 0,
            needsRefresh: false,
            actionError: null,
            session: current.session
              ? {
                  ...current.session,
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  accountId: detail.id,
                  error: null,
                }
              : current.session,
            sessionHint:
              current.needsRefresh || committedFields.includes("displayName")
                ? t("accountPool.upstreamAccounts.batchOauth.completed", {
                    name:
                      detail.displayName ||
                      current.displayName ||
                      `#${detail.id}`,
                  })
                : current.sessionHint,
            duplicateWarning: detail.duplicateInfo
              ? {
                  accountId: detail.id,
                  displayName: detail.displayName,
                  peerAccountIds: detail.duplicateInfo.peerAccountIds,
                  reasons: detail.duplicateInfo.reasons,
                }
              : null,
          };
        });
        return reconcileBatchOauthMotherRowsAfterSave(nextRows, rowId, detail);
      });
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        metadataBusy: false,
        metadataError: err instanceof Error ? err.message : String(err),
      }));
    }
  }

  const handleBatchMetadataChange = (
    rowId: string,
    field: "displayName" | "groupName" | "note" | "callbackUrl",
    value: string,
  ) => {
    updateBatchRow(rowId, (row: BatchOauthRow) => {
      if (row.busyAction || row.mailboxBusyAction || row.metadataBusy) {
        return row;
      }
      const nextRow = {
        ...row,
        [field]: value,
        metadataError:
          canEditCompletedBatchOauthRowMetadata(row) && field !== "callbackUrl"
            ? null
            : row.metadataError,
      };
      return {
        ...nextRow,
        actionError: null,
      };
    });
  };

  const handleBatchCompletedTextFieldBlur = (
    rowId: string,
    field: "displayName" | "note",
  ) => {
    const row = batchRowsRef.current.find((item: BatchOauthRow) => item.id === rowId);
    if (!row || !canEditCompletedBatchOauthRowMetadata(row)) return;
    if (field === "displayName") {
      void persistCompletedBatchRowMetadata(
        rowId,
        { displayName: row.displayName.trim() },
        ["displayName"],
      );
      return;
    }
    void persistCompletedBatchRowMetadata(rowId, { note: row.note.trim() }, [
      "note",
    ]);
  };

  const handleBatchCompletedTextFieldKeyDown = (
    event: KeyboardEvent<HTMLInputElement>,
  ) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    event.currentTarget.blur();
  };

  const handleBatchGroupValueChange = (rowId: string, value: string) => {
    const row = batchRowsRef.current.find((item: BatchOauthRow) => item.id === rowId);
    if (!row) return;
    const normalizedValue = value.trim();
    const nextGroupName = normalizedValue || batchDefaultGroupName.trim();
    updateBatchRow(rowId, (current: BatchOauthRow) => {
      if (
        current.busyAction ||
        current.mailboxBusyAction ||
        current.metadataBusy
      ) {
        return current;
      }
      return {
        ...current,
        groupName: nextGroupName,
        inheritsDefaultGroup: normalizedValue === "",
        metadataError: canEditCompletedBatchOauthRowMetadata(current)
          ? null
          : current.metadataError,
        actionError: null,
      };
    });
    if (!canEditCompletedBatchOauthRowMetadata(row)) return;
    void persistCompletedBatchRowMetadata(rowId, { groupName: nextGroupName }, [
      "groupName",
    ]);
  };

  const handleBatchMotherToggle = (rowId: string) => {
    const row = batchRowsRef.current.find((item: BatchOauthRow) => item.id === rowId);
    if (!row || row.busyAction || row.mailboxBusyAction || row.metadataBusy) {
      return;
    }
    const nextIsMother = !row.isMother;
    if (!canEditCompletedBatchOauthRowMetadata(row)) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        isMother: nextIsMother,
      }));
      return;
    }
    void persistCompletedBatchRowMetadata(rowId, { isMother: nextIsMother }, [
      "isMother",
    ]);
  };

  const handleBatchDefaultGroupChange = (value: string) => {
    const nextTrimmed = value.trim();
    const completedRowIdsToPersist: string[] = [];

    const nextRows = enforceBatchMotherDraftUniqueness(
      batchRows.map((row: BatchOauthRow) => {
        if (row.busyAction || row.mailboxBusyAction || row.metadataBusy) {
          return row;
        }
        if (
          row.session?.status === "completed" &&
          !canEditCompletedBatchOauthRowMetadata(row)
        ) {
          return row;
        }
        if (!row.inheritsDefaultGroup) return row;
        if (canEditCompletedBatchOauthRowMetadata(row)) {
          completedRowIdsToPersist.push(row.id);
          return {
            ...row,
            groupName: nextTrimmed,
            inheritsDefaultGroup: true,
            metadataError: null,
          };
        }
        return {
          ...row,
          groupName: nextTrimmed,
          inheritsDefaultGroup: true,
          actionError: null,
        };
      }),
    );

    setBatchDefaultGroupName(value);
    setBatchRows(nextRows);
    completedRowIdsToPersist.forEach((rowId) => {
      void persistCompletedBatchRowMetadata(rowId, { groupName: nextTrimmed }, [
        "groupName",
      ]);
    });
  };

  useEffect(() => {
    const normalizedBatchTagIds = normalizeBatchTagIds(batchTagIds);
    const previousBatchTagIds = previousBatchTagIdsRef.current;
    const baselineSignature =
      buildCompletedBatchOauthSharedTagBaselineSignature(batchRows, items);
    if (!batchSharedTagSyncEnabledRef.current) {
      previousBatchTagIdsRef.current = normalizedBatchTagIds;
      previousCompletedSharedTagBaselineRef.current = baselineSignature;
      return;
    }
    if (
      previousBatchTagIds != null &&
      batchTagIdsEqual(normalizedBatchTagIds, previousBatchTagIds) &&
      baselineSignature === previousCompletedSharedTagBaselineRef.current
    ) {
      return;
    }
    previousBatchTagIdsRef.current = normalizedBatchTagIds;
    previousCompletedSharedTagBaselineRef.current = baselineSignature;
    setBatchRows((current: BatchOauthRow[]) => {
      let changed = false;
      const nextRows = current.map((row: BatchOauthRow) => {
        if (!canEditCompletedBatchOauthRowMetadata(row)) return row;
        const persistedTagIds = resolveCompletedBatchOauthRowPersistedTagIds(
          row,
          items,
        );
        const nextPendingSharedTagIds = batchTagIdsEqual(
          normalizedBatchTagIds,
          persistedTagIds,
        )
          ? null
          : normalizedBatchTagIds;
        if (
          batchTagIdsEqual(row.pendingSharedTagIds, nextPendingSharedTagIds) &&
          row.sharedTagSyncAttempts === 0 &&
          (nextPendingSharedTagIds == null || row.metadataError == null)
        ) {
          return row;
        }
        changed = true;
        return {
          ...row,
          pendingSharedTagIds: nextPendingSharedTagIds,
          sharedTagSyncAttempts: 0,
          metadataError: nextPendingSharedTagIds ? null : row.metadataError,
        };
      });
      return changed ? nextRows : current;
    });
  }, [batchRows, batchTagIds, items]);

  useEffect(() => {
    batchRows.forEach((row: BatchOauthRow) => {
      if (
        !canEditCompletedBatchOauthRowMetadata(row) ||
        row.metadataBusy ||
        row.pendingSharedTagIds == null ||
        row.sharedTagSyncAttempts >= MAX_SHARED_TAG_SYNC_ATTEMPTS ||
        batchTagIdsEqual(row.pendingSharedTagIds, row.metadataPersisted?.tagIds)
      ) {
        return;
      }
      void persistCompletedBatchRowMetadata(
        row.id,
        { tagIds: row.pendingSharedTagIds },
        ["tagIds"],
      );
    });
  }, [batchRows]);

  const handleTabChange = (tab: CreateTab) => {
    if (tab !== "import" && importValidationState?.checking) {
      void (async () => {
        const jobId = importValidationJobIdRef.current;
        importValidationJobIdRef.current = null;
        importValidationEventCleanupRef.current?.();
        importValidationEventCleanupRef.current = null;
        importValidationEventSourceRef.current?.close();
        importValidationEventSourceRef.current = null;
        if (jobId) {
          try {
            await stopImportedOauthValidationJob(jobId);
          } catch {
            // ignore best-effort cancellation while leaving the page
          }
        }
      })();
      setImportValidationDialogOpen(false);
      setImportValidationState(null);
    }
    setActiveTab(tab);
    if (isRelinking) return;
    const search = tab === "oauth" ? "?mode=oauth" : `?mode=${tab}`;
    navigate(`${location.pathname}${search}`, { replace: true });
  };


  return {
    appendBatchRow,
    scheduleSingleMailboxToneReset,
    updateBatchRow,
    scheduleBatchMailboxToneReset,
    removeBatchRow,
    toggleBatchNoteExpanded,
    persistCompletedBatchRowMetadata,
    handleBatchMetadataChange,
    handleBatchCompletedTextFieldBlur,
    handleBatchCompletedTextFieldKeyDown,
    handleBatchGroupValueChange,
    handleBatchMotherToggle,
    handleBatchDefaultGroupChange,
    handleTabChange,
  };
}
