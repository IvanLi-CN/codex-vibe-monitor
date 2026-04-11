/* eslint-disable @typescript-eslint/no-explicit-any */
import { AppIcon } from "../../components/AppIcon";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Tooltip } from "../../components/ui/tooltip";
import { AccountTagField } from "../../components/AccountTagField";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";
import { UpstreamAccountCreateApiKeySection } from "./UpstreamAccountCreate.api-key-section";
import { UpstreamAccountCreateBatchOauthSection } from "./UpstreamAccountCreate.batch-oauth-section";
import { UpstreamAccountCreateImportSection } from "./UpstreamAccountCreate.import-section";
import { UpstreamAccountCreateOauthSection } from "./UpstreamAccountCreate.oauth-section";

export function UpstreamAccountCreatePrimaryCard() {
  const {
    activeTab,
    appendBatchRow,
    batchDefaultGroupName,
    batchSharedTagSyncEnabledRef,
    batchTagIds,
    buildActionTooltip,
    cn,
    groupSuggestions,
    handleBatchDefaultGroupChange,
    handleCreateTag,
    handleDeleteTag,
    hasBatchMetadataBusy,
    hasGroupSettings,
    normalizeGroupName,
    openGroupNoteEditor,
    pageCreatedTagIds,
    setBatchRows,
    setBatchTagIds,
    t,
    tagFieldLabels,
    tagItems,
    updateTag,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
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
                  t("accountPool.upstreamAccounts.batchOauth.createTitle"),
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
                  onClick={() => openGroupNoteEditor(batchDefaultGroupName)}
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
                    setBatchRows((current: any[]) =>
                      current.map((row: any) => ({
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
                {t("accountPool.upstreamAccounts.batchOauth.actions.addRow")}
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
                ? t("accountPool.upstreamAccounts.oauth.createDescription")
                : activeTab === "import"
                  ? t(
                      "accountPool.upstreamAccounts.import.createDescription",
                    )
                  : t("accountPool.upstreamAccounts.apiKey.createDescription")}
            </CardDescription>
          </>
        )}
      </CardHeader>
      <CardContent
        className={cn("grid gap-4", activeTab === "apiKey" && "md:grid-cols-2")}
      >
        {activeTab === "oauth" ? (
          <UpstreamAccountCreateOauthSection />
        ) : activeTab === "batchOauth" ? (
          <UpstreamAccountCreateBatchOauthSection />
        ) : activeTab === "import" ? (
          <UpstreamAccountCreateImportSection />
        ) : (
          <UpstreamAccountCreateApiKeySection />
        )}
      </CardContent>
    </Card>
  );
}
