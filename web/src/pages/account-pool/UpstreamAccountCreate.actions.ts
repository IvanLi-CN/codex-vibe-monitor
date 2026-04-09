/* eslint-disable @typescript-eslint/ban-ts-comment, react-hooks/exhaustive-deps */
// @ts-nocheck
import { useCallback } from "react";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";

export function useUpstreamAccountCreateActions(ctx: UpstreamAccountCreateControllerContext) {
  const {
    activeOauthMailboxSession,
    apiKeyDisplayName,
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
    oauthDisplayName,
    oauthDisplayNameConflict,
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
    removeOauthMailboxSession,
    resolveMailboxIssue,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupNoteForName,
    resolveRequiredGroupProxyState,
    scheduleBatchMailboxToneReset,
    scheduleSingleMailboxToneReset,
    session,
    setActionError,
    setBatchManualCopyRowId,
    setBusyAction,
    setManualCopyOpen,
    setOauthCallbackUrl,
    setOauthDisplayName,
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
    updateBatchRow
  } = ctx;

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
    if (oauthGroupProxyState.error) {
      setActionError(oauthGroupProxyState.error);
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
        groupBoundProxyKeys: oauthGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: oauthGroupProxyState.nodeShuntEnabled,
        note: oauthNote,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName),
        groupConcurrencyLimit:
          resolvePendingGroupConcurrencyLimitForName(oauthGroupName),
        includeGroupNote: Boolean(
          normalizedGroupName && !isExistingGroup(groups, normalizedGroupName),
        ),
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSession: activeOauthMailboxSession,
      });
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: oauthGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: oauthGroupProxyState.nodeShuntEnabled,
        note: oauthNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName) || undefined,
        concurrencyLimit:
          resolvePendingGroupConcurrencyLimitForName(oauthGroupName),
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
      await persistDraftGroupSettings(oauthGroupName);
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
          await persistDraftGroupSettings(oauthGroupName);
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
    const currentRow = batchRowsRef.current.find((item) => item.id === rowId);
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
    const fallbackHint = t(
      "accountPool.upstreamAccounts.batchOauth.copyInlineFallback",
    );
    setBatchManualCopyRowId((current) => {
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
    const row = batchRows.find((item) => item.id === rowId);
    if (!row) return;
    if (row.needsRefresh) return;

    const groupProxyState = resolveRequiredGroupProxyState(row.groupName);
    if (groupProxyState.error) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        actionError: groupProxyState.error,
      }));
      return;
    }

    updateBatchRow(rowId, (current) => ({
      ...current,
      busyAction: "generate",
      actionError: null,
    }));

    try {
      const oauthLoginSessionPayload = buildOauthLoginSessionUpdatePayload({
        displayName: row.displayName,
        groupName: groupProxyState.normalizedGroupName,
        groupBoundProxyKeys: groupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: groupProxyState.nodeShuntEnabled,
        note: row.note,
        groupNote: resolvePendingGroupNoteForName(row.groupName),
        groupConcurrencyLimit: resolvePendingGroupConcurrencyLimitForName(
          row.groupName,
        ),
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
        groupName: groupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: groupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: groupProxyState.nodeShuntEnabled,
        note: row.note.trim() || undefined,
        tagIds: batchTagIds,
        groupNote: resolvePendingGroupNoteForName(row.groupName) || undefined,
        concurrencyLimit: resolvePendingGroupConcurrencyLimitForName(
          row.groupName,
        ),
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
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        callbackUrl: "",
        session: response,
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
        updateBatchRow(rowId, (current) => ({
          ...(current.session?.loginId !== response.loginId
            ? current
            : {
                ...current,
                sessionHint: copyFeedback.sessionHint,
                actionError: copyFeedback.actionError,
              }),
        }));
      } else if (
        batchSessionFeedbackStateByRowRef.current[rowId]?.loginId ===
        response.loginId
      ) {
        setBatchManualCopyRowId((current) =>
          current === rowId ? null : current,
        );
      }
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
        setBatchManualCopyRowId((current) =>
          current === rowId ? null : current,
        );
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

    updateBatchRow(rowId, (current) => ({
      ...current,
      sessionHint: copyFeedback.sessionHint,
      actionError: copyFeedback.actionError,
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
      await persistDraftGroupSettings(row.groupName);
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
          await persistDraftGroupSettings(row.groupName);
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
    if (apiKeyGroupProxyState.error) {
      setActionError(apiKeyGroupProxyState.error);
      return;
    }
    setActionError(null);
    setBusyAction("apiKey");
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        groupName: apiKeyGroupProxyState.normalizedGroupName || undefined,
        groupBoundProxyKeys: apiKeyGroupProxyState.boundProxyKeys,
        groupNodeShuntEnabled: apiKeyGroupProxyState.nodeShuntEnabled,
        note: apiKeyNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(apiKeyGroupName) || undefined,
        concurrencyLimit:
          resolvePendingGroupConcurrencyLimitForName(apiKeyGroupName),
        apiKey: apiKeyValue.trim(),
        upstreamBaseUrl: apiKeyUpstreamBaseUrl.trim() || undefined,
        isMother: apiKeyIsMother,
        localPrimaryLimit: normalizeNumberInput(apiKeyPrimaryLimit),
        localSecondaryLimit: normalizeNumberInput(apiKeySecondaryLimit),
        localLimitUnit: apiKeyLimitUnit.trim() || "requests",
        tagIds: apiKeyTagIds,
      });
      let postCreateWarning: string | null = null;
      try {
        await persistDraftGroupSettings(apiKeyGroupName);
      } catch (error) {
        postCreateWarning = t(
          "accountPool.upstreamAccounts.partialSuccess.createdButGroupSettingsFailed",
          {
            error: error instanceof Error ? error.message : String(error),
          },
        );
      }
      notifyMotherChange(response);
      navigate("/account-pool/upstream-accounts", {
        state: {
          selectedAccountId: response.id,
          openDetail: true,
          postCreateWarning,
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
    handleCreateApiKey
  };
}
