/* eslint-disable @typescript-eslint/no-explicit-any */
import { ImportedOauthValidationDialog } from "../../components/ImportedOauthValidationDialog";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { DuplicateAccountDetailDialog } from "./UpstreamAccountCreate.shared";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";
import { useGroupNoteCatalogAutoRefresh } from "./useGroupNoteCatalogAutoRefresh";

export function UpstreamAccountCreateDialogs() {
  const {
    GROUP_UPSTREAM_429_RETRY_OPTIONS,
    accountKindLabel,
    accountStatusLabel,
    closeGroupNoteEditor,
    duplicateDetail,
    duplicateDetailLoading,
    duplicateDetailOpen,
    formatDuplicateReasons,
    forwardProxyNodes,
    forwardProxyCatalogState,
    groupNoteBusy,
    groupNoteEditor,
    groupNoteError,
    handleCloseImportedOauthValidationDialog,
    handleImportValidatedOauth,
    handleRetryImportedOauthFailed,
    handleRetryImportedOauthOne,
    handleSaveGroupNote,
    importGroupProxyState,
    importValidationDialogOpen,
    importValidationState,
    locale,
    normalizeEnabledGroupUpstream429MaxRetries,
    normalizeGroupUpstream429MaxRetries,
    refreshForwardProxyBindings,
    setDuplicateDetail,
    setDuplicateDetailOpen,
    setGroupNoteEditor,
    setGroupNoteError,
    t,
  } = useUpstreamAccountCreateViewContext();

  useGroupNoteCatalogAutoRefresh({
    open: groupNoteEditor.open,
    refresh: refreshForwardProxyBindings,
    catalogState: forwardProxyCatalogState,
  });

  return (
    <>
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
        proxyBindingsCatalogKind={forwardProxyCatalogState?.kind}
        proxyBindingsCatalogFreshness={forwardProxyCatalogState?.freshness}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current: any) => ({ ...current, note: value }));
        }}
        onConcurrencyLimitChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current: any) => ({
            ...current,
            concurrencyLimit: value,
          }));
        }}
        onBoundProxyKeysChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current: any) => ({
            ...current,
            boundProxyKeys: value,
          }));
        }}
        onNodeShuntEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current: any) => ({
            ...current,
            nodeShuntEnabled: value,
          }));
        }}
        upstream429RetryEnabled={groupNoteEditor.upstream429RetryEnabled}
        upstream429MaxRetries={groupNoteEditor.upstream429MaxRetries}
        onUpstream429RetryEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current: any) => ({
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
          setGroupNoteEditor((current: any) => ({
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
          (value: number) => ({
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
        proxyBindingsLoadingLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.loading",
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
        proxyBindingsChartTotalLabel={t("live.proxy.table.requestTooltip.total")}
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
    </>
  );
}
