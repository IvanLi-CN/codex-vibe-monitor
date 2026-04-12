/* eslint-disable @typescript-eslint/no-explicit-any */
import { AppIcon } from "../../components/AppIcon";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { Input } from "../../components/ui/input";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { BatchOauthActionButton } from "../../components/account-pool/BatchOauthActionButton";
import { OauthMailboxChip } from "../../components/account-pool/OauthMailboxChip";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { MotherAccountToggle } from "../../components/MotherAccountToggle";
import {
  DuplicateWarningPopover,
} from "./UpstreamAccountCreate.shared";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreateBatchOauthSection() {
  const {
    batchCounts,
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
    batchStatusVariant,
    buildActionTooltip,
    cn,
    formatDuplicateReasons,
    groupSuggestions,
    handleBatchAttachMailbox,
    handleBatchCancelMailboxEdit,
    handleBatchCompleteOauth,
    handleBatchCompletedTextFieldBlur,
    handleBatchCompletedTextFieldKeyDown,
    handleBatchCopyMailbox,
    handleBatchCopyMailboxCode,
    handleBatchCopyOauthUrl,
    handleBatchGenerateMailbox,
    handleBatchGenerateOauthUrl,
    handleBatchGroupValueChange,
    handleBatchMailboxEditorValueChange,
    handleBatchMailboxFetch,
    handleBatchMetadataChange,
    handleBatchMotherToggle,
    handleBatchStartMailboxEdit,
    hasGroupSettings,
    isActivePendingOauthSession,
    isExpiredIso,
    isRefreshableMailboxSession,
    normalizeGroupName,
    openDuplicateDetailDialog,
    openGroupNoteEditor,
    refreshClockMs,
    removeBatchRow,
    resolveRequiredGroupProxyState,
    setBatchManualCopyRowId,
    t,
    toggleBatchNoteExpanded,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
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
          {batchRows.map((row: any, index: number) => {
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
  );
}
