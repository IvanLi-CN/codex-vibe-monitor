/* eslint-disable @typescript-eslint/ban-ts-comment */
// @ts-nocheck
import { AppIcon } from "../../components/AppIcon";
import { Link } from "react-router-dom";
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
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { FormFieldFeedback } from "../../components/ui/form-field-feedback";
import { Input } from "../../components/ui/input";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "../../components/ui/segmented-control";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { BatchOauthActionButton } from "../../components/account-pool/BatchOauthActionButton";
import { OauthMailboxChip } from "../../components/account-pool/OauthMailboxChip";
import { AccountTagField } from "../../components/AccountTagField";
import { ImportedOauthValidationDialog } from "../../components/ImportedOauthValidationDialog";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { MotherAccountToggle } from "../../components/MotherAccountToggle";
import {
  DuplicateAccountDetailDialog,
  DuplicateWarningPopover,
} from "./UpstreamAccountCreate.shared";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreatePageSections() {
  const {
    GROUP_UPSTREAM_429_RETRY_OPTIONS,
    accountKindLabel,
    accountStatusLabel,
    actionError,
    activeOauthMailboxSession,
    activeTab,
    apiKeyDisplayName,
    apiKeyDisplayNameConflict,
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
    groupNoteBusy,
    groupNoteEditor,
    groupNoteError,
    groupSuggestions,
    handleAttachOauthMailbox,
    handleBatchAttachMailbox,
    handleBatchCancelMailboxEdit,
    handleBatchCompleteOauth,
    handleBatchCompletedTextFieldBlur,
    handleBatchCompletedTextFieldKeyDown,
    handleBatchCopyMailbox,
    handleBatchCopyMailboxCode,
    handleBatchCopyOauthUrl,
    handleBatchDefaultGroupChange,
    handleBatchGenerateMailbox,
    handleBatchGenerateOauthUrl,
    handleBatchGroupValueChange,
    handleBatchMailboxEditorValueChange,
    handleBatchMailboxFetch,
    handleBatchMetadataChange,
    handleBatchMotherToggle,
    handleBatchStartMailboxEdit,
    handleClearImportSelection,
    handleCloseImportedOauthValidationDialog,
    handleCompleteOauth,
    handleCopyOauthUrl,
    handleCopySingleInvite,
    handleCopySingleMailbox,
    handleCopySingleMailboxCode,
    handleCreateApiKey,
    handleCreateTag,
    handleDeleteTag,
    handleGenerateOauthMailbox,
    handleGenerateOauthUrl,
    handleImportFilesChange,
    handleImportValidatedOauth,
    handleImportedOauthPaste,
    handleImportedOauthPasteDraftChange,
    handleRetryImportedOauthFailed,
    handleRetryImportedOauthOne,
    handleSaveGroupNote,
    handleTabChange,
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
    oauthDisplayName,
    oauthDisplayNameConflict,
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
    relinkSummary,
    removeBatchRow,
    resolveRequiredGroupProxyState,
    selectAllReadonlyText,
    session,
    sessionHint,
    setActionError,
    setApiKeyDisplayName,
    setApiKeyGroupName,
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
    setImportGroupName,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteError,
    setImportTagIds,
    setManualCopyOpen,
    setOauthCallbackUrl,
    setOauthDisplayName,
    setOauthGroupName,
    setOauthIsMother,
    setOauthMailboxInput,
    setOauthNote,
    setOauthTagIds,
    t,
    tagFieldLabels,
    tagItems,
    toggleBatchNoteExpanded,
    updateTag,
    writesEnabled
  } = useUpstreamAccountCreateViewContext();

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
                          hasGroupSettings(batchDefaultGroupName)
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
                        writesEnabled={writesEnabled && !hasBatchMetadataBusy}
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
                          hasGroupSettings(oauthGroupName)
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
                        onClick={() => openGroupNoteEditor(oauthGroupName)}
                        disabled={
                          !writesEnabled || !normalizeGroupName(oauthGroupName)
                        }
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </Button>
                    </div>
                    {oauthGroupProxyState.error ? (
                      <p className="mt-2 text-xs text-error">
                        {oauthGroupProxyState.error}
                      </p>
                    ) : null}
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
                            Boolean(oauthGroupProxyState.error) ||
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
                            const rowGroupProxyError =
                              resolveRequiredGroupProxyState(
                                row.groupName,
                              ).error;
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
                            const rowHasActiveOauthUrl =
                              isActivePendingOauthSession(row.session);
                            const authUrl = rowHasActiveOauthUrl
                              ? (row.session?.authUrl ?? "")
                              : "";
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
                                              disabled: oauthLocked,
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
                                            hasGroupSettings(row.groupName)
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
                                      {rowGroupProxyError ? (
                                        <p className="text-xs text-error">
                                          {rowGroupProxyError}
                                        </p>
                                      ) : null}
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
                                        <BatchOauthActionButton
                                          mode={
                                            rowHasActiveOauthUrl
                                              ? "copy"
                                              : "generate"
                                          }
                                          primaryAriaLabel={
                                            rowHasActiveOauthUrl
                                              ? t(
                                                  "accountPool.upstreamAccounts.actions.copyOauthUrl",
                                                )
                                              : t(
                                                  "accountPool.upstreamAccounts.actions.generateOauthUrl",
                                                )
                                          }
                                          regenerateAriaLabel={t(
                                            "accountPool.upstreamAccounts.actions.regenerateOauthUrl",
                                          )}
                                          popoverTitle={
                                            rowHasActiveOauthUrl
                                              ? t(
                                                  "accountPool.upstreamAccounts.batchOauth.tooltip.copyTitle",
                                                )
                                              : isPending
                                                ? t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateTitle",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.generateTitle",
                                                  )
                                          }
                                          popoverDescription={
                                            rowHasActiveOauthUrl
                                              ? t(
                                                  "accountPool.upstreamAccounts.batchOauth.tooltip.copyBody",
                                                )
                                              : isPending
                                                ? t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateBody",
                                                  )
                                                : t(
                                                    "accountPool.upstreamAccounts.batchOauth.tooltip.generateBody",
                                                  )
                                          }
                                          remainingLabel={batchOauthSessionRemainingLabel(
                                            row.session,
                                            refreshClockMs,
                                            t,
                                          )}
                                          expiresAtLabel={batchOauthSessionExpiresAtLabel(
                                            row.session,
                                            t,
                                          )}
                                          manualCopyTitle={t(
                                            "accountPool.upstreamAccounts.oauth.manualCopyTitle",
                                          )}
                                          manualCopyDescription={t(
                                            "accountPool.upstreamAccounts.oauth.manualCopyDescription",
                                          )}
                                          manualCopyValue={
                                            batchManualCopyRowId === row.id
                                              ? authUrl
                                              : null
                                          }
                                          busy={row.busyAction === "generate"}
                                          disabled={
                                            rowHasActiveOauthUrl
                                              ? !authUrl || oauthLocked
                                              : oauthLocked ||
                                                Boolean(rowGroupProxyError)
                                          }
                                          regenerateDisabled={
                                            oauthLocked ||
                                            Boolean(rowGroupProxyError)
                                          }
                                          onPrimaryAction={() => {
                                            if (rowHasActiveOauthUrl) {
                                              void handleBatchCopyOauthUrl(
                                                row.id,
                                              );
                                              return;
                                            }
                                            void handleBatchGenerateOauthUrl(
                                              row.id,
                                            );
                                          }}
                                          onRegenerate={() =>
                                            void handleBatchGenerateOauthUrl(
                                              row.id,
                                            )
                                          }
                                          onManualCopyOpenChange={(
                                            nextOpen,
                                          ) => {
                                            setBatchManualCopyRowId(
                                              nextOpen ? row.id : null,
                                            );
                                          }}
                                        />
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
                  <label className="field md:col-span-2">
                    <span className="field-label">
                      {t("accountPool.upstreamAccounts.import.paste.label")}
                    </span>
                    <textarea
                      className={cn(
                        "min-h-36 rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
                        importPasteError
                          ? "border-error/70 focus-visible:ring-error"
                          : "",
                      )}
                      name="importOauthPasteDraft"
                      value={importPasteDraft}
                      onChange={handleImportedOauthPasteDraftChange}
                      onPaste={handleImportedOauthPaste}
                      placeholder={t(
                        "accountPool.upstreamAccounts.import.paste.placeholder",
                      )}
                      autoCapitalize="none"
                      spellCheck={false}
                      aria-invalid={importPasteError ? "true" : "false"}
                      disabled={!writesEnabled || importPasteBusy}
                    />
                    {importPasteError ? (
                      <p className="mt-2 text-sm text-error">
                        {importPasteError}
                      </p>
                    ) : importPasteBusy ? (
                      <p className="mt-2 inline-flex items-center gap-2 text-sm text-base-content/65">
                        <Spinner className="size-4" />
                        {t(
                          "accountPool.upstreamAccounts.import.paste.validating",
                        )}
                      </p>
                    ) : (
                      <p className="mt-2 text-sm text-base-content/65">
                        {t("accountPool.upstreamAccounts.import.paste.hint")}
                      </p>
                    )}
                    <div className="mt-3 flex flex-wrap gap-2">
                      <Button
                        type="button"
                        variant="secondary"
                        onClick={() =>
                          void handleValidateImportedOauthPasteDraft()
                        }
                        disabled={
                          !writesEnabled ||
                          importPasteBusy ||
                          importPasteDraft.trim().length === 0
                        }
                      >
                        {t("accountPool.upstreamAccounts.import.paste.action")}
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => {
                          setImportPasteDraft("");
                          setImportPasteDraftSerial(null);
                          setImportPasteError(null);
                        }}
                        disabled={
                          importPasteBusy || importPasteDraft.length === 0
                        }
                      >
                        {t(
                          "accountPool.upstreamAccounts.import.paste.clearDraft",
                        )}
                      </Button>
                    </div>
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
                          hasGroupSettings(importGroupName)
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
                    {importGroupProxyState.error ? (
                      <p className="mt-2 text-xs text-error">
                        {importGroupProxyState.error}
                      </p>
                    ) : null}
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
                      disabled={
                        !writesEnabled ||
                        importFiles.length === 0 ||
                        Boolean(importGroupProxyState.error)
                      }
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
                          hasGroupSettings(apiKeyGroupName)
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
                    {apiKeyGroupProxyState.error ? (
                      <p className="mt-2 text-xs text-error">
                        {apiKeyGroupProxyState.error}
                      </p>
                    ) : null}
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
                        Boolean(apiKeyUpstreamBaseUrlError) ||
                        Boolean(apiKeyGroupProxyState.error)
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
        importDisabledReason={importGroupProxyState.error}
        onClose={handleCloseImportedOauthValidationDialog}
        onRetryFailed={() => void handleRetryImportedOauthFailed()}
        onRetryOne={(sourceId) => void handleRetryImportedOauthOne(sourceId)}
        onImportValid={() => void handleImportValidatedOauth()}
      />
      <UpstreamAccountGroupNoteDialog
        open={groupNoteEditor.open}
        groupName={groupNoteEditor.groupName}
        note={groupNoteEditor.note}
        concurrencyLimit={groupNoteEditor.concurrencyLimit}
        boundProxyKeys={groupNoteEditor.boundProxyKeys}
        nodeShuntEnabled={groupNoteEditor.nodeShuntEnabled}
        availableProxyNodes={forwardProxyNodes}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({ ...current, note: value }));
        }}
        onConcurrencyLimitChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            concurrencyLimit: value,
          }));
        }}
        onBoundProxyKeysChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            boundProxyKeys: value,
          }));
        }}
        onNodeShuntEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            nodeShuntEnabled: value,
          }));
        }}
        upstream429RetryEnabled={groupNoteEditor.upstream429RetryEnabled}
        upstream429MaxRetries={groupNoteEditor.upstream429MaxRetries}
        onUpstream429RetryEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            upstream429RetryEnabled: value,
            upstream429MaxRetries: value
              ? normalizeEnabledGroupUpstream429MaxRetries(
                  current.upstream429MaxRetries,
                )
              : normalizeGroupUpstream429MaxRetries(
                  current.upstream429MaxRetries,
                ),
          }));
        }}
        onUpstream429MaxRetriesChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            upstream429MaxRetries: current.upstream429RetryEnabled
              ? normalizeEnabledGroupUpstream429MaxRetries(value)
              : normalizeGroupUpstream429MaxRetries(value),
          }));
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
        concurrencyLimitLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.label",
        )}
        concurrencyLimitHint={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.hint",
        )}
        concurrencyLimitCurrentLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.current",
        )}
        concurrencyLimitUnlimitedLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.unlimited",
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
        nodeShuntLabel={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.label",
        )}
        nodeShuntHint={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.hint",
        )}
        nodeShuntToggleLabel={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.toggle",
        )}
        nodeShuntWarning={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.warning",
        )}
        upstream429RetryLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.label",
        )}
        upstream429RetryHint={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.hint",
        )}
        upstream429RetryToggleLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.toggle",
        )}
        upstream429RetryCountLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.countLabel",
        )}
        upstream429RetryCountOptions={GROUP_UPSTREAM_429_RETRY_OPTIONS.map(
          (value) => ({
            value,
            label:
              value === 1
                ? t(
                    "accountPool.upstreamAccounts.groupNotes.upstream429.countOnce",
                  )
                : t(
                    "accountPool.upstreamAccounts.groupNotes.upstream429.countMany",
                    { count: value },
                  ),
          }),
        )}
        proxyBindingsLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.label",
        )}
        proxyBindingsHint={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.hint",
        )}
        proxyBindingsAutomaticLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.automatic",
        )}
        proxyBindingsEmptyLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.empty",
        )}
        proxyBindingsMissingLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.missing",
        )}
        proxyBindingsUnavailableLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.unavailable",
        )}
        proxyBindingsChartLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel",
        )}
        proxyBindingsChartSuccessLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess",
        )}
        proxyBindingsChartFailureLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure",
        )}
        proxyBindingsChartEmptyLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty",
        )}
        proxyBindingsChartTotalLabel={t(
          "live.proxy.table.requestTooltip.total",
        )}
        proxyBindingsChartAriaLabel={t("live.proxy.table.requestTrendAria")}
        proxyBindingsChartInteractionHint={t("live.chart.tooltip.instructions")}
        proxyBindingsChartLocaleTag={locale === "zh" ? "zh-CN" : "en-US"}
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
