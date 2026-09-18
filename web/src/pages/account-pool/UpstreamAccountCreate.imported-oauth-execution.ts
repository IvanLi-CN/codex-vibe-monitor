import { useCallback } from "react";
import type { ImportedOauthValidationDialogState } from "../../features/account-pool/ImportedOauthValidationDialog";
import type { ImportedOauthValidationRow, ImportOauthCredentialFilePayload } from "../../lib/api";
import { isExistingGroup, normalizeGroupName } from "../../lib/upstreamAccountGroups";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import {
  buildImportedOauthStateFromRows,
  chunkImportedOauthItems,
  markImportedOauthRowsAsError,
  summarizeImportedOauthBatchErrors,
} from "./UpstreamAccountCreate.shared";

type ImportOauthAccountResult = {
  sourceId: string;
  status: string;
  detail?: string | null;
};

type PreparedImport = {
  batches: ImportOauthCredentialFilePayload[][];
  currentRows: ImportedOauthValidationRow[];
  currentImportGroupProxyState: ReturnType<
    UpstreamAccountCreateControllerContext["resolveRequiredGroupProxyState"]
  >;
  importGroupNote?: string;
  importGroupConcurrencyLimit: number;
};

type WorkingImport = {
  workingItems: ImportOauthCredentialFilePayload[];
  workingRows: ImportedOauthValidationRow[];
  importedAny: boolean;
  batchErrors: string[];
};

function importedOauthErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function prepareImportedOauthImport(
  ctx: UpstreamAccountCreateControllerContext,
): PreparedImport | null {
  const {
    groupDraftConcurrencyLimits,
    groupDraftNotes,
    groups,
    importFiles,
    importGroupName,
    importValidationState,
    resolveRequiredGroupProxyState,
  } = ctx;
  const currentImportGroupProxyState = resolveRequiredGroupProxyState(importGroupName);
  if (currentImportGroupProxyState.error) return null;
  const currentRows = importValidationState?.rows ?? [];
  const selectedItems = importFiles.filter((item: ImportOauthCredentialFilePayload) =>
    currentRows.some(
      (row: ImportedOauthValidationRow) =>
        row.sourceId === item.sourceId && (row.status === "ok" || row.status === "ok_exhausted"),
    ),
  );
  if (selectedItems.length === 0) return null;
  const normalizedImportGroupName = normalizeGroupName(importGroupName);
  const isDraftGroup =
    normalizedImportGroupName && !isExistingGroup(groups, normalizedImportGroupName);
  return {
    batches: chunkImportedOauthItems(selectedItems),
    currentRows,
    currentImportGroupProxyState,
    importGroupNote: isDraftGroup
      ? groupDraftNotes[normalizedImportGroupName]?.trim() || undefined
      : undefined,
    importGroupConcurrencyLimit: isDraftGroup
      ? (groupDraftConcurrencyLimits[normalizedImportGroupName] ?? 0)
      : 0,
  };
}

function classifyImportedOauthBatchResults(results: ImportOauthAccountResult[]): {
  importedSourceIds: Set<string>;
  failedResultsBySourceId: Map<string, ImportOauthAccountResult>;
} {
  return {
    importedSourceIds: new Set(
      results
        .filter((result) => result.status === "created" || result.status === "updated_existing")
        .map((result) => result.sourceId),
    ),
    failedResultsBySourceId: new Map(
      results
        .filter((result) => result.status === "failed")
        .map((result) => [result.sourceId, result] as const),
    ),
  };
}

function reconcileImportedOauthBatch({
  workingItems,
  workingRows,
  results,
}: {
  workingItems: ImportOauthCredentialFilePayload[];
  workingRows: ImportedOauthValidationRow[];
  results: ImportOauthAccountResult[];
}): {
  workingItems: ImportOauthCredentialFilePayload[];
  workingRows: ImportedOauthValidationRow[];
  importedAny: boolean;
} {
  const { importedSourceIds, failedResultsBySourceId } = classifyImportedOauthBatchResults(results);
  return {
    importedAny: importedSourceIds.size > 0,
    workingItems: workingItems.filter((item) => !importedSourceIds.has(item.sourceId)),
    workingRows: workingRows
      .filter((row) => !importedSourceIds.has(row.sourceId))
      .map((row) => {
        const failedResult = failedResultsBySourceId.get(row.sourceId);
        return failedResult
          ? { ...row, status: "error", detail: failedResult.detail ?? row.detail }
          : row;
      }),
  };
}

