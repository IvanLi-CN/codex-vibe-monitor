import { AppIcon } from "../../components/AppIcon";
import { Link } from "react-router-dom";
import { Button } from "../../components/ui/button";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { FormFieldFeedback } from "../../components/ui/form-field-feedback";
import { Input } from "../../components/ui/input";
import { AccountTagField } from "../../components/AccountTagField";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { MotherAccountToggle } from "../../components/MotherAccountToggle";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreateApiKeySection() {
  const {
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
    busyAction,
    cn,
    groupSuggestions,
    handleCreateApiKey,
    handleCreateTag,
    handleDeleteTag,
    hasGroupSettings,
    normalizeGroupName,
    openGroupNoteEditor,
    pageCreatedTagIds,
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
      onToggle={() => setApiKeyIsMother((current: boolean) => !current)}
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
  );
}
