/* eslint-disable react-refresh/only-export-components */
import {
  useEffect,
  useState,
} from "react";
import { AppIcon } from "../../components/AppIcon";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import {
  Popover,
  PopoverArrow,
  PopoverContent,
  PopoverTrigger,
} from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import {
  IMPORT_VALIDATION_PAGE_SIZE,
  type ImportedOauthValidationDialogState,
} from "../../components/ImportedOauthValidationDialog";
import type {
  ImportOauthCredentialFilePayload,
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
  normalizeMotherGroupKey,
} from "../../lib/upstreamMother";

export type CreateTab = "oauth" | "batchOauth" | "apiKey" | "import";
type BatchOauthBusyAction = "generate" | "complete" | null;
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
  concurrencyLimit: number;
  boundProxyKeys: string[];
  nodeShuntEnabled: boolean;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
};
export type MailboxCopyTone = "idle" | "copied" | "manual";
export const MAILBOX_REFRESH_INTERVAL_MS = 5_000;
export const MAILBOX_REFRESH_TICK_MS = 1_000;
export const OAUTH_SESSION_SYNC_DEBOUNCE_MS = 300;
export const OAUTH_SESSION_SYNC_RETRY_MS = 1_000;
export const MAX_SHARED_TAG_SYNC_ATTEMPTS = 2;
export const GROUP_UPSTREAM_429_RETRY_OPTIONS = [1, 2, 3, 4, 5] as const;
export const IMPORTED_OAUTH_DUPLICATE_DETAIL =
  "duplicate credential in current import selection";

export type PendingOauthSessionSnapshot = {
  loginId: string;
  payload: UpdateOauthLoginSessionPayload;
  signature: string;
  baseUpdatedAt: string | null;
};

export type BatchOauthRow = {
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

export type CreatePageDraft = {
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
    new Set(
      values.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
}

export function normalizeGroupUpstream429MaxRetries(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

export function normalizeEnabledGroupUpstream429MaxRetries(
  value?: number | null,
): number {
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
  if (!nextRefreshAt)
    return t("accountPool.upstreamAccounts.oauth.refreshScheduledUnknown");
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
    session &&
    session.status === "pending" &&
    session.authUrl &&
    !isExpiredIso(session.expiresAt),
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
  return "oauth";
}

export function createBatchOauthRow(id: string, groupName = ""): BatchOauthRow {
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
  const normalizedRight = normalizeBatchTagIds(
    Array.isArray(right) ? right : [],
  );
  if (normalizedLeft.length !== normalizedRight.length) return false;
  return normalizedLeft.every(
    (value, index) => value === normalizedRight[index],
  );
}

export function normalizeBatchOauthPersistedMetadata(
  value: Partial<BatchOauthPersistedMetadata> | null | undefined,
): BatchOauthPersistedMetadata | null {
  if (!value) return null;
  return {
    displayName:
      typeof value.displayName === "string" ? value.displayName.trim() : "",
    groupName:
      typeof value.groupName === "string" ? value.groupName.trim() : "",
    note: typeof value.note === "string" ? value.note.trim() : "",
    isMother: value.isMother === true,
    tagIds: normalizeBatchTagIds(
      Array.isArray(value.tagIds) ? value.tagIds : [],
    ),
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
      const baselineTagIds = resolveCompletedBatchOauthRowPersistedTagIds(
        row,
        items,
      );
      return baselineTagIds == null
        ? true
        : !batchTagIdsEqual(baselineTagIds, next.tagIds);
    }
    const baseline = resolveCompletedBatchOauthCommittedFieldBaseline(
      row,
      field,
      items,
    );
    if (baseline == null) return true;
    return baseline !== next[field];
  });
}

export function findCompletedBatchOauthAccount(
  row: Pick<BatchOauthRow, "session">,
  items: UpstreamAccountSummary[],
) {
  const accountId = row.session?.accountId;
  return accountId == null
    ? null
    : (items.find((item) => item.id === accountId) ?? null);
}

export function resolveCompletedBatchOauthRowPersistedTagIds(
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

export function resolveCompletedBatchOauthRowBaselineTagIds(
  row: Pick<BatchOauthRow, "session" | "metadataPersisted">,
  items: UpstreamAccountSummary[],
  fallbackTagIds: number[],
) {
  return (
    resolveCompletedBatchOauthRowPersistedTagIds(row, items) ??
    normalizeBatchTagIds(fallbackTagIds)
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

export function createImportedOauthSourceId(file: File, index: number) {
  return `${file.name}:${file.size}:${file.lastModified}:${index}`;
}

export function createImportedOauthPastedSourceId(serial: number) {
  return `pasted:${serial}`;
}

export function createImportedOauthPastedFileName(serial: number) {
  return `Pasted credential #${serial}.json`;
}

function normalizeImportedOauthRequiredString(
  value: unknown,
  fieldName: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (typeof value !== "string" || value.trim().length === 0) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName,
      }),
    };
  }
  return {
    ok: true as const,
    value: value.trim(),
  };
}

