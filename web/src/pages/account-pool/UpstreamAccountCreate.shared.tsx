import type { ImportedOauthValidationDialogState } from "../../features/account-pool/ImportedOauthValidationDialog";
import type {
  LoginSessionStatusResponse,
  OauthMailboxSession,
  OauthMailboxSessionSupported,
  OauthMailboxStatus,
  UpdateOauthLoginSessionPayload,
  UpstreamAccountSummary,
} from "../../lib/api";
import { normalizeMotherGroupKey } from "../../lib/upstreamMother";

export {
  accountKindVariant,
  accountStatusVariant,
  DuplicateAccountDetailDialog,
  DuplicateDetailField,
  DuplicateWarningPopover,
} from "./UpstreamAccountCreate.duplicate-ui";
export { OAuthIdentityConfirmationAlert } from "./UpstreamAccountCreate.identity-confirmation";

export type {
  ParsedImportedOauthCredentialCandidate,
  ParsedImportedOauthCredentialRejection,
} from "./UpstreamAccountCreate.imported-oauth-parser";
export {
  buildImportedOauthMatchKeyFromValues,
  convertImportedWebSessionRecord,
} from "./UpstreamAccountCreate.imported-oauth-parser";
export {
  applyImportedOauthDuplicateStatuses,
  buildImportedOauthMatchKey,
  buildImportedOauthPendingState,
  buildImportedOauthStateFromRows,
  buildImportedOauthStateFromSnapshot,
  chunkImportedOauthItems,
  convertImportedWebSessionDocumentLocally,
  formatImportedOauthSelectionLabel,
  getImportedOauthPasteValidationError,
  getImportedOauthValidationStatusLabel,
  markImportedOauthRowsAsError,
  mergeImportedOauthValidationRow,
  mergeImportedOauthValidationRows,
  parseImportedOauthCredentialDocumentLocally,
  parseImportedOauthPasteDraft,
  replaceImportedOauthValidationRows,
  summarizeImportedOauthBatchErrors,
  validateImportedOauthCredentialLocally,
} from "./UpstreamAccountCreate.imported-oauth-state";

export type CreateTab = "oauth" | "batchOauth" | "apiKey" | "import" | "importSession";
type BatchOauthBusyAction = "generate" | "complete" | "confirm" | null;

export type { ImportedOauthValidationDialogState };
export type MailboxBusyAction = "attach" | "generate" | null;
export type BatchOauthPersistedMetadata = {
  displayName: string;
  groupName: string;
  note: string;
  isMother: boolean;
  tagIds: number[];
};
export type DuplicateWarningState = {
  accountId: number;
  displayName: string;
  peerAccountIds: number[];
  reasons: string[];
};
export type GroupNoteEditorState = {
  open: boolean;
  groupName: string;
  note: string;
  existing: boolean;
  accountCount: number;
  concurrencyLimit: number;
  boundProxyKeys: string[];
  nodeShuntEnabled: boolean;
  singleAccountRotationEnabled: boolean;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
  onSaved?: ((groupName: string) => void) | null;
  onDeleted?: ((groupName: string) => void) | null;
};
export type MailboxCopyTone = "idle" | "copied" | "manual";
export const MAILBOX_REFRESH_INTERVAL_MS = 5_000;
export const MAILBOX_REFRESH_TICK_MS = 1_000;
export const OAUTH_SESSION_SYNC_DEBOUNCE_MS = 300;
export const OAUTH_SESSION_SYNC_RETRY_MS = 1_000;
export const MAX_SHARED_TAG_SYNC_ATTEMPTS = 2;
export const GROUP_UPSTREAM_429_RETRY_OPTIONS = [1, 2, 3, 4, 5] as const;
export type PendingOauthSessionSnapshot = {
  loginId: string;
  payload: UpdateOauthLoginSessionPayload;
  signature: string;
  baseUpdatedAt: string | null;
};

