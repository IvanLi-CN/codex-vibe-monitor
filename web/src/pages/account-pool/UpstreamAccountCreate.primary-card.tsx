import { AppIcon } from "../../features/shared/AppIcon";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Tooltip } from "../../components/ui/tooltip";
import { UpstreamAccountGroupCombobox } from "../../features/account-pool/UpstreamAccountGroupCombobox";
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
    buildActionTooltip,
    cn,
    formatGroupAccountCountLabel,
    groupOptions,
    handleBatchDefaultGroupCreateRequest,
    handleBatchDefaultGroupChange,
    hasBatchMetadataBusy,
    hasGroupSettings,
    normalizeGroupName,
    openGroupNoteEditor,
    t,
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
                  options={groupOptions}
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
                      "accountPool.upstreamAccounts.fields.groupNameConfigureValue",
                      { value },
                    )
                  }
                  onCreateRequested={handleBatchDefaultGroupCreateRequest}
                  formatAccountCountLabel={formatGroupAccountCountLabel}
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
                  : activeTab === "importSession"
                    ? t(
                        "accountPool.upstreamAccounts.importSession.createTitle",
                      )
                  : t("accountPool.upstreamAccounts.apiKey.createTitle")}
            </CardTitle>
            <CardDescription>
              {activeTab === "oauth"
                ? t("accountPool.upstreamAccounts.oauth.createDescription")
                : activeTab === "import"
                  ? t(
                      "accountPool.upstreamAccounts.import.createDescription",
                    )
                  : activeTab === "importSession"
                    ? t(
                        "accountPool.upstreamAccounts.importSession.createDescription",
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
        ) : activeTab === "importSession" ? (
          <UpstreamAccountCreateImportSection />
        ) : (
          <UpstreamAccountCreateApiKeySection />
        )}
      </CardContent>
    </Card>
  );
}