function decodeImportedOauthBase64UrlUtf8(input: string) {
  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(
    normalized.length + ((4 - (normalized.length % 4)) % 4),
    "=",
  );
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

function parseImportedOauthJwtPayload(
  token: string,
  tokenName: "access_token" | "id_token",
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const parts = token.split(".");
  if (
    parts.length !== 3 ||
    parts.some((part) => part.trim().length === 0)
  ) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidJwt", {
        tokenName,
      }),
    };
  }

  try {
    const decoded = decodeImportedOauthBase64UrlUtf8(parts[1]);
    const payload = JSON.parse(decoded) as Record<string, unknown>;
    return {
      ok: true as const,
      payload,
    };
  } catch {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidJwt", {
        tokenName,
      }),
    };
  }
}

function parseImportedOauthJwtExpiration(payload: Record<string, unknown>) {
  const exp = payload.exp;
  if (typeof exp !== "number" || !Number.isFinite(exp)) {
    return null;
  }
  return exp;
}

function isImportedOauthRfc3339Timestamp(value: string) {
  const normalized = value.trim();
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(
      normalized,
    )
  ) {
    return false;
  }
  return !Number.isNaN(Date.parse(normalized));
}

function extractImportedOauthJwtEmail(payload: Record<string, unknown>) {
  if (typeof payload.email === "string" && payload.email.trim().length > 0) {
    return payload.email.trim();
  }
  const profile = payload.profile;
  if (
    profile &&
    typeof profile === "object" &&
    typeof (profile as Record<string, unknown>).email === "string"
  ) {
    return ((profile as Record<string, unknown>).email as string).trim();
  }
  return null;
}

function extractImportedOauthJwtAccountId(payload: Record<string, unknown>) {
  const auth = payload.auth;
  if (
    !auth ||
    typeof auth !== "object" ||
    typeof (auth as Record<string, unknown>).chatgpt_account_id !== "string"
  ) {
    return null;
  }
  return ((auth as Record<string, unknown>).chatgpt_account_id as string).trim();
}

export function buildImportedOauthMatchKeyFromValues(
  email: string | null | undefined,
  chatgptAccountId: string | null | undefined,
) {
  const normalizedAccountId = chatgptAccountId?.trim().toLocaleLowerCase();
  if (normalizedAccountId) {
    return `account:${normalizedAccountId}`;
  }
  const normalizedEmail = email?.trim().toLocaleLowerCase();
  if (normalizedEmail) {
    return `email:${normalizedEmail}`;
  }
  return null;
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

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.paste.singleObjectError"),
    };
  }

  const record = parsed as Record<string, unknown>;
  const sourceType = record.type;
  if (
    typeof sourceType !== "string" ||
    sourceType.trim().length === 0 ||
    sourceType.trim().toLowerCase() !== "codex"
  ) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidType"),
    };
  }

  const emailResult = normalizeImportedOauthRequiredString(record.email, "email", t);
  if (!emailResult.ok) return emailResult;
  const accountIdResult = normalizeImportedOauthRequiredString(
    record.account_id,
    "account_id",
    t,
  );
  if (!accountIdResult.ok) return accountIdResult;
  const accessTokenResult = normalizeImportedOauthRequiredString(
    record.access_token,
    "access_token",
    t,
  );
  if (!accessTokenResult.ok) return accessTokenResult;
  const refreshTokenResult = normalizeImportedOauthRequiredString(
    record.refresh_token,
    "refresh_token",
    t,
  );
  if (!refreshTokenResult.ok) return refreshTokenResult;
  const idTokenResult = normalizeImportedOauthRequiredString(
    record.id_token,
    "id_token",
    t,
  );
  if (!idTokenResult.ok) return idTokenResult;

  if (
    Object.prototype.hasOwnProperty.call(record, "expired") &&
    record.expired != null &&
    typeof record.expired !== "string"
  ) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidExpired"),
    };
  }

  const idTokenPayload = parseImportedOauthJwtPayload(
    idTokenResult.value,
    "id_token",
    t,
  );
  if (!idTokenPayload.ok) return idTokenPayload;

  const jwtEmail = extractImportedOauthJwtEmail(idTokenPayload.payload);
  if (
    jwtEmail &&
    jwtEmail.toLowerCase() !== emailResult.value.toLowerCase()
  ) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.emailMismatch"),
    };
  }

  const jwtAccountId = extractImportedOauthJwtAccountId(idTokenPayload.payload);
  if (jwtAccountId && jwtAccountId !== accountIdResult.value) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.accountIdMismatch"),
    };
  }

  const rawExpired =
    typeof record.expired === "string" ? record.expired.trim() : "";
  if (rawExpired) {
    if (!isImportedOauthRfc3339Timestamp(rawExpired)) {
      return {
        ok: false as const,
        error: t("accountPool.upstreamAccounts.import.local.invalidExpired"),
      };
    }
  } else {
    const accessTokenPayload = parseImportedOauthJwtPayload(
      accessTokenResult.value,
      "access_token",
      t,
    );
    const accessTokenExp = accessTokenPayload.ok
      ? parseImportedOauthJwtExpiration(accessTokenPayload.payload)
      : null;
    const idTokenExp = parseImportedOauthJwtExpiration(idTokenPayload.payload);
    if (accessTokenExp == null && idTokenExp == null) {
      return {
        ok: false as const,
        error: t("accountPool.upstreamAccounts.import.local.missingExpiry"),
      };
    }
  }

  return {
    ok: true as const,
    normalizedContent,
    email: emailResult.value,
    chatgptAccountId: accountIdResult.value,
    matchKey: buildImportedOauthMatchKeyFromValues(
      emailResult.value,
      accountIdResult.value,
    ),
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
      return t(
        "accountPool.upstreamAccounts.import.validation.status.duplicate",
      );
    case "ok":
      return t("accountPool.upstreamAccounts.import.validation.status.ok");
    case "ok_exhausted":
      return t(
        "accountPool.upstreamAccounts.import.validation.status.exhausted",
      );
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
  return validateImportedOauthCredentialLocally(content, t);
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
  row: Pick<ImportedOauthValidationRow, "email" | "chatgptAccountId">,
) {
  return buildImportedOauthMatchKeyFromValues(row.email, row.chatgptAccountId);
}

