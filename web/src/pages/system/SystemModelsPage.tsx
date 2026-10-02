import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { Input } from "../../components/ui/input";
import { Switch } from "../../components/ui/switch";
import { AppIcon } from "../../features/shared/AppIcon";
import { ModelsDevSyncDialog } from "../../features/system/models-dev-sync/ModelsDevSyncDialog";
import { useTranslation } from "../../i18n";
import {
  deleteManagedModel,
  fetchSettings,
  type PricingEntry,
  type SettingsPayload,
  updateManagedModelPreset,
  updatePricingSettings,
} from "../../lib/api";

type PriceDraft = {
  model: string;
  input: string;
  output: string;
  cacheRead: string;
  cacheWrite: string;
  reasoning: string;
};

const PRICE_FIELDS = [
  ["inputPer1m", "input"],
  ["outputPer1m", "output"],
  ["cacheReadPer1m", "cacheRead"],
  ["cacheWritePer1m", "cacheWrite"],
  ["reasoningPer1m", "reasoning"],
] as const;

function priceText(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 8,
  }).format(value);
}

function optionalNumber(value: string): number | null | undefined {
  const normalized = value.trim();
  if (!normalized) return null;
  const parsed = Number(normalized);
  if (!Number.isFinite(parsed) || parsed < 0) return undefined;
  return parsed;
}

function localPriceFields(entry: PricingEntry | undefined) {
  return {
    inputPer1m: entry?.inputPer1m ?? null,
    outputPer1m: entry?.outputPer1m ?? null,
    cacheReadPer1m: entry?.cacheReadPer1m ?? entry?.cacheInputPer1m ?? null,
    cacheWritePer1m: entry?.cacheWritePer1m ?? null,
    reasoningPer1m: entry?.reasoningPer1m ?? null,
  };
}

function formatDraft(entry?: PricingEntry, model = ""): PriceDraft {
  return {
    model: entry?.model ?? model,
    input: entry ? String(entry.inputPer1m) : "",
    output: entry ? String(entry.outputPer1m) : "",
    cacheRead:
      entry?.cacheReadPer1m == null && entry?.cacheInputPer1m == null
        ? ""
        : String(entry.cacheReadPer1m ?? entry.cacheInputPer1m),
    cacheWrite: entry?.cacheWritePer1m == null ? "" : String(entry.cacheWritePer1m),
    reasoning: entry?.reasoningPer1m == null ? "" : String(entry.reasoningPer1m),
  };
}