function publishImportedOauthProgress(
  ctx: UpstreamAccountCreateControllerContext,
  working: WorkingImport,
  importing: boolean,
  persistItems = true,
): void {
  if (persistItems) {
    ctx.importFilesRevisionRef.current += 1;
    ctx.importFilesRef.current = working.workingItems;
    ctx.setImportFiles(working.workingItems);
  }
  ctx.setImportValidationState(() => {
    if (working.workingRows.length === 0) return null;
    return {
      ...buildImportedOauthStateFromRows(working.workingRows, working.workingItems),
      importing,
      importError: summarizeImportedOauthBatchErrors(working.batchErrors),
    };
  });
}

async function importOauthBatch(
  ctx: UpstreamAccountCreateControllerContext,
  prepared: PreparedImport,
  batch: ImportOauthCredentialFilePayload[],
  validationJobId: string | undefined,
): Promise<ImportOauthAccountResult[]> {
  return (
    await ctx.importOauthAccounts({
      items: batch,
      selectedSourceIds: batch.map((item) => item.sourceId),
      validationJobId,
      groupName: prepared.currentImportGroupProxyState.normalizedGroupName || undefined,
      groupBoundProxyKeys: prepared.currentImportGroupProxyState.boundProxyKeys,
      groupNodeShuntEnabled: prepared.currentImportGroupProxyState.nodeShuntEnabled,
      groupSingleAccountRotationEnabled: ctx.resolveGroupSingleAccountRotationEnabledForName(
        ctx.importGroupName,
      ),
      groupNote: prepared.importGroupNote,
      concurrencyLimit: prepared.importGroupConcurrencyLimit,
      tagIds: ctx.importTagIds,
    })
  ).results;
}

async function executeImportedOauthImport(
  ctx: UpstreamAccountCreateControllerContext,
): Promise<void> {
  const prepared = prepareImportedOauthImport(ctx);
  if (!prepared) {
    const proxyState = ctx.resolveRequiredGroupProxyState(ctx.importGroupName);
    if (proxyState.error) {
      ctx.setActionError(proxyState.error);
      ctx.setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
        current ? { ...current, importError: proxyState.error } : current,
      );
    }
    return;
  }
  const validationJobId = ctx.importValidationJobIdRef.current ?? undefined;
  const working: WorkingImport = {
    workingItems: [...ctx.importFiles],
    workingRows: [...prepared.currentRows],
    importedAny: false,
    batchErrors: [],
  };
  ctx.setImportValidationState((current: ImportedOauthValidationDialogState | null) =>
    current ? { ...current, importing: true, importError: null } : current,
  );
  for (const batch of prepared.batches) {
    const batchSourceIds = new Set(batch.map((item) => item.sourceId));
    try {
      const results = await importOauthBatch(ctx, prepared, batch, validationJobId);
      const next = reconcileImportedOauthBatch({
        workingItems: working.workingItems,
        workingRows: working.workingRows,
        results,
      });
      working.workingItems = next.workingItems;
      working.workingRows = next.workingRows;
      working.importedAny ||= next.importedAny;
      publishImportedOauthProgress(ctx, working, true);
    } catch (error) {
      const message = importedOauthErrorMessage(error);
      working.batchErrors.push(message);
      working.workingRows = markImportedOauthRowsAsError(
        working.workingRows,
        batchSourceIds,
        message,
      );
      publishImportedOauthProgress(ctx, working, true, false);
    }
  }
  if (working.importedAny) {
    ctx.setImportInputKey((current: number) => current + 1);
  }
  ctx.setImportValidationState(() => {
    if (working.workingRows.length === 0) return null;
    return {
      ...buildImportedOauthStateFromRows(working.workingRows, working.workingItems),
      importing: false,
      importError: summarizeImportedOauthBatchErrors(working.batchErrors),
    };
  });
  if (working.workingRows.length === 0) ctx.setImportValidationDialogOpen(false);
}

export function useImportedOauthExecution(ctx: UpstreamAccountCreateControllerContext) {
  return {
    handleImportValidatedOauth: useCallback(() => executeImportedOauthImport(ctx), [ctx]),
  };
}