export function applyImportedOauthDuplicateStatuses(
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

export function mergeImportedOauthValidationRows(
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

export function mergeImportedOauthValidationRow(
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

export function replaceImportedOauthValidationRows(
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
    new Set(
      messages.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
  return normalized.length > 0 ? normalized.join(" | ") : null;
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

export function normalizeMailboxAddressKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

export function mailboxInputMatchesSession(
  input: string,
  session: OauthMailboxSessionSupported | null,
) {
  if (!session) return false;
  return (
    normalizeMailboxAddressKey(input) ===
    normalizeMailboxAddressKey(session.emailAddress)
  );
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
      (item) =>
        item.id !== excludeId &&
        normalizeDisplayNameKey(item.displayName) === normalized,
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
    (currentSession.status !== "pending" &&
      currentSession.status !== "completed")
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
  groupName,
  groupBoundProxyKeys,
  groupNodeShuntEnabled,
  note,
  groupNote,
  groupConcurrencyLimit,
  includeGroupNote,
  tagIds,
  isMother,
  mailboxSession,
}: {
  displayName: string;
  groupName: string;
  groupBoundProxyKeys: string[];
  groupNodeShuntEnabled: boolean;
  note: string;
  groupNote: string;
  groupConcurrencyLimit: number;
  includeGroupNote: boolean;
  tagIds: number[];
  isMother: boolean;
  mailboxSession: OauthMailboxSessionSupported | null;
}): UpdateOauthLoginSessionPayload {
  const normalizedGroupName = groupName.trim();
  return {
    displayName: displayName.trim(),
    groupName: normalizedGroupName,
    groupBoundProxyKeys,
    groupNodeShuntEnabled,
    note: note.trim(),
    ...(normalizedGroupName && includeGroupNote
      ? { groupNote: groupNote.trim() }
      : {}),
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

export function applyBatchMotherDraftRules(
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

export function enforceBatchMotherDraftUniqueness(rows: BatchOauthRow[]) {
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

export function batchStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "completed") return "success";
  if (status === "completedNeedsRefresh") return "warning";
  if (status === "pending") return "warning";
  if (status === "failed" || status === "expired") return "error";
  return "secondary";
}

export function batchRowStatus(row: BatchOauthRow) {
  if (row.needsRefresh) return "completedNeedsRefresh";
  return row.session?.status ?? "draft";
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

export function batchMailboxCodeVariant(
  row: BatchOauthRow,
): "default" | "secondary" | "outline" {
  const code = row.mailboxStatus?.latestCode?.value;
  if (!code) return "secondary";
  return row.mailboxCodeTone === "copied" ? "outline" : "default";
}

export function batchMailboxCodeLabel(row: BatchOauthRow) {
  return row.mailboxStatus?.latestCode?.value ?? "------";
}

export function batchMailboxRefreshVariant(
  row: BatchOauthRow,
): "outline" | "secondary" {
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
  const seconds = Math.max(
    0,
    Math.ceil((row.mailboxNextRefreshAt - now) / 1000),
  );
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

export function resolveMailboxIssue(
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

export function DuplicateWarningPopover({
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

export function DuplicateDetailField({
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

export function accountStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "active") return "success";
  if (status === "syncing") return "warning";
  if (status === "error" || status === "needs_reauth") return "error";
  return "secondary";
}

export function accountKindVariant(kind: string): "secondary" | "success" {
  return kind === "oauth_codex" ? "success" : "secondary";
}

export function DuplicateAccountDetailDialog({
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