export default function SystemModelsPage() {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<SettingsPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [pageError, setPageError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyModel, setBusyModel] = useState<string | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editorIsNew, setEditorIsNew] = useState(false);
  const [editorDraft, setEditorDraft] = useState<PriceDraft>(() => formatDraft());
  const [editorError, setEditorError] = useState<string | null>(null);
  const [savingPrice, setSavingPrice] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [syncOpen, setSyncOpen] = useState(false);
  const syncTriggerRef = useRef<HTMLButtonElement>(null);

  const reloadSettings = useCallback(async () => {
    const loaded = await fetchSettings();
    setSettings(loaded);
    return loaded;
  }, []);

  const retryLoadSettings = useCallback(async () => {
    setLoading(true);
    setPageError(null);
    try {
      await reloadSettings();
    } catch (error) {
      setPageError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, [reloadSettings]);

  useEffect(() => {
    let active = true;
    void fetchSettings()
      .then((loaded) => {
        if (active) setSettings(loaded);
      })
      .catch((error: unknown) => {
        if (active) setPageError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  const pricesByModel = useMemo(
    () => new Map((settings?.pricing.entries ?? []).map((entry) => [entry.model, entry])),
    [settings?.pricing.entries],
  );
  const managedModels = useMemo(() => {
    return Array.from(
      new Set([
        ...(settings?.proxy.models ?? []),
        ...(settings?.pricing.entries.map((entry) => entry.model) ?? []),
      ]),
    ).sort((a, b) => a.localeCompare(b));
  }, [settings?.pricing.entries, settings?.proxy.models]);
  const enabledModels = useMemo(
    () => new Set(settings?.proxy.enabledModels ?? []),
    [settings?.proxy.enabledModels],
  );

  const saveManualPrice = useCallback(async () => {
    if (!settings || savingPrice) return;
    const model = editorDraft.model.trim();
    const input = optionalNumber(editorDraft.input);
    const output = optionalNumber(editorDraft.output);
    const cacheRead = optionalNumber(editorDraft.cacheRead);
    const cacheWrite = optionalNumber(editorDraft.cacheWrite);
    const reasoning = optionalNumber(editorDraft.reasoning);
    if (!model || model.length > 128 || input == null || output == null) {
      setEditorError(t("system.models.editor.required"));
      return;
    }
    if ([input, output, cacheRead, cacheWrite, reasoning].some((value) => value === undefined)) {
      setEditorError(t("system.models.editor.invalidPrice"));
      return;
    }
    const entry: PricingEntry = {
      model,
      inputPer1m: input,
      outputPer1m: output,
      cacheInputPer1m: cacheRead ?? null,
      cacheReadPer1m: cacheRead ?? null,
      cacheWritePer1m: cacheWrite ?? null,
      reasoningPer1m: reasoning ?? null,
      source: "custom",
    };
    const entries = settings.pricing.entries.filter((current) => current.model !== model);
    entries.push(entry);
    setSavingPrice(true);
    setEditorError(null);
    try {
      await updatePricingSettings({
        catalogVersion: settings.pricing.catalogVersion,
        entries: entries.sort((a, b) => a.model.localeCompare(b.model)),
      });
      await reloadSettings();
      setEditorOpen(false);
    } catch (error) {
      setEditorError(error instanceof Error ? error.message : String(error));
    } finally {
      setSavingPrice(false);
    }
  }, [editorDraft, reloadSettings, savingPrice, settings, t]);

  const handleDeleteModel = useCallback(async () => {
    if (!deleteTarget || busyModel) return;
    const model = deleteTarget;
    setBusyModel(model);
    setActionError(null);
    try {
      await deleteManagedModel(model);
      setSettings((current) =>
        current
          ? {
              ...current,
              proxy: {
                ...current.proxy,
                models: current.proxy.models.filter((candidate) => candidate !== model),
                enabledModels: current.proxy.enabledModels.filter(
                  (candidate) => candidate !== model,
                ),
              },
              pricing: {
                ...current.pricing,
                entries: current.pricing.entries.filter((entry) => entry.model !== model),
              },
            }
          : current,
      );
      setDeleteTarget(null);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusyModel(null);
    }
  }, [busyModel, deleteTarget]);

  const handlePresetChange = useCallback(
    async (model: string, enabled: boolean) => {
      if (busyModel) return;
      setBusyModel(model);
      setActionError(null);
      try {
        const proxy = await updateManagedModelPreset(model, enabled);
        setSettings((current) => (current ? { ...current, proxy } : current));
      } catch (error) {
        setActionError(error instanceof Error ? error.message : String(error));
      } finally {
        setBusyModel(null);
      }
    },
    [busyModel],
  );

  const handleSyncPricesApplied = useCallback(
    (pricing: SettingsPayload["pricing"], entries: PricingEntry[]) => {
      setSettings((current) =>
        current
          ? {
              ...current,
              pricing,
              proxy: {
                ...current.proxy,
                models: Array.from(
                  new Set([...current.proxy.models, ...entries.map((entry) => entry.model)]),
                ).sort((a, b) => a.localeCompare(b)),
              },
            }
          : current,
      );
    },
    [],
  );

  const openPriceEditor = useCallback(
    (model?: string) => {
      const entry = model ? pricesByModel.get(model) : undefined;
      setEditorIsNew(!model);
      setEditorDraft(formatDraft(entry, model));
      setEditorError(null);
      setEditorOpen(true);
    },
    [pricesByModel],
  );

  if (loading) {
    return (
      <div className="settings-page mx-auto max-w-full space-y-4 pb-2" aria-busy="true">
        <div className="h-8 w-48 animate-pulse rounded bg-base-200" />
        <div className="h-24 animate-pulse rounded-lg bg-base-200" />
        <div className="h-72 animate-pulse rounded-lg bg-base-200" />
      </div>
    );
  }

  if (!settings) {
    return (
      <div className="settings-page mx-auto max-w-full space-y-4 pb-2">
        <h1 className="text-2xl font-semibold">{t("system.models.title")}</h1>
        <Alert variant="error" role="alert">
          {pageError ?? t("system.models.loadFailed")}
        </Alert>
        <Button type="button" variant="secondary" onClick={() => void retryLoadSettings()}>
          {t("system.models.retry")}
        </Button>
      </div>
    );
  }

  return (
    <div className="settings-page mx-auto max-w-full space-y-5 pb-2">
      <header className="flex flex-wrap items-end justify-between gap-4">
        <div className="min-w-0">
          <h1 className="text-2xl font-semibold">{t("system.models.title")}</h1>
          <p className="mt-1 max-w-3xl text-sm leading-6 text-base-content/70">
            {t("system.models.description")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" onClick={() => openPriceEditor()}>
            <AppIcon name="plus" className="mr-2 h-4 w-4" aria-hidden />
            {t("system.models.add")}
          </Button>
          <Button ref={syncTriggerRef} type="button" onClick={() => setSyncOpen(true)}>
            <AppIcon name="sync" className="mr-2 h-4 w-4" aria-hidden />
            {t("system.models.syncAll")}
          </Button>
        </div>
      </header>

      {pageError ? (
        <Alert variant="error" role="alert">
          {pageError}
        </Alert>
      ) : null}
      {actionError ? (
        <Alert variant="error" role="alert">
          {actionError}
        </Alert>
      ) : null}

      <section aria-label={t("system.models.listTitle")}>
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-base-300/75 pb-3">
          <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <h2 className="text-base font-semibold">{t("system.models.listTitle")}</h2>
            <span className="text-xs text-base-content/65">
              {t("system.models.modelCount", { count: managedModels.length })}
            </span>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={loading}
            onClick={() => void retryLoadSettings()}
            aria-label={t("system.models.refresh")}
            title={t("system.models.refresh")}
          >
            <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
          </Button>
        </div>

        {managedModels.length === 0 ? (
          <div className="py-12 text-center text-sm text-base-content/65">
            {t("system.models.empty")}
          </div>
        ) : (
          <>
            <div className="hidden overflow-x-auto desktop:block">
              <table className="w-full min-w-[66rem] table-fixed text-sm">
                <thead className="border-b border-base-300/70 text-left text-xs text-base-content/65">
                  <tr>
                    <th className="w-[22%] py-3 pr-4 font-medium">{t("system.models.model")}</th>
                    <th className="w-[12%] py-3 pr-3 font-medium">
                      {t("settings.pricing.columns.input")}
                    </th>
                    <th className="w-[12%] py-3 pr-3 font-medium">
                      {t("settings.pricing.columns.output")}
                    </th>
                    <th className="w-[12%] py-3 pr-3 font-medium">
                      {t("settings.pricing.columns.cacheRead")}
                    </th>
                    <th className="w-[12%] py-3 pr-3 font-medium">
                      {t("settings.pricing.columns.cacheWrite")}
                    </th>
                    <th className="w-[12%] py-3 pr-3 font-medium">
                      {t("settings.pricing.columns.reasoning")}
                    </th>
                    <th className="w-[9%] py-3 pr-3 font-medium">{t("system.models.preset")}</th>
                    <th className="w-[9%] py-3 text-right font-medium">
                      {t("settings.pricing.columns.actions")}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-base-300/60">
                  {managedModels.map((model) => {
                    const price = pricesByModel.get(model);
                    const priceFields = localPriceFields(price);
                    const isBusy = busyModel === model;
                    return (
                      <tr key={model} className="hover:bg-base-200/35">
                        <td className="py-3 pr-4 align-middle">
                          <div className="break-all font-mono text-[13px]">{model}</div>
                          {price ? (
                            <Chip size="compact" tone="secondary" className="mt-1.5">
                              {price.source}
                            </Chip>
                          ) : null}
                        </td>
                        {PRICE_FIELDS.map(([field]) => (
                          <td key={field} className="py-3 pr-3 align-middle tabular-nums">
                            {priceText(priceFields[field])}
                          </td>
                        ))}
                        <td className="py-3 pr-3 align-middle">
                          <Switch
                            checked={enabledModels.has(model)}
                            disabled={isBusy}
                            aria-label={t("system.models.presetFor", { model })}
                            onCheckedChange={(checked) => void handlePresetChange(model, checked)}
                          />
                        </td>
                        <td className="py-3 text-right align-middle">
                          <div className="inline-flex gap-1">
                            <Button
                              type="button"
                              size="icon"
                              variant="ghost"
                              aria-label={t("system.models.editPrice")}
                              title={t("system.models.editPrice")}
                              disabled={isBusy}
                              onClick={() => openPriceEditor(model)}
                            >
                              <AppIcon name="pencil-outline" className="h-4 w-4" aria-hidden />
                            </Button>
                            <Button
                              type="button"
                              size="icon"
                              variant="ghost"
                              aria-label={t("system.models.delete")}
                              title={t("system.models.delete")}
                              disabled={isBusy}
                              onClick={() => setDeleteTarget(model)}
                            >
                              <AppIcon
                                name="trash-can-outline"
                                className="h-4 w-4 text-error"
                                aria-hidden
                              />
                            </Button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            <div className="divide-y divide-base-300/65 desktop:hidden">
              {managedModels.map((model) => {
                const price = pricesByModel.get(model);
                const priceFields = localPriceFields(price);
                const isBusy = busyModel === model;
                return (
                  <article key={model} className="py-4 first:pt-3 last:pb-2">
                    <div className="flex items-start gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="break-all font-mono text-[13px] font-medium">{model}</div>
                        {price ? (
                          <Chip size="compact" tone="secondary" className="mt-1.5">
                            {price.source}
                          </Chip>
                        ) : null}
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          aria-label={t("system.models.editPrice")}
                          title={t("system.models.editPrice")}
                          disabled={isBusy}
                          onClick={() => openPriceEditor(model)}
                        >
                          <AppIcon name="pencil-outline" className="h-4 w-4" aria-hidden />
                        </Button>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          aria-label={t("system.models.delete")}
                          title={t("system.models.delete")}
                          disabled={isBusy}
                          onClick={() => setDeleteTarget(model)}
                        >
                          <AppIcon
                            name="trash-can-outline"
                            className="h-4 w-4 text-error"
                            aria-hidden
                          />
                        </Button>
                      </div>
                    </div>
                    <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
                      {PRICE_FIELDS.map(([field, label]) => (
                        <div key={field} className="flex min-w-0 justify-between gap-2">
                          <dt className="truncate text-base-content/65">
                            {t(`settings.pricing.columns.${label}`)}
                          </dt>
                          <dd className="shrink-0 tabular-nums">{priceText(priceFields[field])}</dd>
                        </div>
                      ))}
                    </dl>
                    <div className="mt-3 flex items-center justify-between border-t border-base-300/60 pt-3">
                      <span className="text-xs text-base-content/70">
                        {t("system.models.preset")}
                      </span>
                      <Switch
                        checked={enabledModels.has(model)}
                        disabled={isBusy}
                        aria-label={t("system.models.presetFor", { model })}
                        onCheckedChange={(checked) => void handlePresetChange(model, checked)}
                      />
                    </div>
                  </article>
                );
              })}
            </div>
          </>
        )}
      </section>

      <Dialog open={editorOpen} onOpenChange={setEditorOpen}>
        <DialogContent className="flex max-h-[calc(100dvh-1rem)] flex-col overflow-hidden desktop:w-[min(38rem,calc(100vw-2rem))]">
          <div className="flex min-h-0 flex-col overflow-y-auto px-5 pb-5 pt-5 desktop:px-6">
            <div className="flex items-start justify-between gap-4">
              <DialogHeader>
                <DialogTitle>
                  {editorIsNew
                    ? t("system.models.editor.addTitle")
                    : t("system.models.editor.title")}
                </DialogTitle>
                <DialogDescription>{t("system.models.editor.description")}</DialogDescription>
              </DialogHeader>
              <DialogCloseIcon aria-label={t("system.models.close")} />
            </div>
            <div className="mt-5 grid gap-4 sm:grid-cols-2">
              <label className="space-y-1.5 sm:col-span-2">
                <span className="text-sm font-medium">{t("system.models.model")}</span>
                <Input
                  value={editorDraft.model}
                  disabled={!editorIsNew}
                  maxLength={128}
                  onChange={(event) =>
                    setEditorDraft((draft) => ({ ...draft, model: event.target.value }))
                  }
                />
              </label>
              {PRICE_FIELDS.map(([field, label]) => {
                const key =
                  field === "inputPer1m"
                    ? "input"
                    : field === "outputPer1m"
                      ? "output"
                      : field === "cacheReadPer1m"
                        ? "cacheRead"
                        : field === "cacheWritePer1m"
                          ? "cacheWrite"
                          : "reasoning";
                return (
                  <label key={field} className="space-y-1.5">
                    <span className="text-sm font-medium">
                      {t(`settings.pricing.columns.${label}`)}
                    </span>
                    <Input
                      type="number"
                      min="0"
                      step="any"
                      inputMode="decimal"
                      value={editorDraft[key]}
                      onChange={(event) =>
                        setEditorDraft((draft) => ({ ...draft, [key]: event.target.value }))
                      }
                    />
                  </label>
                );
              })}
            </div>
            {editorError ? (
              <Alert variant="error" role="alert" className="mt-4">
                {editorError}
              </Alert>
            ) : null}
            <DialogFooter className="mt-6">
              <Button type="button" variant="secondary" onClick={() => setEditorOpen(false)}>
                {t("system.models.cancel")}
              </Button>
              <Button type="button" disabled={savingPrice} onClick={() => void saveManualPrice()}>
                {savingPrice ? t("system.models.saving") : t("system.models.savePrice")}
              </Button>
            </DialogFooter>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteTarget != null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent className="desktop:w-[min(30rem,calc(100vw-2rem))]">
          <div className="px-5 pb-5 pt-5 desktop:px-6">
            <div className="flex items-start justify-between gap-4">
              <DialogHeader>
                <DialogTitle>{t("system.models.deleteTitle")}</DialogTitle>
                <DialogDescription>
                  {t("system.models.deleteDescription", { model: deleteTarget ?? "" })}
                </DialogDescription>
              </DialogHeader>
              <DialogCloseIcon aria-label={t("system.models.close")} />
            </div>
            <DialogFooter className="mt-6">
              <Button type="button" variant="secondary" onClick={() => setDeleteTarget(null)}>
                {t("system.models.cancel")}
              </Button>
              <Button
                type="button"
                variant="destructive"
                disabled={busyModel != null}
                onClick={() => void handleDeleteModel()}
              >
                {busyModel != null ? t("system.models.deleting") : t("system.models.delete")}
              </Button>
            </DialogFooter>
          </div>
        </DialogContent>
      </Dialog>

      <ModelsDevSyncDialog
        open={syncOpen}
        onOpenChange={setSyncOpen}
        returnFocusRef={syncTriggerRef}
        pricesByModel={pricesByModel}
        onPricesApplied={handleSyncPricesApplied}
      />
    </div>
  );
}
