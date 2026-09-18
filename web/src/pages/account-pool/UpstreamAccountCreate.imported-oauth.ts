// biome-ignore-all lint/correctness/useExhaustiveDependencies: synchronization effects deliberately depend on mutable refs and preserve request ordering

import { useCallback } from "react";
import type { ImportedOauthValidationRow, ImportOauthCredentialFilePayload } from "../../lib/api";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import { useImportedOauthExecution } from "./UpstreamAccountCreate.imported-oauth-execution";
import { useImportedOauthProcessing } from "./UpstreamAccountCreate.imported-oauth-processing";
import { useImportedOauthValidation } from "./UpstreamAccountCreate.imported-oauth-validation";

export function useUpstreamAccountCreateImportedOauth(ctx: UpstreamAccountCreateControllerContext) {
  const validation = useImportedOauthValidation(ctx);
  const processing = useImportedOauthProcessing(ctx, validation);
  const execution = useImportedOauthExecution(ctx);
  const {
    importFiles,
    importValidationState,
    setActionError,
    setImportValidationDialogOpen,
    setImportValidationState,
    writesEnabled,
  } = ctx;
  const {
    cancelActiveImportedOauthValidation,
    closeImportValidationEventSource,
    runImportValidation,
  } = validation;

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
      if (item) await runImportValidation([item], { merge: true });
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

  return {
    ...validation,
    ...processing,
    ...execution,
    handleValidateImportedOauth,
    handleRetryImportedOauthOne,
    handleRetryImportedOauthFailed,
    handleCloseImportedOauthValidationDialog,
  };
}
