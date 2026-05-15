/* eslint-disable react-hooks/exhaustive-deps */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useMotherSwitchNotifications } from "../../hooks/useMotherSwitchNotifications";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { usePoolTags } from "../../hooks/usePoolTags";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import type {
  ImportOauthCredentialFilePayload,
  LoginSessionStatusResponse,
  OauthMailboxSession,
  OauthMailboxStatus,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from "../../lib/api";
import { fetchUpstreamAccountDetail, updateOauthLoginSessionKeepalive } from "../../lib/api";
import { copyText, selectAllReadonlyText } from "../../lib/clipboard";
import { emitUpstreamAccountsChanged } from "../../lib/upstreamAccountsEvents";
import { apiConcurrencyLimitToSliderValue } from "../../lib/concurrencyLimit";
import {
  buildGroupOptions,
  isExistingGroup,
  markUpstreamAccountGroupUsed,
  normalizeGroupName,
  readUpstreamAccountGroupUsage,
  resolveMostRecentlyUsedGroupName,
  writeUpstreamAccountGroupUsage,
} from "../../lib/upstreamAccountGroups";
import { validateUpstreamBaseUrl } from "../../lib/upstreamBaseUrl";
import { applyMotherUpdateToItems } from "../../lib/upstreamMother";
import { cn } from "../../lib/utils";
import { useTranslation } from "../../i18n";
import type {
  BatchOauthRow,
  CreatePageLocationState,
  CreateTab,
  DuplicateWarningState,
  GroupNoteEditorState,
  ImportedOauthValidationDialogState,
  MailboxBusyAction,
  MailboxCopyTone,
  PendingOauthSessionSnapshot,
} from "./UpstreamAccountCreate.shared";
import {
  GROUP_UPSTREAM_429_RETRY_OPTIONS,
  MAILBOX_REFRESH_INTERVAL_MS,
  MAILBOX_REFRESH_TICK_MS,
  OAUTH_SESSION_SYNC_DEBOUNCE_MS,
  OAUTH_SESSION_SYNC_RETRY_MS,
  normalizeNumberInput,
  normalizeGroupUpstream429MaxRetries,
  normalizeEnabledGroupUpstream429MaxRetries,
  formatDateTime,
  isActivePendingOauthSession,
  batchOauthSessionRemainingLabel,
  batchOauthSessionExpiresAtLabel,
  parseAccountId,
  parseCreateMode,
  createBatchOauthRow,
  buildBatchOauthPersistedMetadata,
  hydrateBatchOauthRow,
  getNextBatchRowIndex,
  normalizeDisplayNameKey,
  mailboxInputMatchesSession,
  isProbablyValidEmailAddress,
  findDisplayNameConflict,
  invalidatePendingSingleOauthSession,
  buildOauthLoginSessionUpdatePayload,
  buildPendingOauthSessionSnapshot,
  shouldRetryPendingOauthSessionSync,
  batchStatusVariant,
  batchRowStatus,
  batchRowStatusDetail,
  batchMailboxCodeVariant,
  batchMailboxCodeLabel,
  batchMailboxRefreshVariant,
  isExpiredIso,
  isRefreshableMailboxSession,
  batchMailboxRefreshLabel,
  batchMailboxRefreshTooltipDetail,
  resolveMailboxIssue,
  isSupportedMailboxSession,
  buildActionTooltip,
} from "./UpstreamAccountCreate.shared";
import { UpstreamAccountCreateViewProvider } from "./UpstreamAccountCreate.controller-context";
import { useUpstreamAccountCreateGroupDrafts } from "./UpstreamAccountCreate.group-drafts";
import { useUpstreamAccountCreateBatchOauth } from "./UpstreamAccountCreate.batch-oauth";
import { useUpstreamAccountCreateImportedOauth } from "./UpstreamAccountCreate.imported-oauth";
import { useUpstreamAccountCreateActions } from "./UpstreamAccountCreate.actions";
import { UpstreamAccountCreatePageSections } from "./UpstreamAccountCreate.sections";

type PendingOauthSessionSyncRecord = {
  syncedSignature: string | null;
  failedSignature: string | null;
  pendingSignature: string;
  timerId: number | null;
  inFlight: Promise<void> | null;
  lastSnapshot: PendingOauthSessionSnapshot | null;
};

type OauthEmailResolutionState = {
  detail: UpstreamAccountDetail;
  verifiedEmail: string;
  chosenEmail: string;
};

export default function UpstreamAccountCreatePage() {
  const { t, locale } = useTranslation();
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
    refresh,
    saveAccount,
    saveGroupNote,
    deleteGroupNote,
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
  const [relinkDetail, setRelinkDetail] =
    useState<UpstreamAccountDetail | null>(null);
  const [relinkDetailLoading, setRelinkDetailLoading] = useState(false);
  const [relinkDetailError, setRelinkDetailError] = useState<string | null>(
    null,
  );
  const relinkMetadataDirtyRef = useRef(false);
  const isRelinking = relinkAccountId != null;
  const matchingRelinkDetail =
    relinkDetail != null && relinkDetail.id === relinkAccountId
      ? relinkDetail
      : null;
  const relinkAccount = matchingRelinkDetail ?? relinkSummary;
  const relinkReady =
    !isRelinking ||
    (matchingRelinkDetail != null &&
      matchingRelinkDetail.kind === "oauth_codex" &&
      !relinkDetailLoading &&
      relinkDetailError == null);
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
  const [oauthEmail, setOauthEmail] = useState(
    () => draft?.oauth?.email ?? draft?.oauth?.session?.email ?? "",
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
  const [apiKeyEmail, setApiKeyEmail] = useState(
    () => draft?.apiKey?.email ?? "",
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
  const [oauthEmailResolution, setOauthEmailResolution] =
    useState<OauthEmailResolutionState | null>(null);
  const [oauthCompletedDetail, setOauthCompletedDetail] =
    useState<UpstreamAccountDetail | null>(null);
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
  const [importPasteDraft, setImportPasteDraft] = useState("");
  const [importPasteError, setImportPasteError] = useState<string | null>(null);
  const [importPasteBusy, setImportPasteBusy] = useState(false);
  const [importPasteDraftSerial, setImportPasteDraftSerial] = useState<
    number | null
  >(null);
  const [importFiles, setImportFiles] = useState<
    ImportOauthCredentialFilePayload[]
  >([]);
  const importFilesRef = useRef<ImportOauthCredentialFilePayload[]>([]);
  const [importSelectionFeedback, setImportSelectionFeedback] = useState<{
    variant: "warning" | "error";
    messages: string[];
  } | null>(null);
  const [importValidationDialogOpen, setImportValidationDialogOpen] =
    useState(false);
  const [importValidationState, setImportValidationState] =
    useState<ImportedOauthValidationDialogState | null>(null);
  const [importInputKey, setImportInputKey] = useState(0);
  const importPasteSequenceRef = useRef(0);
  const importPasteValidationTokenRef = useRef(0);
  const importPasteDraftRef = useRef("");
  const importFilesRevisionRef = useRef(0);
  const importFileSourceSequenceRef = useRef(0);
  const importValidationEventSourceRef = useRef<EventSource | null>(null);
  const importValidationEventCleanupRef = useRef<(() => void) | null>(null);
  const importValidationJobIdRef = useRef<string | null>(null);
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([]);
  const [groupUsage, setGroupUsage] = useState(() =>
    readUpstreamAccountGroupUsage(),
  );
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
  const [groupDraftBoundProxyKeys, setGroupDraftBoundProxyKeys] = useState<
    Record<string, string[]>
  >({});
  const [groupDraftConcurrencyLimits, setGroupDraftConcurrencyLimits] =
    useState<Record<string, number>>({});
  const [groupDraftNodeShuntEnabled, setGroupDraftNodeShuntEnabled] = useState<
    Record<string, boolean>
  >({});
  const [
    groupDraftUpstream429RetryEnabled,
    setGroupDraftUpstream429RetryEnabled,
  ] = useState<Record<string, boolean>>({});
  const [groupDraftUpstream429MaxRetries, setGroupDraftUpstream429MaxRetries] =
    useState<Record<string, number>>({});
  const [
    persistedGroupNoteSyncDrafts,
    setPersistedGroupNoteSyncDrafts,
  ] = useState<Record<string, string>>({});
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupNoteEditorState>({
    open: false,
    groupName: "",
    note: "",
    existing: false,
    accountCount: 0,
    concurrencyLimit: apiConcurrencyLimitToSliderValue(0),
    boundProxyKeys: [],
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    onSaved: null,
    onDeleted: null,
  });
  const {
    nodes: forwardProxyNodes,
    catalogState: forwardProxyCatalogState,
    refresh: refreshForwardProxyBindings,
  } = useForwardProxyBindingNodes(groupNoteEditor.boundProxyKeys, {
    enabled: groupNoteEditor.open,
    groupName: groupNoteEditor.groupName,
  });
  const [groupNoteBusy, setGroupNoteBusy] = useState(false);
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null);
  const oauthMailboxToneResetRef = useRef<number | null>(null);
  const batchMailboxToneResetRef = useRef<Record<string, number>>({});
  const batchRowsRef = useRef<BatchOauthRow[]>(initialBatchRows);
  const batchSessionFeedbackStateByRowRef = useRef<
    Record<
      string,
      Pick<LoginSessionStatusResponse, "loginId" | "status" | "authUrl"> | null
    >
  >(
    Object.fromEntries(
      initialBatchRows.map((row) => [
        row.id,
        row.session
          ? {
              loginId: row.session.loginId,
              status: row.session.status,
              authUrl: row.session.authUrl ?? null,
            }
          : null,
      ]),
    ),
  );
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
      ...(draft?.batchOauth?.rows ?? []).flatMap((row) =>
        row.session?.status === "pending" ? [row.session.loginId] : [],
      ),
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
    batchSessionFeedbackStateByRowRef.current = Object.fromEntries(
      batchRows.map((row) => [
        row.id,
        row.session
          ? {
              loginId: row.session.loginId,
              status: row.session.status,
              authUrl: row.session.authUrl ?? null,
            }
          : null,
      ]),
    );
  }, [batchRows]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      setRefreshClockMs(Date.now());
    }, MAILBOX_REFRESH_TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  const batchRowIdRef = useRef(getNextBatchRowIndex(initialBatchRows));
  const manualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null);

  const groupOptions = useMemo(
    () =>
      buildGroupOptions(
        items.map((item) => item.groupName),
        groups,
        {
          ...Object.fromEntries(
            Object.keys(groupDraftBoundProxyKeys).map((groupName) => [
              groupName,
              "",
            ]),
          ),
          ...Object.fromEntries(
            Object.keys(groupDraftConcurrencyLimits).map((groupName) => [
              groupName,
              "",
            ]),
          ),
          ...Object.fromEntries(
            Object.keys(groupDraftNodeShuntEnabled).map((groupName) => [
              groupName,
              "",
            ]),
          ),
          ...Object.fromEntries(
            Object.keys(groupDraftUpstream429RetryEnabled).map((groupName) => [
              groupName,
              "",
            ]),
          ),
          ...Object.fromEntries(
            Object.keys(groupDraftUpstream429MaxRetries).map((groupName) => [
              groupName,
              "",
            ]),
          ),
          ...groupDraftNotes,
        },
        groupUsage,
      ),
    [
      groupUsage,
      groupDraftBoundProxyKeys,
      groupDraftConcurrencyLimits,
      groupDraftNodeShuntEnabled,
      groupDraftNotes,
      groupDraftUpstream429MaxRetries,
      groupDraftUpstream429RetryEnabled,
      groups,
      items,
    ],
  );
  const markGroupUsed = useCallback((groupName?: string | null) => {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return;
    setGroupUsage((current) => {
      const next = markUpstreamAccountGroupUsed(current, normalized);
      writeUpstreamAccountGroupUsage(next);
      return next;
    });
  }, []);
  const rememberedBatchDefaultGroupAppliedRef = useRef(false);
  useEffect(() => {
    if (rememberedBatchDefaultGroupAppliedRef.current) return;
    if (draft?.batchOauth?.defaultGroupName) {
      rememberedBatchDefaultGroupAppliedRef.current = true;
      return;
    }
    if (batchDefaultGroupName.trim()) {
      rememberedBatchDefaultGroupAppliedRef.current = true;
      return;
    }
    const rememberedGroupName = resolveMostRecentlyUsedGroupName(
      groupOptions,
      groupUsage,
    );
    if (!rememberedGroupName) return;
    rememberedBatchDefaultGroupAppliedRef.current = true;
    setBatchDefaultGroupName(rememberedGroupName);
    setBatchRows((current) =>
      current.map((row) =>
        row.inheritsDefaultGroup
          ? {
              ...row,
              groupName: rememberedGroupName,
            }
          : row,
      ),
    );
  }, [
    batchDefaultGroupName,
    draft?.batchOauth?.defaultGroupName,
    groupOptions,
    groupUsage,
  ]);
  const formatGroupAccountCountLabel = useCallback(
    (count: number) =>
      t("accountPool.upstreamAccounts.groupOptionCount", { count }),
    [t],
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
  useEffect(() => {
    importFilesRef.current = importFiles;
  }, [importFiles]);

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
  const notifyMotherChange = (updated: UpstreamAccountSummary) => {
    const nextItems = applyMotherUpdateToItems(items, updated);
    notifyMotherSwitches(items, nextItems);
  };
  const markRelinkMetadataDirty = useCallback(() => {
    if (isRelinking) {
      relinkMetadataDirtyRef.current = true;
    }
  }, [isRelinking]);
  const {
    resolveGroupNodeShuntEnabledForName,
    resolvePendingGroupNoteForName,
    shouldIncludePendingGroupNoteForName,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupBoundProxyKeysForName,
    hasGroupSettings,
    resolveRequiredGroupProxyState,
    oauthGroupProxyState,
    importGroupProxyState,
    importSelectionLabel,
    apiKeyGroupProxyState,
    persistDraftGroupSettings,
    openGroupNoteEditor,
    closeGroupNoteEditor,
    handleSaveGroupNote,
    handleDeleteGroupNote,
  } = useUpstreamAccountCreateGroupDrafts({
    apiKeyGroupName,
    batchDefaultGroupName,
    batchRows,
    forwardProxyNodes,
    forwardProxyCatalogState,
    groupDraftBoundProxyKeys,
    groupDraftConcurrencyLimits,
    groupDraftNodeShuntEnabled,
    groupDraftNotes,
    groupDraftUpstream429MaxRetries,
    groupDraftUpstream429RetryEnabled,
    groupNoteBusy,
    groupNoteEditor,
    groups,
    importFiles,
    importGroupName,
    invalidateSingleOauthSessionForMetadataEdit,
    locale,
    oauthGroupName,
    persistedGroupNoteSyncDrafts,
    deleteGroupNote,
    saveGroupNote,
    setGroupDraftBoundProxyKeys,
    setGroupDraftConcurrencyLimits,
    setGroupDraftNodeShuntEnabled,
    setGroupDraftNotes,
    setGroupDraftUpstream429MaxRetries,
    setGroupDraftUpstream429RetryEnabled,
    setGroupNoteBusy,
    setGroupNoteEditor,
    setGroupNoteError,
    setPersistedGroupNoteSyncDrafts,
    setApiKeyGroupName,
    setBatchDefaultGroupName,
    setBatchRows,
    setImportGroupName,
    setOauthGroupName,
    t,
    writesEnabled,
  });

  const singleOauthSessionSnapshot = useMemo(() => {
    if (isRelinking || session?.status !== "pending") return null;
    return buildPendingOauthSessionSnapshot(
      session.loginId,
      buildOauthLoginSessionUpdatePayload({
        displayName: oauthDisplayName,
        email: oauthEmail,
        groupName: oauthGroupName,
        groupBoundProxyKeys:
          resolvePendingGroupBoundProxyKeysForName(oauthGroupName),
        groupNodeShuntEnabled:
          resolveGroupNodeShuntEnabledForName(oauthGroupName),
        note: oauthNote,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName),
        groupConcurrencyLimit:
          resolvePendingGroupConcurrencyLimitForName(oauthGroupName),
        includeGroupNote: shouldIncludePendingGroupNoteForName(oauthGroupName),
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSession: activeOauthMailboxSession,
      }),
      session.updatedAt ?? null,
    );
  }, [
    activeOauthMailboxSession,
    oauthDisplayName,
    oauthEmail,
    oauthGroupName,
    oauthIsMother,
    oauthNote,
    oauthTagIds,
    isRelinking,
    resolveGroupNodeShuntEnabledForName,
    resolvePendingGroupBoundProxyKeysForName,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    shouldIncludePendingGroupNoteForName,
    session?.loginId,
    session?.status,
    session?.updatedAt,
  ]);
  const batchOauthSessionSnapshots = useMemo(() => {
    const snapshots: Record<string, PendingOauthSessionSnapshot> = {};
    for (const row of batchRows) {
      if (row.session?.status !== "pending") continue;
      snapshots[row.session.loginId] = buildPendingOauthSessionSnapshot(
        row.session.loginId,
        buildOauthLoginSessionUpdatePayload({
          displayName: row.displayName,
          email: row.email,
          groupName: row.groupName,
          groupBoundProxyKeys: resolvePendingGroupBoundProxyKeysForName(
            row.groupName,
          ),
          groupNodeShuntEnabled: resolveGroupNodeShuntEnabledForName(
            row.groupName,
          ),
          note: row.note,
          groupNote: resolvePendingGroupNoteForName(row.groupName),
          groupConcurrencyLimit: resolvePendingGroupConcurrencyLimitForName(
            row.groupName,
          ),
          includeGroupNote: shouldIncludePendingGroupNoteForName(
            row.groupName,
          ),
          tagIds: batchTagIds,
          isMother: row.isMother,
          mailboxSession: row.mailboxSession,
        }),
        row.session.updatedAt ?? null,
      );
    }
    return snapshots;
  }, [
    batchRows,
    batchTagIds,
    resolveGroupNodeShuntEnabledForName,
    resolvePendingGroupBoundProxyKeysForName,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    shouldIncludePendingGroupNoteForName,
  ]);
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
  const setPendingOauthSessionSyncError = useCallback(
    (loginId: string, message: string) => {
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
    },
    [],
  );
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
      const currentSnapshot = getPendingOauthSessionSnapshot(loginId);
      const currentRecord = pendingOauthSessionSyncRef.current[loginId];
      const shouldPreserveLocalEmailDraft =
        nextSession.status === "pending" &&
        currentSnapshot != null &&
        currentRecord != null &&
        currentRecord.syncedSignature !== currentSnapshot.signature;
      const nextDraftEmail = shouldPreserveLocalEmailDraft
        ? typeof currentSnapshot.payload.email === "string"
          ? currentSnapshot.payload.email
          : ""
        : nextSession.email ?? "";
      if (singleOauthSessionSnapshotRef.current?.loginId === loginId) {
        setSession((current) =>
          current?.loginId === loginId ? nextSession : current,
        );
        setOauthEmail(nextDraftEmail);
        setOauthMailboxInput(nextDraftEmail);
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
                email: shouldPreserveLocalEmailDraft
                  ? nextDraftEmail
                  : nextSession.email ?? row.email,
                sessionHint:
                  nextSession.status === "pending" ? row.sessionHint : null,
                actionError:
                  nextSession.status === "pending" ? row.actionError : null,
              }
            : row,
        ),
      );
    },
    [getPendingOauthSessionSnapshot],
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
                  const retryRecord =
                    pendingOauthSessionSyncRef.current[loginId];
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
    const activeLoginIds = new Set(
      activeSnapshots.map((snapshot) => snapshot.loginId),
    );

    for (const snapshot of activeSnapshots) {
      let existing = pendingOauthSessionSyncRef.current[snapshot.loginId];
      if (!existing) {
        const shouldStartUnsynced =
          restoredPendingOauthLoginIdsRef.current.delete(snapshot.loginId);
        const createdSyncedSignature =
          createdPendingOauthSessionSignaturesRef.current[snapshot.loginId] ??
          null;
        delete createdPendingOauthSessionSignaturesRef.current[
          snapshot.loginId
        ];
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
        const currentRecord =
          pendingOauthSessionSyncRef.current[snapshot.loginId];
        if (!currentRecord) return;
        currentRecord.timerId = null;
        void runPendingOauthSessionSync(snapshot.loginId).catch(
          () => undefined,
        );
      }, OAUTH_SESSION_SYNC_DEBOUNCE_MS);
    }

    for (const [loginId, record] of Object.entries(
      pendingOauthSessionSyncRef.current,
    )) {
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
      .map((reason: string) => {
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
    relinkMetadataDirtyRef.current = false;
    setRelinkDetail(null);
    setRelinkDetailError(null);
    if (!isRelinking || relinkAccountId == null) {
      setRelinkDetailLoading(false);
      return;
    }
    let cancelled = false;
    setRelinkDetailLoading(true);
    void fetchUpstreamAccountDetail(relinkAccountId)
      .then((detail) => {
        if (cancelled) return;
        setRelinkDetail(detail);
        setRelinkDetailError(
          detail.kind === "oauth_codex"
            ? null
            : t("accountPool.upstreamAccounts.createPage.relinkNonOauth"),
        );
      })
      .catch((err) => {
        if (cancelled) return;
        setRelinkDetail(null);
        setRelinkDetailError(
          err instanceof Error
            ? err.message
            : t("accountPool.upstreamAccounts.createPage.relinkLoadFailed"),
        );
      })
      .finally(() => {
        if (!cancelled) setRelinkDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isRelinking, relinkAccountId, t]);

  useEffect(() => {
    if (!isRelinking || !relinkAccount) return;
    setActiveTab("oauth");
    if (relinkMetadataDirtyRef.current) return;
    setOauthDisplayName((current) => current || relinkAccount.displayName);
    setOauthEmail((current) => current || relinkAccount.email || "");
    setOauthMailboxInput((current) => current || relinkAccount.email || "");
    setOauthGroupName((current) => current || relinkAccount.groupName || "");
    setOauthTagIds((current) =>
      current.length > 0
        ? current
        : (relinkAccount.tags ?? []).map((tag) => tag.id),
    );
    setOauthIsMother((current) => current || relinkAccount.isMother);
    if ("note" in relinkAccount) {
      const relinkNote = relinkAccount.note;
      setOauthNote((current) =>
        current || (typeof relinkNote === "string" ? relinkNote : ""),
      );
    }
  }, [isRelinking, relinkAccount]);

  useEffect(() => {
    if (!manualCopyOpen) return;
    const frame = window.requestAnimationFrame(() => {
      selectAllReadonlyText(manualCopyFieldRef.current);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [manualCopyOpen]);

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
  useEffect(() => {
    setGroupDraftBoundProxyKeys((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);
  useEffect(() => {
    setGroupDraftNodeShuntEnabled((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);
  useEffect(() => {
    setGroupDraftUpstream429RetryEnabled((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);
  useEffect(() => {
    setGroupDraftUpstream429MaxRetries((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);
  useEffect(() => {
    setGroupDraftConcurrencyLimits((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) {
        return current;
      }
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);

  const {
    appendBatchRow,
    scheduleSingleMailboxToneReset,
    updateBatchRow,
    scheduleBatchMailboxToneReset,
    removeBatchRow,
    toggleBatchNoteExpanded,
    handleBatchMetadataChange,
    handleBatchCompletedTextFieldBlur,
    handleBatchCompletedTextFieldKeyDown,
    handleBatchGroupValueChange,
    handleBatchMotherToggle,
    handleBatchDefaultGroupChange,
    handleTabChange,
  } = useUpstreamAccountCreateBatchOauth({
    batchDefaultGroupName,
    batchMailboxToneResetRef,
    batchRowIdRef,
    batchRows,
    batchRowsRef,
    batchSharedTagSyncEnabledRef,
    batchTagIds,
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
    stopImportedOauthValidationJob,
    t,
  });

  const handleOauthGroupCreateRequest = useCallback(
    (groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) => {
          setOauthGroupName(savedGroupName);
          markGroupUsed(savedGroupName);
        },
      });
    },
    [markGroupUsed, openGroupNoteEditor],
  );

  const handleImportGroupCreateRequest = useCallback(
    (groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) => {
          setImportGroupName(savedGroupName);
          markGroupUsed(savedGroupName);
        },
      });
    },
    [markGroupUsed, openGroupNoteEditor],
  );

  const handleApiKeyGroupCreateRequest = useCallback(
    (groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) => {
          setApiKeyGroupName(savedGroupName);
          markGroupUsed(savedGroupName);
        },
      });
    },
    [markGroupUsed, openGroupNoteEditor],
  );

  const handleBatchDefaultGroupCreateRequest = useCallback(
    (groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) => {
          handleBatchDefaultGroupChange(savedGroupName);
          markGroupUsed(savedGroupName);
        },
      });
    },
    [handleBatchDefaultGroupChange, markGroupUsed, openGroupNoteEditor],
  );

  const handleBatchRowGroupCreateRequest = useCallback(
    (rowId: string, groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) => {
          handleBatchGroupValueChange(rowId, savedGroupName);
          markGroupUsed(savedGroupName);
        },
      });
    },
    [handleBatchGroupValueChange, markGroupUsed, openGroupNoteEditor],
  );

  const handleRememberingBatchDefaultGroupChange = useCallback(
    (value: string) => {
      handleBatchDefaultGroupChange(value);
      markGroupUsed(value);
    },
    [handleBatchDefaultGroupChange, markGroupUsed],
  );

  const handleRememberingBatchGroupValueChange = useCallback(
    (rowId: string, value: string) => {
      handleBatchGroupValueChange(rowId, value);
      markGroupUsed(value);
    },
    [handleBatchGroupValueChange, markGroupUsed],
  );

  const setRememberingOauthGroupName = useCallback(
    (value: string) => {
      setOauthGroupName(value);
      markGroupUsed(value);
    },
    [markGroupUsed],
  );

  const setRememberingImportGroupName = useCallback(
    (value: string) => {
      setImportGroupName(value);
      markGroupUsed(value);
    },
    [markGroupUsed],
  );

  const setRememberingApiKeyGroupName = useCallback(
    (value: string) => {
      setApiKeyGroupName(value);
      markGroupUsed(value);
    },
    [markGroupUsed],
  );


  const {
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
  } = useUpstreamAccountCreateImportedOauth({
    activeTab,
    groupDraftConcurrencyLimits,
    groupDraftNotes,
    groups,
    importFiles,
    importFilesRef,
    importFilesRevisionRef,
    importFileSourceSequenceRef,
    importGroupName,
    importGroupProxyState,
    importOauthAccounts,
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
  });


  const {
    clearOauthMailboxSession,
    handleGenerateOauthMailbox,
    handleAttachOauthMailbox,
    handleCopySingleMailbox,
    handleCopySingleMailboxCode,
    handleCopySingleInvite,
    handleGenerateOauthUrl,
    handleCopyOauthUrl,
    handleCompleteOauth,
    handleResolveOauthEmailChoice,
    handleBatchGenerateMailbox,
    handleBatchStartMailboxEdit,
    handleBatchMailboxEditorValueChange,
    handleBatchCancelMailboxEdit,
    handleBatchAttachMailbox,
    handleBatchCopyMailbox,
    handleBatchCopyMailboxCode,
    handleBatchGenerateOauthUrl,
    handleBatchCopyOauthUrl,
    handleBatchCompleteOauth,
    handleResolveBatchOauthEmailChoice,
    handleCreateApiKey,
  } = useUpstreamAccountCreateActions({
    activeOauthMailboxSession,
    apiKeyDisplayName,
    apiKeyEmail,
    apiKeyGroupName,
    apiKeyGroupProxyState,
    apiKeyIsMother,
    apiKeyLimitUnit,
    apiKeyNote,
    apiKeyPrimaryLimit,
    apiKeySecondaryLimit,
    apiKeyTagIds,
    apiKeyUpstreamBaseUrl,
    apiKeyUpstreamBaseUrlError,
    apiKeyValue,
    applyPendingOauthSessionStatus,
    batchOauthSessionSnapshots,
    batchRows,
    batchRowsRef,
    batchSessionFeedbackStateByRowRef,
    batchTagIds,
    beginOauthLogin,
    beginOauthMailboxSession,
    beginOauthMailboxSessionForAddress,
    buildBatchOauthPersistedMetadata,
    buildOauthLoginSessionUpdatePayload,
    buildPendingOauthSessionSnapshot,
    completeOauthLogin,
    copyText,
    createApiKeyAccount,
    createdPendingOauthSessionSignaturesRef,
    displayedOauthMailboxStatus,
    emitUpstreamAccountsChanged,
    fetchUpstreamAccountDetail,
    flushPendingOauthSessionSync,
    formatDateTime,
    getLoginSession,
    groups,
    invalidateRelinkPendingOauthSession,
    invalidateRelinkPendingOauthSessionForMailboxChange,
    isActivePendingOauthSession,
    isExistingGroup,
    isProbablyValidEmailAddress,
    isSupportedMailboxSession,
    navigate,
    normalizeGroupName,
    normalizeNumberInput,
    notifyMotherChange,
    oauthCallbackUrl,
    oauthCompletedDetail,
    oauthDisplayName,
    oauthEmail,
    oauthEmailResolution,
    oauthGroupName,
    oauthGroupProxyState,
    oauthIsMother,
    oauthMailboxAddress,
    oauthMailboxInput,
    oauthMailboxSession,
    oauthNote,
    oauthTagIds,
    persistDraftGroupSettings,
    relinkAccountId,
    relinkDetailError,
    relinkDetailLoading,
    relinkReady,
    removeOauthMailboxSession,
    resolveMailboxIssue,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    resolveRequiredGroupProxyState,
    scheduleBatchMailboxToneReset,
    scheduleSingleMailboxToneReset,
    saveAccount,
    session,
    setActionError,
    setApiKeyEmail,
    setBatchManualCopyRowId,
    setBusyAction,
    setManualCopyOpen,
    setOauthCallbackUrl,
    setOauthCompletedDetail,
    setOauthDisplayName,
    setOauthEmail,
    setOauthEmailResolution,
    setOauthDuplicateWarning,
    setOauthMailboxBusyAction,
    setOauthMailboxCodeTone,
    setOauthMailboxError,
    setOauthMailboxInput,
    setOauthMailboxSession,
    setOauthMailboxStatus,
    setOauthMailboxTone,
    setSession,
    setSessionHint,
    shouldRetryPendingOauthSessionSync,
    singleOauthSessionSnapshot,
    t,
    updateBatchRow,
  });

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
    priorityTier: t("accountPool.tags.dialog.priorityTier"),
    priorityPrimary: t("accountPool.tags.dialog.priorityPrimary"),
    priorityNormal: t("accountPool.tags.dialog.priorityNormal"),
    priorityFallback: t("accountPool.tags.dialog.priorityFallback"),
    fastModeRewriteMode: t("accountPool.tags.dialog.fastModeRewriteMode"),
    fastModeKeepOriginal: t("accountPool.tags.dialog.fastModeKeepOriginal"),
    fastModeFillMissing: t("accountPool.tags.dialog.fastModeFillMissing"),
    fastModeForceAdd: t("accountPool.tags.dialog.fastModeForceAdd"),
    fastModeForceRemove: t("accountPool.tags.dialog.fastModeForceRemove"),
    concurrencyLimit: t("accountPool.tags.dialog.concurrencyLimit"),
    concurrencyHint: t("accountPool.tags.dialog.concurrencyHint"),
    currentValue: t("accountPool.tags.dialog.currentValue"),
    unlimited: t("accountPool.tags.dialog.unlimited"),
    cancel: t("accountPool.tags.dialog.cancel"),
    save: t("accountPool.tags.dialog.save"),
    createAction: t("accountPool.tags.dialog.createAction"),
    validation: t("accountPool.tags.dialog.validation"),
  };

  const viewContext = {
    GROUP_UPSTREAM_429_RETRY_OPTIONS,
    accountKindLabel,
    accountStatusLabel,
    actionError,
    activeOauthMailboxSession,
    activeTab,
    apiKeyDisplayName,
    apiKeyDisplayNameConflict,
    apiKeyEmail,
    apiKeyGroupName,
    apiKeyGroupProxyState,
    apiKeyIsMother,
    apiKeyLimitUnit,
    apiKeyNote,
    apiKeyPrimaryLimit,
    apiKeySecondaryLimit,
    apiKeyTagIds,
    apiKeyUpstreamBaseUrl,
    apiKeyUpstreamBaseUrlError,
    apiKeyValue,
    appendBatchRow,
    batchCounts,
    batchDefaultGroupName,
    batchDisplayNameError,
    batchMailboxCodeLabel,
    batchMailboxCodeVariant,
    batchMailboxRefreshLabel,
    batchMailboxRefreshTooltipDetail,
    batchMailboxRefreshVariant,
    batchManualCopyRowId,
    batchOauthSessionExpiresAtLabel,
    batchOauthSessionRemainingLabel,
    batchRowStatus,
    batchRowStatusDetail,
    batchRows,
    batchSharedTagSyncEnabledRef,
    batchStatusVariant,
    batchTagIds,
    buildActionTooltip,
    busyAction,
    clearOauthMailboxSession,
    closeGroupNoteEditor,
    cn,
    displayedOauthMailboxStatus,
    duplicateDetail,
    duplicateDetailLoading,
    duplicateDetailOpen,
    formatDateTime,
    formatDuplicateReasons,
    forwardProxyNodes,
    forwardProxyCatalogState,
    refreshForwardProxyBindings,
    groupNoteBusy,
    groupNoteEditor,
    groupNoteError,
    groupOptions,
    formatGroupAccountCountLabel,
    handleAttachOauthMailbox,
    handleBatchAttachMailbox,
    handleBatchCancelMailboxEdit,
    handleBatchCompleteOauth,
    handleBatchCompletedTextFieldBlur,
    handleBatchCompletedTextFieldKeyDown,
    handleBatchCopyMailbox,
    handleBatchCopyMailboxCode,
    handleBatchCopyOauthUrl,
    handleBatchDefaultGroupCreateRequest,
    handleBatchDefaultGroupChange: handleRememberingBatchDefaultGroupChange,
    handleBatchGenerateMailbox,
    handleBatchGenerateOauthUrl,
    handleBatchRowGroupCreateRequest,
    handleBatchGroupValueChange: handleRememberingBatchGroupValueChange,
    handleBatchMailboxEditorValueChange,
    handleBatchMailboxFetch,
    handleBatchMetadataChange,
    handleBatchMotherToggle,
    handleResolveBatchOauthEmailChoice,
    handleBatchStartMailboxEdit,
    handleClearImportSelection,
    handleCloseImportedOauthValidationDialog,
    handleCompleteOauth,
    handleCopyOauthUrl,
    handleCopySingleInvite,
    handleCopySingleMailbox,
    handleCopySingleMailboxCode,
    handleCreateApiKey,
    handleApiKeyGroupCreateRequest,
    handleCreateTag,
    handleDeleteTag,
    handleGenerateOauthMailbox,
    handleGenerateOauthUrl,
    handleResolveOauthEmailChoice,
    handleImportFilesChange,
    handleImportGroupCreateRequest,
    handleImportValidatedOauth,
    handleImportedOauthPaste,
    handleImportedOauthPasteDraftChange,
    handleRetryImportedOauthFailed,
    handleRetryImportedOauthOne,
    handleSaveGroupNote,
    handleDeleteGroupNote,
    handleTabChange,
    handleOauthGroupCreateRequest,
    handleValidateImportedOauth,
    handleValidateImportedOauthPasteDraft,
    hasBatchMetadataBusy,
    hasGroupSettings,
    importFiles,
    importGroupName,
    importGroupProxyState,
    importInputKey,
    importPasteBusy,
    importPasteDraft,
    importPasteError,
    importSelectionFeedback,
    importSelectionLabel,
    importTagIds,
    importValidationDialogOpen,
    importValidationState,
    invalidateRelinkPendingOauthSessionForMailboxChange,
    invalidateSingleOauthSessionForMetadataEdit,
    isActivePendingOauthSession,
    isExpiredIso,
    isLoading,
    isRefreshableMailboxSession,
    isRelinking,
    isSupportedMailboxSession,
    listError,
    locale,
    mailboxInputMatchesSession,
    manualCopyFieldRef,
    manualCopyOpen,
    normalizeEnabledGroupUpstream429MaxRetries,
    normalizeGroupName,
    normalizeGroupUpstream429MaxRetries,
    oauthCallbackUrl,
    oauthCompletedDetail,
    oauthDisplayName,
    oauthDisplayNameConflict,
    oauthEmail,
    oauthEmailResolution,
    oauthDuplicateWarning,
    oauthGroupName,
    oauthGroupProxyState,
    oauthIsMother,
    oauthMailboxAddress,
    oauthMailboxBusyAction,
    oauthMailboxCodeStatusBadge,
    oauthMailboxCodeTone,
    oauthMailboxInput,
    oauthMailboxIssue,
    oauthMailboxSession,
    oauthMailboxTone,
    oauthNote,
    oauthSessionActive,
    oauthTagIds,
    openDuplicateDetailDialog,
    openGroupNoteEditor,
    pageCreatedTagIds,
    refreshClockMs,
    refresh,
    relinkDetailError,
    relinkDetailLoading,
    relinkReady,
    relinkSummary: relinkAccount,
    markRelinkMetadataDirty,
    removeBatchRow,
    resolveRequiredGroupProxyState,
    selectAllReadonlyText,
    session,
    sessionHint,
    setActionError,
    setApiKeyDisplayName,
    setApiKeyEmail,
    setApiKeyGroupName: setRememberingApiKeyGroupName,
    setApiKeyIsMother,
    setApiKeyLimitUnit,
    setApiKeyNote,
    setApiKeyPrimaryLimit,
    setApiKeySecondaryLimit,
    setApiKeyTagIds,
    setApiKeyUpstreamBaseUrl,
    setApiKeyValue,
    setBatchManualCopyRowId,
    setBatchRows,
    setBatchTagIds,
    setDuplicateDetail,
    setDuplicateDetailOpen,
    setGroupNoteEditor,
    setGroupNoteError,
    setImportGroupName: setRememberingImportGroupName,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteError,
    setImportSelectionFeedback,
    setImportTagIds,
    setManualCopyOpen,
    setOauthCallbackUrl,
    setOauthCompletedDetail,
    setOauthDisplayName,
    setOauthEmail,
    setOauthEmailResolution,
    setOauthGroupName: setRememberingOauthGroupName,
    setOauthIsMother,
    setOauthMailboxInput,
    setOauthNote,
    setOauthTagIds,
    t,
    tagFieldLabels,
    tagItems,
    toggleBatchNoteExpanded,
    updateTag,
    writesEnabled,
  };


  return (
    <UpstreamAccountCreateViewProvider value={viewContext}>
      <UpstreamAccountCreatePageSections />
    </UpstreamAccountCreateViewProvider>
  );
}
