/* eslint-disable @typescript-eslint/no-explicit-any */
import { AppIcon } from "../../components/AppIcon";
import { Link } from "react-router-dom";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import { Spinner } from "../../components/ui/spinner";
import { AccountTagField } from "../../components/AccountTagField";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreateImportSection() {
  const {
    cn,
    groupSuggestions,
    handleClearImportSelection,
    handleCreateTag,
    handleDeleteTag,
    handleImportFilesChange,
    handleImportedOauthPaste,
    handleImportedOauthPasteDraftChange,
    handleValidateImportedOauth,
    handleValidateImportedOauthPasteDraft,
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
    normalizeGroupName,
    openGroupNoteEditor,
    pageCreatedTagIds,
    setImportGroupName,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteError,
    setImportTagIds,
    t,
    tagFieldLabels,
    tagItems,
    updateTag,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
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
        {importFiles.map((item: any) => (
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
  );
}