export type BatchOauthRow = {
  id: string;
  displayName: string;
  email: string;
  verifiedEmail?: string | null;
  planType?: string | null;
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
  emailResolution?: {
    accountId: number;
    verifiedEmail: string;
    chosenEmail: string;
    displayName: string;
    planType?: string | null;
  } | null;
  pendingSharedTagIds: number[] | null;
  sharedTagSyncAttempts: number;
};

export type CreatePageDraft = {
  oauth?: {
    displayName?: string;
    email?: string;
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
    // Accepted only to discard stale local drafts created before transit accounts were separated.
    groupName?: string;
    note?: string;
    tagIds?: number[];
    boundProxyKeys?: string[];
    apiKeyValue?: string;
    upstreamBaseUrl?: string;
    primaryLimit?: string;
    secondaryLimit?: string;
    limitUnit?: string;
  };
};

export type CreatePageLocationState = {
  draft?: CreatePageDraft;
} | null;

export function normalizeNumberInput(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : undefined;
}

export function normalizeBoundProxyKeys(values?: string[]): string[] {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(values.map((value) => value.trim()).filter((value) => value.length > 0)),
  );
}

export function normalizeGroupUpstream429MaxRetries(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

export function normalizeEnabledGroupUpstream429MaxRetries(value?: number | null): number {
  return Math.max(1, normalizeGroupUpstream429MaxRetries(value) || 1);
}

export function formatDateTime(value?: string | null) {
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

export function formatRelativeRefreshCountdown(
  nextRefreshAt: number | null,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!nextRefreshAt) return t("accountPool.upstreamAccounts.oauth.refreshScheduledUnknown");
  const seconds = Math.max(0, Math.ceil((nextRefreshAt - now) / 1000));
  return t("accountPool.upstreamAccounts.oauth.refreshIn", { seconds });
}

export function formatCountdownClock(targetTimestamp: number, now: number) {
  const totalSeconds = Math.max(0, Math.ceil((targetTimestamp - now) / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export function isActivePendingOauthSession(
  session: LoginSessionStatusResponse | null | undefined,
) {
  return Boolean(
    session && session.status === "pending" && session.authUrl && !isExpiredIso(session.expiresAt),
  );
}

export function batchOauthSessionRemainingLabel(
  session: LoginSessionStatusResponse | null | undefined,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!session?.expiresAt) return null;
  const expiresAt = Date.parse(session.expiresAt);
  if (!Number.isFinite(expiresAt) || expiresAt <= now) return null;
  return t("accountPool.upstreamAccounts.batchOauth.oauthAction.remaining", {
    time: formatCountdownClock(expiresAt, now),
  });
}

export function batchOauthSessionExpiresAtLabel(
  session: LoginSessionStatusResponse | null | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!session?.expiresAt) return null;
  return t("accountPool.upstreamAccounts.batchOauth.oauthAction.expiresAt", {
    timestamp: formatDateTime(session.expiresAt),
  });
}

export function parseAccountId(search: string): number | null {
  const value = new URLSearchParams(search).get("accountId");
  if (!value) return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

export function parseCreateMode(search: string): CreateTab {
  const value = new URLSearchParams(search).get("mode");
  if (value === "batchOauth") return "batchOauth";
  if (value === "apiKey") return "apiKey";
  if (value === "import") return "import";
  if (value === "importSession") return "importSession";
  return "oauth";
}

export function createBatchOauthRow(id: string, groupName = ""): BatchOauthRow {
  return {
    id,
    displayName: "",
    email: "",
    verifiedEmail: null,
    planType: null,
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
    emailResolution: null,
    pendingSharedTagIds: null,
    sharedTagSyncAttempts: 0,
  };
}

export function normalizeBatchTagIds(tagIds: number[]) {
  return Array.from(new Set(tagIds))
    .filter((value) => Number.isInteger(value) && value > 0)
    .sort((left, right) => left - right);
}

export function batchTagIdsEqual(
  left: number[] | null | undefined,
  right: number[] | null | undefined,
) {
  const normalizedLeft = normalizeBatchTagIds(Array.isArray(left) ? left : []);
  const normalizedRight = normalizeBatchTagIds(Array.isArray(right) ? right : []);
  if (normalizedLeft.length !== normalizedRight.length) return false;
  return normalizedLeft.every((value, index) => value === normalizedRight[index]);
}

export function normalizeBatchOauthPersistedMetadata(
  value: Partial<BatchOauthPersistedMetadata> | null | undefined,
): BatchOauthPersistedMetadata | null {
  if (!value) return null;
  return {
    displayName: typeof value.displayName === "string" ? value.displayName.trim() : "",
    groupName: typeof value.groupName === "string" ? value.groupName.trim() : "",
    note: typeof value.note === "string" ? value.note.trim() : "",
    isMother: value.isMother === true,
    tagIds: normalizeBatchTagIds(Array.isArray(value.tagIds) ? value.tagIds : []),
  };
}

export function buildBatchOauthPersistedMetadata(
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

export function resolveCompletedBatchOauthCommittedFieldBaseline(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  field: keyof BatchOauthPersistedMetadata,
  items: UpstreamAccountSummary[],
) {
  if (field === "tagIds") {
    return resolveCompletedBatchOauthRowPersistedTagIds(row, items);
  }
  if (row.metadataPersisted) {
    return row.metadataPersisted[field];
  }
  const account = findCompletedBatchOauthAccount(row, items);
  if (!account) return undefined;
  if (field === "displayName") {
    return account.displayName.trim();
  }
  if (field === "groupName") {
    return (account.groupName ?? "").trim();
  }
  if (field === "isMother") {
    return account.isMother === true;
  }
  return undefined;
}

export function didCompletedBatchOauthCommittedFieldsChange(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  next: BatchOauthPersistedMetadata,
  committedFields: Array<keyof BatchOauthPersistedMetadata>,
  items: UpstreamAccountSummary[],
) {
  return committedFields.some((field) => {
    if (field === "tagIds") {
      const baselineTagIds = resolveCompletedBatchOauthRowPersistedTagIds(row, items);
      return baselineTagIds == null ? true : !batchTagIdsEqual(baselineTagIds, next.tagIds);
    }
    const baseline = resolveCompletedBatchOauthCommittedFieldBaseline(row, field, items);
    if (baseline == null) return true;
    return baseline !== next[field];
  });
}

export function findCompletedBatchOauthAccount(
  row: Pick<BatchOauthRow, "session">,
  items: UpstreamAccountSummary[],
) {
  const accountId = row.session?.accountId;
  return accountId == null ? null : (items.find((item) => item.id === accountId) ?? null);
}

export function resolveCompletedBatchOauthRowPersistedTagIds(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  items: UpstreamAccountSummary[],
) {
  const account = findCompletedBatchOauthAccount(row, items);
  if (account) {
    return normalizeBatchTagIds(account.tags.map((tag) => tag.id));
  }
  return row.metadataPersisted ? normalizeBatchTagIds(row.metadataPersisted.tagIds) : null;
}

export function resolveCompletedBatchOauthRowBaselineTagIds(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  items: UpstreamAccountSummary[],
  fallbackTagIds: number[],
) {
  return (
    resolveCompletedBatchOauthRowPersistedTagIds(row, items) ?? normalizeBatchTagIds(fallbackTagIds)
  );
}

export function buildCompletedBatchOauthSharedTagBaselineSignature(
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

export function hydrateBatchOauthRow(
  seed: Partial<BatchOauthRow> & { id?: string },
  fallbackId: string,
  fallbackGroupName = "",
): BatchOauthRow {
  const hydratedGroupName = seed.groupName ?? fallbackGroupName;
  return {
    ...createBatchOauthRow(seed.id ?? fallbackId, hydratedGroupName),
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
    mailboxError: typeof seed.mailboxError === "string" ? seed.mailboxError : null,
    mailboxTone:
      seed.mailboxTone === "copied" || seed.mailboxTone === "manual" ? seed.mailboxTone : "idle",
    mailboxCodeTone: seed.mailboxCodeTone === "copied" ? "copied" : "idle",
    mailboxBusyAction:
      seed.mailboxBusyAction === "attach" || seed.mailboxBusyAction === "generate"
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
      typeof seed.mailboxEditorError === "string" ? seed.mailboxEditorError : null,
    mailboxRefreshBusy: seed.mailboxRefreshBusy === true,
    mailboxNextRefreshAt:
      typeof seed.mailboxNextRefreshAt === "number" ? seed.mailboxNextRefreshAt : null,
    metadataBusy: seed.metadataBusy === true,
    metadataError: typeof seed.metadataError === "string" ? seed.metadataError : null,
    metadataPersisted: normalizeBatchOauthPersistedMetadata(seed.metadataPersisted),
    email: typeof seed.email === "string" ? seed.email : "",
    verifiedEmail: typeof seed.verifiedEmail === "string" ? seed.verifiedEmail : null,
    planType: typeof seed.planType === "string" ? seed.planType : null,
    emailResolution: seed.emailResolution
      ? {
          accountId: seed.emailResolution.accountId,
          verifiedEmail: seed.emailResolution.verifiedEmail,
          chosenEmail: seed.emailResolution.chosenEmail,
          displayName: seed.emailResolution.displayName,
          planType: seed.emailResolution.planType ?? null,
        }
      : null,
    pendingSharedTagIds: null,
    sharedTagSyncAttempts: 0,
  };
}

export function createImportedOauthSourceId(file: File, index: number) {
  return `${file.name}:${file.size}:${file.lastModified}:${index}`;
}

export function createImportedOauthPastedSourceId(serial: number) {
  return `pasted:${serial}`;
}

export function createImportedOauthPastedFileName(serial: number) {
  return `Pasted credential #${serial}.json`;
}

export function createImportedSessionPastedFileName(serial: number, index?: number) {
  const suffix = index == null ? "" : `-${index + 1}`;
  return `Pasted session #${serial}${suffix}.json`;
}

export function getNextBatchRowIndex(rows: BatchOauthRow[]) {
  return rows.reduce((max, row) => {
    const matched = /^row-(\d+)$/.exec(row.id);
    const current = matched ? Number(matched[1]) : 0;
    return Number.isFinite(current) ? Math.max(max, current + 1) : max;
  }, 1);
}

export function normalizeDisplayNameKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

export function normalizeEmailKey(value?: string | null) {
  const trimmed = value?.trim();
  return trimmed ? trimmed.toLocaleLowerCase() : "";
}

export function generatedDisplayNameFromEmail(email?: string | null) {
  return normalizeEmailKey(email);
}

export function displayNameFollowsEmail(displayName: string, email?: string | null) {
  const normalizedDisplayName = normalizeDisplayNameKey(displayName);
  if (!normalizedDisplayName) return true;
  const generated = generatedDisplayNameFromEmail(email);
  return generated.length > 0 && normalizedDisplayName === generated;
}

export function resolveDisplayNameAfterEmailChange(
  displayName: string,
  previousEmail?: string | null,
  nextEmail?: string | null,
) {
  return displayNameFollowsEmail(displayName, previousEmail)
    ? generatedDisplayNameFromEmail(nextEmail) || displayName
    : displayName;
}

export function shouldPromptOauthEmailChoice(
  verifiedEmail?: string | null,
  chosenEmail?: string | null,
) {
  const normalizedVerified = normalizeEmailKey(verifiedEmail);
  const normalizedChosen = normalizeEmailKey(chosenEmail);
  return normalizedVerified.length > 0 && normalizedVerified !== normalizedChosen;
}

export function normalizeMailboxAddressKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

export function mailboxInputMatchesSession(
  input: string,
  session: OauthMailboxSessionSupported | null,
) {
  if (!session) return false;
  return normalizeMailboxAddressKey(input) === normalizeMailboxAddressKey(session.emailAddress);
}

export function isProbablyValidEmailAddress(value: string) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
}

export function findDisplayNameConflict(
  items: UpstreamAccountSummary[],
  displayName: string,
  excludeId?: number | null,
) {
  const normalized = normalizeDisplayNameKey(displayName);
  if (!normalized) return null;
  return (
    items.find(
      (item) => item.id !== excludeId && normalizeDisplayNameKey(item.displayName) === normalized,
    ) ?? null
  );
}

export function invalidatePendingSingleOauthSession(
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

export function buildOauthLoginSessionUpdatePayload({
  displayName,
  email,
  groupName,
  groupBoundProxyKeys,
  groupNodeShuntEnabled,
  groupSingleAccountRotationEnabled,
  note,
  groupNote,
  groupConcurrencyLimit,
  includeGroupNote,
  tagIds,
  isMother,
  mailboxSession,
}: {
  displayName: string;
  email?: string | null;
  groupName: string;
  groupBoundProxyKeys: string[];
  groupNodeShuntEnabled: boolean;
  groupSingleAccountRotationEnabled: boolean;
  note: string;
  groupNote: string;
  groupConcurrencyLimit: number;
  includeGroupNote: boolean;
  tagIds: number[];
  isMother: boolean;
  mailboxSession: OauthMailboxSessionSupported | null;
}): UpdateOauthLoginSessionPayload {
  const normalizedEmail = typeof email === "string" ? email.trim() : "";
  const normalizedGroupName = groupName.trim();
  return {
    displayName: displayName.trim(),
    email: normalizedEmail || null,
    groupName: normalizedGroupName,
    groupBoundProxyKeys,
    groupNodeShuntEnabled,
    groupSingleAccountRotationEnabled,
    note: note.trim(),
    ...(normalizedGroupName && includeGroupNote ? { groupNote: groupNote.trim() } : {}),
    ...(normalizedGroupName ? { concurrencyLimit: groupConcurrencyLimit } : {}),
    tagIds,
    isMother,
    mailboxSessionId: mailboxSession?.sessionId ?? "",
    mailboxAddress: mailboxSession?.emailAddress ?? "",
  };
}

export function buildPendingOauthSessionSnapshot(
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

export function shouldRetryPendingOauthSessionSync(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return !/^Request failed: (400|401|403|404|409|410|422)\b/.test(message);
}

export function applyBatchMotherDraftRules(rows: BatchOauthRow[], changedRowId: string) {
  const changedRow = rows.find((row) => row.id === changedRowId);
  if (!changedRow?.isMother) return rows;
  const groupKey = normalizeMotherGroupKey(changedRow.groupName);
  return rows.map((row) =>
    row.id !== changedRowId && row.isMother && normalizeMotherGroupKey(row.groupName) === groupKey
      ? { ...row, isMother: false }
      : row,
  );
}

export function enforceBatchMotherDraftUniqueness(rows: BatchOauthRow[]) {
  const winners = new Map<string, string>();
  for (const row of rows) {
    if (!row.isMother) continue;
    winners.set(normalizeMotherGroupKey(row.groupName), row.id);
  }
  return rows.map((row) =>
    row.isMother && winners.get(normalizeMotherGroupKey(row.groupName)) !== row.id
      ? { ...row, isMother: false }
      : row,
  );
}

export function reconcileBatchOauthMotherRowsAfterSave(
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

export function batchStatusVariant(status: string): "success" | "warning" | "error" | "secondary" {
  if (status === "completed") return "success";
  if (status === "completedNeedsRefresh") return "warning";
  if (status === "pending" || status === "needs_identity_confirmation") return "warning";
  if (status === "failed" || status === "expired") return "error";
  return "secondary";
}

export function batchRowStatus(row: BatchOauthRow) {
  if (row.needsRefresh) return "completedNeedsRefresh";
  return row.session?.status ?? "draft";
}

export function resolveBatchOauthMailboxAddress(row: BatchOauthRow) {
  const status = batchRowStatus(row);
  if (status === "completed" || status === "completedNeedsRefresh") {
    return row.email || row.mailboxSession?.emailAddress || row.mailboxInput;
  }
  return row.mailboxSession?.emailAddress ?? row.mailboxInput;
}

export function canEditCompletedBatchOauthRowMetadata(row: BatchOauthRow) {
  const status = batchRowStatus(row);
  return Boolean(
    row.session?.accountId != null &&
      (status === "completed" || status === "completedNeedsRefresh"),
  );
}

export function batchRowStatusDetail(row: BatchOauthRow) {
  if (row.metadataError) return row.metadataError;
  if (row.actionError) return row.actionError;
  if (row.mailboxError) return row.mailboxError;
  if (row.sessionHint) return row.sessionHint;
  if (row.session?.error) return row.session.error;
  if (row.session?.expiresAt) return formatDateTime(row.session.expiresAt);
  return null;
}

export function batchMailboxCodeVariant(row: BatchOauthRow): "default" | "secondary" | "outline" {
  const code = row.mailboxStatus?.latestCode?.value;
  if (!code) return "secondary";
  return row.mailboxCodeTone === "copied" ? "outline" : "default";
}

export function batchMailboxCodeLabel(row: BatchOauthRow) {
  return row.mailboxStatus?.latestCode?.value ?? "------";
}

export function batchMailboxRefreshVariant(row: BatchOauthRow): "outline" | "secondary" {
  return row.mailboxRefreshBusy ? "secondary" : "outline";
}

export function isExpiredIso(value: string | null | undefined) {
  if (!value) return false;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) && timestamp <= Date.now();
}

export function isRefreshableMailboxSession(
  session: OauthMailboxSessionSupported | null | undefined,
) {
  return Boolean(session && !isExpiredIso(session.expiresAt));
}

export function batchMailboxRefreshLabel(
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
  const seconds = Math.max(0, Math.ceil((row.mailboxNextRefreshAt - now) / 1000));
  return t("accountPool.upstreamAccounts.oauth.refreshInShort", { seconds });
}

export function batchMailboxRefreshTooltipDetail(
  row: BatchOauthRow,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (row.mailboxRefreshBusy) {
    return t("accountPool.upstreamAccounts.oauth.refreshing");
  }
  const receivedAt =
    row.mailboxStatus?.latestCode?.updatedAt ?? row.mailboxStatus?.invite?.updatedAt ?? null;
  if (receivedAt) {
    return `${t("accountPool.upstreamAccounts.oauth.receivedAt", {
      timestamp: formatDateTime(receivedAt),
    })} · ${formatRelativeRefreshCountdown(row.mailboxNextRefreshAt, now, t)}`;
  }
  return formatRelativeRefreshCountdown(row.mailboxNextRefreshAt, now, t);
}

export function resolveMailboxIssue(
  session: OauthMailboxSession | null,
  status: OauthMailboxStatus | null,
  localError: string | null,
  expiresAt: string | null | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (session?.supported === false) {
    if (session.reason === "invalid_format") {
      return t("accountPool.upstreamAccounts.oauth.mailboxUnsupportedInvalidFormat");
    }
    if (session.reason === "unsupported_domain") {
      return t("accountPool.upstreamAccounts.oauth.mailboxUnsupportedDomain");
    }
    return t("accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable");
  }
  if (isExpiredIso(expiresAt)) {
    return t("accountPool.upstreamAccounts.oauth.mailboxExpired");
  }
  if (localError) return localError;
  if (status?.error) return status.error;
  return null;
}

export function isSupportedMailboxSession(
  session: OauthMailboxSession | null,
): session is OauthMailboxSessionSupported {
  return Boolean(session && session.supported !== false);
}

export function buildActionTooltip(title: string, description: string) {
  return (
    <div className="space-y-1">
      <p className="font-semibold text-base-content">{title}</p>
      <p className="leading-5 text-base-content/70">{description}</p>
    </div>
  );
}
