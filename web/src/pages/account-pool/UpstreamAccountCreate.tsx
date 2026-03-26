import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from "react";
import { AppIcon } from "../../components/AppIcon";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { FormFieldFeedback } from "../../components/ui/form-field-feedback";
import { Input } from "../../components/ui/input";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "../../components/ui/segmented-control";
import {
  Popover,
  PopoverAnchor,
  PopoverArrow,
  PopoverContent,
  PopoverTrigger,
} from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { OauthMailboxChip } from "../../components/account-pool/OauthMailboxChip";
import { AccountTagField } from "../../components/AccountTagField";
import {
  ImportedOauthValidationDialog,
  IMPORT_VALIDATION_PAGE_SIZE,
  type ImportedOauthValidationDialogState,
} from "../../components/ImportedOauthValidationDialog";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { MotherAccountToggle } from "../../components/MotherAccountToggle";
import { useMotherSwitchNotifications } from "../../hooks/useMotherSwitchNotifications";
import { usePoolTags } from "../../hooks/usePoolTags";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import type {
  ImportOauthCredentialFilePayload,
  ImportedOauthValidationFailedEventPayload,
  ImportedOauthValidationRow,
  ImportedOauthValidationSnapshotEventPayload,
  LoginSessionStatusResponse,
  OauthMailboxSession,
  OauthMailboxSessionSupported,
  OauthMailboxStatus,
  UpdateOauthLoginSessionPayload,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from "../../lib/api";
import {
  createImportedOauthValidationJobEventSource,
  fetchUpstreamAccountDetail,
  normalizeImportedOauthValidationFailedEventPayload,
  normalizeImportedOauthValidationRowEventPayload,
  normalizeImportedOauthValidationSnapshotEventPayload,
  updateOauthLoginSessionKeepalive,
} from "../../lib/api";
import { copyText, selectAllReadonlyText } from "../../lib/clipboard";
import { emitUpstreamAccountsChanged } from "../../lib/upstreamAccountsEvents";
import {
  buildGroupNameSuggestions,
  isExistingGroup,
  normalizeGroupName,
  resolveGroupNote,
} from "../../lib/upstreamAccountGroups";
import { validateUpstreamBaseUrl } from "../../lib/upstreamBaseUrl";
import {
  applyMotherUpdateToItems,
  normalizeMotherGroupKey,
} from "../../lib/upstreamMother";
import { cn } from "../../lib/utils";
import { useTranslation } from "../../i18n";

type CreateTab = "oauth" | "batchOauth" | "apiKey" | "import";
type BatchOauthBusyAction = "generate" | "complete" | null;
type MailboxBusyAction = "attach" | "generate" | null;
type BatchOauthPersistedMetadata = {
  displayName: string;
  groupName: string;
  note: string;
  isMother: boolean;
  tagIds: number[];
};
type DuplicateWarningState = {
  accountId: number;
  displayName: string;
  peerAccountIds: number[];
  reasons: string[];
};
type GroupNoteEditorState = {
  open: boolean;
  groupName: string;
  note: string;
  existing: boolean;
};
type MailboxCopyTone = "idle" | "copied" | "manual";
const MAILBOX_REFRESH_INTERVAL_MS = 5_000;
const MAILBOX_REFRESH_TICK_MS = 1_000;
const OAUTH_SESSION_SYNC_DEBOUNCE_MS = 300;
const OAUTH_SESSION_SYNC_RETRY_MS = 1_000;
const MAX_SHARED_TAG_SYNC_ATTEMPTS = 2;
const IMPORTED_OAUTH_DUPLICATE_DETAIL =
  "duplicate credential in current import selection";

type PendingOauthSessionSnapshot = {
  loginId: string;
  payload: UpdateOauthLoginSessionPayload;
  signature: string;
  baseUpdatedAt: string | null;
};

type PendingOauthSessionSyncRecord = {
  syncedSignature: string | null;
  failedSignature: string | null;
  pendingSignature: string;
  timerId: number | null;
  inFlight: Promise<void> | null;
  lastSnapshot: PendingOauthSessionSnapshot | null;
};

type BatchOauthRow = {
  id: string;
  displayName: string;
  groupName: string;
  inheritsDefaultGroup: boolean;
  isMother: boolean;
  note: string;
  noteExpanded: boolean;
  callbackUrl: string;
  session: LoginSessionStatusResponse | null;
  sessionHint: string | null;
  duplicateWarning: DuplicateWarningState | null;
  needsRefresh: boolean;
  actionError: string | null;
  busyAction: BatchOauthBusyAction;
  mailboxSession: OauthMailboxSessionSupported | null;
  mailboxInput: string;
  mailboxStatus: OauthMailboxStatus | null;
  mailboxError: string | null;
  mailboxTone: MailboxCopyTone;
  mailboxCodeTone: MailboxCopyTone;
  mailboxBusyAction: MailboxBusyAction;
  mailboxEditorOpen: boolean;
  mailboxEditorValue: string;
  mailboxEditorError: string | null;
  mailboxRefreshBusy: boolean;
  mailboxNextRefreshAt: number | null;
  metadataBusy: boolean;
  metadataError: string | null;
  metadataPersisted: BatchOauthPersistedMetadata | null;
  pendingSharedTagIds: number[] | null;
  sharedTagSyncAttempts: number;
};

type CreatePageDraft = {
  oauth?: {
    displayName?: string;
    groupName?: string;
    isMother?: boolean;
    note?: string;
    tagIds?: number[];
    callbackUrl?: string;
    session?: LoginSessionStatusResponse | null;
    sessionHint?: string | null;
    duplicateWarning?: DuplicateWarningState | null;
    actionError?: string | null;
    mailboxSession?: OauthMailboxSessionSupported | null;
    mailboxInput?: string;
    mailboxStatus?: OauthMailboxStatus | null;
    mailboxError?: string | null;
    mailboxTone?: MailboxCopyTone;
    mailboxCodeTone?: MailboxCopyTone;
    mailboxBusyAction?: MailboxBusyAction;
    mailboxRefreshBusy?: boolean;
    mailboxNextRefreshAt?: number | null;
  };
  batchOauth?: {
    defaultGroupName?: string;
    tagIds?: number[];
    rows?: Array<Partial<BatchOauthRow> & { id?: string }>;
  };
  import?: {
    defaultGroupName?: string;
    tagIds?: number[];
  };
  apiKey?: {
    displayName?: string;
    groupName?: string;
    isMother?: boolean;
    note?: string;
    tagIds?: number[];
    apiKeyValue?: string;
    upstreamBaseUrl?: string;
    primaryLimit?: string;
    secondaryLimit?: string;
    limitUnit?: string;
  };
};

type CreatePageLocationState = {
  draft?: CreatePageDraft;
} | null;

function normalizeNumberInput(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function formatDateTime(value?: string | null) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function formatRelativeRefreshCountdown(
  nextRefreshAt: number | null,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!nextRefreshAt)
    return t("accountPool.upstreamAccounts.oauth.refreshScheduledUnknown");
  const seconds = Math.max(0, Math.ceil((nextRefreshAt - now) / 1000));
  return t("accountPool.upstreamAccounts.oauth.refreshIn", { seconds });
}

function parseAccountId(search: string): number | null {
  const value = new URLSearchParams(search).get("accountId");
  if (!value) return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function parseCreateMode(search: string): CreateTab {
  const value = new URLSearchParams(search).get("mode");
  if (value === "batchOauth") return "batchOauth";
  if (value === "apiKey") return "apiKey";
  if (value === "import") return "import";
  return "oauth";
}

function createBatchOauthRow(id: string, groupName = ""): BatchOauthRow {
  return {
    id,
    displayName: "",
    groupName,
    inheritsDefaultGroup: true,
    isMother: false,
    note: "",
    noteExpanded: false,
    callbackUrl: "",
    session: null,
    sessionHint: null,
    duplicateWarning: null,
    needsRefresh: false,
    actionError: null,
    busyAction: null,
    mailboxSession: null,
    mailboxInput: "",
    mailboxStatus: null,
    mailboxError: null,
    mailboxTone: "idle",
    mailboxCodeTone: "idle",
    mailboxBusyAction: null,
    mailboxEditorOpen: false,
    mailboxEditorValue: "",
    mailboxEditorError: null,
    mailboxRefreshBusy: false,
    mailboxNextRefreshAt: null,
    metadataBusy: false,
    metadataError: null,
    metadataPersisted: null,
    pendingSharedTagIds: null,
    sharedTagSyncAttempts: 0,
  };
}

function normalizeBatchTagIds(tagIds: number[]) {
  return Array.from(new Set(tagIds))
    .filter((value) => Number.isInteger(value) && value > 0)
    .sort((left, right) => left - right);
}

function batchTagIdsEqual(
  left: number[] | null | undefined,
  right: number[] | null | undefined,
) {
  const normalizedLeft = normalizeBatchTagIds(Array.isArray(left) ? left : []);
  const normalizedRight = normalizeBatchTagIds(
    Array.isArray(right) ? right : [],
  );
  if (normalizedLeft.length !== normalizedRight.length) return false;
  return normalizedLeft.every((value, index) => value === normalizedRight[index]);
}

function normalizeBatchOauthPersistedMetadata(
  value: Partial<BatchOauthPersistedMetadata> | null | undefined,
): BatchOauthPersistedMetadata | null {
  if (!value) return null;
  return {
    displayName:
      typeof value.displayName === "string" ? value.displayName.trim() : "",
    groupName: typeof value.groupName === "string" ? value.groupName.trim() : "",
    note: typeof value.note === "string" ? value.note.trim() : "",
    isMother: value.isMother === true,
    tagIds: normalizeBatchTagIds(Array.isArray(value.tagIds) ? value.tagIds : []),
  };
}

function buildBatchOauthPersistedMetadata(
  row: Pick<BatchOauthRow, "displayName" | "groupName" | "note" | "isMother">,
  tagIds: number[],
): BatchOauthPersistedMetadata {
  return {
    displayName: row.displayName.trim(),
    groupName: row.groupName.trim(),
    note: row.note.trim(),
    isMother: row.isMother,
    tagIds: normalizeBatchTagIds(tagIds),
  };
}

function batchOauthPersistedMetadataEquals(
  left: BatchOauthPersistedMetadata | null,
  right: BatchOauthPersistedMetadata,
) {
  if (!left) return false;
  if (left.displayName !== right.displayName) return false;
  if (left.groupName !== right.groupName) return false;
  if (left.note !== right.note) return false;
  if (left.isMother !== right.isMother) return false;
  if (left.tagIds.length !== right.tagIds.length) return false;
  return left.tagIds.every((value, index) => value === right.tagIds[index]);
}

function findCompletedBatchOauthAccount(
  row: Pick<BatchOauthRow, "session">,
  items: UpstreamAccountSummary[],
) {
  const accountId = row.session?.accountId;
  return accountId == null
    ? null
    : (items.find((item) => item.id === accountId) ?? null);
}

function resolveCompletedBatchOauthRowPersistedTagIds(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  items: UpstreamAccountSummary[],
) {
  const account = findCompletedBatchOauthAccount(row, items);
  if (account) {
    return normalizeBatchTagIds(account.tags.map((tag) => tag.id));
  }
  return row.metadataPersisted
    ? normalizeBatchTagIds(row.metadataPersisted.tagIds)
    : null;
}

function resolveCompletedBatchOauthRowBaselineTagIds(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  items: UpstreamAccountSummary[],
  fallbackTagIds: number[],
) {
  return (
    resolveCompletedBatchOauthRowPersistedTagIds(row, items) ??
    normalizeBatchTagIds(fallbackTagIds)
  );
}

function buildCompletedBatchOauthSharedTagBaselineSignature(
  rows: BatchOauthRow[],
  items: UpstreamAccountSummary[],
) {
  return rows
    .filter((row) => canEditCompletedBatchOauthRowMetadata(row))
    .map((row) => {
      const tagIds = resolveCompletedBatchOauthRowPersistedTagIds(row, items);
      return `${row.id}:${row.session?.accountId ?? "draft"}:${tagIds == null ? "unknown" : tagIds.join(",")}`;
    })
    .join("|");
}

function hydrateBatchOauthRow(
  seed: Partial<BatchOauthRow> & { id?: string },
  fallbackId: string,
  fallbackGroupName = "",
): BatchOauthRow {
  const hydratedGroupName = seed.groupName ?? fallbackGroupName;
  return {
    ...createBatchOauthRow(
      seed.id ?? fallbackId,
      hydratedGroupName,
    ),
    ...seed,
    id: seed.id ?? fallbackId,
    groupName: hydratedGroupName,
    inheritsDefaultGroup:
      typeof seed.inheritsDefaultGroup === "boolean"
        ? seed.inheritsDefaultGroup
        : !hydratedGroupName.trim() || hydratedGroupName === fallbackGroupName,
    isMother: seed.isMother === true,
    duplicateWarning: seed.duplicateWarning ?? null,
    needsRefresh: seed.needsRefresh === true,
    mailboxSession: seed.mailboxSession ?? null,
    mailboxInput:
      typeof seed.mailboxInput === "string"
        ? seed.mailboxInput
        : (seed.mailboxSession?.emailAddress ?? ""),
    mailboxStatus: seed.mailboxStatus ?? null,
    mailboxError:
      typeof seed.mailboxError === "string" ? seed.mailboxError : null,
    mailboxTone:
      seed.mailboxTone === "copied" || seed.mailboxTone === "manual"
        ? seed.mailboxTone
        : "idle",
    mailboxCodeTone: seed.mailboxCodeTone === "copied" ? "copied" : "idle",
    mailboxBusyAction:
      seed.mailboxBusyAction === "attach" ||
      seed.mailboxBusyAction === "generate"
        ? seed.mailboxBusyAction
        : null,
    mailboxEditorOpen: seed.mailboxEditorOpen === true,
    mailboxEditorValue:
      typeof seed.mailboxEditorValue === "string"
        ? seed.mailboxEditorValue
        : typeof seed.mailboxInput === "string"
          ? seed.mailboxInput
          : (seed.mailboxSession?.emailAddress ?? ""),
    mailboxEditorError:
      typeof seed.mailboxEditorError === "string"
        ? seed.mailboxEditorError
        : null,
    mailboxRefreshBusy: seed.mailboxRefreshBusy === true,
    mailboxNextRefreshAt:
      typeof seed.mailboxNextRefreshAt === "number"
        ? seed.mailboxNextRefreshAt
        : null,
    metadataBusy: seed.metadataBusy === true,
    metadataError:
      typeof seed.metadataError === "string" ? seed.metadataError : null,
    metadataPersisted: normalizeBatchOauthPersistedMetadata(
      seed.metadataPersisted,
    ),
    pendingSharedTagIds: Array.isArray(seed.pendingSharedTagIds)
      ? normalizeBatchTagIds(seed.pendingSharedTagIds)
      : null,
    sharedTagSyncAttempts:
      typeof seed.sharedTagSyncAttempts === "number" &&
      Number.isFinite(seed.sharedTagSyncAttempts) &&
      seed.sharedTagSyncAttempts > 0
        ? Math.trunc(seed.sharedTagSyncAttempts)
        : 0,
  };
}

function createImportedOauthSourceId(file: File, index: number) {
  return `${file.name}:${file.size}:${file.lastModified}:${index}`;
}

function buildImportedOauthPendingState(
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

function formatImportedOauthSelectionLabel(
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

function buildImportedOauthStateFromRows(
  rows: ImportedOauthValidationRow[],
  items: ImportOauthCredentialFilePayload[],
): ImportedOauthValidationDialogState {
  const duplicateInInput = rows.filter(
    (row) => row.status === "duplicate_in_input",
  ).length;
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

function buildImportedOauthStateFromSnapshot(
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

function chunkImportedOauthItems(
  items: ImportOauthCredentialFilePayload[],
  size: number = IMPORT_VALIDATION_PAGE_SIZE,
) {
  const batches: ImportOauthCredentialFilePayload[][] = [];
  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }
  return batches;
}

function buildImportedOauthMatchKey(
  row: Pick<ImportedOauthValidationRow, "email" | "chatgptAccountId">,
) {
  const normalizedAccountId = row.chatgptAccountId?.trim().toLocaleLowerCase();
  if (normalizedAccountId) {
    return `account:${normalizedAccountId}`;
  }
  const normalizedEmail = row.email?.trim().toLocaleLowerCase();
  if (normalizedEmail) {
    return `email:${normalizedEmail}`;
  }
  return null;
}

function applyImportedOauthDuplicateStatuses(
  rows: ImportedOauthValidationRow[],
) {
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

function mergeImportedOauthValidationRows(
  currentRows: ImportedOauthValidationRow[],
  nextRows: ImportedOauthValidationRow[],
  retriedSourceIds: Set<string>,
) {
  const nextBySourceId = new Map(
    nextRows.map((row) => [row.sourceId, row] as const),
  );
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

function mergeImportedOauthValidationRow(
  currentRows: ImportedOauthValidationRow[],
  nextRow: ImportedOauthValidationRow,
  retriedSourceIds: Set<string>,
) {
  return mergeImportedOauthValidationRows(
    currentRows,
    [nextRow],
    retriedSourceIds,
  );
}

function replaceImportedOauthValidationRows(
  currentRows: ImportedOauthValidationRow[],
  nextRows: ImportedOauthValidationRow[],
) {
  const nextBySourceId = new Map(
    nextRows.map((row) => [row.sourceId, row] as const),
  );
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

function markImportedOauthRowsAsError(
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

function summarizeImportedOauthBatchErrors(messages: string[]) {
  const normalized = Array.from(
    new Set(
      messages.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
  return normalized.length > 0 ? normalized.join(" | ") : null;
}

function getNextBatchRowIndex(rows: BatchOauthRow[]) {
  return rows.reduce((max, row) => {
    const matched = /^row-(\d+)$/.exec(row.id);
    const current = matched ? Number(matched[1]) : 0;
    return Number.isFinite(current) ? Math.max(max, current + 1) : max;
  }, 1);
}

function normalizeDisplayNameKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

function normalizeMailboxAddressKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

function mailboxInputMatchesSession(
  input: string,
  session: OauthMailboxSessionSupported | null,
) {
  if (!session) return false;
  return (
    normalizeMailboxAddressKey(input) ===
    normalizeMailboxAddressKey(session.emailAddress)
  );
}

function isProbablyValidEmailAddress(value: string) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
}

function findDisplayNameConflict(
  items: UpstreamAccountSummary[],
  displayName: string,
  excludeId?: number | null,
) {
  const normalized = normalizeDisplayNameKey(displayName);
  if (!normalized) return null;
  return (
    items.find(
      (item) =>
        item.id !== excludeId &&
        normalizeDisplayNameKey(item.displayName) === normalized,
    ) ?? null
  );
}

function invalidatePendingSingleOauthSession(
  currentSession: LoginSessionStatusResponse | null,
  setSession: (value: LoginSessionStatusResponse | null) => void,
  setSessionHint: (value: string | null) => void,
  setOauthCallbackUrl: (value: string) => void,
  setManualCopyOpen: (value: boolean) => void,
  setActionError: (value: string | null) => void,
  setOauthDuplicateWarning: (value: DuplicateWarningState | null) => void,
  regenerateRequiredLabel: string,
) {
  if (
    !currentSession ||
    (currentSession.status !== "pending" && currentSession.status !== "completed")
  ) {
    return;
  }
  setSession(null);
  setSessionHint(regenerateRequiredLabel);
  setOauthCallbackUrl("");
  setManualCopyOpen(false);
  setActionError(null);
  setOauthDuplicateWarning(null);
}

function buildOauthLoginSessionUpdatePayload({
  displayName,
  groupName,
  note,
  groupNote,
  includeGroupNote,
  tagIds,
  isMother,
  mailboxSession,
}: {
  displayName: string;
  groupName: string;
  note: string;
  groupNote: string;
  includeGroupNote: boolean;
  tagIds: number[];
  isMother: boolean;
  mailboxSession: OauthMailboxSessionSupported | null;
}): UpdateOauthLoginSessionPayload {
  const normalizedGroupName = groupName.trim();
  return {
    displayName: displayName.trim(),
    groupName: normalizedGroupName,
    note: note.trim(),
    ...(normalizedGroupName && includeGroupNote
      ? { groupNote: groupNote.trim() }
      : {}),
    tagIds,
    isMother,
    mailboxSessionId: mailboxSession?.sessionId ?? "",
    mailboxAddress: mailboxSession?.emailAddress ?? "",
  };
}

function buildPendingOauthSessionSnapshot(
  loginId: string,
  payload: UpdateOauthLoginSessionPayload,
  baseUpdatedAt?: string | null,
): PendingOauthSessionSnapshot {
  const normalizedBaseUpdatedAt = baseUpdatedAt?.trim() || null;
  return {
    loginId,
    payload,
    signature: JSON.stringify({
      payload,
      baseUpdatedAt: normalizedBaseUpdatedAt,
    }),
    baseUpdatedAt: normalizedBaseUpdatedAt,
  };
}

function shouldRetryPendingOauthSessionSync(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return !/^Request failed: (400|401|403|404|409|410|422)\b/.test(message);
}

function applyBatchMotherDraftRules(
  rows: BatchOauthRow[],
  changedRowId: string,
) {
  const changedRow = rows.find((row) => row.id === changedRowId);
  if (!changedRow?.isMother) return rows;
  const groupKey = normalizeMotherGroupKey(changedRow.groupName);
  return rows.map((row) =>
    row.id !== changedRowId &&
    row.isMother &&
    normalizeMotherGroupKey(row.groupName) === groupKey
      ? { ...row, isMother: false }
      : row,
  );
}

function enforceBatchMotherDraftUniqueness(rows: BatchOauthRow[]) {
  const winners = new Map<string, string>();
  for (const row of rows) {
    if (!row.isMother) continue;
    winners.set(normalizeMotherGroupKey(row.groupName), row.id);
  }
  return rows.map((row) =>
    row.isMother &&
    winners.get(normalizeMotherGroupKey(row.groupName)) !== row.id
      ? { ...row, isMother: false }
      : row,
  );
}

function reconcileBatchOauthMotherRowsAfterSave(
  rows: BatchOauthRow[],
  savedRowId: string,
  updated: Pick<UpstreamAccountSummary, "groupName" | "isMother">,
) {
  if (updated.isMother !== true) return rows;
  const groupKey = normalizeMotherGroupKey(updated.groupName);
  return rows.map((row) => {
    if (
      row.id === savedRowId ||
      !row.isMother ||
      normalizeMotherGroupKey(row.groupName) !== groupKey
    ) {
      return row;
    }
    return {
      ...row,
      isMother: false,
      metadataPersisted: row.metadataPersisted
        ? {
            ...row.metadataPersisted,
            isMother: false,
          }
        : row.metadataPersisted,
    };
  });
}

function batchStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "completed") return "success";
  if (status === "completedNeedsRefresh") return "warning";
  if (status === "pending") return "warning";
  if (status === "failed" || status === "expired") return "error";
  return "secondary";
}

function batchRowStatus(row: BatchOauthRow) {
  if (row.needsRefresh) return "completedNeedsRefresh";
  return row.session?.status ?? "draft";
}

function canEditCompletedBatchOauthRowMetadata(row: BatchOauthRow) {
  const status = batchRowStatus(row);
  return Boolean(
    row.session?.accountId != null &&
      (status === "completed" || status === "completedNeedsRefresh"),
  );
}

function batchRowStatusDetail(row: BatchOauthRow) {
  if (row.metadataError) return row.metadataError;
  if (row.actionError) return row.actionError;
  if (row.mailboxError) return row.mailboxError;
  if (row.sessionHint) return row.sessionHint;
  if (row.session?.error) return row.session.error;
  if (row.session?.expiresAt) return formatDateTime(row.session.expiresAt);
  return null;
}

function batchMailboxCodeVariant(
  row: BatchOauthRow,
): "default" | "secondary" | "outline" {
  const code = row.mailboxStatus?.latestCode?.value;
  if (!code) return "secondary";
  return row.mailboxCodeTone === "copied" ? "outline" : "default";
}

function batchMailboxCodeLabel(row: BatchOauthRow) {
  return row.mailboxStatus?.latestCode?.value ?? "------";
}

function batchMailboxRefreshVariant(
  row: BatchOauthRow,
): "outline" | "secondary" {
  return row.mailboxRefreshBusy ? "secondary" : "outline";
}

function isExpiredIso(value: string | null | undefined) {
  if (!value) return false;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) && timestamp <= Date.now();
}

function isRefreshableMailboxSession(
  session: OauthMailboxSessionSupported | null | undefined,
) {
  return Boolean(session && !isExpiredIso(session.expiresAt));
}

function batchMailboxRefreshLabel(
  row: BatchOauthRow,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (row.mailboxRefreshBusy) {
    return t("accountPool.upstreamAccounts.oauth.refreshingShort");
  }
  if (!isRefreshableMailboxSession(row.mailboxSession)) {
    return t("accountPool.upstreamAccounts.actions.fetchMailboxStatus");
  }
  if (!row.mailboxNextRefreshAt) {
    return t("accountPool.upstreamAccounts.actions.fetchMailboxStatus");
  }
  const seconds = Math.max(
    0,
    Math.ceil((row.mailboxNextRefreshAt - now) / 1000),
  );
  return t("accountPool.upstreamAccounts.oauth.refreshInShort", { seconds });
}

function batchMailboxRefreshTooltipDetail(
  row: BatchOauthRow,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (row.mailboxRefreshBusy) {
    return t("accountPool.upstreamAccounts.oauth.refreshing");
  }
  const receivedAt =
    row.mailboxStatus?.latestCode?.updatedAt ??
    row.mailboxStatus?.invite?.updatedAt ??
    null;
  if (receivedAt) {
    return `${t("accountPool.upstreamAccounts.oauth.receivedAt", {
      timestamp: formatDateTime(receivedAt),
    })} · ${formatRelativeRefreshCountdown(row.mailboxNextRefreshAt, now, t)}`;
  }
  return formatRelativeRefreshCountdown(row.mailboxNextRefreshAt, now, t);
}

function resolveMailboxIssue(
  session: OauthMailboxSession | null,
  status: OauthMailboxStatus | null,
  localError: string | null,
  expiresAt: string | null | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (session?.supported === false) {
    if (session.reason === "invalid_format") {
      return t(
        "accountPool.upstreamAccounts.oauth.mailboxUnsupportedInvalidFormat",
      );
    }
    if (session.reason === "unsupported_domain") {
      return t("accountPool.upstreamAccounts.oauth.mailboxUnsupportedDomain");
    }
    return t(
      "accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable",
    );
  }
  if (isExpiredIso(expiresAt)) {
    return t("accountPool.upstreamAccounts.oauth.mailboxExpired");
  }
  if (localError) return localError;
  if (status?.error) return status.error;
  return null;
}

function isSupportedMailboxSession(
  session: OauthMailboxSession | null,
): session is OauthMailboxSessionSupported {
  return Boolean(session && session.supported !== false);
}

function buildActionTooltip(title: string, description: string) {
  return (
    <div className="space-y-1">
      <p className="font-semibold text-base-content">{title}</p>
      <p className="leading-5 text-base-content/70">{description}</p>
    </div>
  );
}

function DuplicateWarningPopover({
  duplicateWarning,
  summaryTitle,
  summaryBody,
  openDetailsLabel,
  onOpenDetails,
  side = "top",
}: {
  duplicateWarning: DuplicateWarningState;
  summaryTitle: string;
  summaryBody: string;
  openDetailsLabel: string;
  onOpenDetails: (accountId: number) => void;
  side?: "top" | "right" | "bottom" | "left";
}) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setOpen(true);
  }, [duplicateWarning.accountId, summaryTitle, summaryBody]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex h-5 w-5 shrink-0 items-center justify-center text-warning transition-colors hover:text-warning/90"
          aria-label={summaryTitle}
        >
          <AppIcon name="alert-outline" className="h-5 w-5" aria-hidden />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side={side}
        sideOffset={10}
        onOpenAutoFocus={(event: Event) => event.preventDefault()}
        className="w-[16.5rem] rounded-2xl border border-warning/45 bg-base-100 p-0 shadow-[0_16px_38px_rgba(15,23,42,0.16)]"
      >
        <div className="space-y-3 p-3">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 text-warning">
              <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold leading-5 text-warning">
                {summaryTitle}
              </p>
              <p className="text-[11px] leading-5 text-base-content/72">
                {summaryBody}
              </p>
            </div>
          </div>
          <div className="flex justify-end">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 rounded-full px-2.5 text-xs font-semibold text-warning hover:bg-warning/10 hover:text-warning"
              onClick={() => {
                setOpen(false);
                onOpenDetails(duplicateWarning.accountId);
              }}
            >
              {openDetailsLabel}
            </Button>
          </div>
        </div>
        <PopoverArrow
          className="fill-base-100 stroke-warning/45 stroke-[0.8]"
          width={16}
          height={8}
        />
      </PopoverContent>
    </Popover>
  );
}

function DuplicateDetailField({
  label,
  value,
}: {
  label: string;
  value?: string | null;
}) {
  return (
    <div className="rounded-2xl border border-base-300/70 bg-base-100/82 px-3 py-3">
      <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/45">
        {label}
      </p>
      <p className="mt-1 break-all text-sm text-base-content/82">
        {value?.trim() ? value : "—"}
      </p>
    </div>
  );
}

function accountStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "active") return "success";
  if (status === "syncing") return "warning";
  if (status === "error" || status === "needs_reauth") return "error";
  return "secondary";
}

function accountKindVariant(kind: string): "secondary" | "success" {
  return kind === "oauth_codex" ? "success" : "secondary";
}

function DuplicateAccountDetailDialog({
  open,
  detail,
  isLoading,
  onClose,
  title,
  description,
  duplicateLabel,
  closeLabel,
  formatDuplicateReasons,
  statusLabel,
  kindLabel,
  fieldLabels,
}: {
  open: boolean;
  detail: UpstreamAccountDetail | null;
  isLoading: boolean;
  onClose: () => void;
  title: string;
  description: string;
  duplicateLabel: string;
  closeLabel: string;
  formatDuplicateReasons: (
    duplicateInfo?: UpstreamAccountDuplicateInfo | null,
  ) => string;
  statusLabel: (status: string) => string;
  kindLabel: (kind: string) => string;
  fieldLabels: {
    groupName: string;
    email: string;
    accountId: string;
    userId: string;
    lastSuccessSync: string;
  };
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen: boolean) => !nextOpen && onClose()}
    >
      <DialogContent className="max-h-[85vh] overflow-hidden p-0 sm:max-w-[38rem]">
        <div className="flex items-start justify-between gap-4 border-b border-base-300/70 px-5 py-4">
          <DialogHeader className="min-w-0">
            <DialogTitle className="truncate">
              {detail?.displayName ?? title}
            </DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} />
        </div>
        <div className="space-y-4 overflow-y-auto px-5 py-5">
          {isLoading ? (
            <div className="flex min-h-44 items-center justify-center">
              <Spinner />
            </div>
          ) : detail ? (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={accountStatusVariant(detail.status)}>
                  {statusLabel(detail.status)}
                </Badge>
                <Badge variant={accountKindVariant(detail.kind)}>
                  {kindLabel(detail.kind)}
                </Badge>
                {detail.duplicateInfo ? (
                  <Badge variant="warning">{duplicateLabel}</Badge>
                ) : null}
              </div>
              {detail.duplicateInfo ? (
                <Alert variant="warning">
                  <AppIcon
                    name="alert-outline"
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden
                  />
                  <div>
                    <p className="font-semibold text-warning">
                      {duplicateLabel}
                    </p>
                    <p className="mt-1 text-sm text-warning/90">
                      {`命中：${formatDuplicateReasons(detail.duplicateInfo)}。关联账号 ID：${detail.duplicateInfo.peerAccountIds.join(", ") || "—"}。`}
                    </p>
                  </div>
                </Alert>
              ) : null}
              <div className="grid gap-3 md:grid-cols-2">
                <DuplicateDetailField
                  label={fieldLabels.groupName}
                  value={detail.groupName ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.email}
                  value={detail.email ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.accountId}
                  value={detail.chatgptAccountId ?? detail.maskedApiKey ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.userId}
                  value={detail.chatgptUserId ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.lastSuccessSync}
                  value={formatDateTime(detail.lastSuccessfulSyncAt)}
                />
              </div>
            </>
          ) : (
            <p className="text-sm text-base-content/65">{description}</p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function UpstreamAccountCreatePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const locationState = (location.state as CreatePageLocationState) ?? null;
  const draft = locationState?.draft ?? null;
  const {
    items,
    groups = [],
    writesEnabled,
    isLoading,
    listError,
    beginOauthLogin,
    beginOauthMailboxSession,
    beginOauthMailboxSessionForAddress,
    getLoginSession,
    updateOauthLogin,
    getOauthMailboxStatuses,
    removeOauthMailboxSession,
    completeOauthLogin,
    createApiKeyAccount,
    startImportedOauthValidationJob,
    stopImportedOauthValidationJob,
    importOauthAccounts,
    saveAccount,
    saveGroupNote,
  } = useUpstreamAccounts();
  const { items: tagItems, createTag, updateTag, deleteTag } = usePoolTags();
  const notifyMotherSwitches = useMotherSwitchNotifications();

  const relinkAccountId = useMemo(
    () => parseAccountId(location.search),
    [location.search],
  );
  const relinkSummary = useMemo(
    () =>
      relinkAccountId == null
        ? null
        : (items.find((item) => item.id === relinkAccountId) ?? null),
    [items, relinkAccountId],
  );
  const isRelinking = relinkAccountId != null;
  const initialBatchRows = useMemo(() => {
    const defaultGroupName = draft?.batchOauth?.defaultGroupName ?? "";
    if (!draft?.batchOauth?.rows?.length) {
      return Array.from({ length: 5 }, (_, index) =>
        createBatchOauthRow(`row-${index + 1}`, defaultGroupName),
      );
    }
    return draft.batchOauth.rows.map((row, index) =>
      hydrateBatchOauthRow(row, `row-${index + 1}`, defaultGroupName),
    );
  }, [draft]);

  const [activeTab, setActiveTab] = useState<CreateTab>(() =>
    isRelinking ? "oauth" : parseCreateMode(location.search),
  );
  const [oauthDisplayName, setOauthDisplayName] = useState(
    () => draft?.oauth?.displayName ?? "",
  );
  const [oauthGroupName, setOauthGroupName] = useState(
    () => draft?.oauth?.groupName ?? "",
  );
  const [oauthIsMother, setOauthIsMother] = useState(
    () => draft?.oauth?.isMother === true,
  );
  const [oauthNote, setOauthNote] = useState(() => draft?.oauth?.note ?? "");
  const [oauthTagIds, setOauthTagIds] = useState<number[]>(
    () => draft?.oauth?.tagIds ?? [],
  );
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState(
    () => draft?.oauth?.callbackUrl ?? "",
  );
  const [oauthMailboxSession, setOauthMailboxSession] =
    useState<OauthMailboxSession | null>(
      () => draft?.oauth?.mailboxSession ?? null,
    );
  const [oauthMailboxInput, setOauthMailboxInput] = useState(
    () =>
      draft?.oauth?.mailboxInput ??
      draft?.oauth?.mailboxSession?.emailAddress ??
      "",
  );
  const [oauthMailboxStatus, setOauthMailboxStatus] =
    useState<OauthMailboxStatus | null>(
      () => draft?.oauth?.mailboxStatus ?? null,
    );
  const [oauthMailboxError, setOauthMailboxError] = useState<string | null>(
    () => draft?.oauth?.mailboxError ?? null,
  );
  const [oauthMailboxTone, setOauthMailboxTone] = useState<MailboxCopyTone>(
    () =>
      draft?.oauth?.mailboxTone === "copied" ||
      draft?.oauth?.mailboxTone === "manual"
        ? draft.oauth.mailboxTone
        : "idle",
  );
  const [oauthMailboxCodeTone, setOauthMailboxCodeTone] =
    useState<MailboxCopyTone>(() => draft?.oauth?.mailboxCodeTone ?? "idle");
  const [oauthMailboxBusyAction, setOauthMailboxBusyAction] =
    useState<MailboxBusyAction>(() =>
      draft?.oauth?.mailboxBusyAction === "attach" ||
      draft?.oauth?.mailboxBusyAction === "generate"
        ? draft.oauth.mailboxBusyAction
        : null,
    );
  const [oauthMailboxRefreshBusy, setOauthMailboxRefreshBusy] = useState(false);
  const [refreshClockMs, setRefreshClockMs] = useState(() => Date.now());
  const [apiKeyDisplayName, setApiKeyDisplayName] = useState(
    () => draft?.apiKey?.displayName ?? "",
  );
  const [apiKeyGroupName, setApiKeyGroupName] = useState(
    () => draft?.apiKey?.groupName ?? "",
  );
  const [apiKeyIsMother, setApiKeyIsMother] = useState(
    () => draft?.apiKey?.isMother === true,
  );
  const [apiKeyNote, setApiKeyNote] = useState(() => draft?.apiKey?.note ?? "");
  const [apiKeyTagIds, setApiKeyTagIds] = useState<number[]>(
    () => draft?.apiKey?.tagIds ?? [],
  );
  const [apiKeyValue, setApiKeyValue] = useState(
    () => draft?.apiKey?.apiKeyValue ?? "",
  );
  const [apiKeyUpstreamBaseUrl, setApiKeyUpstreamBaseUrl] = useState(
    () => draft?.apiKey?.upstreamBaseUrl ?? "",
  );
  const [apiKeyPrimaryLimit, setApiKeyPrimaryLimit] = useState(
    () => draft?.apiKey?.primaryLimit ?? "",
  );
  const [apiKeySecondaryLimit, setApiKeySecondaryLimit] = useState(
    () => draft?.apiKey?.secondaryLimit ?? "",
  );
  const [apiKeyLimitUnit, setApiKeyLimitUnit] = useState(
    () => draft?.apiKey?.limitUnit ?? "requests",
  );
  const [session, setSession] = useState<LoginSessionStatusResponse | null>(
    () => draft?.oauth?.session ?? null,
  );
  const [sessionHint, setSessionHint] = useState<string | null>(
    () => draft?.oauth?.sessionHint ?? null,
  );
  const [oauthDuplicateWarning, setOauthDuplicateWarning] =
    useState<DuplicateWarningState | null>(
      () => draft?.oauth?.duplicateWarning ?? null,
    );
  const [duplicateDetailOpen, setDuplicateDetailOpen] = useState(false);
  const [duplicateDetailLoading, setDuplicateDetailLoading] = useState(false);
  const [duplicateDetail, setDuplicateDetail] =
    useState<UpstreamAccountDetail | null>(null);
  const [actionError, setActionError] = useState<string | null>(
    () => draft?.oauth?.actionError ?? null,
  );
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [manualCopyOpen, setManualCopyOpen] = useState(false);
  const [batchManualCopyRowId, setBatchManualCopyRowId] = useState<
    string | null
  >(null);
  const [batchDefaultGroupName, setBatchDefaultGroupName] = useState(
    () => draft?.batchOauth?.defaultGroupName ?? "",
  );
  const [batchTagIds, setBatchTagIds] = useState<number[]>(
    () => draft?.batchOauth?.tagIds ?? [],
  );
  const batchSharedTagSyncEnabledRef = useRef(
    Object.prototype.hasOwnProperty.call(draft?.batchOauth ?? {}, "tagIds"),
  );
  const [importGroupName, setImportGroupName] = useState(
    () => draft?.import?.defaultGroupName ?? "",
  );
  const [importTagIds, setImportTagIds] = useState<number[]>(
    () => draft?.import?.tagIds ?? [],
  );
  const [importFiles, setImportFiles] = useState<
    ImportOauthCredentialFilePayload[]
  >([]);
  const [importSelectionLabel, setImportSelectionLabel] = useState<
    string | null
  >(null);
  const [importValidationDialogOpen, setImportValidationDialogOpen] =
    useState(false);
  const [importValidationState, setImportValidationState] =
    useState<ImportedOauthValidationDialogState | null>(null);
  const [importInputKey, setImportInputKey] = useState(0);
  const importValidationEventSourceRef = useRef<EventSource | null>(null);
  const importValidationEventCleanupRef = useRef<(() => void) | null>(null);
  const importValidationJobIdRef = useRef<string | null>(null);
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([]);
  const previousBatchTagIdsRef = useRef<number[] | null>(null);
  const previousCompletedSharedTagBaselineRef = useRef<string | null>(null);
  const [batchRows, setBatchRows] = useState<BatchOauthRow[]>(
    () => initialBatchRows,
  );
  const hasBatchMetadataBusy = useMemo(
    () => batchRows.some((row) => row.metadataBusy),
    [batchRows],
  );
  const [groupDraftNotes, setGroupDraftNotes] = useState<
    Record<string, string>
  >({});
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupNoteEditorState>({
    open: false,
    groupName: "",
    note: "",
    existing: false,
  });
  const [groupNoteBusy, setGroupNoteBusy] = useState(false);
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null);
  const oauthMailboxToneResetRef = useRef<number | null>(null);
  const batchMailboxToneResetRef = useRef<Record<string, number>>({});
  const batchRowsRef = useRef<BatchOauthRow[]>(initialBatchRows);
  const pendingOauthSessionSyncRef = useRef<
    Record<string, PendingOauthSessionSyncRecord>
  >({});
  const singleOauthSessionSnapshotRef =
    useRef<PendingOauthSessionSnapshot | null>(null);
  const batchOauthSessionSnapshotsRef = useRef<
    Record<string, PendingOauthSessionSnapshot>
  >({});
  const createdPendingOauthSessionSignaturesRef = useRef<
    Record<string, string>
  >({});
  const dispatchAllPendingOauthSessionKeepaliveSyncRef = useRef<() => void>(
    () => undefined,
  );
  const restoredPendingOauthLoginIdsRef = useRef(
    new Set<string>([
      ...(draft?.oauth?.session?.status === "pending"
        ? [draft.oauth.session.loginId]
        : []),
      ...((draft?.batchOauth?.rows ?? []).flatMap((row) =>
        row.session?.status === "pending" ? [row.session.loginId] : [],
      )),
    ]),
  );
  const activeOauthMailboxSession = useMemo(
    () =>
      isSupportedMailboxSession(oauthMailboxSession) &&
      mailboxInputMatchesSession(oauthMailboxInput, oauthMailboxSession)
        ? oauthMailboxSession
        : null,
    [oauthMailboxInput, oauthMailboxSession],
  );
  const refreshableOauthMailboxSession = useMemo(
    () =>
      isRefreshableMailboxSession(activeOauthMailboxSession)
        ? activeOauthMailboxSession
        : null,
    [activeOauthMailboxSession, refreshClockMs],
  );
  const resolvedOauthMailboxSession =
    activeOauthMailboxSession ??
    (oauthMailboxSession && !isSupportedMailboxSession(oauthMailboxSession)
      ? oauthMailboxSession
      : null);
  const displayedOauthMailboxStatus = activeOauthMailboxSession
    ? oauthMailboxStatus
    : null;
  const oauthMailboxIssue = resolveMailboxIssue(
    resolvedOauthMailboxSession,
    displayedOauthMailboxStatus,
    activeOauthMailboxSession ||
      (oauthMailboxSession && !isSupportedMailboxSession(oauthMailboxSession))
      ? oauthMailboxError
      : null,
    activeOauthMailboxSession?.expiresAt ?? null,
    t,
  );
  const oauthMailboxCodeStatusBadge = useMemo(() => {
    if (oauthMailboxRefreshBusy) return "checking";
    if (
      resolvedOauthMailboxSession &&
      oauthMailboxError &&
      (oauthMailboxError ===
        t("accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable") ||
        oauthMailboxError ===
          t("accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed"))
    ) {
      return "failed";
    }
    return null;
  }, [
    oauthMailboxError,
    oauthMailboxRefreshBusy,
    resolvedOauthMailboxSession,
    t,
  ]);
  useEffect(() => {
    return () => {
      dispatchAllPendingOauthSessionKeepaliveSyncRef.current();
      if (oauthMailboxToneResetRef.current != null) {
        window.clearTimeout(oauthMailboxToneResetRef.current);
      }
      Object.values(batchMailboxToneResetRef.current).forEach((timerId) => {
        window.clearTimeout(timerId);
      });
      Object.values(pendingOauthSessionSyncRef.current).forEach((record) => {
        if (record.timerId != null) {
          window.clearTimeout(record.timerId);
        }
      });
    };
  }, []);
  useEffect(() => {
    batchRowsRef.current = batchRows;
  }, [batchRows]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      setRefreshClockMs(Date.now());
    }, MAILBOX_REFRESH_TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    setBatchRows((current) => {
      let changed = false;
      const nextRows = current.map((row) => {
        const persistedTagIds = resolveCompletedBatchOauthRowPersistedTagIds(
          row,
          items,
        );
        if (
          !canEditCompletedBatchOauthRowMetadata(row) ||
          row.metadataPersisted != null ||
          persistedTagIds == null
        ) {
          return row;
        }
        changed = true;
        return {
          ...row,
          metadataPersisted: buildBatchOauthPersistedMetadata(
            row,
            persistedTagIds,
          ),
        };
      });
      return changed ? nextRows : current;
    });
  }, [batchRows, items]);
  const batchRowIdRef = useRef(getNextBatchRowIndex(initialBatchRows));
  const manualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null);
  const batchManualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null);

  const groupSuggestions = useMemo(
    () =>
      buildGroupNameSuggestions(
        items.map((item) => item.groupName),
        groups,
        groupDraftNotes,
      ),
    [groupDraftNotes, groups, items],
  );
  const oauthConflictExcludeId =
    relinkAccountId ??
    (session?.status === "completed" ? (session.accountId ?? null) : null);
  const oauthDisplayNameConflict = useMemo(
    () =>
      findDisplayNameConflict(items, oauthDisplayName, oauthConflictExcludeId),
    [items, oauthConflictExcludeId, oauthDisplayName],
  );
  const apiKeyDisplayNameConflict = useMemo(
    () => findDisplayNameConflict(items, apiKeyDisplayName),
    [apiKeyDisplayName, items],
  );
  const batchDraftNameCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const row of batchRows) {
      if (row.session?.status === "completed") continue;
      const key = normalizeDisplayNameKey(row.displayName);
      if (!key) continue;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return counts;
  }, [batchRows]);
  const batchDisplayNameError = (row: BatchOauthRow) => {
    const existingConflict = findDisplayNameConflict(
      items,
      row.displayName,
      row.session?.accountId ?? null,
    );
    if (existingConflict) {
      return t("accountPool.upstreamAccounts.validation.displayNameDuplicate");
    }
    const key = normalizeDisplayNameKey(row.displayName);
    if (key && (batchDraftNameCounts.get(key) ?? 0) > 1) {
      return t("accountPool.upstreamAccounts.validation.displayNameDuplicate");
    }
    return null;
  };
  const invalidateCurrentSingleOauthSession = useCallback(() => {
    invalidatePendingSingleOauthSession(
      session,
      setSession,
      setSessionHint,
      setOauthCallbackUrl,
      setManualCopyOpen,
      setActionError,
      setOauthDuplicateWarning,
      t("accountPool.upstreamAccounts.oauth.regenerateRequired"),
    );
  }, [session, t]);
  const invalidateRelinkPendingOauthSession = useCallback(() => {
    if (!isRelinking) return;
    invalidateCurrentSingleOauthSession();
  }, [invalidateCurrentSingleOauthSession, isRelinking]);
  const invalidateCompletedSingleOauthRetrySession = useCallback(() => {
    if (isRelinking || session?.status !== "completed") return;
    invalidateCurrentSingleOauthSession();
  }, [invalidateCurrentSingleOauthSession, isRelinking, session?.status]);
  const invalidateSingleOauthSessionForMetadataEdit = useCallback(() => {
    invalidateRelinkPendingOauthSession();
    invalidateCompletedSingleOauthRetrySession();
  }, [
    invalidateCompletedSingleOauthRetrySession,
    invalidateRelinkPendingOauthSession,
  ]);
  const invalidateRelinkPendingOauthSessionForMailboxChange = useCallback(
    (nextInput: string) => {
      if (!isRelinking || !activeOauthMailboxSession) return;
      if (mailboxInputMatchesSession(nextInput, activeOauthMailboxSession)) {
        return;
      }
      invalidateCurrentSingleOauthSession();
    },
    [
      activeOauthMailboxSession,
      invalidateCurrentSingleOauthSession,
      isRelinking,
    ],
  );
  const singleOauthSessionSnapshot = useMemo(() => {
    if (isRelinking || session?.status !== "pending") return null;
    const normalizedGroupName = normalizeGroupName(oauthGroupName);
    return buildPendingOauthSessionSnapshot(
      session.loginId,
      buildOauthLoginSessionUpdatePayload({
        displayName: oauthDisplayName,
        groupName: oauthGroupName,
        note: oauthNote,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName),
        includeGroupNote: Boolean(
          normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
        ),
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSession: activeOauthMailboxSession,
      }),
      session.updatedAt ?? null,
    );
  }, [
    activeOauthMailboxSession,
    oauthDisplayName,
    oauthGroupName,
    oauthIsMother,
    oauthNote,
    oauthTagIds,
    isRelinking,
    groups,
    resolvePendingGroupNoteForName,
    session?.loginId,
    session?.status,
    session?.updatedAt,
  ]);
  const batchOauthSessionSnapshots = useMemo(() => {
    const snapshots: Record<string, PendingOauthSessionSnapshot> = {};
    for (const row of batchRows) {
      if (row.session?.status !== "pending") continue;
      const normalizedGroupName = normalizeGroupName(row.groupName);
      snapshots[row.session.loginId] = buildPendingOauthSessionSnapshot(
        row.session.loginId,
        buildOauthLoginSessionUpdatePayload({
          displayName: row.displayName,
          groupName: row.groupName,
          note: row.note,
          groupNote: resolvePendingGroupNoteForName(row.groupName),
          includeGroupNote: Boolean(
            normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
          ),
          tagIds: batchTagIds,
          isMother: row.isMother,
          mailboxSession: row.mailboxSession,
        }),
        row.session.updatedAt ?? null,
      );
    }
    return snapshots;
  }, [batchRows, batchTagIds, groups, resolvePendingGroupNoteForName]);
  singleOauthSessionSnapshotRef.current = singleOauthSessionSnapshot;
  batchOauthSessionSnapshotsRef.current = batchOauthSessionSnapshots;
  const getActivePendingOauthSessionSnapshots = useCallback(() => {
    const snapshots: PendingOauthSessionSnapshot[] = [];
    if (singleOauthSessionSnapshotRef.current) {
      snapshots.push(singleOauthSessionSnapshotRef.current);
    }
    snapshots.push(...Object.values(batchOauthSessionSnapshotsRef.current));
    return snapshots;
  }, []);
  const getPendingOauthSessionSnapshot = useCallback((loginId: string) => {
    if (singleOauthSessionSnapshotRef.current?.loginId === loginId) {
      return singleOauthSessionSnapshotRef.current;
    }
    return batchOauthSessionSnapshotsRef.current[loginId] ?? null;
  }, []);
  const storePendingOauthSessionSnapshot = useCallback(
    (snapshot: PendingOauthSessionSnapshot) => {
      if (session?.loginId === snapshot.loginId) {
        singleOauthSessionSnapshotRef.current = snapshot;
        return;
      }
      batchOauthSessionSnapshotsRef.current[snapshot.loginId] = snapshot;
    },
    [session?.loginId],
  );
  const setPendingOauthSessionSyncError = useCallback((loginId: string, message: string) => {
    if (singleOauthSessionSnapshotRef.current?.loginId === loginId) {
      setActionError(message);
      return;
    }
    setBatchRows((current) =>
      current.map((row) =>
        row.session?.loginId === loginId
          ? {
              ...row,
              actionError: message,
            }
          : row,
      ),
    );
  }, []);
  const clearPendingOauthSessionSyncError = useCallback((loginId: string) => {
    if (singleOauthSessionSnapshotRef.current?.loginId === loginId) {
      setActionError(null);
      return;
    }
    setBatchRows((current) =>
      current.map((row) =>
        row.session?.loginId === loginId
          ? {
              ...row,
              actionError: null,
            }
          : row,
      ),
    );
  }, []);
  const applyPendingOauthSessionStatus = useCallback(
    (loginId: string, nextSession: LoginSessionStatusResponse) => {
      if (singleOauthSessionSnapshotRef.current?.loginId === loginId) {
        setSession((current) =>
          current?.loginId === loginId ? nextSession : current,
        );
        if (nextSession.status !== "pending") {
          setSessionHint(null);
          setActionError(null);
        }
        return;
      }
      setBatchRows((current) =>
        current.map((row) =>
          row.session?.loginId === loginId
            ? {
                ...row,
                session: nextSession,
                sessionHint:
                  nextSession.status === "pending" ? row.sessionHint : null,
                actionError:
                  nextSession.status === "pending" ? row.actionError : null,
              }
            : row,
        ),
      );
    },
    [],
  );
  const runPendingOauthSessionSync = useCallback(
    async (loginId: string, options?: { force?: boolean }) => {
      while (true) {
        const record = pendingOauthSessionSyncRef.current[loginId];
        const snapshot = getPendingOauthSessionSnapshot(loginId);
        if (!record || !snapshot) return;
        record.pendingSignature = snapshot.signature;
        if (record.syncedSignature === snapshot.signature) {
          return;
        }
        if (!options?.force && record.failedSignature === snapshot.signature) {
          return;
        }
        if (record.inFlight) {
          try {
            await record.inFlight;
          } catch {
            // Ignore stale failures so the latest snapshot can decide whether a retry is needed.
          }
          continue;
        }

        const { payload, signature, baseUpdatedAt } = snapshot;
        const request = (
          baseUpdatedAt
            ? updateOauthLogin(loginId, payload, baseUpdatedAt)
            : updateOauthLogin(loginId, payload)
        )
          .then((nextSession) => {
            const nextSyncedSignature =
              nextSession.syncApplied === false
                ? null
                : buildPendingOauthSessionSnapshot(
                    loginId,
                    payload,
                    nextSession.updatedAt ?? baseUpdatedAt ?? null,
                  ).signature;
            const currentRecord = pendingOauthSessionSyncRef.current[loginId];
            if (currentRecord) {
              currentRecord.syncedSignature = nextSyncedSignature;
              currentRecord.failedSignature = null;
            }
            applyPendingOauthSessionStatus(loginId, nextSession);
            if (nextSession.status === "completed" && nextSession.accountId) {
              emitUpstreamAccountsChanged();
            }
            clearPendingOauthSessionSyncError(loginId);
          })
          .catch(async (err) => {
            const currentRecord = pendingOauthSessionSyncRef.current[loginId];
            if (currentRecord) {
              currentRecord.failedSignature = signature;
              if (currentRecord.timerId != null) {
                window.clearTimeout(currentRecord.timerId);
                currentRecord.timerId = null;
              }
            }
            let latestSession: LoginSessionStatusResponse | null = null;
            try {
              latestSession = await getLoginSession(loginId);
            } catch {
              latestSession = null;
            }
            if (latestSession && latestSession.status !== "pending") {
              const latestRecord = pendingOauthSessionSyncRef.current[loginId];
              if (latestRecord) {
                latestRecord.failedSignature =
                  latestSession.status === "completed" ? null : signature;
                latestRecord.syncedSignature =
                  latestSession.status === "completed" ? signature : null;
              }
              applyPendingOauthSessionStatus(loginId, latestSession);
              if (latestSession.accountId) {
                emitUpstreamAccountsChanged();
              }
              if (latestSession.status === "completed") {
                clearPendingOauthSessionSyncError(loginId);
              } else {
                setPendingOauthSessionSyncError(
                  loginId,
                  latestSession.error ??
                    (err instanceof Error ? err.message : String(err)),
                );
              }
              return;
            }
            if (shouldRetryPendingOauthSessionSync(err)) {
              const latestRecord = pendingOauthSessionSyncRef.current[loginId];
              if (latestRecord?.failedSignature === signature) {
                latestRecord.timerId = window.setTimeout(() => {
                  const retryRecord = pendingOauthSessionSyncRef.current[loginId];
                  if (!retryRecord) return;
                  retryRecord.timerId = null;
                  void runPendingOauthSessionSync(loginId, {
                    force: true,
                  }).catch(() => undefined);
                }, OAUTH_SESSION_SYNC_RETRY_MS);
              }
            }
            setPendingOauthSessionSyncError(
              loginId,
              latestSession?.error ??
                (err instanceof Error ? err.message : String(err)),
            );
            throw err;
          })
          .finally(() => {
            const currentRecord = pendingOauthSessionSyncRef.current[loginId];
            if (currentRecord?.inFlight === request) {
              currentRecord.inFlight = null;
            }
          });
        record.inFlight = request;
        await request;
        return;
      }
    },
    [
      applyPendingOauthSessionStatus,
      clearPendingOauthSessionSyncError,
      getPendingOauthSessionSnapshot,
      getLoginSession,
      setPendingOauthSessionSyncError,
      updateOauthLogin,
    ],
  );
  const flushPendingOauthSessionSync = useCallback(
    async (
      loginId: string | null | undefined,
      snapshotOverride?: PendingOauthSessionSnapshot | null,
    ) => {
      if (!loginId) return;
      if (snapshotOverride && snapshotOverride.loginId === loginId) {
        storePendingOauthSessionSnapshot(snapshotOverride);
      }
      const snapshot = getPendingOauthSessionSnapshot(loginId);
      if (!snapshot) return;
      let record = pendingOauthSessionSyncRef.current[loginId];
      if (!record) {
        record = pendingOauthSessionSyncRef.current[loginId] = {
          syncedSignature: null,
          failedSignature: null,
          pendingSignature: snapshot.signature,
          timerId: null,
          inFlight: null,
          lastSnapshot: snapshot,
        };
      }
      record.pendingSignature = snapshot.signature;
      record.lastSnapshot = snapshot;
      if (record.timerId != null) {
        window.clearTimeout(record.timerId);
        record.timerId = null;
      }
      if (record.inFlight) {
        try {
          await record.inFlight;
        } catch {
          // Ignore stale failures so an explicit flush can retry the latest snapshot.
        }
      }
      record = pendingOauthSessionSyncRef.current[loginId];
      if (snapshotOverride && snapshotOverride.loginId === loginId) {
        storePendingOauthSessionSnapshot(snapshotOverride);
      }
      const latestSnapshot = getPendingOauthSessionSnapshot(loginId);
      if (!record || !latestSnapshot) return;
      if (record.syncedSignature !== latestSnapshot.signature) {
        await runPendingOauthSessionSync(loginId, { force: true });
      }
    },
    [
      getPendingOauthSessionSnapshot,
      runPendingOauthSessionSync,
      storePendingOauthSessionSnapshot,
    ],
  );
  const dispatchPendingOauthSessionKeepaliveSync = useCallback(
    (
      loginId: string | null | undefined,
      snapshotOverride?: PendingOauthSessionSnapshot | null,
    ) => {
      if (!loginId || !writesEnabled) return;
      if (snapshotOverride && snapshotOverride.loginId === loginId) {
        storePendingOauthSessionSnapshot(snapshotOverride);
      }
      const snapshot = getPendingOauthSessionSnapshot(loginId);
      if (!snapshot) return;
      let record = pendingOauthSessionSyncRef.current[loginId];
      if (!record) {
        record = pendingOauthSessionSyncRef.current[loginId] = {
          syncedSignature: null,
          failedSignature: null,
          pendingSignature: snapshot.signature,
          timerId: null,
          inFlight: null,
          lastSnapshot: snapshot,
        };
      }
      record.pendingSignature = snapshot.signature;
      record.lastSnapshot = snapshot;
      if (record.timerId != null) {
        window.clearTimeout(record.timerId);
        record.timerId = null;
      }
      // Unload keepalive must still send the latest metadata even when a normal
      // sync request is already in flight because browsers may cancel that
      // request during navigation.
      if (record.syncedSignature === snapshot.signature) {
        return;
      }
      const request = snapshot.baseUpdatedAt
        ? updateOauthLoginSessionKeepalive(
            loginId,
            snapshot.payload,
            snapshot.baseUpdatedAt,
          )
        : updateOauthLoginSessionKeepalive(loginId, snapshot.payload);
      void request.catch(() => undefined);
    },
    [
      getPendingOauthSessionSnapshot,
      storePendingOauthSessionSnapshot,
      writesEnabled,
    ],
  );
  const flushAllPendingOauthSessionSync = useCallback(() => {
    if (!writesEnabled) return;
    const seenLoginIds = new Set<string>();
    getActivePendingOauthSessionSnapshots().forEach((snapshot) => {
      seenLoginIds.add(snapshot.loginId);
      void flushPendingOauthSessionSync(snapshot.loginId, snapshot).catch(
        () => undefined,
      );
    });
    Object.keys(pendingOauthSessionSyncRef.current).forEach((loginId) => {
      if (seenLoginIds.has(loginId)) return;
      void flushPendingOauthSessionSync(loginId).catch(() => undefined);
    });
  }, [
    flushPendingOauthSessionSync,
    getActivePendingOauthSessionSnapshots,
    writesEnabled,
  ]);
  const dispatchAllPendingOauthSessionKeepaliveSync = useCallback(() => {
    if (!writesEnabled) return;
    const seenLoginIds = new Set<string>();
    getActivePendingOauthSessionSnapshots().forEach((snapshot) => {
      seenLoginIds.add(snapshot.loginId);
      dispatchPendingOauthSessionKeepaliveSync(snapshot.loginId, snapshot);
    });
    Object.keys(pendingOauthSessionSyncRef.current).forEach((loginId) => {
      if (seenLoginIds.has(loginId)) return;
      dispatchPendingOauthSessionKeepaliveSync(loginId);
    });
  }, [
    dispatchPendingOauthSessionKeepaliveSync,
    getActivePendingOauthSessionSnapshots,
    writesEnabled,
  ]);
  useEffect(() => {
    dispatchAllPendingOauthSessionKeepaliveSyncRef.current =
      dispatchAllPendingOauthSessionKeepaliveSync;
  }, [dispatchAllPendingOauthSessionKeepaliveSync]);
  useEffect(() => {
    if (!writesEnabled) {
      for (const record of Object.values(pendingOauthSessionSyncRef.current)) {
        if (record.timerId != null) {
          window.clearTimeout(record.timerId);
          record.timerId = null;
        }
      }
      return;
    }

    const activeSnapshots = [
      ...(singleOauthSessionSnapshot ? [singleOauthSessionSnapshot] : []),
      ...Object.values(batchOauthSessionSnapshots),
    ];
    const activeLoginIds = new Set(activeSnapshots.map((snapshot) => snapshot.loginId));

    for (const snapshot of activeSnapshots) {
      let existing = pendingOauthSessionSyncRef.current[snapshot.loginId];
      if (!existing) {
        const shouldStartUnsynced =
          restoredPendingOauthLoginIdsRef.current.delete(snapshot.loginId);
        const createdSyncedSignature =
          createdPendingOauthSessionSignaturesRef.current[snapshot.loginId] ??
          null;
        delete createdPendingOauthSessionSignaturesRef.current[snapshot.loginId];
        existing = pendingOauthSessionSyncRef.current[snapshot.loginId] = {
          syncedSignature: shouldStartUnsynced
            ? null
            : (createdSyncedSignature ?? snapshot.signature),
          failedSignature: null,
          pendingSignature: snapshot.signature,
          timerId: null,
          inFlight: null,
          lastSnapshot: snapshot,
        };
      }
      existing.pendingSignature = snapshot.signature;
      existing.lastSnapshot = snapshot;
      if (existing.syncedSignature === snapshot.signature) {
        if (existing.timerId != null) {
          window.clearTimeout(existing.timerId);
          existing.timerId = null;
        }
        continue;
      }
      if (existing.failedSignature === snapshot.signature) {
        continue;
      }
      if (existing.timerId != null) {
        window.clearTimeout(existing.timerId);
        existing.timerId = null;
      }
      existing.timerId = window.setTimeout(() => {
        const currentRecord = pendingOauthSessionSyncRef.current[snapshot.loginId];
        if (!currentRecord) return;
        currentRecord.timerId = null;
        void runPendingOauthSessionSync(snapshot.loginId).catch(() => undefined);
      }, OAUTH_SESSION_SYNC_DEBOUNCE_MS);
    }

    for (const [loginId, record] of Object.entries(pendingOauthSessionSyncRef.current)) {
      if (activeLoginIds.has(loginId)) continue;
      if (record.timerId != null) {
        window.clearTimeout(record.timerId);
      }
      delete pendingOauthSessionSyncRef.current[loginId];
      delete createdPendingOauthSessionSignaturesRef.current[loginId];
    }
  }, [
    batchOauthSessionSnapshots,
    runPendingOauthSessionSync,
    singleOauthSessionSnapshot,
    writesEnabled,
  ]);
  useEffect(() => {
    if (!writesEnabled) return;

    const flushPendingSync = () => {
      flushAllPendingOauthSessionSync();
    };
    const flushPendingSyncKeepalive = () => {
      dispatchAllPendingOauthSessionKeepaliveSync();
    };

    window.addEventListener("blur", flushPendingSync);
    window.addEventListener("beforeunload", flushPendingSyncKeepalive);
    window.addEventListener("pagehide", flushPendingSyncKeepalive);

    return () => {
      window.removeEventListener("blur", flushPendingSync);
      window.removeEventListener("beforeunload", flushPendingSyncKeepalive);
      window.removeEventListener("pagehide", flushPendingSyncKeepalive);
    };
  }, [
    dispatchAllPendingOauthSessionKeepaliveSync,
    flushAllPendingOauthSessionSync,
    writesEnabled,
  ]);
  const formatDuplicateReasons = (
    duplicateInfo?: UpstreamAccountDuplicateInfo | null,
  ) => {
    const reasons = duplicateInfo?.reasons ?? [];
    return reasons
      .map((reason) => {
        if (reason === "sharedChatgptAccountId") {
          return t(
            "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId",
          );
        }
        if (reason === "sharedChatgptUserId") {
          return t(
            "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId",
          );
        }
        return reason;
      })
      .join(" / ");
  };
  const accountStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.status.${status}`);
  const accountKindLabel = (kind: string) =>
    kind === "oauth_codex"
      ? t("accountPool.upstreamAccounts.kind.oauth")
      : t("accountPool.upstreamAccounts.kind.apiKey");
  const openDuplicateDetailDialog = async (accountId: number) => {
    setDuplicateDetailOpen(true);
    setDuplicateDetailLoading(true);
    try {
      const response = await fetchUpstreamAccountDetail(accountId);
      setDuplicateDetail(response);
    } catch {
      setDuplicateDetail(null);
    } finally {
      setDuplicateDetailLoading(false);
    }
  };
  const apiKeyUpstreamBaseUrlError = useMemo(() => {
    const code = validateUpstreamBaseUrl(apiKeyUpstreamBaseUrl);
    if (code === "invalid_absolute_url") {
      return t(
        "accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid",
      );
    }
    if (code === "query_or_fragment_not_allowed") {
      return t(
        "accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment",
      );
    }
    return null;
  }, [apiKeyUpstreamBaseUrl, t]);
  const oauthMailboxAddress =
    activeOauthMailboxSession?.emailAddress ?? oauthMailboxInput;

  const handleCreateTag = async (payload: Parameters<typeof createTag>[0]) => {
    const detail = await createTag(payload);
    setPageCreatedTagIds((current) =>
      current.includes(detail.id) ? current : [...current, detail.id],
    );
    return detail;
  };

  const handleDeleteTag = async (tagId: number) => {
    await deleteTag(tagId);
    setPageCreatedTagIds((current) =>
      current.filter((value) => value !== tagId),
    );
    setOauthTagIds((current) => current.filter((value) => value !== tagId));
    setApiKeyTagIds((current) => current.filter((value) => value !== tagId));
    setBatchTagIds((current) => current.filter((value) => value !== tagId));
  };

  useEffect(() => {
    if (isRelinking) {
      setActiveTab("oauth");
      return;
    }
    setActiveTab(parseCreateMode(location.search));
  }, [isRelinking, location.search]);

  useEffect(() => {
    if (!isRelinking || !relinkSummary) return;
    setActiveTab("oauth");
    setOauthDisplayName((current) => current || relinkSummary.displayName);
    setOauthGroupName((current) => current || relinkSummary.groupName || "");
    setOauthTagIds((current) =>
      current.length > 0
        ? current
        : (relinkSummary.tags ?? []).map((tag) => tag.id),
    );
    setOauthIsMother((current) => current || relinkSummary.isMother);
  }, [isRelinking, relinkSummary]);

  useEffect(() => {
    if (!manualCopyOpen) return;
    const frame = window.requestAnimationFrame(() => {
      selectAllReadonlyText(manualCopyFieldRef.current);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [manualCopyOpen]);

  useEffect(() => {
    if (!batchManualCopyRowId) return;
    const frame = window.requestAnimationFrame(() => {
      selectAllReadonlyText(batchManualCopyFieldRef.current);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [batchManualCopyRowId]);

  useEffect(() => {
    if (oauthMailboxSession) return;
    setOauthMailboxError(null);
  }, [oauthMailboxSession]);

  useEffect(() => {
    if (!refreshableOauthMailboxSession) {
      setOauthMailboxRefreshBusy(false);
      return;
    }
    let cancelled = false;
    const poll = async () => {
      setOauthMailboxRefreshBusy(true);
      try {
        const [status] = await getOauthMailboxStatuses([
          refreshableOauthMailboxSession.sessionId,
        ]);
        if (cancelled) return;
        if (!status) {
          setOauthMailboxError((current) =>
            current && current.trim()
              ? current
              : isExpiredIso(refreshableOauthMailboxSession.expiresAt)
                ? t("accountPool.upstreamAccounts.oauth.mailboxExpired")
                : t(
                    "accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable",
                  ),
          );
          return;
        }
        setOauthMailboxStatus((current) => {
          if (
            status.latestCode?.value &&
            status.latestCode.value !== current?.latestCode?.value
          ) {
            setOauthMailboxCodeTone("idle");
          }
          return status;
        });
        setOauthMailboxError(status.error ?? null);
      } catch {
        if (!cancelled) {
          setOauthMailboxError(
            t("accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed"),
          );
        }
      } finally {
        if (!cancelled) {
          setOauthMailboxRefreshBusy(false);
        }
      }
    };
    void poll();
    const timer = window.setInterval(() => {
      void poll();
    }, MAILBOX_REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [getOauthMailboxStatuses, refreshableOauthMailboxSession, t]);

  const activeBatchMailboxSessionIds = useMemo(
    () =>
      batchRows
        .filter((row) => isRefreshableMailboxSession(row.mailboxSession))
        .map((row) => row.mailboxSession?.sessionId ?? "")
        .filter((value) => value.length > 0),
    [batchRows, refreshClockMs],
  );
  const activeBatchMailboxSessionIdsKey = useMemo(
    () => activeBatchMailboxSessionIds.join("|"),
    [activeBatchMailboxSessionIds],
  );

  useEffect(() => {
    const sessionIds = activeBatchMailboxSessionIdsKey
      ? activeBatchMailboxSessionIdsKey
          .split("|")
          .filter((value) => value.length > 0)
      : [];
    if (sessionIds.length === 0) {
      setBatchRows((current) =>
        current.map((row) =>
          row.mailboxRefreshBusy || row.mailboxNextRefreshAt != null
            ? { ...row, mailboxRefreshBusy: false, mailboxNextRefreshAt: null }
            : row,
        ),
      );
      return;
    }
    let cancelled = false;
    const poll = async () => {
      setBatchRows((current) =>
        current.map((row) =>
          isRefreshableMailboxSession(row.mailboxSession)
            ? { ...row, mailboxRefreshBusy: true, mailboxNextRefreshAt: null }
            : row,
        ),
      );
      try {
        const statuses = await getOauthMailboxStatuses(sessionIds);
        if (cancelled) return;
        const bySessionId = new Map(
          statuses.map((status) => [status.sessionId, status]),
        );
        setBatchRows((current) =>
          current.map((row) => {
            const sessionId = row.mailboxSession?.sessionId;
            if (!sessionId) {
              return row;
            }
            const nextStatus = bySessionId.get(sessionId) ?? row.mailboxStatus;
            const nextError = bySessionId.has(sessionId)
              ? (bySessionId.get(sessionId)?.error ?? null)
              : row.mailboxError && row.mailboxError.trim()
                ? row.mailboxError
                : isExpiredIso(row.mailboxSession?.expiresAt)
                  ? t("accountPool.upstreamAccounts.oauth.mailboxExpired")
                  : t(
                      "accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable",
                    );
            const previousCode = row.mailboxStatus?.latestCode?.value ?? null;
            const nextCode = nextStatus?.latestCode?.value ?? null;
            if (
              row.mailboxStatus === (nextStatus ?? null) &&
              row.mailboxError === nextError
            ) {
              return row;
            }
            return {
              ...row,
              mailboxStatus: nextStatus ?? null,
              mailboxError: nextError,
              mailboxRefreshBusy: false,
              mailboxNextRefreshAt: Date.now() + MAILBOX_REFRESH_INTERVAL_MS,
              mailboxCodeTone:
                nextCode && previousCode && nextCode !== previousCode
                  ? "idle"
                  : row.mailboxCodeTone,
            };
          }),
        );
      } catch {
        if (!cancelled) {
          setBatchRows((current) =>
            current.map((row) =>
              row.mailboxSession
                ? {
                    ...row,
                    mailboxRefreshBusy: false,
                    mailboxNextRefreshAt:
                      Date.now() + MAILBOX_REFRESH_INTERVAL_MS,
                    mailboxError: isExpiredIso(row.mailboxSession.expiresAt)
                      ? t("accountPool.upstreamAccounts.oauth.mailboxExpired")
                      : t(
                          "accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed",
                        ),
                  }
                : row,
            ),
          );
        }
      }
    };
    void poll();
    const timer = window.setInterval(() => {
      void poll();
    }, MAILBOX_REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeBatchMailboxSessionIdsKey, getOauthMailboxStatuses, t]);

  const handleBatchMailboxFetch = useCallback(
    async (rowId: string) => {
      const row = batchRows.find((item) => item.id === rowId);
      const sessionId = row?.mailboxSession?.sessionId;
      if (!sessionId || !isRefreshableMailboxSession(row.mailboxSession))
        return;
      setBatchRows((current) =>
        current.map((item) =>
          item.id === rowId
            ? { ...item, mailboxRefreshBusy: true, mailboxNextRefreshAt: null }
            : item,
        ),
      );
      try {
        const [status] = await getOauthMailboxStatuses([sessionId]);
        setBatchRows((current) =>
          current.map((item) => {
            if (item.id !== rowId || !item.mailboxSession) return item;
            if (!status) {
              return {
                ...item,
                mailboxRefreshBusy: false,
                mailboxNextRefreshAt: Date.now() + MAILBOX_REFRESH_INTERVAL_MS,
                mailboxError:
                  item.mailboxError && item.mailboxError.trim()
                    ? item.mailboxError
                    : isExpiredIso(item.mailboxSession.expiresAt)
                      ? t("accountPool.upstreamAccounts.oauth.mailboxExpired")
                      : t(
                          "accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable",
                        ),
              };
            }
            const previousCode = item.mailboxStatus?.latestCode?.value ?? null;
            const nextCode = status.latestCode?.value ?? null;
            return {
              ...item,
              mailboxStatus: status,
              mailboxError: status.error ?? null,
              mailboxRefreshBusy: false,
              mailboxNextRefreshAt: Date.now() + MAILBOX_REFRESH_INTERVAL_MS,
              mailboxCodeTone:
                nextCode && previousCode && nextCode !== previousCode
                  ? "idle"
                  : item.mailboxCodeTone,
            };
          }),
        );
      } catch {
        setBatchRows((current) =>
          current.map((item) =>
            item.id === rowId && item.mailboxSession
              ? {
                  ...item,
                  mailboxRefreshBusy: false,
                  mailboxNextRefreshAt:
                    Date.now() + MAILBOX_REFRESH_INTERVAL_MS,
                  mailboxError: isExpiredIso(item.mailboxSession.expiresAt)
                    ? t("accountPool.upstreamAccounts.oauth.mailboxExpired")
                    : t(
                        "accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed",
                      ),
                }
              : item,
          ),
        );
      }
    },
    [batchRows, getOauthMailboxStatuses, t],
  );

  useEffect(() => {
    setGroupDraftNotes((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);

  function resolveGroupNoteForName(groupName: string) {
    return resolveGroupNote(groups, groupDraftNotes, groupName);
  }
  function resolvePendingGroupNoteForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized || isExistingGroup(groups, normalized)) return "";
    return groupDraftNotes[normalized]?.trim() ?? "";
  }
  function hasGroupNote(groupName: string) {
    return resolveGroupNoteForName(groupName).trim().length > 0;
  }

  const openGroupNoteEditor = (groupName: string) => {
    if (!writesEnabled) return;
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return;
    setGroupNoteError(null);
    setGroupNoteEditor({
      open: true,
      groupName: normalized,
      note: resolveGroupNoteForName(normalized),
      existing: isExistingGroup(groups, normalized),
    });
  };

  const closeGroupNoteEditor = () => {
    if (groupNoteBusy) return;
    setGroupNoteEditor((current) => ({ ...current, open: false }));
    setGroupNoteError(null);
  };

  const handleSaveGroupNote = async () => {
    if (!writesEnabled) return;
    const normalizedGroupName = normalizeGroupName(groupNoteEditor.groupName);
    if (!normalizedGroupName) return;
    const normalizedNote = groupNoteEditor.note.trim();
    const shouldInvalidateSingleOauthSessionForDraftGroup =
      normalizeGroupName(oauthGroupName) === normalizedGroupName &&
      resolvePendingGroupNoteForName(oauthGroupName).trim() !== normalizedNote;
    setGroupNoteError(null);
    if (!groupNoteEditor.existing) {
      setGroupDraftNotes((current) => {
        const next = { ...current };
        if (normalizedNote) {
          next[normalizedGroupName] = normalizedNote;
        } else {
          delete next[normalizedGroupName];
        }
        return next;
      });
      if (shouldInvalidateSingleOauthSessionForDraftGroup) {
        invalidateSingleOauthSessionForMetadataEdit();
      }
      setGroupNoteEditor((current) => ({ ...current, open: false }));
      return;
    }

    setGroupNoteBusy(true);
    try {
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
      });
      setGroupDraftNotes((current) => {
        if (!(normalizedGroupName in current)) return current;
        const next = { ...current };
        delete next[normalizedGroupName];
        return next;
      });
      setGroupNoteEditor((current) => ({ ...current, open: false }));
    } catch (err) {
      setGroupNoteError(err instanceof Error ? err.message : String(err));
    } finally {
      setGroupNoteBusy(false);
    }
  };

  const appendBatchRow = () => {
    const nextId = `row-${batchRowIdRef.current++}`;
    setBatchRows((current) => [
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
    setBatchRows((current) =>
      enforceBatchMotherDraftUniqueness(
        applyBatchMotherDraftRules(
          current.map((row) => (row.id === rowId ? updater(row) : row)),
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
        updateBatchRow(rowId, (current) => ({
          ...current,
          mailboxTone: "idle",
        }));
        delete batchMailboxToneResetRef.current[rowId];
      }, 1600);
    },
    [updateBatchRow],
  );

  const removeBatchRow = (rowId: string) => {
    const mailboxSessionId = batchRows.find((row) => row.id === rowId)
      ?.mailboxSession?.sessionId;
    setBatchRows((current) => {
      const remaining = current.filter((row) => row.id !== rowId);
      return remaining.length > 0
        ? remaining
        : [
            createBatchOauthRow(
              `row-${batchRowIdRef.current++}`,
              batchDefaultGroupName.trim(),
            ),
          ];
    });
    setBatchManualCopyRowId((current) => (current === rowId ? null : current));
    if (mailboxSessionId) {
      void removeOauthMailboxSession(mailboxSessionId).catch(() => undefined);
    }
  };

  const toggleBatchNoteExpanded = (rowId: string) => {
    updateBatchRow(rowId, (row) => ({
      ...row,
      noteExpanded: !row.noteExpanded,
    }));
  };

  async function persistCompletedBatchRowMetadata(
    rowId: string,
    overrides: Partial<BatchOauthPersistedMetadata>,
    committedFields: Array<keyof BatchOauthPersistedMetadata>,
  ) {
    const sourceRow = batchRowsRef.current.find((item) => item.id === rowId);
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
      updateBatchRow(rowId, (current) => ({
        ...current,
        metadataError: t(
          "accountPool.upstreamAccounts.validation.displayNameDuplicate",
        ),
      }));
      return;
    }

    if (batchOauthPersistedMetadataEquals(sourceRow.metadataPersisted, nextMetadata)) {
      updateBatchRow(rowId, (current) =>
        current.metadataError
          ? {
              ...current,
              metadataError: null,
            }
          : current,
      );
      return;
    }

    updateBatchRow(rowId, (current) => ({
      ...current,
      metadataBusy: true,
      metadataError: null,
      sharedTagSyncAttempts: isPendingSharedTagSyncAttempt
        ? current.sharedTagSyncAttempts + 1
        : current.sharedTagSyncAttempts,
    }));

    try {
      const detail = await saveAccount(accountId, {
        displayName: nextMetadata.displayName || undefined,
        groupName: nextMetadata.groupName,
        note: nextMetadata.note,
        isMother: nextMetadata.isMother,
        tagIds: nextMetadata.tagIds,
        groupNote:
          resolvePendingGroupNoteForName(nextMetadata.groupName) || undefined,
      });
      notifyMotherChange(detail);
      setBatchRows((currentRows) => {
        const nextRows = currentRows.map((current) => {
          if (current.id !== rowId) return current;
          const nextPersisted = buildBatchOauthPersistedMetadata(
            {
              displayName: detail.displayName,
              groupName: detail.groupName ?? "",
              note: detail.note ?? "",
              isMother: detail.isMother === true,
            },
            (detail.tags ?? []).map((tag) => tag.id),
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
      updateBatchRow(rowId, (current) => ({
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
    updateBatchRow(rowId, (row) => {
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
    const row = batchRowsRef.current.find((item) => item.id === rowId);
    if (!row || !canEditCompletedBatchOauthRowMetadata(row)) return;
    if (field === "displayName") {
      void persistCompletedBatchRowMetadata(
        rowId,
        { displayName: row.displayName.trim() },
        ["displayName"],
      );
      return;
    }
    void persistCompletedBatchRowMetadata(
      rowId,
      { note: row.note.trim() },
      ["note"],
    );
  };

  const handleBatchCompletedTextFieldKeyDown = (
    event: KeyboardEvent<HTMLInputElement>,
  ) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    event.currentTarget.blur();
  };

  const handleBatchGroupValueChange = (rowId: string, value: string) => {
    const row = batchRowsRef.current.find((item) => item.id === rowId);
    if (!row) return;
    updateBatchRow(rowId, (current) => {
      if (current.busyAction || current.mailboxBusyAction || current.metadataBusy) {
        return current;
      }
      return {
        ...current,
        groupName: value,
        inheritsDefaultGroup: value.trim() === "",
        metadataError: canEditCompletedBatchOauthRowMetadata(current)
          ? null
          : current.metadataError,
        actionError: null,
      };
    });
    if (!canEditCompletedBatchOauthRowMetadata(row)) return;
    void persistCompletedBatchRowMetadata(
      rowId,
      { groupName: value.trim() },
      ["groupName"],
    );
  };

  const handleBatchMotherToggle = (rowId: string) => {
    const row = batchRowsRef.current.find((item) => item.id === rowId);
    if (!row || row.busyAction || row.mailboxBusyAction || row.metadataBusy) {
      return;
    }
    const nextIsMother = !row.isMother;
    if (!canEditCompletedBatchOauthRowMetadata(row)) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        isMother: nextIsMother,
      }));
      return;
    }
    void persistCompletedBatchRowMetadata(
      rowId,
      { isMother: nextIsMother },
      ["isMother"],
    );
  };

  const handleBatchDefaultGroupChange = (value: string) => {
    const nextTrimmed = value.trim();
    const completedRowIdsToPersist: string[] = [];

    const nextRows = enforceBatchMotherDraftUniqueness(
      batchRows.map((row) => {
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
      void persistCompletedBatchRowMetadata(
        rowId,
        { groupName: nextTrimmed },
        ["groupName"],
      );
    });
  };

  useEffect(() => {
    const normalizedBatchTagIds = normalizeBatchTagIds(batchTagIds);
    const previousBatchTagIds = previousBatchTagIdsRef.current;
    const baselineSignature = buildCompletedBatchOauthSharedTagBaselineSignature(
      batchRows,
      items,
    );
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
    setBatchRows((current) => {
      let changed = false;
      const nextRows = current.map((row) => {
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
    batchRows.forEach((row) => {
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
        setImportValidationState((current) => {
          const baselineRows = current?.rows ?? buildImportedOauthPendingState(allItems).rows;
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

      const handleSnapshot = (event: Event) => {
        if (importValidationJobIdRef.current !== jobId) return;
        const message = event as MessageEvent<string>;
        try {
          const payload = normalizeImportedOauthValidationSnapshotEventPayload(
            JSON.parse(message.data),
          );
          if (merge) {
            setImportValidationState((current) => {
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
          setImportValidationState((current) =>
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
          setImportValidationState((current) =>
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
          setImportValidationState((current) =>
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
          setImportValidationState((current) =>
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
    [
      closeImportValidationEventSource,
    ],
  );

  const runImportValidation = useCallback(
    async (
      items: ImportOauthCredentialFilePayload[],
      options?: { merge?: boolean },
    ) => {
      if (items.length === 0) return;
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
        const response = await startImportedOauthValidationJob({ items });
        setImportValidationState((current) => {
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
        setImportValidationState((current) => {
          const baseline =
            current ?? buildImportedOauthPendingState(allItems);
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
      startImportedOauthValidationJob,
    ],
  );

  const handleImportFilesChange = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const selectedFiles = Array.from(event.target.files ?? []);
      setActionError(null);
      if (importValidationJobIdRef.current) {
        await cancelActiveImportedOauthValidation({ closeDialog: true });
      } else {
        setImportValidationDialogOpen(false);
        setImportValidationState(null);
        closeImportValidationEventSource();
      }
      if (selectedFiles.length === 0) {
        setImportFiles([]);
        setImportSelectionLabel(null);
        return;
      }
      try {
        const items = await Promise.all(
          selectedFiles.map(async (file, index) => ({
            sourceId: createImportedOauthSourceId(file, index),
            fileName: file.name,
            content: await file.text(),
          })),
        );
        setImportFiles(items);
        setImportSelectionLabel(formatImportedOauthSelectionLabel(items, t));
      } catch (err) {
        setImportFiles([]);
        setImportSelectionLabel(null);
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [cancelActiveImportedOauthValidation, closeImportValidationEventSource, t],
  );

  const handleClearImportSelection = useCallback(() => {
    void (async () => {
      if (importValidationJobIdRef.current) {
        await cancelActiveImportedOauthValidation({ closeDialog: true });
      } else {
        closeImportValidationEventSource();
        setImportValidationDialogOpen(false);
        setImportValidationState(null);
      }
      setImportFiles([]);
      setImportSelectionLabel(null);
      setImportInputKey((current) => current + 1);
    })();
  }, [cancelActiveImportedOauthValidation, closeImportValidationEventSource]);

  const handleValidateImportedOauth = useCallback(async () => {
    if (!writesEnabled || importFiles.length === 0) return;
    setActionError(null);
    await runImportValidation(importFiles);
  }, [importFiles, runImportValidation, writesEnabled]);

  const handleRetryImportedOauthOne = useCallback(
    async (sourceId: string) => {
      const item = importFiles.find(
        (candidate) => candidate.sourceId === sourceId,
      );
      if (!item) return;
      await runImportValidation([item], { merge: true });
    },
    [importFiles, runImportValidation],
  );

  const handleRetryImportedOauthFailed = useCallback(async () => {
    const failedSourceIds = new Set(
      (importValidationState?.rows ?? [])
        .filter((row) => row.status === "invalid" || row.status === "error")
        .map((row) => row.sourceId),
    );
    if (failedSourceIds.size === 0) return;
    await runImportValidation(
      importFiles.filter((item) => failedSourceIds.has(item.sourceId)),
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
    const currentRows = importValidationState?.rows ?? [];
    const validSourceIds = currentRows
      .filter((row) => row.status === "ok" || row.status === "ok_exhausted")
      .map((row) => row.sourceId);
    if (validSourceIds.length === 0) return;
    const validSourceIdSet = new Set(validSourceIds);
    const selectedItems = importFiles.filter((item) =>
      validSourceIdSet.has(item.sourceId),
    );
    const batches = chunkImportedOauthItems(selectedItems);
    const normalizedImportGroupName = normalizeGroupName(importGroupName);
    const importGroupNote =
      normalizedImportGroupName &&
      !isExistingGroup(groups, normalizedImportGroupName)
        ? groupDraftNotes[normalizedImportGroupName]?.trim() || undefined
        : undefined;
    const validationJobId = importValidationJobIdRef.current ?? undefined;
    let workingItems = [...importFiles];
    let workingRows = [...currentRows];
    let importedAny = false;
    const batchErrors: string[] = [];

    setImportValidationState((current) =>
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
          groupName: normalizedImportGroupName || undefined,
          groupNote: importGroupNote,
          tagIds: importTagIds,
        });
        const importedSourceIds = new Set(
          response.results
            .filter(
              (result) =>
                result.status === "created" ||
                result.status === "updated_existing",
            )
            .map((result) => result.sourceId),
        );
        const failedResultsBySourceId = new Map(
          response.results
            .filter((result) => result.status === "failed")
            .map((result) => [result.sourceId, result] as const),
        );

        if (importedSourceIds.size > 0) {
          importedAny = true;
        }
        workingItems = workingItems.filter(
          (item) => !importedSourceIds.has(item.sourceId),
        );
        workingRows = workingRows
          .filter((row) => !importedSourceIds.has(row.sourceId))
          .map((row) => {
            const failedResult = failedResultsBySourceId.get(row.sourceId);
            if (!failedResult) return row;
            return {
              ...row,
              status: "error",
              detail: failedResult.detail ?? row.detail,
            };
          });

        setImportFiles(workingItems);
        setImportSelectionLabel(
          formatImportedOauthSelectionLabel(workingItems, t),
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
      setImportInputKey((current) => current + 1);
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
    groups,
    importFiles,
    importGroupName,
    importOauthAccounts,
    importTagIds,
    importValidationState?.rows,
    t,
  ]);

  const clearOauthMailboxSession = useCallback(
    (
      sessionToRemoveId?: string | null,
      options?: { deleteRemote?: boolean },
    ) => {
      setOauthMailboxSession(null);
      setOauthMailboxStatus(null);
      setOauthMailboxError(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      if (sessionToRemoveId && options?.deleteRemote !== false) {
        void removeOauthMailboxSession(sessionToRemoveId).catch(
          () => undefined,
        );
      }
    },
    [removeOauthMailboxSession],
  );

  const notifyMotherChange = (updated: UpstreamAccountSummary) => {
    const nextItems = applyMotherUpdateToItems(items, updated);
    notifyMotherSwitches(items, nextItems);
  };

  const handleGenerateOauthMailbox = async () => {
    const previousSessionId = isSupportedMailboxSession(oauthMailboxSession)
      ? oauthMailboxSession.sessionId
      : null;
    setOauthMailboxBusyAction("generate");
    setActionError(null);
    setOauthMailboxError(null);
    try {
      const response = await beginOauthMailboxSession();
      if (!isSupportedMailboxSession(response)) {
        setOauthMailboxSession(response);
        setOauthMailboxInput(response.emailAddress);
        setOauthMailboxStatus(null);
        setOauthMailboxTone("idle");
        setOauthMailboxCodeTone("idle");
        return;
      }
      setOauthMailboxSession(response);
      setOauthMailboxInput(response.emailAddress);
      setOauthDisplayName((current) =>
        current.trim() ? current : response.emailAddress,
      );
      setOauthMailboxStatus(null);
      setOauthMailboxError(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      invalidateRelinkPendingOauthSession();
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(
          () => undefined,
        );
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setOauthMailboxBusyAction(null);
    }
  };

  const handleAttachOauthMailbox = async () => {
    const normalizedAddress = oauthMailboxInput.trim();
    if (!normalizedAddress) {
      invalidateRelinkPendingOauthSessionForMailboxChange("");
      setOauthMailboxSession(null);
      setOauthMailboxStatus(null);
      setOauthMailboxError(null);
      return;
    }
    const previousSessionId = isSupportedMailboxSession(oauthMailboxSession)
      ? oauthMailboxSession.sessionId
      : null;
    setOauthMailboxBusyAction("attach");
    setActionError(null);
    setOauthMailboxError(null);
    try {
      const response =
        await beginOauthMailboxSessionForAddress(normalizedAddress);
      setOauthMailboxSession(response);
      setOauthMailboxInput(response.emailAddress);
      setOauthMailboxStatus(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      if (isSupportedMailboxSession(response)) {
        if (!previousSessionId || previousSessionId !== response.sessionId) {
          invalidateRelinkPendingOauthSession();
        }
        setOauthDisplayName((current) =>
          current.trim() ? current : response.emailAddress,
        );
      } else if (previousSessionId) {
        invalidateRelinkPendingOauthSession();
      }
      if (
        previousSessionId &&
        (!isSupportedMailboxSession(response) ||
          previousSessionId !== response.sessionId)
      ) {
        void removeOauthMailboxSession(previousSessionId).catch(
          () => undefined,
        );
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setOauthMailboxBusyAction(null);
    }
  };

  const handleCopySingleMailbox = async () => {
    if (!oauthMailboxAddress) return;
    const result = await copyText(oauthMailboxAddress, {
      preferExecCommand: true,
    });
    if (!result.ok) {
      setOauthMailboxTone("manual");
      return;
    }
    setOauthMailboxTone("copied");
    scheduleSingleMailboxToneReset();
  };

  const handleCopySingleMailboxCode = async () => {
    const value = displayedOauthMailboxStatus?.latestCode?.value;
    if (!value) return;
    const result = await copyText(value, { preferExecCommand: true });
    if (result.ok) {
      setOauthMailboxCodeTone("copied");
    }
  };

  const handleCopySingleInvite = async () => {
    const value = displayedOauthMailboxStatus?.invite?.copyValue;
    if (!value) return;
    await copyText(value, { preferExecCommand: true });
  };

  const handleGenerateOauthUrl = async () => {
    if (oauthDisplayNameConflict) {
      setActionError(null);
      return;
    }
    setActionError(null);
    setSessionHint(null);
    setOauthDuplicateWarning(null);
    setBusyAction("oauth-generate");
    try {
      const normalizedGroupName = normalizeGroupName(oauthGroupName);
      const oauthLoginSessionPayload = buildOauthLoginSessionUpdatePayload({
        displayName: oauthDisplayName,
        groupName: oauthGroupName,
        note: oauthNote,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName),
        includeGroupNote: Boolean(
          normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
        ),
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSession: activeOauthMailboxSession,
      });
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupName.trim() || undefined,
        note: oauthNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName) || undefined,
        accountId: relinkAccountId ?? undefined,
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSessionId: activeOauthMailboxSession?.sessionId,
        mailboxAddress: activeOauthMailboxSession?.emailAddress,
      });
      createdPendingOauthSessionSignaturesRef.current[response.loginId] =
        buildPendingOauthSessionSnapshot(
          response.loginId,
          oauthLoginSessionPayload,
          response.updatedAt ?? null,
        ).signature;
      setSession(response);
      setManualCopyOpen(false);
      setOauthCallbackUrl("");
      setSessionHint(
        t("accountPool.upstreamAccounts.oauth.generated", {
          expiresAt: formatDateTime(response.expiresAt),
        }),
      );
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  };

  const handleCopyOauthUrl = async () => {
    if (!session?.authUrl) return;
    setActionError(null);
    let authUrlToCopy = session.authUrl;
    try {
      await flushPendingOauthSessionSync(
        session.loginId,
        singleOauthSessionSnapshot,
      );
      const latestSession = await getLoginSession(session.loginId);
      applyPendingOauthSessionStatus(session.loginId, latestSession);
      if (latestSession.status !== "pending" || !latestSession.authUrl) {
        setManualCopyOpen(false);
        return;
      }
      authUrlToCopy = latestSession.authUrl;
    } catch (err) {
      if (!shouldRetryPendingOauthSessionSync(err)) {
        return;
      }
      // Fall back to the last known pending auth URL on transient sync failures.
    }
    const result = await copyText(authUrlToCopy, {
      preferExecCommand: true,
    });
    if (result.ok) {
      setManualCopyOpen(false);
      setSessionHint(t("accountPool.upstreamAccounts.oauth.copied"));
      return;
    }

    setManualCopyOpen(true);
    setSessionHint(t("accountPool.upstreamAccounts.oauth.copyFailed"));
  };

  const handleCompleteOauth = async () => {
    if (!session) return;
    setActionError(null);
    setBusyAction("oauth-complete");
    try {
      await flushPendingOauthSessionSync(
        session.loginId,
        singleOauthSessionSnapshot,
      );
      const detail = await completeOauthLogin(session.loginId, {
        callbackUrl: oauthCallbackUrl.trim(),
        mailboxSessionId: activeOauthMailboxSession?.sessionId,
        mailboxAddress: activeOauthMailboxSession?.emailAddress,
      });
      notifyMotherChange(detail);
      setSession({
        ...session,
        status: "completed",
        accountId: detail.id,
        authUrl: null,
        redirectUri: null,
      });
      if (detail.duplicateInfo) {
        setOauthDuplicateWarning({
          accountId: detail.id,
          displayName: detail.displayName,
          peerAccountIds: detail.duplicateInfo.peerAccountIds,
          reasons: detail.duplicateInfo.reasons,
        });
      } else {
        navigate("/account-pool/upstream-accounts", {
          state: {
            selectedAccountId: detail.id,
            openDetail: true,
            duplicateWarning: null,
          },
        });
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      let latestSession: LoginSessionStatusResponse | null = null;
      try {
        latestSession = await getLoginSession(session.loginId);
      } catch {
        latestSession = null;
      }
      setSession((current) => latestSession ?? current);
      if (latestSession?.status === "completed" && latestSession.accountId) {
        setActionError(null);
        emitUpstreamAccountsChanged();
        try {
          const detail = await fetchUpstreamAccountDetail(
            latestSession.accountId,
          );
          notifyMotherChange(detail);
          if (detail.duplicateInfo) {
            setOauthDuplicateWarning({
              accountId: detail.id,
              displayName: detail.displayName,
              peerAccountIds: detail.duplicateInfo.peerAccountIds,
              reasons: detail.duplicateInfo.reasons,
            });
          } else {
            navigate("/account-pool/upstream-accounts", {
              state: {
                selectedAccountId: detail.id,
                openDetail: true,
                duplicateWarning: null,
              },
            });
          }
        } catch {
          navigate("/account-pool/upstream-accounts", {
            state: {
              selectedAccountId: latestSession.accountId,
              openDetail: true,
              duplicateWarning: null,
            },
          });
        }
        return;
      }
      if (
        latestSession?.status === "failed" ||
        latestSession?.status === "expired"
      ) {
        setOauthCallbackUrl("");
        setSessionHint(latestSession.error ?? message);
        setOauthDuplicateWarning(null);
      }
      setActionError(message);
    } finally {
      setBusyAction(null);
    }
  };

  const handleBatchGenerateMailbox = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    if (!row) return;

    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxBusyAction: "generate",
      mailboxEditorOpen: false,
      mailboxEditorValue: current.mailboxInput,
      actionError: null,
    }));

    try {
      const response = await beginOauthMailboxSession();
      if (!isSupportedMailboxSession(response)) {
        updateBatchRow(rowId, (current) => ({
          ...current,
          mailboxBusyAction: null,
          mailboxError: t(
            "accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable",
          ),
          actionError: null,
        }));
        return;
      }
      const previousSessionId = row.mailboxSession?.sessionId;
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxSession: response,
        mailboxInput: response.emailAddress,
        mailboxEditorValue: response.emailAddress,
        displayName: current.displayName.trim()
          ? current.displayName
          : response.emailAddress,
        mailboxStatus: null,
        mailboxError: null,
        mailboxTone: "idle",
        mailboxCodeTone: "idle",
        actionError: null,
      }));
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(
          () => undefined,
        );
      }
    } catch (err) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxError: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchStartMailboxEdit = (rowId: string) => {
    updateBatchRow(rowId, (current) => {
      if (
        current.busyAction ||
        current.mailboxBusyAction ||
        current.session?.status === "completed" ||
        current.needsRefresh
      ) {
        return current;
      }
      const baseValue =
        current.mailboxInput || current.mailboxSession?.emailAddress || "";
      return {
        ...current,
        mailboxEditorOpen: true,
        mailboxEditorValue: baseValue,
        mailboxEditorError: null,
        actionError: null,
      };
    });
  };

  const handleBatchMailboxEditorValueChange = (
    rowId: string,
    value: string,
  ) => {
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxEditorValue: value,
      mailboxEditorError: null,
    }));
  };

  const handleBatchCancelMailboxEdit = (rowId: string) => {
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxEditorOpen: false,
      mailboxEditorValue:
        current.mailboxInput || current.mailboxSession?.emailAddress || "",
      mailboxEditorError: null,
    }));
  };

  const handleBatchAttachMailbox = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    if (!row) return;
    const normalizedAddress = row.mailboxEditorValue.trim();
    if (!normalizedAddress) return;
    if (!isProbablyValidEmailAddress(normalizedAddress)) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxEditorError: t(
          "accountPool.upstreamAccounts.batchOauth.validation.mailboxFormat",
        ),
      }));
      return;
    }

    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxBusyAction: "attach",
      actionError: null,
      mailboxError: null,
      mailboxEditorError: null,
    }));

    const previousSessionId = row.mailboxSession?.sessionId ?? null;
    try {
      const response =
        await beginOauthMailboxSessionForAddress(normalizedAddress);
      const unsupportedError = isSupportedMailboxSession(response)
        ? null
        : resolveMailboxIssue(response, null, null, null, t);

      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxEditorOpen: false,
        mailboxEditorValue: response.emailAddress,
        mailboxEditorError: null,
        mailboxSession: isSupportedMailboxSession(response) ? response : null,
        mailboxInput: response.emailAddress,
        mailboxStatus: null,
        mailboxError: unsupportedError,
        mailboxTone: "idle",
        mailboxCodeTone: "idle",
        displayName:
          isSupportedMailboxSession(response) && !current.displayName.trim()
            ? response.emailAddress
            : current.displayName,
        actionError: null,
      }));

      if (
        previousSessionId &&
        (!isSupportedMailboxSession(response) ||
          previousSessionId !== response.sessionId)
      ) {
        void removeOauthMailboxSession(previousSessionId).catch(
          () => undefined,
        );
      }
    } catch (err) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchCopyMailbox = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    const value = row?.mailboxSession?.emailAddress ?? row?.mailboxInput ?? "";
    if (!value) return;
    const result = await copyText(value, { preferExecCommand: true });
    if (!result.ok) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxTone: "manual",
      }));
      return;
    }
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxTone: "copied",
    }));
    scheduleBatchMailboxToneReset(rowId);
  };

  const handleBatchCopyMailboxCode = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    const value = row?.mailboxStatus?.latestCode?.value;
    if (!value) return;
    const result = await copyText(value, { preferExecCommand: true });
    if (!result.ok) return;
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxCodeTone: "copied",
    }));
  };

  const handleBatchGenerateOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    if (!row) return;
    if (row.needsRefresh) return;

    updateBatchRow(rowId, (current) => ({
      ...current,
      busyAction: "generate",
      actionError: null,
    }));

    try {
      const normalizedGroupName = normalizeGroupName(row.groupName);
      const oauthLoginSessionPayload = buildOauthLoginSessionUpdatePayload({
        displayName: row.displayName,
        groupName: row.groupName,
        note: row.note,
        groupNote: resolvePendingGroupNoteForName(row.groupName),
        includeGroupNote: Boolean(
          normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
        ),
        tagIds: batchTagIds,
        isMother: row.isMother,
        mailboxSession: row.mailboxSession,
      });
      const response = await beginOauthLogin({
        displayName: row.displayName.trim() || undefined,
        groupName: row.groupName.trim() || undefined,
        note: row.note.trim() || undefined,
        tagIds: batchTagIds,
        groupNote: resolvePendingGroupNoteForName(row.groupName) || undefined,
        isMother: row.isMother,
        mailboxSessionId: row.mailboxSession?.sessionId,
        mailboxAddress: row.mailboxSession?.emailAddress,
      });
        createdPendingOauthSessionSignaturesRef.current[response.loginId] =
          buildPendingOauthSessionSnapshot(
            response.loginId,
            oauthLoginSessionPayload,
            response.updatedAt ?? null,
          ).signature;
      setBatchManualCopyRowId((current) =>
        current === rowId ? null : current,
      );
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        callbackUrl: "",
        session: response,
        sessionHint: t("accountPool.upstreamAccounts.oauth.generated", {
          expiresAt: formatDateTime(response.expiresAt),
        }),
        needsRefresh: false,
        actionError: null,
      }));
    } catch (err) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchCopyOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    if (!row?.session?.authUrl) return;

    updateBatchRow(rowId, (current) => ({
      ...current,
      actionError: null,
    }));

    let authUrlToCopy = row.session.authUrl;
    try {
      await flushPendingOauthSessionSync(
        row.session.loginId,
        batchOauthSessionSnapshots[row.session.loginId] ?? null,
      );
      const latestSession = await getLoginSession(row.session.loginId);
      applyPendingOauthSessionStatus(row.session.loginId, latestSession);
      if (latestSession.status !== "pending" || !latestSession.authUrl) {
        setBatchManualCopyRowId((current) => (current === rowId ? null : current));
        return;
      }
      authUrlToCopy = latestSession.authUrl;
    } catch (err) {
      if (!shouldRetryPendingOauthSessionSync(err)) {
        return;
      }
      // Fall back to the last known pending auth URL on transient sync failures.
    }

    const result = await copyText(authUrlToCopy, {
      preferExecCommand: true,
    });

    setBatchManualCopyRowId(result.ok ? null : rowId);

    updateBatchRow(rowId, (current) => ({
      ...current,
      sessionHint: result.ok
        ? t("accountPool.upstreamAccounts.oauth.copied")
        : t("accountPool.upstreamAccounts.batchOauth.copyInlineFallback"),
      actionError: result.ok
        ? null
        : t("accountPool.upstreamAccounts.batchOauth.copyInlineFallback"),
    }));
  };

  const handleBatchCompleteOauth = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId);
    if (!row?.session) return;

    updateBatchRow(rowId, (current) => ({
      ...current,
      busyAction: "complete",
      actionError: null,
    }));

    try {
      await flushPendingOauthSessionSync(
        row.session.loginId,
        batchOauthSessionSnapshots[row.session.loginId] ?? null,
      );
      const detail = await completeOauthLogin(row.session.loginId, {
        callbackUrl: row.callbackUrl.trim(),
        mailboxSessionId: row.mailboxSession?.sessionId,
        mailboxAddress: row.mailboxSession?.emailAddress,
      });
      notifyMotherChange(detail);
      updateBatchRow(rowId, (current) => {
        const baseSession = (current.session ??
          row.session) as LoginSessionStatusResponse;
        return {
          ...current,
          busyAction: null,
          session: {
            loginId: baseSession.loginId,
            status: "completed",
            authUrl: null,
            redirectUri: null,
            expiresAt: baseSession.expiresAt,
            accountId: detail.id,
            error: baseSession.error ?? null,
          },
          sessionHint: t("accountPool.upstreamAccounts.batchOauth.completed", {
            name: detail.displayName || current.displayName || `#${detail.id}`,
          }),
          duplicateWarning: detail.duplicateInfo
            ? {
                accountId: detail.id,
                displayName: detail.displayName,
                peerAccountIds: detail.duplicateInfo.peerAccountIds,
                reasons: detail.duplicateInfo.reasons,
              }
            : null,
          needsRefresh: false,
          actionError: null,
          metadataError: null,
          metadataPersisted: buildBatchOauthPersistedMetadata(
            {
              displayName: detail.displayName,
              groupName: detail.groupName ?? "",
              note: detail.note ?? "",
              isMother: detail.isMother === true,
            },
            (detail.tags ?? []).map((tag) => tag.id),
          ),
          pendingSharedTagIds: null,
          sharedTagSyncAttempts: 0,
          isMother: detail.isMother === true,
        };
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      let latestSession: LoginSessionStatusResponse | null = null;
      try {
        latestSession = await getLoginSession(row.session.loginId);
      } catch {
        latestSession = null;
      }
      if (latestSession?.status === "completed" && latestSession.accountId) {
        emitUpstreamAccountsChanged();
        try {
          const detail = await fetchUpstreamAccountDetail(
            latestSession.accountId,
          );
          notifyMotherChange(detail);
          updateBatchRow(rowId, (current) => {
            const baseSession = (current.session ??
              row.session) as LoginSessionStatusResponse;
            return {
              ...current,
              busyAction: null,
              session: {
                loginId: baseSession.loginId,
                status: "completed",
                authUrl: null,
                redirectUri: null,
                expiresAt: baseSession.expiresAt,
                accountId: detail.id,
                error: null,
              },
              callbackUrl: "",
              sessionHint: t(
                "accountPool.upstreamAccounts.batchOauth.completed",
                {
                  name:
                    detail.displayName ||
                    current.displayName ||
                    `#${detail.id}`,
                },
              ),
              duplicateWarning: detail.duplicateInfo
                ? {
                    accountId: detail.id,
                    displayName: detail.displayName,
                    peerAccountIds: detail.duplicateInfo.peerAccountIds,
                    reasons: detail.duplicateInfo.reasons,
                  }
                : null,
              needsRefresh: false,
              actionError: null,
              metadataError: null,
              metadataPersisted: buildBatchOauthPersistedMetadata(
                {
                  displayName: detail.displayName,
                  groupName: detail.groupName ?? "",
                  note: detail.note ?? "",
                  isMother: detail.isMother === true,
                },
                (detail.tags ?? []).map((tag) => tag.id),
              ),
              pendingSharedTagIds: null,
              sharedTagSyncAttempts: 0,
              isMother: detail.isMother === true,
            };
          });
        } catch {
          updateBatchRow(rowId, (current) => {
            const baseSession = (current.session ??
              row.session) as LoginSessionStatusResponse;
            return {
              ...current,
              busyAction: null,
              session: {
                loginId: baseSession.loginId,
                status: "completed",
                authUrl: null,
                redirectUri: null,
                expiresAt: baseSession.expiresAt,
                accountId: latestSession.accountId,
                error: null,
              },
              callbackUrl: "",
              sessionHint: null,
              duplicateWarning: current.duplicateWarning,
              needsRefresh: true,
              actionError: t(
                "accountPool.upstreamAccounts.batchOauth.completedNeedsRefresh",
              ),
            };
          });
        }
        return;
      }

      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        session: latestSession ?? current.session,
        callbackUrl:
          latestSession?.status === "failed" ||
          latestSession?.status === "expired"
            ? ""
            : current.callbackUrl,
        sessionHint:
          latestSession?.status === "failed" ||
          latestSession?.status === "expired"
            ? (latestSession.error ?? current.sessionHint)
            : current.sessionHint,
        duplicateWarning:
          latestSession?.status === "failed" ||
          latestSession?.status === "expired"
            ? null
            : current.duplicateWarning,
        needsRefresh: false,
        actionError: message,
      }));
    }
  };

  const handleCreateApiKey = async () => {
    if (apiKeyUpstreamBaseUrlError) return;
    setActionError(null);
    setBusyAction("apiKey");
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        groupName: apiKeyGroupName.trim() || undefined,
        note: apiKeyNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(apiKeyGroupName) || undefined,
        apiKey: apiKeyValue.trim(),
        upstreamBaseUrl: apiKeyUpstreamBaseUrl.trim() || undefined,
        isMother: apiKeyIsMother,
        localPrimaryLimit: normalizeNumberInput(apiKeyPrimaryLimit),
        localSecondaryLimit: normalizeNumberInput(apiKeySecondaryLimit),
        localLimitUnit: apiKeyLimitUnit.trim() || "requests",
        tagIds: apiKeyTagIds,
      });
      notifyMotherChange(response);
      navigate("/account-pool/upstream-accounts", {
        state: {
          selectedAccountId: response.id,
          openDetail: true,
        },
      });
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  };

  const oauthSessionActive = session?.status === "pending";
  const batchCounts = batchRows.reduce(
    (accumulator, row) => {
      const status = batchRowStatus(row);
      accumulator.total += 1;
      if (status === "completed") accumulator.completed += 1;
      else if (status === "pending" || status === "completedNeedsRefresh")
        accumulator.pending += 1;
      else accumulator.draft += 1;
      return accumulator;
    },
    { total: 0, draft: 0, pending: 0, completed: 0 },
  );
  const tagFieldLabels = {
    label: t("accountPool.tags.field.label"),
    add: t("accountPool.tags.field.add"),
    empty: t("accountPool.tags.field.empty"),
    searchPlaceholder: t("accountPool.tags.field.searchPlaceholder"),
    searchEmpty: t("accountPool.tags.field.searchEmpty"),
    createInline: (value: string) =>
      t("accountPool.tags.field.createInline", {
        value: value || t("accountPool.tags.field.newTag"),
      }),
    selectedFromCurrentPage: t("accountPool.tags.field.currentPage"),
    remove: t("accountPool.tags.field.remove"),
    deleteAndRemove: t("accountPool.tags.field.deleteAndRemove"),
    edit: t("accountPool.tags.field.edit"),
    createTitle: t("accountPool.tags.dialog.createTitle"),
    editTitle: t("accountPool.tags.dialog.editTitle"),
    dialogDescription: t("accountPool.tags.dialog.description"),
    name: t("accountPool.tags.dialog.name"),
    namePlaceholder: t("accountPool.tags.dialog.namePlaceholder"),
    guardEnabled: t("accountPool.tags.dialog.guardEnabled"),
    lookbackHours: t("accountPool.tags.dialog.lookbackHours"),
    maxConversations: t("accountPool.tags.dialog.maxConversations"),
    allowCutOut: t("accountPool.tags.dialog.allowCutOut"),
    allowCutIn: t("accountPool.tags.dialog.allowCutIn"),
    cancel: t("accountPool.tags.dialog.cancel"),
    save: t("accountPool.tags.dialog.save"),
    createAction: t("accountPool.tags.dialog.createAction"),
    validation: t("accountPool.tags.dialog.validation"),
  };

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <Button
                asChild
                variant="ghost"
                size="sm"
                className="mb-1 self-start px-0"
              >
                <Link to="/account-pool/upstream-accounts">
                  <AppIcon
                    name="arrow-left"
                    className="mr-2 h-4 w-4"
                    aria-hidden
                  />
                  {t("accountPool.upstreamAccounts.actions.backToList")}
                </Link>
              </Button>
              <h2 className="section-title">
                {isRelinking
                  ? t("accountPool.upstreamAccounts.createPage.relinkTitle")
                  : t("accountPool.upstreamAccounts.createPage.title")}
              </h2>
              <p className="section-description">
                {isRelinking
                  ? t(
                      "accountPool.upstreamAccounts.createPage.relinkDescription",
                      {
                        name:
                          relinkSummary?.displayName ??
                          t("accountPool.upstreamAccounts.unavailable"),
                      },
                    )
                  : t("accountPool.upstreamAccounts.createPage.description")}
              </p>
            </div>
            {isLoading ? <Spinner className="text-primary" /> : null}
          </div>

          {!writesEnabled ? (
            <Alert variant="warning">
              <AppIcon
                name="shield-key-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>
                <p className="font-medium">
                  {t("accountPool.upstreamAccounts.writesDisabledTitle")}
                </p>
                <p className="mt-1 text-sm text-warning/90">
                  {t("accountPool.upstreamAccounts.writesDisabledBody")}
                </p>
              </div>
            </Alert>
          ) : null}

          {listError || actionError ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{actionError ?? listError}</div>
            </Alert>
          ) : null}

          {session ? (
            <Alert
              variant={
                session.status === "completed"
                  ? "success"
                  : session.status === "pending"
                    ? "info"
                    : "warning"
              }
            >
              <AppIcon
                name={
                  session.status === "completed"
                    ? "check-circle-outline"
                    : "link-variant-plus"
                }
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="space-y-1">
                <p className="font-medium">
                  {t(
                    `accountPool.upstreamAccounts.oauth.status.${session.status}`,
                  )}
                </p>
                <p className="text-sm opacity-90">
                  {sessionHint ??
                    session.error ??
                    formatDateTime(session.expiresAt)}
                </p>
              </div>
            </Alert>
          ) : sessionHint ? (
            <Alert variant="warning">
              <AppIcon
                name="refresh-circle"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="text-sm">{sessionHint}</div>
            </Alert>
          ) : null}

          {!isRelinking ? (
            <SegmentedControl
              className="self-start"
              role="tablist"
              aria-label={t(
                "accountPool.upstreamAccounts.createPage.tabsLabel",
              )}
            >
              {(["oauth", "batchOauth", "import", "apiKey"] as const).map(
                (tab) => (
                  <SegmentedControlItem
                    key={tab}
                    active={activeTab === tab}
                    role="tab"
                    aria-selected={activeTab === tab}
                    onClick={() => handleTabChange(tab)}
                  >
                    {tab === "oauth"
                      ? t("accountPool.upstreamAccounts.createPage.tabs.oauth")
                      : tab === "batchOauth"
                        ? t(
                            "accountPool.upstreamAccounts.createPage.tabs.batchOauth",
                          )
                        : tab === "import"
                          ? t(
                              "accountPool.upstreamAccounts.createPage.tabs.import",
                            )
                          : t(
                              "accountPool.upstreamAccounts.createPage.tabs.apiKey",
                            )}
                  </SegmentedControlItem>
                ),
              )}
            </SegmentedControl>
          ) : null}

          <Card className="border-base-300/80 bg-base-100/72">
            <CardHeader className={cn(activeTab === "batchOauth" && "gap-3")}>
              {activeTab === "batchOauth" ? (
                <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex min-w-0 items-center gap-2">
                    <CardTitle className="shrink-0">
                      {t("accountPool.upstreamAccounts.batchOauth.createTitle")}
                    </CardTitle>
                    <Tooltip
                      content={buildActionTooltip(
                        t(
                          "accountPool.upstreamAccounts.batchOauth.createTitle",
                        ),
                        t(
                          "accountPool.upstreamAccounts.batchOauth.createDescription",
                        ),
                      )}
                    >
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-base-300/70 bg-base-100/72 text-base-content/55 transition hover:border-base-300 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                        aria-label={t(
                          "accountPool.upstreamAccounts.batchOauth.createDescription",
                        )}
                      >
                        <AppIcon
                          name="information-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </button>
                    </Tooltip>
                  </div>
                  <div className="flex w-full flex-wrap items-center justify-end gap-2 lg:w-auto lg:flex-nowrap lg:self-start">
                    <div className="flex min-w-0 items-center gap-2 sm:w-[24rem]">
                      <UpstreamAccountGroupCombobox
                        name="batchOauthDefaultGroupName"
                        value={batchDefaultGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t(
                          "accountPool.upstreamAccounts.batchOauth.defaultGroupPlaceholder",
                        )}
                        searchPlaceholder={t(
                          "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                        )}
                        emptyLabel={t(
                          "accountPool.upstreamAccounts.fields.groupNameEmpty",
                        )}
                        createLabel={(value) =>
                          t(
                            "accountPool.upstreamAccounts.fields.groupNameUseValue",
                            { value },
                          )
                        }
                        onValueChange={handleBatchDefaultGroupChange}
                        ariaLabel={t(
                          "accountPool.upstreamAccounts.batchOauth.defaultGroupLabel",
                        )}
                        disabled={!writesEnabled || hasBatchMetadataBusy}
                        className="min-w-0 flex-1"
                        triggerClassName="h-10 min-w-0 whitespace-nowrap rounded-lg"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          hasGroupNote(batchDefaultGroupName)
                            ? "secondary"
                            : "outline"
                        }
                        className="h-10 w-10 shrink-0 rounded-full"
                        aria-label={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        title={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        onClick={() =>
                          openGroupNoteEditor(batchDefaultGroupName)
                        }
                        disabled={
                          !writesEnabled ||
                          hasBatchMetadataBusy ||
                          !normalizeGroupName(batchDefaultGroupName)
                        }
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </Button>
                    </div>
                    <div className="w-full lg:w-[24rem]">
                      <AccountTagField
                        tags={tagItems}
                        selectedTagIds={batchTagIds}
                        writesEnabled={
                          writesEnabled && !hasBatchMetadataBusy
                        }
                        pageCreatedTagIds={pageCreatedTagIds}
                        labels={tagFieldLabels}
                        onChange={(nextTagIds) => {
                          batchSharedTagSyncEnabledRef.current = true;
                          setBatchTagIds(nextTagIds);
                          setBatchRows((current) =>
                            current.map((row) => ({
                              ...row,
                              actionError: null,
                            })),
                          );
                        }}
                        onCreateTag={handleCreateTag}
                        onUpdateTag={updateTag}
                        onDeleteTag={handleDeleteTag}
                      />
                    </div>
                    <Button
                      type="button"
                      variant="secondary"
                      onClick={appendBatchRow}
                      disabled={!writesEnabled || hasBatchMetadataBusy}
                      className="h-10 shrink-0 rounded-lg"
                    >
                      <AppIcon
                        name="playlist-plus"
                        className="mr-2 h-4 w-4"
                        aria-hidden
                      />
                      {t(
                        "accountPool.upstreamAccounts.batchOauth.actions.addRow",
                      )}
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <CardTitle>
                    {activeTab === "oauth"
                      ? t("accountPool.upstreamAccounts.oauth.createTitle")
                      : activeTab === "import"
                        ? t("accountPool.upstreamAccounts.import.createTitle")
                        : t("accountPool.upstreamAccounts.apiKey.createTitle")}
                  </CardTitle>
                  <CardDescription>
                    {activeTab === "oauth"
                      ? t(
                          "accountPool.upstreamAccounts.oauth.createDescription",
                        )
                      : activeTab === "import"
                        ? t(
                            "accountPool.upstreamAccounts.import.createDescription",
                          )
                        : t(
                            "accountPool.upstreamAccounts.apiKey.createDescription",
                          )}
                  </CardDescription>
                </>
              )}
            </CardHeader>
            <CardContent
              className={cn(
                "grid gap-4",
                activeTab === "apiKey" && "md:grid-cols-2",
              )}
            >
              {activeTab === "oauth" ? (
                <>
                  <div className="field">
                    <label
                      htmlFor="oauth-display-name"
                      className="field-label shrink-0"
                    >
                      {t("accountPool.upstreamAccounts.fields.displayName")}
                    </label>
                    <div className="relative">
                      <Input
                        id="oauth-display-name"
                        name="oauthDisplayName"
                        value={oauthDisplayName}
                        aria-invalid={oauthDisplayNameConflict != null}
                        onChange={(event) => {
                          setOauthDisplayName(event.target.value);
                          setActionError(null);
                          invalidateSingleOauthSessionForMetadataEdit();
                        }}
                      />
                      {oauthDisplayNameConflict ? (
                        <FloatingFieldError
                          message={t(
                            "accountPool.upstreamAccounts.validation.displayNameDuplicate",
                          )}
                        />
                      ) : null}
                    </div>
                  </div>
                  <div className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.mailboxAddress")}
                    </span>
                    <div className="grid gap-2">
                      <div className="flex flex-col gap-2 sm:flex-row">
                        <Input
                          name="oauthMailboxInput"
                          placeholder={t(
                            "accountPool.upstreamAccounts.oauth.mailboxInputPlaceholder",
                          )}
                          value={oauthMailboxInput}
                          onChange={(event) => {
                            const nextValue = event.target.value;
                            setOauthMailboxInput(nextValue);
                            setActionError(null);
                            invalidateRelinkPendingOauthSessionForMailboxChange(
                              nextValue,
                            );
                            if (
                              oauthMailboxSession &&
                              (!isSupportedMailboxSession(
                                oauthMailboxSession,
                              ) ||
                                !mailboxInputMatchesSession(
                                  nextValue,
                                  oauthMailboxSession,
                                ))
                            ) {
                              clearOauthMailboxSession(
                                isSupportedMailboxSession(oauthMailboxSession)
                                  ? oauthMailboxSession.sessionId
                                  : null,
                                { deleteRemote: false },
                              );
                            }
                          }}
                          disabled={
                            !writesEnabled ||
                            oauthMailboxBusyAction != null ||
                            session?.status === "completed"
                          }
                        />
                        <div className="flex gap-2">
                          <Tooltip
                            content={buildActionTooltip(
                              t(
                                "accountPool.upstreamAccounts.actions.useMailboxAddress",
                              ),
                              t(
                                "accountPool.upstreamAccounts.oauth.mailboxHint",
                              ),
                            )}
                          >
                            <Button
                              type="button"
                              size="icon"
                              variant="secondary"
                              className="h-10 w-10 shrink-0 rounded-full"
                              aria-label={t(
                                "accountPool.upstreamAccounts.actions.useMailboxAddress",
                              )}
                              title={t(
                                "accountPool.upstreamAccounts.actions.useMailboxAddress",
                              )}
                              onClick={() => void handleAttachOauthMailbox()}
                              disabled={
                                !writesEnabled ||
                                oauthMailboxBusyAction != null ||
                                session?.status === "completed" ||
                                !oauthMailboxInput.trim()
                              }
                            >
                              {oauthMailboxBusyAction === "attach" ? (
                                <AppIcon
                                  name="loading"
                                  className="h-4 w-4 animate-spin"
                                  aria-hidden
                                />
                              ) : (
                                <AppIcon
                                  name="check-bold"
                                  className="h-4 w-4"
                                  aria-hidden
                                />
                              )}
                            </Button>
                          </Tooltip>
                          <Tooltip
                            content={buildActionTooltip(
                              t(
                                "accountPool.upstreamAccounts.actions.generateMailbox",
                              ),
                              t(
                                "accountPool.upstreamAccounts.oauth.mailboxHint",
                              ),
                            )}
                          >
                            <Button
                              type="button"
                              size="icon"
                              variant="secondary"
                              className="h-10 w-10 shrink-0 rounded-full"
                              aria-label={t(
                                "accountPool.upstreamAccounts.actions.generateMailbox",
                              )}
                              title={t(
                                "accountPool.upstreamAccounts.actions.generateMailbox",
                              )}
                              onClick={() => void handleGenerateOauthMailbox()}
                              disabled={
                                !writesEnabled ||
                                oauthMailboxBusyAction != null ||
                                session?.status === "completed"
                              }
                            >
                              {oauthMailboxBusyAction === "generate" ? (
                                <AppIcon
                                  name="loading"
                                  className="h-4 w-4 animate-spin"
                                  aria-hidden
                                />
                              ) : (
                                <AppIcon
                                  name="auto-fix"
                                  className="h-4 w-4"
                                  aria-hidden
                                />
                              )}
                            </Button>
                          </Tooltip>
                        </div>
                      </div>
                      <p className="text-xs text-base-content/65">
                        {t("accountPool.upstreamAccounts.oauth.mailboxHint")}
                      </p>
                      {activeOauthMailboxSession ? (
                        <div className="flex flex-wrap items-center gap-2">
                          <OauthMailboxChip
                            emailAddress={oauthMailboxAddress}
                            emptyLabel={t(
                              "accountPool.upstreamAccounts.oauth.mailboxEmpty",
                            )}
                            copyAriaLabel={t(
                              "accountPool.upstreamAccounts.actions.copyMailbox",
                            )}
                            copyHintLabel={t(
                              "accountPool.upstreamAccounts.actions.copyMailboxHint",
                            )}
                            copiedLabel={t(
                              "accountPool.upstreamAccounts.actions.copied",
                            )}
                            manualCopyLabel={t(
                              "accountPool.upstreamAccounts.actions.manualCopyMailbox",
                            )}
                            manualBadgeLabel={t(
                              "accountPool.upstreamAccounts.actions.manual",
                            )}
                            tone={oauthMailboxTone}
                            onCopy={() => void handleCopySingleMailbox()}
                          />
                          <Badge
                            variant={
                              activeOauthMailboxSession.source === "attached"
                                ? "secondary"
                                : "success"
                            }
                          >
                            {activeOauthMailboxSession.source === "attached"
                              ? t(
                                  "accountPool.upstreamAccounts.oauth.mailboxAttached",
                                )
                              : t(
                                  "accountPool.upstreamAccounts.oauth.mailboxGenerated",
                                )}
                          </Badge>
                        </div>
                      ) : null}
                    </div>
                  </div>
                  <label className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.groupName")}
                    </span>
                    <div className="flex items-center gap-2">
                      <UpstreamAccountGroupCombobox
                        name="oauthGroupName"
                        value={oauthGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t(
                          "accountPool.upstreamAccounts.fields.groupNamePlaceholder",
                        )}
                        searchPlaceholder={t(
                          "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                        )}
                        emptyLabel={t(
                          "accountPool.upstreamAccounts.fields.groupNameEmpty",
                        )}
                        createLabel={(value) =>
                          t(
                            "accountPool.upstreamAccounts.fields.groupNameUseValue",
                            { value },
                          )
                        }
                        onValueChange={(value) => {
                          setOauthGroupName(value);
                          setActionError(null);
                          invalidateSingleOauthSessionForMetadataEdit();
                        }}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          hasGroupNote(oauthGroupName) ? "secondary" : "outline"
                        }
                        className="shrink-0 rounded-full"
                        aria-label={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        title={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        onClick={() => openGroupNoteEditor(oauthGroupName)}
                        disabled={
                          !writesEnabled ||
                          !normalizeGroupName(oauthGroupName)
                        }
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </Button>
                    </div>
                  </label>
                  <MotherAccountToggle
                    checked={oauthIsMother}
                    disabled={!writesEnabled}
                    label={t("accountPool.upstreamAccounts.mother.toggleLabel")}
                    description={t(
                      "accountPool.upstreamAccounts.mother.toggleDescription",
                    )}
                    onToggle={() => {
                      setOauthIsMother((current) => !current);
                      setActionError(null);
                      invalidateSingleOauthSessionForMetadataEdit();
                    }}
                  />
                  <label className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.note")}
                    </span>
                    <textarea
                      className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                      name="oauthNote"
                      value={oauthNote}
                      onChange={(event) => {
                        setOauthNote(event.target.value);
                        setActionError(null);
                        invalidateSingleOauthSessionForMetadataEdit();
                      }}
                    />
                  </label>
                  <AccountTagField
                    tags={tagItems}
                    selectedTagIds={oauthTagIds}
                    writesEnabled={writesEnabled}
                    pageCreatedTagIds={pageCreatedTagIds}
                    labels={tagFieldLabels}
                    onChange={(nextTagIds) => {
                      setOauthTagIds(nextTagIds);
                      setActionError(null);
                      invalidateSingleOauthSessionForMetadataEdit();
                    }}
                    onCreateTag={handleCreateTag}
                    onUpdateTag={updateTag}
                    onDeleteTag={handleDeleteTag}
                  />

                  {oauthMailboxIssue ? (
                    <Alert
                      variant={
                        isSupportedMailboxSession(oauthMailboxSession) &&
                        isExpiredIso(oauthMailboxSession.expiresAt)
                          ? "warning"
                          : "error"
                      }
                    >
                      <AppIcon
                        name={
                          isSupportedMailboxSession(oauthMailboxSession) &&
                          isExpiredIso(oauthMailboxSession.expiresAt)
                            ? "alert-outline"
                            : "alert-circle-outline"
                        }
                        className="mt-0.5 h-4 w-4 shrink-0"
                        aria-hidden
                      />
                      <div className="text-sm">{oauthMailboxIssue}</div>
                    </Alert>
                  ) : null}

                  <div className="grid gap-4 rounded-2xl border border-base-300/80 bg-base-100/72 p-4 sm:grid-cols-2">
                    <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="flex items-center gap-2 text-sm font-semibold text-base-content">
                            {t(
                              "accountPool.upstreamAccounts.oauth.codeCardTitle",
                            )}
                            {oauthMailboxCodeStatusBadge === "checking" ? (
                              <Badge
                                variant="secondary"
                                className="h-5 gap-1 rounded-full px-1.5 py-0 text-[10px] font-medium leading-none"
                              >
                                <Spinner size="sm" className="h-2.5 w-2.5" />
                                {t(
                                  "accountPool.upstreamAccounts.oauth.mailboxCheckingBadge",
                                )}
                              </Badge>
                            ) : null}
                            {oauthMailboxCodeStatusBadge === "failed" ? (
                              <Badge
                                variant="error"
                                className="h-5 rounded-full px-1.5 py-0 text-[10px] font-medium leading-none"
                              >
                                {t(
                                  "accountPool.upstreamAccounts.oauth.mailboxCheckFailedBadge",
                                )}
                              </Badge>
                            ) : null}
                          </p>
                          <p className="mt-1 text-xs text-base-content/65">
                            {displayedOauthMailboxStatus?.latestCode?.updatedAt
                              ? t(
                                  "accountPool.upstreamAccounts.oauth.receivedAt",
                                  {
                                    timestamp: formatDateTime(
                                      displayedOauthMailboxStatus.latestCode
                                        .updatedAt,
                                    ),
                                  },
                                )
                              : t(
                                  "accountPool.upstreamAccounts.oauth.codeCardEmpty",
                                )}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant={
                            oauthMailboxCodeTone === "copied"
                              ? "outline"
                              : "default"
                          }
                          size="sm"
                          disabled={
                            !displayedOauthMailboxStatus?.latestCode?.value
                          }
                          onClick={() => void handleCopySingleMailboxCode()}
                        >
                          <AppIcon
                            name="content-copy"
                            className="mr-1.5 h-4 w-4"
                            aria-hidden
                          />
                          {t("accountPool.upstreamAccounts.actions.copyCode")}
                        </Button>
                      </div>
                      <p className="mt-4 font-mono text-2xl font-semibold tracking-[0.24em] text-base-content">
                        {displayedOauthMailboxStatus?.latestCode?.value ?? "—"}
                      </p>
                    </div>
                    <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="text-sm font-semibold text-base-content">
                            {t(
                              "accountPool.upstreamAccounts.oauth.inviteCardTitle",
                            )}
                          </p>
                          <p className="mt-1 text-xs text-base-content/65">
                            {displayedOauthMailboxStatus?.invite?.updatedAt
                              ? t(
                                  "accountPool.upstreamAccounts.oauth.receivedAt",
                                  {
                                    timestamp: formatDateTime(
                                      displayedOauthMailboxStatus.invite
                                        .updatedAt,
                                    ),
                                  },
                                )
                              : (displayedOauthMailboxStatus?.invite?.subject ??
                                t(
                                  "accountPool.upstreamAccounts.oauth.inviteCardEmpty",
                                ))}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          disabled={
                            !displayedOauthMailboxStatus?.invite?.copyValue
                          }
                          onClick={() => void handleCopySingleInvite()}
                        >
                          <AppIcon
                            name="content-copy"
                            className="mr-1.5 h-4 w-4"
                            aria-hidden
                          />
                          {t("accountPool.upstreamAccounts.actions.copyInvite")}
                        </Button>
                      </div>
                      <div className="mt-4 flex items-center gap-3">
                        <Badge
                          variant={
                            displayedOauthMailboxStatus?.invited
                              ? "success"
                              : "secondary"
                          }
                          className="shrink-0 whitespace-nowrap rounded-full px-2.5 py-1 text-sm leading-none"
                        >
                          {displayedOauthMailboxStatus?.invited
                            ? t(
                                "accountPool.upstreamAccounts.oauth.invitedState",
                              )
                            : t(
                                "accountPool.upstreamAccounts.oauth.notInvitedState",
                              )}
                        </Badge>
                        <span className="min-w-0 flex-1 truncate text-sm text-base-content/70">
                          {displayedOauthMailboxStatus?.invite?.copyValue ??
                            "—"}
                        </span>
                      </div>
                    </div>
                  </div>

                  <div className="rounded-2xl border border-base-300/80 bg-base-200/40 p-4 sm:p-5">
                    <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                      <div className="space-y-1">
                        <h3 className="text-sm font-semibold text-base-content">
                          {t(
                            "accountPool.upstreamAccounts.oauth.manualFlowTitle",
                          )}
                        </h3>
                        <p className="text-sm text-base-content/70">
                          {t(
                            "accountPool.upstreamAccounts.oauth.manualFlowDescription",
                          )}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-wrap gap-2">
                        <Button
                          type="button"
                          variant="secondary"
                          onClick={() => void handleGenerateOauthUrl()}
                          disabled={
                            busyAction === "oauth-generate" ||
                            !writesEnabled ||
                            oauthDisplayNameConflict != null ||
                            session?.status === "completed"
                          }
                        >
                          {busyAction === "oauth-generate" ? (
                            <AppIcon
                              name="loading"
                              className="mr-2 h-4 w-4 animate-spin"
                              aria-hidden
                            />
                          ) : (
                            <AppIcon
                              name="link-variant-plus"
                              className="mr-2 h-4 w-4"
                              aria-hidden
                            />
                          )}
                          {session?.status === "pending"
                            ? t(
                                "accountPool.upstreamAccounts.actions.regenerateOauthUrl",
                              )
                            : t(
                                "accountPool.upstreamAccounts.actions.generateOauthUrl",
                              )}
                        </Button>
                        <Popover
                          open={manualCopyOpen}
                          onOpenChange={setManualCopyOpen}
                        >
                          <PopoverTrigger asChild>
                            <Button
                              type="button"
                              variant="secondary"
                              onClick={() => void handleCopyOauthUrl()}
                              disabled={
                                !oauthSessionActive || !session?.authUrl
                              }
                            >
                              <AppIcon
                                name="content-copy"
                                className="mr-2 h-4 w-4"
                                aria-hidden
                              />
                              {t(
                                "accountPool.upstreamAccounts.actions.copyOauthUrl",
                              )}
                            </Button>
                          </PopoverTrigger>
                          <PopoverContent
                            align="end"
                            sideOffset={10}
                            className="w-[min(36rem,calc(100vw-2rem))] rounded-2xl border-base-300 bg-base-100 p-4 shadow-xl"
                          >
                            <div className="space-y-3">
                              <div className="space-y-1">
                                <p className="text-sm font-semibold text-base-content">
                                  {t(
                                    "accountPool.upstreamAccounts.oauth.manualCopyTitle",
                                  )}
                                </p>
                                <p className="text-sm text-base-content/65">
                                  {t(
                                    "accountPool.upstreamAccounts.oauth.manualCopyDescription",
                                  )}
                                </p>
                              </div>
                              <textarea
                                ref={manualCopyFieldRef}
                                readOnly
                                value={session?.authUrl ?? ""}
                                className="min-h-28 w-full rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm focus-visible:outline-none"
                                onClick={(event) =>
                                  selectAllReadonlyText(event.currentTarget)
                                }
                                onFocus={(event) =>
                                  selectAllReadonlyText(event.currentTarget)
                                }
                              />
                            </div>
                          </PopoverContent>
                        </Popover>
                      </div>
                    </div>

                    <div className="mt-4 grid gap-4">
                      <div className="grid gap-4">
                        <label className="field">
                          <span className="field-label">
                            {t(
                              "accountPool.upstreamAccounts.oauth.callbackUrlLabel",
                            )}
                          </span>
                          <textarea
                            name="oauthCallbackUrl"
                            value={oauthCallbackUrl}
                            onChange={(event) =>
                              setOauthCallbackUrl(event.target.value)
                            }
                            placeholder={t(
                              "accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder",
                            )}
                            className="min-h-24 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                          />
                          <span className="text-xs text-base-content/60">
                            {t(
                              "accountPool.upstreamAccounts.oauth.callbackUrlDescription",
                            )}
                          </span>
                        </label>
                      </div>
                    </div>
                  </div>

                  <div className="flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">
                        {t("accountPool.upstreamAccounts.actions.cancel")}
                      </Link>
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleCompleteOauth()}
                      disabled={
                        !oauthSessionActive ||
                        !oauthCallbackUrl.trim() ||
                        busyAction === "oauth-complete" ||
                        !writesEnabled ||
                        oauthDisplayNameConflict != null
                      }
                    >
                      {busyAction === "oauth-complete" ? (
                        <AppIcon
                          name="loading"
                          className="mr-2 h-4 w-4 animate-spin"
                          aria-hidden
                        />
                      ) : (
                        <AppIcon
                          name="check-decagram-outline"
                          className="mr-2 h-4 w-4"
                          aria-hidden
                        />
                      )}
                      {t("accountPool.upstreamAccounts.actions.completeOauth")}
                    </Button>
                    {oauthDuplicateWarning ? (
                      <DuplicateWarningPopover
                        duplicateWarning={oauthDuplicateWarning}
                        summaryTitle={t(
                          "accountPool.upstreamAccounts.duplicate.compactTitle",
                        )}
                        summaryBody={t(
                          "accountPool.upstreamAccounts.duplicate.compactBody",
                          {
                            reasons: formatDuplicateReasons(
                              oauthDuplicateWarning,
                            ),
                            peers:
                              oauthDuplicateWarning.peerAccountIds.join(", "),
                          },
                        )}
                        openDetailsLabel={t(
                          "accountPool.upstreamAccounts.actions.openDetails",
                        )}
                        onOpenDetails={openDuplicateDetailDialog}
                      />
                    ) : null}
                  </div>
                </>
              ) : activeTab === "batchOauth" ? (
                <>
                  <div className="space-y-3">
                    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t(
                            "accountPool.upstreamAccounts.batchOauth.summary.total",
                          )}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">
                          {batchCounts.total}
                        </p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t(
                            "accountPool.upstreamAccounts.batchOauth.summary.draft",
                          )}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">
                          {batchCounts.draft}
                        </p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t(
                            "accountPool.upstreamAccounts.batchOauth.summary.pending",
                          )}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">
                          {batchCounts.pending}
                        </p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t(
                            "accountPool.upstreamAccounts.batchOauth.summary.completed",
                          )}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">
                          {batchCounts.completed}
                        </p>
                      </div>
                    </div>

                    <div className="overflow-hidden rounded-[1.35rem] border border-base-300/80 bg-base-100/92 shadow-sm shadow-base-300/20">
                      <table className="w-full table-fixed text-sm">
                        <colgroup>
                          <col className="w-14" />
                          <col className="w-[44%]" />
                          <col className="w-[56%]" />
                        </colgroup>
                        <thead className="bg-base-100/86">
                          <tr className="border-b border-base-300/80">
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              #
                            </th>
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              {t(
                                "accountPool.upstreamAccounts.batchOauth.tableAccountColumn",
                              )}
                            </th>
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              {t(
                                "accountPool.upstreamAccounts.batchOauth.tableFlowColumn",
                              )}
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          {batchRows.map((row, index) => {
                            const status = batchRowStatus(row);
                            const statusDetail = batchRowStatusDetail(row);
                            const duplicateNameError =
                              batchDisplayNameError(row);
                            const isCompleted = status === "completed";
                            const isRecoveredNeedsRefresh =
                              status === "completedNeedsRefresh";
                            const isPending = status === "pending";
                            const isBusy = row.busyAction != null;
                            const isMailboxBusy = row.mailboxBusyAction != null;
                            const metadataLocked =
                              !writesEnabled ||
                              isBusy ||
                              isMailboxBusy ||
                              row.metadataBusy;
                            const oauthLocked =
                              !writesEnabled ||
                              isBusy ||
                              isMailboxBusy ||
                              row.metadataBusy ||
                              isCompleted ||
                              isRecoveredNeedsRefresh;
                            const authUrl = row.session?.authUrl ?? "";
                            const rowMailboxAddress =
                              row.mailboxSession?.emailAddress ??
                              row.mailboxInput;
                            const rowInvited = row.mailboxStatus?.invited;
                            return (
                              <tr
                                key={row.id}
                                data-testid={`batch-oauth-row-${row.id}`}
                                className="align-top border-b border-base-300/70 last:border-b-0"
                              >
                                <td className="px-3 py-4">
                                  <Tooltip
                                    content={buildActionTooltip(
                                      rowInvited
                                        ? t(
                                            "accountPool.upstreamAccounts.batchOauth.tooltip.invitedTitle",
                                          )
                                        : t(
                                            "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedTitle",
                                          ),
                                      rowInvited
                                        ? t(
                                            "accountPool.upstreamAccounts.batchOauth.tooltip.invitedBody",
                                          )
                                        : t(
                                            "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedBody",
                                          ),
                                    )}
                                  >
                                    <button
                                      type="button"
                                      className={cn(
                                        "inline-flex h-8 min-w-8 items-center justify-center rounded-full border px-2 text-sm font-semibold shadow-sm transition-colors",
                                        rowInvited
                                          ? "border-success bg-success text-success-content hover:bg-success/90"
                                          : "border-base-300/80 bg-base-100 text-base-content/72 hover:border-base-300 hover:bg-base-100",
                                      )}
                                      aria-label={
                                        rowInvited
                                          ? t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.invitedTitle",
                                            )
                                          : t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedTitle",
                                            )
                                      }
                                    >
                                      {index + 1}
                                    </button>
                                  </Tooltip>
                                </td>
                                <td className="px-3 py-4">
                                  <div className="grid gap-3">
                                    <div className="field min-w-0 gap-2 whitespace-nowrap">
                                      <div className="flex items-center gap-3">
                                        <label
                                          htmlFor={`batch-oauth-display-name-${row.id}`}
                                          className="field-label shrink-0"
                                        >
                                          {t(
                                            "accountPool.upstreamAccounts.fields.displayName",
                                          )}
                                        </label>
                                        <div className="flex min-w-0 flex-1 items-center justify-end gap-2">
                                          <OauthMailboxChip
                                            emailAddress={rowMailboxAddress}
                                            emptyLabel={t(
                                              "accountPool.upstreamAccounts.oauth.mailboxEmpty",
                                            )}
                                            copyAriaLabel={t(
                                              "accountPool.upstreamAccounts.actions.copyMailbox",
                                            )}
                                            copyHintLabel={t(
                                              "accountPool.upstreamAccounts.actions.copyMailboxHint",
                                            )}
                                            copiedLabel={t(
                                              "accountPool.upstreamAccounts.actions.copied",
                                            )}
                                            manualCopyLabel={t(
                                              "accountPool.upstreamAccounts.actions.manualCopyMailbox",
                                            )}
                                            manualBadgeLabel={t(
                                              "accountPool.upstreamAccounts.actions.manual",
                                            )}
                                            tone={row.mailboxTone}
                                            onCopy={() =>
                                              void handleBatchCopyMailbox(
                                                row.id,
                                              )
                                            }
                                            editor={{
                                              draftValue:
                                                row.mailboxEditorValue,
                                              inputName: `batchOauthMailboxEditor-${row.id}`,
                                              inputAriaLabel: t(
                                                "accountPool.upstreamAccounts.fields.mailboxAddress",
                                              ),
                                              inputPlaceholder: t(
                                                "accountPool.upstreamAccounts.oauth.mailboxInputPlaceholder",
                                              ),
                                              editAriaLabel: t(
                                                "accountPool.upstreamAccounts.batchOauth.actions.editMailbox",
                                              ),
                                              editHintLabel: t(
                                                "accountPool.upstreamAccounts.batchOauth.tooltip.editMailboxBody",
                                              ),
                                              submitAriaLabel: t(
                                                "accountPool.upstreamAccounts.batchOauth.actions.submitMailbox",
                                              ),
                                              cancelAriaLabel: t(
                                                "accountPool.upstreamAccounts.batchOauth.actions.cancelMailboxEdit",
                                              ),
                                              startEditing: () =>
                                                handleBatchStartMailboxEdit(
                                                  row.id,
                                                ),
                                              onDraftValueChange: (value) =>
                                                handleBatchMailboxEditorValueChange(
                                                  row.id,
                                                  value,
                                                ),
                                              onSubmit: () =>
                                                void handleBatchAttachMailbox(
                                                  row.id,
                                                ),
                                              onCancel: () =>
                                                handleBatchCancelMailboxEdit(
                                                  row.id,
                                                ),
                                              editing: row.mailboxEditorOpen,
                                              busy:
                                                row.mailboxBusyAction ===
                                                "attach",
                                              inputInvalid:
                                                row.mailboxEditorError != null,
                                              inputError:
                                                row.mailboxEditorError,
                                              disabled:
                                                oauthLocked,
                                              submitDisabled:
                                                !row.mailboxEditorValue.trim() ||
                                                row.mailboxEditorError != null,
                                            }}
                                          />
                                          {row.mailboxSession ? (
                                            <Badge
                                              variant={
                                                row.mailboxSession.source ===
                                                "attached"
                                                  ? "secondary"
                                                  : "success"
                                              }
                                            >
                                              {row.mailboxSession.source ===
                                              "attached"
                                                ? t(
                                                    "accountPool.upstreamAccounts.oauth.mailboxAttached",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.oauth.mailboxGenerated",
                                                  )}
                                            </Badge>
                                          ) : null}
                                          <Tooltip
                                            content={buildActionTooltip(
                                              t(
                                                "accountPool.upstreamAccounts.actions.generateMailbox",
                                              ),
                                              t(
                                                "accountPool.upstreamAccounts.oauth.mailboxHint",
                                              ),
                                            )}
                                          >
                                            <Button
                                              type="button"
                                              size="icon"
                                              variant="secondary"
                                              className="h-7 w-7 shrink-0 rounded-full"
                                              aria-label={t(
                                                "accountPool.upstreamAccounts.actions.generateMailbox",
                                              )}
                                              title={t(
                                                "accountPool.upstreamAccounts.actions.generateMailbox",
                                              )}
                                              onClick={() =>
                                                void handleBatchGenerateMailbox(
                                                  row.id,
                                                )
                                              }
                                              disabled={oauthLocked}
                                            >
                                              {row.mailboxBusyAction ===
                                              "generate" ? (
                                                <AppIcon
                                                  name="loading"
                                                  className="h-3.5 w-3.5 animate-spin"
                                                  aria-hidden
                                                />
                                              ) : (
                                                <AppIcon
                                                  name="auto-fix"
                                                  className="h-3.5 w-3.5"
                                                  aria-hidden
                                                />
                                              )}
                                            </Button>
                                          </Tooltip>
                                        </div>
                                      </div>
                                      <div className="relative">
                                        <Input
                                          id={`batch-oauth-display-name-${row.id}`}
                                          name={`batchOauthDisplayName-${row.id}`}
                                          value={row.displayName}
                                          disabled={metadataLocked}
                                          aria-invalid={
                                            duplicateNameError != null
                                          }
                                          className="min-w-0"
                                          onChange={(event) =>
                                            handleBatchMetadataChange(
                                              row.id,
                                              "displayName",
                                              event.target.value,
                                            )
                                          }
                                          onBlur={() =>
                                            handleBatchCompletedTextFieldBlur(
                                              row.id,
                                              "displayName",
                                            )
                                          }
                                          onKeyDown={
                                            handleBatchCompletedTextFieldKeyDown
                                          }
                                        />
                                        {duplicateNameError ? (
                                          <FloatingFieldError
                                            message={duplicateNameError}
                                          />
                                        ) : null}
                                      </div>
                                    </div>
                                    <label className="field min-w-0 gap-2 whitespace-nowrap">
                                      <span className="field-label">
                                        {t(
                                          "accountPool.upstreamAccounts.fields.groupName",
                                        )}
                                      </span>
                                      <div className="flex min-w-0 items-center gap-2">
                                        <UpstreamAccountGroupCombobox
                                          name={`batchOauthGroupName-${row.id}`}
                                          value={row.groupName}
                                          suggestions={groupSuggestions}
                                          placeholder={t(
                                            "accountPool.upstreamAccounts.fields.groupNamePlaceholder",
                                          )}
                                          searchPlaceholder={t(
                                            "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                                          )}
                                          emptyLabel={t(
                                            "accountPool.upstreamAccounts.fields.groupNameEmpty",
                                          )}
                                          createLabel={(value) =>
                                            t(
                                              "accountPool.upstreamAccounts.fields.groupNameUseValue",
                                              { value },
                                            )
                                          }
                                          onValueChange={(value) =>
                                            handleBatchGroupValueChange(
                                              row.id,
                                              value,
                                            )
                                          }
                                          disabled={metadataLocked}
                                          className="min-w-0 flex-1"
                                          triggerClassName="min-w-0 whitespace-nowrap"
                                        />
                                        <Button
                                          type="button"
                                          size="icon"
                                          variant={
                                            hasGroupNote(row.groupName)
                                              ? "secondary"
                                              : "outline"
                                          }
                                          className="h-10 w-10 shrink-0 rounded-full"
                                          aria-label={t(
                                            "accountPool.upstreamAccounts.groupNotes.actions.edit",
                                          )}
                                          title={t(
                                            "accountPool.upstreamAccounts.groupNotes.actions.edit",
                                          )}
                                          onClick={() =>
                                            openGroupNoteEditor(row.groupName)
                                          }
                                          disabled={
                                            !writesEnabled ||
                                            metadataLocked ||
                                            !normalizeGroupName(row.groupName)
                                          }
                                        >
                                          <AppIcon
                                            name="file-document-edit-outline"
                                            className="h-4 w-4"
                                            aria-hidden
                                          />
                                        </Button>
                                      </div>
                                    </label>
                                    {row.noteExpanded ? (
                                      <label className="field min-w-0 gap-2 whitespace-nowrap">
                                        <span className="field-label">
                                          {t(
                                            "accountPool.upstreamAccounts.fields.note",
                                          )}
                                        </span>
                                        <Input
                                          name={`batchOauthNote-${row.id}`}
                                          value={row.note}
                                          disabled={metadataLocked}
                                          className="min-w-0"
                                          onChange={(event) =>
                                            handleBatchMetadataChange(
                                              row.id,
                                              "note",
                                              event.target.value,
                                            )
                                          }
                                          onBlur={() =>
                                            handleBatchCompletedTextFieldBlur(
                                              row.id,
                                              "note",
                                            )
                                          }
                                          onKeyDown={
                                            handleBatchCompletedTextFieldKeyDown
                                          }
                                        />
                                      </label>
                                    ) : null}
                                  </div>
                                </td>
                                <td className="px-3 py-4">
                                  <div className="grid gap-3">
                                    <label className="field min-w-0 gap-2 whitespace-nowrap">
                                      <span className="field-label">
                                        {t(
                                          "accountPool.upstreamAccounts.oauth.callbackUrlLabel",
                                        )}
                                      </span>
                                      <Input
                                        name={`batchOauthCallbackUrl-${row.id}`}
                                        value={row.callbackUrl}
                                        disabled={oauthLocked}
                                        placeholder={t(
                                          "accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder",
                                        )}
                                        className="min-w-0"
                                        onChange={(event) =>
                                          handleBatchMetadataChange(
                                            row.id,
                                            "callbackUrl",
                                            event.target.value,
                                          )
                                        }
                                      />
                                    </label>
                                    <div className="flex items-center gap-3">
                                      <div className="flex flex-wrap items-center gap-2">
                                        <div className="flex items-center gap-1 rounded-full bg-base-200/80 p-1">
                                          <Tooltip
                                            content={buildActionTooltip(
                                              isPending
                                                ? t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateTitle",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.generateTitle",
                                                  ),
                                              isPending
                                                ? t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateBody",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.generateBody",
                                                  ),
                                            )}
                                          >
                                            <Button
                                              type="button"
                                              size="icon"
                                              variant={
                                                isPending
                                                  ? "destructive"
                                                  : "default"
                                              }
                                              className="h-9 w-9 shrink-0 rounded-full"
                                              aria-label={
                                                isPending
                                                  ? t(
                                                      "accountPool.upstreamAccounts.actions.regenerateOauthUrl",
                                                    )
                                                  : t(
                                                      "accountPool.upstreamAccounts.actions.generateOauthUrl",
                                                    )
                                              }
                                              onClick={() =>
                                                void handleBatchGenerateOauthUrl(
                                                  row.id,
                                                )
                                              }
                                              disabled={oauthLocked}
                                            >
                                              {row.busyAction === "generate" ? (
                                                <Spinner size="sm" />
                                              ) : (
                                                <AppIcon
                                                  name={
                                                    isPending
                                                      ? "refresh"
                                                      : "link-variant-plus"
                                                  }
                                                  className="h-4 w-4"
                                                  aria-hidden
                                                />
                                              )}
                                            </Button>
                                          </Tooltip>
                                          <Tooltip
                                            content={buildActionTooltip(
                                              t(
                                                "accountPool.upstreamAccounts.batchOauth.tooltip.copyTitle",
                                              ),
                                              t(
                                                "accountPool.upstreamAccounts.batchOauth.tooltip.copyBody",
                                              ),
                                            )}
                                          >
                                            <Popover
                                              open={
                                                batchManualCopyRowId === row.id
                                              }
                                              onOpenChange={(
                                                nextOpen: boolean,
                                              ) => {
                                                setBatchManualCopyRowId(
                                                  nextOpen ? row.id : null,
                                                );
                                              }}
                                            >
                                              <PopoverAnchor asChild>
                                                <Button
                                                  type="button"
                                                  size="icon"
                                                  variant={
                                                    authUrl
                                                      ? "default"
                                                      : "secondary"
                                                  }
                                                  className="h-9 w-9 shrink-0 rounded-full"
                                                  aria-label={t(
                                                    "accountPool.upstreamAccounts.actions.copyOauthUrl",
                                                  )}
                                                  onClick={() =>
                                                    void handleBatchCopyOauthUrl(
                                                      row.id,
                                                    )
                                                  }
                                                  disabled={!authUrl || oauthLocked}
                                                >
                                                  <AppIcon
                                                    name="content-copy"
                                                    className="h-4 w-4"
                                                    aria-hidden
                                                  />
                                                </Button>
                                              </PopoverAnchor>
                                              <PopoverContent
                                                align="start"
                                                sideOffset={10}
                                                className="w-[min(32rem,calc(100vw-2rem))] rounded-2xl border-base-300 bg-base-100 p-4 shadow-xl"
                                              >
                                                <div className="space-y-3">
                                                  <div className="space-y-1">
                                                    <p className="text-sm font-semibold text-base-content">
                                                      {t(
                                                        "accountPool.upstreamAccounts.oauth.manualCopyTitle",
                                                      )}
                                                    </p>
                                                    <p className="text-sm text-base-content/65">
                                                      {t(
                                                        "accountPool.upstreamAccounts.oauth.manualCopyDescription",
                                                      )}
                                                    </p>
                                                  </div>
                                                  <textarea
                                                    ref={
                                                      batchManualCopyRowId ===
                                                      row.id
                                                        ? batchManualCopyFieldRef
                                                        : undefined
                                                    }
                                                    readOnly
                                                    value={authUrl}
                                                    className="min-h-28 w-full rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm focus-visible:outline-none"
                                                    onClick={(event) =>
                                                      selectAllReadonlyText(
                                                        event.currentTarget,
                                                      )
                                                    }
                                                    onFocus={(event) =>
                                                      selectAllReadonlyText(
                                                        event.currentTarget,
                                                      )
                                                    }
                                                  />
                                                </div>
                                              </PopoverContent>
                                            </Popover>
                                          </Tooltip>
                                        </div>
                                        {row.mailboxSession ? (
                                          <Tooltip
                                            content={buildActionTooltip(
                                              row.mailboxCodeTone === "copied"
                                                ? t(
                                                    "accountPool.upstreamAccounts.actions.copied",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.copyCodeTitle",
                                                  ),
                                              row.mailboxStatus?.latestCode
                                                ?.value ??
                                                t(
                                                  "accountPool.upstreamAccounts.batchOauth.codeMissing",
                                                ),
                                            )}
                                          >
                                            <Button
                                              type="button"
                                              size="sm"
                                              variant={batchMailboxCodeVariant(
                                                row,
                                              )}
                                              className="h-9 shrink-0 rounded-full px-3 font-mono text-xs font-bold tracking-[0.22em]"
                                              aria-label={t(
                                                "accountPool.upstreamAccounts.actions.copyCode",
                                              )}
                                              onClick={() =>
                                                void handleBatchCopyMailboxCode(
                                                  row.id,
                                                )
                                              }
                                              disabled={
                                                !row.mailboxStatus?.latestCode
                                                  ?.value
                                              }
                                            >
                                              {batchMailboxCodeLabel(row)}
                                            </Button>
                                          </Tooltip>
                                        ) : null}
                                        {row.mailboxSession ? (
                                          <Tooltip
                                            content={buildActionTooltip(
                                              t(
                                                "accountPool.upstreamAccounts.actions.fetchMailboxStatus",
                                              ),
                                              batchMailboxRefreshTooltipDetail(
                                                row,
                                                refreshClockMs,
                                                t,
                                              ),
                                            )}
                                          >
                                            <Button
                                              type="button"
                                              size="sm"
                                              variant={batchMailboxRefreshVariant(
                                                row,
                                              )}
                                              className="h-9 shrink-0 rounded-full px-3 text-xs font-semibold"
                                              aria-label={t(
                                                "accountPool.upstreamAccounts.actions.fetchMailboxStatus",
                                              )}
                                              onClick={() =>
                                                void handleBatchMailboxFetch(
                                                  row.id,
                                                )
                                              }
                                              disabled={
                                                !isRefreshableMailboxSession(
                                                  row.mailboxSession,
                                                ) || row.mailboxRefreshBusy
                                              }
                                            >
                                              {row.mailboxRefreshBusy ? (
                                                <Spinner
                                                  size="sm"
                                                  className="mr-1.5"
                                                />
                                              ) : (
                                                <AppIcon
                                                  name="refresh"
                                                  className="mr-1.5 h-3.5 w-3.5"
                                                  aria-hidden
                                                />
                                              )}
                                              {batchMailboxRefreshLabel(
                                                row,
                                                refreshClockMs,
                                                t,
                                              )}
                                            </Button>
                                          </Tooltip>
                                        ) : null}
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.noteTitle",
                                            ),
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.noteBody",
                                            ),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant="ghost"
                                            className={cn(
                                              "h-9 w-9 shrink-0 rounded-full border shadow-sm",
                                              row.noteExpanded ||
                                                row.note.trim()
                                                ? "border-base-300 bg-base-100 text-base-content hover:bg-base-100"
                                                : "border-base-300/80 bg-base-100/72 text-base-content/68 hover:border-base-300 hover:bg-base-100",
                                            )}
                                            aria-label={
                                              row.noteExpanded
                                                ? t(
                                                    "accountPool.upstreamAccounts.batchOauth.actions.collapseNote",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.actions.expandNote",
                                                  )
                                            }
                                            onClick={() =>
                                              toggleBatchNoteExpanded(row.id)
                                            }
                                          >
                                            <AppIcon
                                              name={
                                                row.noteExpanded
                                                  ? "chevron-up"
                                                  : "note-text-outline"
                                              }
                                              className="h-4 w-4"
                                              aria-hidden
                                            />
                                          </Button>
                                        </Tooltip>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.completeTitle",
                                            ),
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.completeBody",
                                            ),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={t(
                                              "accountPool.upstreamAccounts.actions.completeOauth",
                                            )}
                                            onClick={() =>
                                              void handleBatchCompleteOauth(
                                                row.id,
                                              )
                                            }
                                            disabled={
                                              !writesEnabled ||
                                              oauthLocked ||
                                              isCompleted ||
                                              !isPending ||
                                              !row.callbackUrl.trim() ||
                                              duplicateNameError != null
                                            }
                                          >
                                            {row.busyAction === "complete" ? (
                                              <Spinner size="sm" />
                                            ) : (
                                              <AppIcon
                                                name="check-bold"
                                                className="h-4 w-4"
                                                aria-hidden
                                              />
                                            )}
                                          </Button>
                                        </Tooltip>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.motherTitle",
                                            ),
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.motherBody",
                                            ),
                                          )}
                                        >
                                          <MotherAccountToggle
                                            checked={row.isMother}
                                            disabled={metadataLocked}
                                            iconOnly
                                            label={t(
                                              "accountPool.upstreamAccounts.mother.badge",
                                            )}
                                            ariaLabel={t(
                                              "accountPool.upstreamAccounts.batchOauth.actions.toggleMother",
                                            )}
                                            onToggle={() =>
                                              handleBatchMotherToggle(row.id)
                                            }
                                          />
                                        </Tooltip>
                                        {row.duplicateWarning ? (
                                          <DuplicateWarningPopover
                                            duplicateWarning={
                                              row.duplicateWarning
                                            }
                                            summaryTitle={t(
                                              "accountPool.upstreamAccounts.duplicate.compactTitle",
                                            )}
                                            summaryBody={t(
                                              "accountPool.upstreamAccounts.duplicate.compactBody",
                                              {
                                                reasons: formatDuplicateReasons(
                                                  row.duplicateWarning,
                                                ),
                                                peers:
                                                  row.duplicateWarning.peerAccountIds.join(
                                                    ", ",
                                                  ),
                                              },
                                            )}
                                            openDetailsLabel={t(
                                              "accountPool.upstreamAccounts.actions.openDetails",
                                            )}
                                            onOpenDetails={
                                              openDuplicateDetailDialog
                                            }
                                          />
                                        ) : null}
                                      </div>
                                      <div className="ml-auto flex shrink-0 items-center gap-2">
                                        <Badge
                                          variant={batchStatusVariant(status)}
                                        >
                                          {t(
                                            `accountPool.upstreamAccounts.batchOauth.status.${status}`,
                                          )}
                                        </Badge>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.removeTitle",
                                            ),
                                            t(
                                              "accountPool.upstreamAccounts.batchOauth.tooltip.removeBody",
                                            ),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant="destructive"
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={t(
                                              "accountPool.upstreamAccounts.batchOauth.actions.removeRow",
                                            )}
                                            onClick={() =>
                                              removeBatchRow(row.id)
                                            }
                                            disabled={
                                              oauthLocked || isCompleted
                                            }
                                          >
                                            <AppIcon
                                              name="delete-outline"
                                              className="h-4 w-4"
                                              aria-hidden
                                            />
                                          </Button>
                                        </Tooltip>
                                      </div>
                                    </div>
                                    <p
                                      className={cn(
                                        "text-xs leading-5",
                                        row.mailboxError
                                          ? isExpiredIso(
                                              row.mailboxSession?.expiresAt,
                                            )
                                            ? "text-warning/90"
                                            : "text-error"
                                          : "text-base-content/65",
                                      )}
                                    >
                                      {statusDetail ??
                                        t(
                                          "accountPool.upstreamAccounts.batchOauth.statusDetail.draft",
                                        )}
                                    </p>
                                  </div>
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  </div>

                  <p className="text-sm text-base-content/65">
                    {t("accountPool.upstreamAccounts.batchOauth.footerHint")}
                  </p>
                </>
              ) : activeTab === "import" ? (
                <>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.import.fileInputLabel")}
                    </span>
                    <Input
                      key={importInputKey}
                      type="file"
                      name="importOauthFiles"
                      accept=".json,application/json"
                      multiple
                      onChange={(event) => void handleImportFilesChange(event)}
                      disabled={!writesEnabled}
                    />
                  </label>
                  <div className="md:col-span-2 rounded-2xl border border-base-300/80 bg-base-200/35 p-4">
                    <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                      <div>
                        <p className="text-sm font-semibold text-base-content">
                          {t(
                            "accountPool.upstreamAccounts.import.selectedFilesTitle",
                          )}
                        </p>
                        <p className="mt-1 text-sm text-base-content/65">
                          {importSelectionLabel ??
                            t(
                              "accountPool.upstreamAccounts.import.selectedFilesEmpty",
                            )}
                        </p>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={handleClearImportSelection}
                        disabled={importFiles.length === 0}
                      >
                        {t(
                          "accountPool.upstreamAccounts.import.clearSelection",
                        )}
                      </Button>
                    </div>
                    {importFiles.length > 0 ? (
                      <div className="mt-3 flex flex-wrap gap-2">
                        {importFiles.map((item) => (
                          <Badge
                            key={item.sourceId}
                            variant="secondary"
                            className="max-w-full"
                          >
                            <span className="truncate">{item.fileName}</span>
                          </Badge>
                        ))}
                      </div>
                    ) : null}
                  </div>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.groupName")}
                    </span>
                    <div className="flex items-center gap-2">
                      <UpstreamAccountGroupCombobox
                        name="importGroupName"
                        value={importGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t(
                          "accountPool.upstreamAccounts.import.defaultGroupPlaceholder",
                        )}
                        searchPlaceholder={t(
                          "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                        )}
                        emptyLabel={t(
                          "accountPool.upstreamAccounts.fields.groupNameEmpty",
                        )}
                        createLabel={(value) =>
                          t(
                            "accountPool.upstreamAccounts.fields.groupNameUseValue",
                            { value },
                          )
                        }
                        onValueChange={setImportGroupName}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          hasGroupNote(importGroupName)
                            ? "secondary"
                            : "outline"
                        }
                        className="shrink-0 rounded-full"
                        aria-label={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        title={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        onClick={() => openGroupNoteEditor(importGroupName)}
                        disabled={
                          !writesEnabled || !normalizeGroupName(importGroupName)
                        }
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </Button>
                    </div>
                    <p className="mt-2 text-xs text-base-content/65">
                      {t(
                        "accountPool.upstreamAccounts.import.defaultMetadataHint",
                      )}
                    </p>
                  </label>
                  <div className="md:col-span-2">
                    <AccountTagField
                      tags={tagItems}
                      selectedTagIds={importTagIds}
                      writesEnabled={writesEnabled}
                      pageCreatedTagIds={pageCreatedTagIds}
                      labels={tagFieldLabels}
                      onChange={setImportTagIds}
                      onCreateTag={handleCreateTag}
                      onUpdateTag={updateTag}
                      onDeleteTag={handleDeleteTag}
                    />
                  </div>
                  <div className="md:col-span-2 flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">
                        {t("accountPool.upstreamAccounts.actions.cancel")}
                      </Link>
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleValidateImportedOauth()}
                      disabled={!writesEnabled || importFiles.length === 0}
                    >
                      <AppIcon
                        name="check-decagram-outline"
                        className="mr-2 h-4 w-4"
                        aria-hidden
                      />
                      {t("accountPool.upstreamAccounts.import.validateAction")}
                    </Button>
                  </div>
                </>
              ) : (
                <>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.displayName")}
                    </span>
                    <div className="relative">
                      <Input
                        name="apiKeyDisplayName"
                        value={apiKeyDisplayName}
                        aria-invalid={apiKeyDisplayNameConflict != null}
                        onChange={(event) =>
                          setApiKeyDisplayName(event.target.value)
                        }
                      />
                      {apiKeyDisplayNameConflict ? (
                        <FloatingFieldError
                          message={t(
                            "accountPool.upstreamAccounts.validation.displayNameDuplicate",
                          )}
                        />
                      ) : null}
                    </div>
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.groupName")}
                    </span>
                    <div className="flex items-center gap-2">
                      <UpstreamAccountGroupCombobox
                        name="apiKeyGroupName"
                        value={apiKeyGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t(
                          "accountPool.upstreamAccounts.fields.groupNamePlaceholder",
                        )}
                        searchPlaceholder={t(
                          "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                        )}
                        emptyLabel={t(
                          "accountPool.upstreamAccounts.fields.groupNameEmpty",
                        )}
                        createLabel={(value) =>
                          t(
                            "accountPool.upstreamAccounts.fields.groupNameUseValue",
                            { value },
                          )
                        }
                        onValueChange={setApiKeyGroupName}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          hasGroupNote(apiKeyGroupName)
                            ? "secondary"
                            : "outline"
                        }
                        className="shrink-0 rounded-full"
                        aria-label={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        title={t(
                          "accountPool.upstreamAccounts.groupNotes.actions.edit",
                        )}
                        onClick={() => openGroupNoteEditor(apiKeyGroupName)}
                        disabled={
                          !writesEnabled || !normalizeGroupName(apiKeyGroupName)
                        }
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </Button>
                    </div>
                  </label>
                  <div className="md:col-span-2">
                    <MotherAccountToggle
                      checked={apiKeyIsMother}
                      disabled={!writesEnabled}
                      label={t(
                        "accountPool.upstreamAccounts.mother.toggleLabel",
                      )}
                      description={t(
                        "accountPool.upstreamAccounts.mother.toggleDescription",
                      )}
                      onToggle={() => setApiKeyIsMother((current) => !current)}
                    />
                  </div>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.apiKey")}
                    </span>
                    <Input
                      name="apiKeyValue"
                      value={apiKeyValue}
                      onChange={(event) => setApiKeyValue(event.target.value)}
                    />
                  </label>
                  <label className="field md:col-span-2">
                    <FormFieldFeedback
                      label={t(
                        "accountPool.upstreamAccounts.fields.upstreamBaseUrl",
                      )}
                      message={apiKeyUpstreamBaseUrlError}
                      messageClassName="md:max-w-[min(30rem,calc(100%-9rem))]"
                    />
                    <div className="relative">
                      <Input
                        name="apiKeyUpstreamBaseUrl"
                        value={apiKeyUpstreamBaseUrl}
                        onChange={(event) =>
                          setApiKeyUpstreamBaseUrl(event.target.value)
                        }
                        placeholder={t(
                          "accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder",
                        )}
                        autoCapitalize="none"
                        spellCheck={false}
                        aria-invalid={
                          apiKeyUpstreamBaseUrlError ? "true" : "false"
                        }
                        className={cn(
                          apiKeyUpstreamBaseUrlError
                            ? "border-error/70 focus-visible:ring-error"
                            : "",
                        )}
                      />
                    </div>
                  </label>
                  <label className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.primaryLimit")}
                    </span>
                    <Input
                      name="apiKeyPrimaryLimit"
                      value={apiKeyPrimaryLimit}
                      onChange={(event) =>
                        setApiKeyPrimaryLimit(event.target.value)
                      }
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.secondaryLimit")}
                    </span>
                    <Input
                      name="apiKeySecondaryLimit"
                      value={apiKeySecondaryLimit}
                      onChange={(event) =>
                        setApiKeySecondaryLimit(event.target.value)
                      }
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.limitUnit")}
                    </span>
                    <Input
                      name="apiKeyLimitUnit"
                      value={apiKeyLimitUnit}
                      onChange={(event) =>
                        setApiKeyLimitUnit(event.target.value)
                      }
                    />
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.fields.note")}
                    </span>
                    <textarea
                      className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                      name="apiKeyNote"
                      value={apiKeyNote}
                      onChange={(event) => setApiKeyNote(event.target.value)}
                    />
                  </label>
                  <div className="md:col-span-2">
                    <AccountTagField
                      tags={tagItems}
                      selectedTagIds={apiKeyTagIds}
                      writesEnabled={writesEnabled}
                      pageCreatedTagIds={pageCreatedTagIds}
                      labels={tagFieldLabels}
                      onChange={setApiKeyTagIds}
                      onCreateTag={handleCreateTag}
                      onUpdateTag={updateTag}
                      onDeleteTag={handleDeleteTag}
                    />
                  </div>
                  <div className="md:col-span-2 flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">
                        {t("accountPool.upstreamAccounts.actions.cancel")}
                      </Link>
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleCreateApiKey()}
                      disabled={
                        busyAction === "apiKey" ||
                        !writesEnabled ||
                        apiKeyDisplayNameConflict != null ||
                        Boolean(apiKeyUpstreamBaseUrlError)
                      }
                    >
                      {busyAction === "apiKey" ? (
                        <AppIcon
                          name="loading"
                          className="mr-2 h-4 w-4 animate-spin"
                          aria-hidden
                        />
                      ) : (
                        <AppIcon
                          name="content-save-plus-outline"
                          className="mr-2 h-4 w-4"
                          aria-hidden
                        />
                      )}
                      {t("accountPool.upstreamAccounts.actions.createApiKey")}
                    </Button>
                  </div>
                </>
              )}
            </CardContent>
          </Card>
        </div>
      </section>
      <ImportedOauthValidationDialog
        open={importValidationDialogOpen}
        state={importValidationState}
        onClose={handleCloseImportedOauthValidationDialog}
        onRetryFailed={() => void handleRetryImportedOauthFailed()}
        onRetryOne={(sourceId) => void handleRetryImportedOauthOne(sourceId)}
        onImportValid={() => void handleImportValidatedOauth()}
      />
      <UpstreamAccountGroupNoteDialog
        open={groupNoteEditor.open}
        groupName={groupNoteEditor.groupName}
        note={groupNoteEditor.note}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({ ...current, note: value }));
        }}
        onClose={closeGroupNoteEditor}
        onSave={() => void handleSaveGroupNote()}
        title={t("accountPool.upstreamAccounts.groupNotes.dialogTitle")}
        existingDescription={t(
          "accountPool.upstreamAccounts.groupNotes.existingDescription",
        )}
        draftDescription={t(
          "accountPool.upstreamAccounts.groupNotes.draftDescription",
        )}
        noteLabel={t("accountPool.upstreamAccounts.fields.note")}
        notePlaceholder={t(
          "accountPool.upstreamAccounts.groupNotes.notePlaceholder",
        )}
        cancelLabel={t("accountPool.upstreamAccounts.actions.cancel")}
        saveLabel={t("accountPool.upstreamAccounts.actions.save")}
        closeLabel={t("accountPool.upstreamAccounts.actions.closeDetails")}
        existingBadgeLabel={t(
          "accountPool.upstreamAccounts.groupNotes.badges.existing",
        )}
        draftBadgeLabel={t(
          "accountPool.upstreamAccounts.groupNotes.badges.draft",
        )}
      />
      <DuplicateAccountDetailDialog
        open={duplicateDetailOpen}
        detail={duplicateDetail}
        isLoading={duplicateDetailLoading}
        onClose={() => {
          setDuplicateDetailOpen(false);
          setDuplicateDetail(null);
        }}
        title={t("accountPool.upstreamAccounts.detailTitle")}
        description={t("accountPool.upstreamAccounts.detailEmptyDescription")}
        duplicateLabel={t("accountPool.upstreamAccounts.duplicate.badge")}
        closeLabel={t("accountPool.upstreamAccounts.actions.closeDetails")}
        formatDuplicateReasons={formatDuplicateReasons}
        statusLabel={accountStatusLabel}
        kindLabel={accountKindLabel}
        fieldLabels={{
          groupName: t("accountPool.upstreamAccounts.fields.groupName"),
          email: t("accountPool.upstreamAccounts.fields.email"),
          accountId: t("accountPool.upstreamAccounts.fields.accountId"),
          userId: t("accountPool.upstreamAccounts.fields.userId"),
          lastSuccessSync: t(
            "accountPool.upstreamAccounts.fields.lastSuccessSync",
          ),
        }}
      />
    </div>
  );
}
