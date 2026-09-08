import { Link, useLocation } from "react-router-dom";
import { Button } from "../../components/ui/button";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { FormFieldFeedback } from "../../components/ui/form-field-feedback";
import { Input } from "../../components/ui/input";
import { ForwardProxyBindingSelector } from "../../features/forward-proxy/ForwardProxyBindingSelector";
import { AppIcon } from "../../features/shared/AppIcon";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";

export function UpstreamAccountCreateApiKeySection() {
  const location = useLocation();
  const listPath = location.pathname.startsWith("/account-pool/transits")
    ? "/account-pool/transits"
    : "/account-pool/pool";
  const {
    apiKeyBoundProxyKeys,
    apiKeyDisplayName,
    apiKeyDisplayNameConflict,
    apiKeyForwardProxyCatalogState,
    apiKeyForwardProxyNodes,
    apiKeyLimitUnit,
    apiKeyNote,
    apiKeyPrimaryLimit,
    apiKeySecondaryLimit,
    apiKeyUpstreamBaseUrl,
    apiKeyUpstreamBaseUrlError,
    apiKeyValue,
    busyAction,
    cn,
    handleCreateApiKey,
    setApiKeyBoundProxyKeys,
    setApiKeyDisplayName,
    setApiKeyLimitUnit,
    setApiKeyNote,
    setApiKeyPrimaryLimit,
    setApiKeySecondaryLimit,
    setApiKeyUpstreamBaseUrl,
    setApiKeyValue,
    t,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
    <>
      <label className="field md:col-span-2">
        <span className="field-label">{t("accountPool.upstreamAccounts.fields.displayName")}</span>
        <div className="relative">
          <Input
            name="apiKeyDisplayName"
            value={apiKeyDisplayName}
            aria-invalid={apiKeyDisplayNameConflict != null}
            onChange={(event) => setApiKeyDisplayName(event.target.value)}
          />
          {apiKeyDisplayNameConflict ? (
            <FloatingFieldError
              message={t("accountPool.upstreamAccounts.validation.displayNameDuplicate")}
            />
          ) : null}
        </div>
      </label>
      <label className="field md:col-span-2">
        <span className="field-label">{t("accountPool.upstreamAccounts.fields.apiKey")}</span>
        <Input
          name="apiKeyValue"
          value={apiKeyValue}
          onChange={(event) => setApiKeyValue(event.target.value)}
        />
      </label>
      <label className="field md:col-span-2">
        <FormFieldFeedback
          label={t("accountPool.upstreamAccounts.fields.upstreamBaseUrl")}
          message={apiKeyUpstreamBaseUrlError}
          messageClassName="md:max-w-[min(30rem,calc(100%-9rem))]"
        />
        <div className="relative">
          <Input
            name="apiKeyUpstreamBaseUrl"
            value={apiKeyUpstreamBaseUrl}
            onChange={(event) => setApiKeyUpstreamBaseUrl(event.target.value)}
            placeholder={t("accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder")}
            autoCapitalize="none"
            spellCheck={false}
            aria-invalid={apiKeyUpstreamBaseUrlError ? "true" : "false"}
            className={cn(
              apiKeyUpstreamBaseUrlError ? "border-error/70 focus-visible:ring-error" : "",
            )}
          />
        </div>
      </label>
      <div className="field md:col-span-2">
        <FormFieldFeedback
          label={t("accountPool.upstreamAccounts.transitProxy.label")}
          message={
            apiKeyBoundProxyKeys.length === 0
              ? t("accountPool.upstreamAccounts.transitProxy.required")
              : null
          }
        />
        <ForwardProxyBindingSelector
          selectedKeys={apiKeyBoundProxyKeys}
          availableProxyNodes={apiKeyForwardProxyNodes}
          disabled={busyAction === "apiKey" || !writesEnabled}
          catalogKind={apiKeyForwardProxyCatalogState.kind}
          catalogFreshness={apiKeyForwardProxyCatalogState.freshness}
          onChange={(nextKeys) => {
            const normalized = nextKeys.map((key) => key.trim()).filter(Boolean);
            setApiKeyBoundProxyKeys(normalized.length > 0 ? normalized : ["__direct__"]);
          }}
          showAutomaticNotice={false}
          labels={{
            loading: t("accountPool.upstreamAccounts.proxyBindings.loading"),
            empty: t("accountPool.upstreamAccounts.proxyBindings.dialogEmpty"),
            missing: t("accountPool.upstreamAccounts.proxyBindings.statusMissing"),
            unavailable: t("accountPool.upstreamAccounts.proxyBindings.statusUnavailable"),
            chartLabel: t("accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel"),
            chartSuccess: t("accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess"),
            chartFailure: t("accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure"),
            chartEmpty: t("accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty"),
            chartTotal: t("accountPool.upstreamAccounts.groupNotes.proxyBindings.chartTotal"),
            chartAriaLabel: t(
              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartAriaLabel",
            ),
            chartInteractionHint: t(
              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartInteractionHint",
            ),
            chartLocaleTag: typeof navigator === "undefined" ? "en-US" : navigator.language,
          }}
        />
      </div>
      <label className="field">
        <span className="field-label">{t("accountPool.upstreamAccounts.fields.primaryLimit")}</span>
        <Input
          name="apiKeyPrimaryLimit"
          value={apiKeyPrimaryLimit}
          onChange={(event) => setApiKeyPrimaryLimit(event.target.value)}
        />
      </label>
      <label className="field">
        <span className="field-label">
          {t("accountPool.upstreamAccounts.fields.secondaryLimit")}
        </span>
        <Input
          name="apiKeySecondaryLimit"
          value={apiKeySecondaryLimit}
          onChange={(event) => setApiKeySecondaryLimit(event.target.value)}
        />
      </label>
      <label className="field">
        <span className="field-label">{t("accountPool.upstreamAccounts.fields.limitUnit")}</span>
        <Input
          name="apiKeyLimitUnit"
          value={apiKeyLimitUnit}
          onChange={(event) => setApiKeyLimitUnit(event.target.value)}
        />
      </label>
      <label className="field md:col-span-2">
        <span className="field-label">{t("accountPool.upstreamAccounts.fields.note")}</span>
        <textarea
          className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
          name="apiKeyNote"
          value={apiKeyNote}
          onChange={(event) => setApiKeyNote(event.target.value)}
        />
      </label>
      <div className="md:col-span-2 flex flex-wrap justify-end gap-2">
        <Button asChild type="button" variant="ghost">
          <Link to={listPath}>{t("accountPool.upstreamAccounts.actions.cancel")}</Link>
        </Button>
        <Button
          type="button"
          onClick={() => void handleCreateApiKey()}
          disabled={
            busyAction === "apiKey" ||
            !writesEnabled ||
            apiKeyDisplayNameConflict != null ||
            Boolean(apiKeyUpstreamBaseUrlError) ||
            apiKeyBoundProxyKeys.length === 0
          }
        >
          {busyAction === "apiKey" ? (
            <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
          ) : (
            <AppIcon name="content-save-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
          )}
          {t("accountPool.upstreamAccounts.actions.createApiKey")}
        </Button>
      </div>
    </>
  );
}
