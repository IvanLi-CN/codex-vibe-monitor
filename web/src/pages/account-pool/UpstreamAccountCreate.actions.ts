// biome-ignore-all lint/correctness/useExhaustiveDependencies: action callbacks intentionally use current refs for pending OAuth state
import { useCallback } from "react";
import type { LoginSessionStatusResponse, UpstreamAccountDetail } from "../../lib/api";
import { writeApiKeyLastGroupName } from "../../lib/upstreamAccountGroups";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import {
  type BatchOauthRow,
  normalizeEmailKey,
  resolveBatchOauthMailboxAddress,
  resolveDisplayNameAfterEmailChange,
  shouldPromptOauthEmailChoice,
} from "./UpstreamAccountCreate.shared";

export function useUpstreamAccountCreateActions(ctx: UpstreamAccountCreateControllerContext) {
  const {
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
    confirmOauthOverwrite,
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
    relinkAccountId,
    relinkDetailError,
    relinkDetailLoading,
    relinkReady,
    removeOauthMailboxSession,
    resolveMailboxIssue,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    resolveGroupSingleAccountRotationEnabledForName,
    resolveRequiredGroupProxyState,
    scheduleBatchMailboxToneReset,
    scheduleSingleMailboxToneReset,
    saveAccount,
    session,
    setActionError,
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
  } = ctx;

  const clearOauthMailboxSession = useCallback(
    (sessionToRemoveId?: string | null, options?: { deleteRemote?: boolean }) => {
      setOauthMailboxSession(null);
      setOauthMailboxStatus(null);
      setOauthMailboxError(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      if (sessionToRemoveId && options?.deleteRemote !== false) {
        void removeOauthMailboxSession(sessionToRemoveId).catch(() => undefined);
      }
    },
    [removeOauthMailboxSession],
  );

  const finalizeSingleOauthFlow = useCallback(
    (detail: UpstreamAccountDetail) => {
      setOauthCompletedDetail(detail);
      setOauthEmail(detail.email ?? "");
      setOauthMailboxInput(detail.email ?? "");
      setOauthDisplayName(detail.displayName);
      setOauthDuplicateWarning(
        detail.duplicateInfo
          ? {
              accountId: detail.id,
              displayName: detail.displayName,
              peerAccountIds: detail.duplicateInfo.peerAccountIds,
              reasons: detail.duplicateInfo.reasons,
            }
          : null,
      );
    },
    [
      setOauthCompletedDetail,
      setOauthDisplayName,
      setOauthDuplicateWarning,
      setOauthEmail,
      setOauthMailboxInput,
    ],
  );

  const maybePromptSingleOauthEmailResolution = useCallback(
    (detail: UpstreamAccountDetail) => {
      const verifiedEmail = detail.verifiedEmail?.trim() ?? "";
      const chosenEmail = detail.email?.trim() ?? "";
      if (!shouldPromptOauthEmailChoice(verifiedEmail, chosenEmail)) {
        setOauthEmailResolution(null);
        finalizeSingleOauthFlow(detail);
        return;
      }
      setOauthEmailResolution({
        detail,
        verifiedEmail,
        chosenEmail,
      });
    },
    [finalizeSingleOauthFlow, setOauthEmailResolution],
  );

  const handleResolveOauthEmailChoice = useCallback(
    async (choice: "verified" | "entered") => {
      const currentResolution = oauthEmailResolution;
      if (!currentResolution) return;
      const nextEmail =
        choice === "verified" ? currentResolution.verifiedEmail : currentResolution.chosenEmail;
      setActionError(null);
      setBusyAction("oauth-email-choice");
      try {
        const detail =
          normalizeEmailKey(currentResolution.detail.email) === normalizeEmailKey(nextEmail)
            ? currentResolution.detail
            : await saveAccount(currentResolution.detail.id, {
                email: nextEmail.trim() ? nextEmail : null,
              });
        setOauthEmailResolution(null);
        finalizeSingleOauthFlow(detail);
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [
      oauthEmailResolution,
      finalizeSingleOauthFlow,
      saveAccount,
      setActionError,
      setBusyAction,
      setOauthEmailResolution,
    ],
  );
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
        setOauthEmail(response.emailAddress);
        setOauthDisplayName((current: string) =>
          resolveDisplayNameAfterEmailChange(current, oauthEmail, response.emailAddress),
        );
        setOauthMailboxStatus(null);
        setOauthMailboxTone("idle");
        setOauthMailboxCodeTone("idle");
        return;
      }
      setOauthMailboxSession(response);
      setOauthMailboxInput(response.emailAddress);
      setOauthEmail(response.emailAddress);
      setOauthDisplayName((current: string) =>
        resolveDisplayNameAfterEmailChange(current, oauthEmail, response.emailAddress),
      );
      setOauthMailboxStatus(null);
      setOauthMailboxError(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      invalidateRelinkPendingOauthSession();
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined);
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
      setOauthEmail("");
      setOauthDisplayName((current: string) =>
        resolveDisplayNameAfterEmailChange(current, oauthEmail, ""),
      );
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
      const response = await beginOauthMailboxSessionForAddress(normalizedAddress);
      setOauthMailboxSession(response);
      setOauthMailboxInput(response.emailAddress);
      setOauthEmail(response.emailAddress);
      setOauthDisplayName((current: string) =>
        resolveDisplayNameAfterEmailChange(current, oauthEmail, response.emailAddress),
      );
      setOauthMailboxStatus(null);
      setOauthMailboxTone("idle");
      setOauthMailboxCodeTone("idle");
      if (isSupportedMailboxSession(response)) {
        if (!previousSessionId || previousSessionId !== response.sessionId) {
          invalidateRelinkPendingOauthSession();
        }
      } else if (previousSessionId) {
        invalidateRelinkPendingOauthSession();
      }
      if (
        previousSessionId &&
        (!isSupportedMailboxSession(response) || previousSessionId !== response.sessionId)
      ) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined);
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
    if (!relinkReady) {
      setActionError(
        relinkDetailLoading
          ? t("accountPool.upstreamAccounts.createPage.relinkLoading")
          : (relinkDetailError ?? t("accountPool.upstreamAccounts.createPage.relinkLoadFailed")),
      );
      return;
    }
    if (oauthGroupProxyState.error) {
      setActionError(oauthGroupProxyState.error);
      return;
    }
    setActionError(null);
    setSessionHint(null);
    setOauthEmailResolution(null);
    setOauthCompletedDetail(null);
    setOauthDuplicateWarning(null);
    setBusyAction("oauth-generate");
    try {
      const normalizedGroupName = normalizeGroupName(oauthGroupName);
      const oauthLoginSessionPayload = buildOauthLoginSessionUpdatePayload({
        displayName: oauthDisplayName,
        email: oauthEmail,
        groupName: oauthGroupName,
        groupBoundProxyKeys: oauthGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: oauthGroupProxyState.nodeShuntEnabled,
        groupSingleAccountRotationEnabled:
          resolveGroupSingleAccountRotationEnabledForName(oauthGroupName),
        note: oauthNote,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName),
        groupConcurrencyLimit: resolvePendingGroupConcurrencyLimitForName(oauthGroupName),
        includeGroupNote: Boolean(
          normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
        ),
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSession: activeOauthMailboxSession,
      });
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        email: oauthEmail.trim() || undefined,
        groupName: oauthGroupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: oauthGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: oauthGroupProxyState.nodeShuntEnabled,
        groupSingleAccountRotationEnabled:
          resolveGroupSingleAccountRotationEnabledForName(oauthGroupName),
        note: oauthNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName) || undefined,
        concurrencyLimit: resolvePendingGroupConcurrencyLimitForName(oauthGroupName),
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
      setOauthEmail(response.email ?? oauthEmail);
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
      await flushPendingOauthSessionSync(session.loginId, singleOauthSessionSnapshot);
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
      await flushPendingOauthSessionSync(session.loginId, singleOauthSessionSnapshot);
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
        email: detail.email ?? session.email ?? null,
      });
      maybePromptSingleOauthEmailResolution(detail);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      let latestSession: LoginSessionStatusResponse | null = null;
      try {
        latestSession = await getLoginSession(session.loginId);
      } catch {
        latestSession = null;
      }
      setSession((current: LoginSessionStatusResponse | null) => latestSession ?? current);
      if (latestSession?.status === "completed" && latestSession.accountId) {
        setActionError(null);
        emitUpstreamAccountsChanged();
        try {
          const detail = await fetchUpstreamAccountDetail(latestSession.accountId);
          notifyMotherChange(detail);
          maybePromptSingleOauthEmailResolution(detail);
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
      if (latestSession?.status === "failed" || latestSession?.status === "expired") {
        setOauthCallbackUrl("");
        setOauthCompletedDetail(null);
        setSessionHint(latestSession.error ?? message);
        setOauthDuplicateWarning(null);
      }
      if (latestSession?.status === "needs_identity_confirmation") {
        setActionError(null);
        setSessionHint(t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.required"));
        return;
      }
      setActionError(message);
    } finally {
      setBusyAction(null);
    }
  };

  const handleConfirmOauthIdentityOverwrite = async () => {
    if (!session || session.status !== "needs_identity_confirmation") return;
    setActionError(null);
    setBusyAction("oauth-confirm-identity");
    try {
      const detail = await confirmOauthOverwrite(session.loginId);
      notifyMotherChange(detail);
      emitUpstreamAccountsChanged();
      setSession({
        ...session,
        status: "completed",
        accountId: detail.id,
        authUrl: null,
        redirectUri: null,
        email: detail.email ?? session.email ?? null,
        error: null,
        identityConfirmation: null,
      });
      setOauthCallbackUrl("");
      setSessionHint(
        t("accountPool.upstreamAccounts.batchOauth.completed", {
          name: detail.displayName || `#${detail.id}`,
        }),
      );
      maybePromptSingleOauthEmailResolution(detail);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      let latestSession: LoginSessionStatusResponse | null = null;
      try {
        latestSession = await getLoginSession(session.loginId);
      } catch {
        latestSession = null;
      }
      setSession((current: LoginSessionStatusResponse | null) => latestSession ?? current);
      if (latestSession?.status === "failed" || latestSession?.status === "expired") {
        setSessionHint(latestSession.error ?? message);
        setOauthCallbackUrl("");
      }
      setActionError(message);
    } finally {
      setBusyAction(null);
    }
  };

  const handleBatchGenerateMailbox = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row) return;

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxBusyAction: "generate",
      mailboxEditorOpen: false,
      mailboxEditorValue: current.mailboxInput,
      actionError: null,
    }));

    try {
      const response = await beginOauthMailboxSession();
      if (!isSupportedMailboxSession(response)) {
        updateBatchRow(rowId, (current: BatchOauthRow) => ({
          ...current,
          mailboxBusyAction: null,
          mailboxError: t("accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable"),
          actionError: null,
        }));
        return;
      }
      const previousSessionId = row.mailboxSession?.sessionId;
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxSession: response,
        mailboxInput: response.emailAddress,
        email: response.emailAddress,
        displayName: resolveDisplayNameAfterEmailChange(
          current.displayName,
          current.email,
          response.emailAddress,
        ),
        mailboxEditorValue: response.emailAddress,
        mailboxStatus: null,
        mailboxError: null,
        mailboxTone: "idle",
        mailboxCodeTone: "idle",
        actionError: null,
      }));
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined);
      }
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxError: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchStartMailboxEdit = (rowId: string) => {
    updateBatchRow(rowId, (current: BatchOauthRow) => {
      if (
        current.busyAction ||
        current.mailboxBusyAction ||
        current.session?.status === "completed" ||
        current.needsRefresh
      ) {
        return current;
      }
      const baseValue = current.mailboxInput || current.mailboxSession?.emailAddress || "";
      return {
        ...current,
        mailboxEditorOpen: true,
        mailboxEditorValue: baseValue,
        mailboxEditorError: null,
        actionError: null,
      };
    });
  };

  const handleBatchMailboxEditorValueChange = (rowId: string, value: string) => {
    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxEditorValue: value,
      mailboxEditorError: null,
    }));
  };

  const handleBatchCancelMailboxEdit = (rowId: string) => {
    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxEditorOpen: false,
      mailboxEditorValue: current.mailboxInput || current.mailboxSession?.emailAddress || "",
      mailboxEditorError: null,
    }));
  };

  const handleBatchAttachMailbox = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row) return;
    const normalizedAddress = row.mailboxEditorValue.trim();
    if (!normalizedAddress) return;
    if (!isProbablyValidEmailAddress(normalizedAddress)) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxEditorError: t("accountPool.upstreamAccounts.batchOauth.validation.mailboxFormat"),
      }));
      return;
    }

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxBusyAction: "attach",
      actionError: null,
      mailboxError: null,
      mailboxEditorError: null,
    }));

    const previousSessionId = row.mailboxSession?.sessionId ?? null;
    try {
      const response = await beginOauthMailboxSessionForAddress(normalizedAddress);
      const unsupportedError = isSupportedMailboxSession(response)
        ? null
        : resolveMailboxIssue(response, null, null, null, t);

      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxBusyAction: null,
        mailboxEditorOpen: false,
        mailboxEditorValue: response.emailAddress,
        mailboxEditorError: null,
        mailboxSession: isSupportedMailboxSession(response) ? response : null,
        mailboxInput: response.emailAddress,
        email: response.emailAddress,
        displayName: resolveDisplayNameAfterEmailChange(
          current.displayName,
          current.email,
          response.emailAddress,
        ),
        mailboxStatus: null,
        mailboxError: unsupportedError,
        mailboxTone: "idle",
        mailboxCodeTone: "idle",
        actionError: null,
      }));

      if (
        previousSessionId &&
        (!isSupportedMailboxSession(response) || previousSessionId !== response.sessionId)
      ) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined);
      }
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxBusyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchCopyMailbox = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    const value = row ? resolveBatchOauthMailboxAddress(row) : "";
    if (!value) return;
    const result = await copyText(value, { preferExecCommand: true });
    if (!result.ok) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        mailboxTone: "manual",
      }));
      return;
    }
    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxTone: "copied",
    }));
    scheduleBatchMailboxToneReset(rowId);
  };

  const handleBatchCopyMailboxCode = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    const value = row?.mailboxStatus?.latestCode?.value;
    if (!value) return;
    const result = await copyText(value, { preferExecCommand: true });
    if (!result.ok) return;
    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      mailboxCodeTone: "copied",
    }));
  };

  const canApplyBatchOauthCopyFeedback = (rowId: string, loginId: string) => {
    const session = batchSessionFeedbackStateByRowRef.current[rowId];
    if (
      !session ||
      session.loginId !== loginId ||
      session.status !== "pending" ||
      !session.authUrl
    ) {
      return false;
    }
    const currentRow = batchRowsRef.current.find((item: BatchOauthRow) => item.id === rowId);
    if (currentRow?.session?.loginId !== loginId) {
      return true;
    }
    return isActivePendingOauthSession(currentRow.session);
  };

  const resolveBatchOauthCopyFeedback = (
    rowId: string,
    loginId: string,
    result: Awaited<ReturnType<typeof copyText>>,
    successHint: string,
  ) => {
    if (!canApplyBatchOauthCopyFeedback(rowId, loginId)) {
      return null;
    }
    const fallbackHint = t("accountPool.upstreamAccounts.batchOauth.copyInlineFallback");
    setBatchManualCopyRowId((current: string | null) => {
      if (!canApplyBatchOauthCopyFeedback(rowId, loginId)) {
        return current;
      }
      return result.ok ? (current === rowId ? null : current) : rowId;
    });
    return {
      sessionHint: result.ok ? successHint : fallbackHint,
      actionError: result.ok ? null : fallbackHint,
    };
  };

  const handleBatchGenerateOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row) return;
    if (row.needsRefresh) return;

    const groupProxyState = resolveRequiredGroupProxyState(row.groupName);
    if (groupProxyState.error) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        busyAction: null,
        actionError: groupProxyState.error,
      }));
      return;
    }

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      busyAction: "generate",
      actionError: null,
    }));

    try {
      const oauthLoginSessionPayload = buildOauthLoginSessionUpdatePayload({
        displayName: row.displayName,
        email: row.email,
        groupName: groupProxyState.normalizedGroupName,
        groupBoundProxyKeys: groupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: groupProxyState.nodeShuntEnabled,
        groupSingleAccountRotationEnabled: resolveGroupSingleAccountRotationEnabledForName(
          row.groupName,
        ),
        note: row.note,
        groupNote: resolvePendingGroupNoteForName(row.groupName),
        groupConcurrencyLimit: resolvePendingGroupConcurrencyLimitForName(row.groupName),
        includeGroupNote: Boolean(
          groupProxyState.normalizedGroupName &&
            !isExistingGroup(groups, groupProxyState.normalizedGroupName),
        ),
        tagIds: batchTagIds,
        isMother: row.isMother,
        mailboxSession: row.mailboxSession,
      });
      const response = await beginOauthLogin({
        displayName: row.displayName.trim() || undefined,
        email: row.email.trim() || undefined,
        groupName: groupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: groupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: groupProxyState.nodeShuntEnabled,
        groupSingleAccountRotationEnabled: resolveGroupSingleAccountRotationEnabledForName(
          row.groupName,
        ),
        note: row.note.trim() || undefined,
        tagIds: batchTagIds,
        groupNote: resolvePendingGroupNoteForName(row.groupName) || undefined,
        concurrencyLimit: resolvePendingGroupConcurrencyLimitForName(row.groupName),
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
      const generatedHint = t("accountPool.upstreamAccounts.oauth.generated", {
        expiresAt: formatDateTime(response.expiresAt),
      });
      batchSessionFeedbackStateByRowRef.current[rowId] = {
        loginId: response.loginId,
        status: response.status,
        authUrl: response.authUrl ?? null,
      };
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        busyAction: null,
        callbackUrl: "",
        session: response,
        email: response.email ?? current.email,
        sessionHint: generatedHint,
        needsRefresh: false,
        actionError: null,
      }));
      if (response.authUrl) {
        const copyResult = await copyText(response.authUrl, {
          preferExecCommand: true,
        });
        const copyFeedback = resolveBatchOauthCopyFeedback(
          rowId,
          response.loginId,
          copyResult,
          t("accountPool.upstreamAccounts.batchOauth.generatedAndCopied"),
        );
        if (!copyFeedback) {
          return;
        }
        updateBatchRow(rowId, (current: BatchOauthRow) => ({
          ...(current.session?.loginId !== response.loginId
            ? current
            : {
                ...current,
                sessionHint: copyFeedback.sessionHint,
                actionError: copyFeedback.actionError,
              }),
        }));
      } else if (batchSessionFeedbackStateByRowRef.current[rowId]?.loginId === response.loginId) {
        setBatchManualCopyRowId((current: string | null) => (current === rowId ? null : current));
      }
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        busyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleBatchCopyOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row?.session?.authUrl) return;

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
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
        setBatchManualCopyRowId((current: string | null) => (current === rowId ? null : current));
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

    const copyFeedback = resolveBatchOauthCopyFeedback(
      rowId,
      row.session.loginId,
      result,
      t("accountPool.upstreamAccounts.oauth.copied"),
    );
    if (!copyFeedback) {
      return;
    }

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      sessionHint: copyFeedback.sessionHint,
      actionError: copyFeedback.actionError,
    }));
  };

  const handleBatchCompleteOauth = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row?.session) return;

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
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
      updateBatchRow(rowId, (current: BatchOauthRow) => {
        const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse;
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
            email: detail.email ?? baseSession.email ?? null,
            error: baseSession.error ?? null,
          },
          email: detail.email ?? current.email,
          verifiedEmail: detail.verifiedEmail ?? null,
          planType: detail.planType ?? null,
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
            (detail.tags ?? []).map((tag: { id: number }) => tag.id),
          ),
          pendingSharedTagIds: null,
          sharedTagSyncAttempts: 0,
          isMother: detail.isMother === true,
          emailResolution: shouldPromptOauthEmailChoice(detail.verifiedEmail, detail.email)
            ? {
                accountId: detail.id,
                verifiedEmail: detail.verifiedEmail?.trim() ?? "",
                chosenEmail: detail.email?.trim() ?? "",
                displayName: detail.displayName,
                planType: detail.planType ?? null,
              }
            : null,
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
          const detail = await fetchUpstreamAccountDetail(latestSession.accountId);
          notifyMotherChange(detail);
          updateBatchRow(rowId, (current: BatchOauthRow) => {
            const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse;
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
                email: detail.email ?? null,
                error: null,
              },
              callbackUrl: "",
              email: detail.email ?? current.email,
              verifiedEmail: detail.verifiedEmail ?? null,
              planType: detail.planType ?? null,
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
                (detail.tags ?? []).map((tag: { id: number }) => tag.id),
              ),
              pendingSharedTagIds: null,
              sharedTagSyncAttempts: 0,
              isMother: detail.isMother === true,
              emailResolution: shouldPromptOauthEmailChoice(detail.verifiedEmail, detail.email)
                ? {
                    accountId: detail.id,
                    verifiedEmail: detail.verifiedEmail?.trim() ?? "",
                    chosenEmail: detail.email?.trim() ?? "",
                    displayName: detail.displayName,
                    planType: detail.planType ?? null,
                  }
                : null,
            };
          });
        } catch {
          updateBatchRow(rowId, (current: BatchOauthRow) => {
            const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse;
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
              actionError: t("accountPool.upstreamAccounts.batchOauth.completedNeedsRefresh"),
            };
          });
        }
        return;
      }
      if (latestSession?.status === "needs_identity_confirmation") {
        updateBatchRow(rowId, (current: BatchOauthRow) => ({
          ...current,
          busyAction: null,
          session: latestSession,
          callbackUrl: "",
          sessionHint: t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.required"),
          duplicateWarning: current.duplicateWarning,
          needsRefresh: false,
          actionError: null,
        }));
        return;
      }

      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        busyAction: null,
        session: latestSession ?? current.session,
        callbackUrl:
          latestSession?.status === "failed" || latestSession?.status === "expired"
            ? ""
            : current.callbackUrl,
        sessionHint:
          latestSession?.status === "failed" || latestSession?.status === "expired"
            ? (latestSession.error ?? current.sessionHint)
            : current.sessionHint,
        duplicateWarning:
          latestSession?.status === "failed" || latestSession?.status === "expired"
            ? null
            : current.duplicateWarning,
        needsRefresh: false,
        actionError: message,
      }));
    }
  };

  const handleBatchConfirmOauthIdentityOverwrite = async (rowId: string) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    if (!row?.session || row.session.status !== "needs_identity_confirmation") return;

    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      busyAction: "confirm",
      actionError: null,
    }));

    try {
      const detail = await confirmOauthOverwrite(row.session.loginId);
      notifyMotherChange(detail);
      updateBatchRow(rowId, (current: BatchOauthRow) => {
        const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse;
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
            email: detail.email ?? baseSession.email ?? null,
            error: null,
          },
          callbackUrl: "",
          email: detail.email ?? current.email,
          verifiedEmail: detail.verifiedEmail ?? null,
          planType: detail.planType ?? null,
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
            (detail.tags ?? []).map((tag: { id: number }) => tag.id),
          ),
          pendingSharedTagIds: null,
          sharedTagSyncAttempts: 0,
          isMother: detail.isMother === true,
          emailResolution: shouldPromptOauthEmailChoice(detail.verifiedEmail, detail.email)
            ? {
                accountId: detail.id,
                verifiedEmail: detail.verifiedEmail?.trim() ?? "",
                chosenEmail: detail.email?.trim() ?? "",
                displayName: detail.displayName,
                planType: detail.planType ?? null,
              }
            : null,
        };
      });
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        busyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleResolveBatchOauthEmailChoice = async (
    rowId: string,
    choice: "verified" | "entered",
  ) => {
    const row = batchRows.find((item: BatchOauthRow) => item.id === rowId);
    const resolution = row?.emailResolution;
    if (!row || !resolution) return;
    const nextEmail = choice === "verified" ? resolution.verifiedEmail : resolution.chosenEmail;
    const needsSave = normalizeEmailKey(row.email) !== normalizeEmailKey(nextEmail);
    updateBatchRow(rowId, (current: BatchOauthRow) => ({
      ...current,
      metadataBusy: needsSave,
      actionError: null,
      metadataError: null,
    }));
    if (!needsSave) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        emailResolution: null,
      }));
      return;
    }
    try {
      const detail = await saveAccount(resolution.accountId, {
        email: nextEmail.trim() ? nextEmail : null,
      });
      notifyMotherChange(detail);
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        displayName: detail.displayName,
        email: detail.email ?? current.email,
        verifiedEmail: detail.verifiedEmail ?? current.verifiedEmail ?? null,
        planType: detail.planType ?? current.planType ?? null,
        isMother: detail.isMother === true,
        metadataBusy: false,
        metadataError: null,
        actionError: null,
        duplicateWarning: detail.duplicateInfo
          ? {
              accountId: detail.id,
              displayName: detail.displayName,
              peerAccountIds: detail.duplicateInfo.peerAccountIds,
              reasons: detail.duplicateInfo.reasons,
            }
          : null,
        sessionHint: t("accountPool.upstreamAccounts.batchOauth.completed", {
          name: detail.displayName || current.displayName || `#${detail.id}`,
        }),
        metadataPersisted: buildBatchOauthPersistedMetadata(
          {
            displayName: detail.displayName,
            groupName: detail.groupName ?? "",
            note: detail.note ?? "",
            isMother: detail.isMother === true,
          },
          (detail.tags ?? []).map((tag: { id: number }) => tag.id),
        ),
        emailResolution: null,
      }));
    } catch (err) {
      updateBatchRow(rowId, (current: BatchOauthRow) => ({
        ...current,
        metadataBusy: false,
        actionError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  const handleCreateApiKey = async () => {
    if (apiKeyUpstreamBaseUrlError) return;
    if (apiKeyGroupProxyState.error) {
      setActionError(apiKeyGroupProxyState.error);
      return;
    }
    setActionError(null);
    setBusyAction("apiKey");
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        email: apiKeyEmail.trim() || undefined,
        groupName: apiKeyGroupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: apiKeyGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: apiKeyGroupProxyState.nodeShuntEnabled,
        groupSingleAccountRotationEnabled:
          resolveGroupSingleAccountRotationEnabledForName(apiKeyGroupName),
        note: apiKeyNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(apiKeyGroupName) || undefined,
        concurrencyLimit: resolvePendingGroupConcurrencyLimitForName(apiKeyGroupName),
        apiKey: apiKeyValue.trim(),
        upstreamBaseUrl: apiKeyUpstreamBaseUrl.trim() || undefined,
        isMother: apiKeyIsMother,
        localPrimaryLimit: normalizeNumberInput(apiKeyPrimaryLimit),
        localSecondaryLimit: normalizeNumberInput(apiKeySecondaryLimit),
        localLimitUnit: apiKeyLimitUnit.trim() || "requests",
        tagIds: apiKeyTagIds,
      });
      writeApiKeyLastGroupName(apiKeyGroupProxyState.normalizedGroupName);
      notifyMotherChange(response);
      navigate("/account-pool/upstream-accounts", {
        state: {
          selectedAccountId: response.id,
          openDetail: true,
          postCreateWarning: null,
        },
      });
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  };

  return {
    clearOauthMailboxSession,
    handleGenerateOauthMailbox,
    handleAttachOauthMailbox,
    handleCopySingleMailbox,
    handleCopySingleMailboxCode,
    handleCopySingleInvite,
    handleGenerateOauthUrl,
    handleCopyOauthUrl,
    handleCompleteOauth,
    handleConfirmOauthIdentityOverwrite,
    handleResolveOauthEmailChoice,
    handleBatchGenerateMailbox,
    handleBatchStartMailboxEdit,
    handleBatchMailboxEditorValueChange,
    handleBatchCancelMailboxEdit,
    handleBatchAttachMailbox,
    handleBatchCopyMailbox,
    handleBatchCopyMailboxCode,
    canApplyBatchOauthCopyFeedback,
    resolveBatchOauthCopyFeedback,
    handleBatchGenerateOauthUrl,
    handleBatchCopyOauthUrl,
    handleBatchCompleteOauth,
    handleBatchConfirmOauthIdentityOverwrite,
    handleResolveBatchOauthEmailChoice,
    handleCreateApiKey,
  };
}
