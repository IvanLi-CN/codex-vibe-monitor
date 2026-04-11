import { AppIcon } from "../../components/AppIcon";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { Input } from "../../components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { OauthMailboxChip } from "../../components/account-pool/OauthMailboxChip";
import { AccountTagField } from "../../components/AccountTagField";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { MotherAccountToggle } from "../../components/MotherAccountToggle";
import {
  DuplicateWarningPopover,
} from "./UpstreamAccountCreate.shared";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreateOauthSection() {
  const {
    activeOauthMailboxSession,
    buildActionTooltip,
    busyAction,
    clearOauthMailboxSession,
    displayedOauthMailboxStatus,
    formatDateTime,
    formatDuplicateReasons,
    groupSuggestions,
    handleAttachOauthMailbox,
    handleCompleteOauth,
    handleCopyOauthUrl,
    handleCopySingleInvite,
    handleCopySingleMailbox,
    handleCopySingleMailboxCode,
    handleCreateTag,
    handleDeleteTag,
    handleGenerateOauthMailbox,
    handleGenerateOauthUrl,
    hasGroupSettings,
    invalidateRelinkPendingOauthSessionForMailboxChange,
    invalidateSingleOauthSessionForMetadataEdit,
    isExpiredIso,
    isSupportedMailboxSession,
    mailboxInputMatchesSession,
    manualCopyFieldRef,
    manualCopyOpen,
    normalizeGroupName,
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
    selectAllReadonlyText,
    session,
    setActionError,
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
    updateTag,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
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
      setOauthIsMother((current: boolean) => !current);
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
  );
}
