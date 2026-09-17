import {
  IMPORT_VALIDATION_PAGE_SIZE,
  type ImportedOauthValidationDialogState,
} from "../../features/account-pool/ImportedOauthValidationDialog";
import type {
  ImportedOauthValidationRow,
  ImportedOauthValidationSnapshotEventPayload,
  ImportOauthCredentialFilePayload,
} from "../../lib/api";
import type {
  ParsedImportedOauthCredentialCandidate,
  ParsedImportedOauthCredentialRejection,
} from "./UpstreamAccountCreate.imported-oauth-parser";
import {
  buildImportedOauthCandidateFromStandardRecord,
  buildImportedOauthCandidateFromSub2apiAccount,
  buildImportedOauthMatchKeyFromValues,
  buildImportedOauthSub2apiAccountLabel,
  collectImportedWebSessionLikeObjects,
  convertImportedWebSessionRecord,
  firstImportedNonEmptyString,
  isImportedPlainObject,
  isSupportedSub2apiOauthAccount,
} from "./UpstreamAccountCreate.imported-oauth-parser";

const IMPORTED_OAUTH_DUPLICATE_DETAIL = "duplicate credential in current import selection";

export function convertImportedWebSessionDocumentLocally(
  content: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const normalizedContent = content.trim();
  if (!normalizedContent) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.importSession.paste.emptyError"),
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(normalizedContent);
  } catch {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.invalidJsonError"),
    };
  }

  const sources = collectImportedWebSessionLikeObjects(parsed);
  if (sources.length === 0) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.importSession.local.noSession"),
    };
  }

  try {
    const items = sources.map((source) =>
      convertImportedWebSessionRecord(source.value, source.path, t),
    );
    return {
      ok: true as const,
      items,
    };
  } catch (err) {
    return {
      ok: false as const,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

export function validateImportedOauthCredentialLocally(
  content: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const normalizedContent = content.trim();
  if (!normalizedContent) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.emptyError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.emptyError")],
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(normalizedContent);
  } catch {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.invalidJsonError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.invalidJsonError")],
    };
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.singleObjectError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.singleObjectError")],
    };
  }

  const record = parsed as Record<string, unknown>;
  if (isSupportedSub2apiOauthAccount(record)) {
    const candidate = buildImportedOauthCandidateFromSub2apiAccount(record, t);
    if (!candidate.ok) {
      return {
        ok: false as const,
        error: candidate.error,
        errors: [candidate.error],
      };
    }
    return candidate;
  }

  return buildImportedOauthCandidateFromStandardRecord(record, t);
}

export function parseImportedOauthCredentialDocumentLocally(
  content: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const normalizedContent = content.trim();
  if (!normalizedContent) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.emptyError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.emptyError")],
      rejected: [] as ParsedImportedOauthCredentialRejection[],
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(normalizedContent);
  } catch {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.invalidJsonError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.invalidJsonError")],
      rejected: [] as ParsedImportedOauthCredentialRejection[],
    };
  }

  if (!isImportedPlainObject(parsed)) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.singleObjectError"),
      errors: [t("accountPool.upstreamAccounts.import.paste.singleObjectError")],
      rejected: [] as ParsedImportedOauthCredentialRejection[],
    };
  }

  const record = parsed as Record<string, unknown>;
  if (
    firstImportedNonEmptyString(record.type)?.toLowerCase() === "sub2api-data" &&
    Array.isArray(record.accounts)
  ) {
    const candidates: ParsedImportedOauthCredentialCandidate[] = [];
    const rejected: ParsedImportedOauthCredentialRejection[] = [];
    record.accounts.forEach((entry, index) => {
      if (!isImportedPlainObject(entry)) {
        rejected.push({
          sourceLabel: `account ${index + 1}`,
          reason: t("accountPool.upstreamAccounts.import.local.unsupportedSub2apiAccount"),
        });
        return;
      }
      if (!isSupportedSub2apiOauthAccount(entry)) {
        rejected.push({
          sourceLabel: buildImportedOauthSub2apiAccountLabel(entry, index),
          reason: t("accountPool.upstreamAccounts.import.local.unsupportedSub2apiAccount"),
        });
        return;
      }
      const candidate = buildImportedOauthCandidateFromSub2apiAccount(entry, t);
      if (!candidate.ok) {
        rejected.push({
          sourceLabel: candidate.sourceLabel,
          reason: candidate.error,
        });
        return;
      }
      candidates.push(candidate);
    });

    if (candidates.length === 0) {
      return {
        ok: false as const,
        error: t("accountPool.upstreamAccounts.import.local.noSupportedSub2apiAccounts"),
        errors: [t("accountPool.upstreamAccounts.import.local.noSupportedSub2apiAccounts")],
        rejected,
      };
    }

    return {
      ok: true as const,
      candidates,
      rejected,
    };
  }

  const single = validateImportedOauthCredentialLocally(content, t);
  if (!single.ok) {
    return {
      ...single,
      rejected: [] as ParsedImportedOauthCredentialRejection[],
    };
  }
  return {
    ok: true as const,
    candidates: [single],
    rejected: [] as ParsedImportedOauthCredentialRejection[],
  };
}

export function getImportedOauthValidationStatusLabel(
  status: ImportedOauthValidationRow["status"],
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  switch (status) {
    case "pending":
      return t("accountPool.upstreamAccounts.import.validation.status.pending");
    case "duplicate_in_input":
      return t("accountPool.upstreamAccounts.import.validation.status.duplicate");
    case "ok":
      return t("accountPool.upstreamAccounts.import.validation.status.ok");
    case "ok_exhausted":
      return t("accountPool.upstreamAccounts.import.validation.status.exhausted");
    case "invalid":
      return t("accountPool.upstreamAccounts.import.validation.status.invalid");
    case "error":
      return t("accountPool.upstreamAccounts.import.validation.status.error");
    default:
      return status;
  }
}

export function parseImportedOauthPasteDraft(
  content: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  return parseImportedOauthCredentialDocumentLocally(content, t);
}

export function getImportedOauthPasteValidationError(
  row: ImportedOauthValidationRow,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const detail = row.detail?.trim();
  if (detail) return detail;
  return getImportedOauthValidationStatusLabel(row.status, t);
}

export function buildImportedOauthPendingState(
  items: ImportOauthCredentialFilePayload[],
): ImportedOauthValidationDialogState {
  return {
    inputFiles: items.length,
    uniqueInInput: items.length,
    duplicateInInput: 0,
    checking: true,
    importing: false,
    rows: items.map((item) => ({
      sourceId: item.sourceId,
      fileName: item.fileName,
      email: null,
      chatgptAccountId: null,
      chatgptUserId: null,
      displayName: null,
      tokenExpiresAt: null,
      matchedAccount: null,
      status: "pending",
      detail: null,
      attempts: 0,
    })),
    importError: null,
  };
}

export function formatImportedOauthSelectionLabel(
  items: ImportOauthCredentialFilePayload[],
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (items.length === 0) return null;
  return items.length === 1
    ? (items[0]?.fileName ?? null)
    : t("accountPool.upstreamAccounts.import.filesSelected", {
        count: items.length,
      });
}

export function buildImportedOauthStateFromRows(
  rows: ImportedOauthValidationRow[],
  items: ImportOauthCredentialFilePayload[],
): ImportedOauthValidationDialogState {
  const duplicateInInput = rows.filter((row) => row.status === "duplicate_in_input").length;
  return {
    inputFiles: items.length,
    uniqueInInput: Math.max(0, rows.length - duplicateInInput),
    duplicateInInput,
    checking: false,
    importing: false,
    rows,
    importError: null,
  };
}

export function buildImportedOauthStateFromSnapshot(
  snapshot: ImportedOauthValidationSnapshotEventPayload["snapshot"],
): ImportedOauthValidationDialogState {
  return {
    inputFiles: snapshot.inputFiles,
    uniqueInInput: snapshot.uniqueInInput,
    duplicateInInput: snapshot.duplicateInInput,
    checking: true,
    importing: false,
    rows: snapshot.rows,
    importError: null,
  };
}

export function chunkImportedOauthItems(
  items: ImportOauthCredentialFilePayload[],
  size: number = IMPORT_VALIDATION_PAGE_SIZE,
) {
  const batches: ImportOauthCredentialFilePayload[][] = [];
  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }
  return batches;
}

export function buildImportedOauthMatchKey(
  row: Pick<ImportedOauthValidationRow, "email" | "chatgptAccountId" | "chatgptUserId">,
) {
  return buildImportedOauthMatchKeyFromValues(row.chatgptUserId, row.email, row.chatgptAccountId);
}

export function applyImportedOauthDuplicateStatuses(rows: ImportedOauthValidationRow[]) {
  const seenKeys = new Set<string>();
  return rows.map((row) => {
    if (row.status === "pending") return row;
    const matchKey = buildImportedOauthMatchKey(row);
    if (!matchKey) return row;
    if (seenKeys.has(matchKey)) {
      return {
        ...row,
        matchedAccount: null,
        status: "duplicate_in_input",
        detail: IMPORTED_OAUTH_DUPLICATE_DETAIL,
      };
    }
    seenKeys.add(matchKey);
    return row;
  });
}

export function mergeImportedOauthValidationRows(
  currentRows: ImportedOauthValidationRow[],
  nextRows: ImportedOauthValidationRow[],
  retriedSourceIds: Set<string>,
) {
  const nextBySourceId = new Map(nextRows.map((row) => [row.sourceId, row] as const));
  return applyImportedOauthDuplicateStatuses(
    currentRows.map((row) => {
      const nextRow = nextBySourceId.get(row.sourceId);
      if (!nextRow) return row;
      return {
        ...row,
        ...nextRow,
        attempts: retriedSourceIds.has(row.sourceId)
          ? Math.max(nextRow.attempts, row.attempts + 1)
          : nextRow.attempts,
      };
    }),
  );
}

export function mergeImportedOauthValidationRow(
  currentRows: ImportedOauthValidationRow[],
  nextRow: ImportedOauthValidationRow,
  retriedSourceIds: Set<string>,
) {
  return mergeImportedOauthValidationRows(currentRows, [nextRow], retriedSourceIds);
}

export function replaceImportedOauthValidationRows(
  currentRows: ImportedOauthValidationRow[],
  nextRows: ImportedOauthValidationRow[],
) {
  const nextBySourceId = new Map(nextRows.map((row) => [row.sourceId, row] as const));
  return applyImportedOauthDuplicateStatuses(
    currentRows.map((row) => {
      const nextRow = nextBySourceId.get(row.sourceId);
      if (!nextRow) return row;
      return {
        ...row,
        ...nextRow,
      };
    }),
  );
}

export function markImportedOauthRowsAsError(
  currentRows: ImportedOauthValidationRow[],
  sourceIds: Set<string>,
  message: string,
) {
  return currentRows.map((row) => {
    if (!sourceIds.has(row.sourceId)) return row;
    return {
      ...row,
      status: "error",
      detail: message,
      attempts: Math.max(1, row.attempts + 1),
    };
  });
}

export function summarizeImportedOauthBatchErrors(messages: string[]) {
  const normalized = Array.from(
    new Set(messages.map((value) => value.trim()).filter((value) => value.length > 0)),
  );
  return normalized.length > 0 ? normalized.join(" | ") : null;
}
