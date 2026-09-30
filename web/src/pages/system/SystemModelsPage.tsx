import { useCallback, useEffect, useMemo, useState } from "react";
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
import { SelectField } from "../../components/ui/select-field";
import { Switch } from "../../components/ui/switch";
import { AppIcon } from "../../features/shared/AppIcon";
import { useTranslation } from "../../i18n";
import {
  applyModelsDevPriceSync,
  deleteManagedModel,
  fetchSettings,
  type ModelsDevPriceCandidate,
  type ModelsDevSyncPreview,
  type PricingEntry,
  previewModelsDevPriceSync,
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

type ModelCandidateGroup = {
  model: string;
  candidates: ModelsDevPriceCandidate[];
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

function candidatePriceFields(candidate: ModelsDevPriceCandidate) {
  return {
    inputPer1m: candidate.inputPer1m,
    outputPer1m: candidate.outputPer1m,
    cacheReadPer1m: candidate.cacheReadPer1m,
    cacheWritePer1m: candidate.cacheWritePer1m,
    reasoningPer1m: candidate.reasoningPer1m,
  };
}

function pricesEqual(local: PricingEntry | undefined, candidate: ModelsDevPriceCandidate): boolean {
  const existing = localPriceFields(local);
  const incoming = candidatePriceFields(candidate);
  return PRICE_FIELDS.every(([field]) => existing[field] === incoming[field]);
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
  const [syncState, setSyncState] = useState<
    "loading" | "ready" | "error" | "applying" | "applied"
  >("loading");
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncPreview, setSyncPreview] = useState<ModelsDevSyncPreview | null>(null);
  const [syncSearch, setSyncSearch] = useState("");
  const [selectedProviders, setSelectedProviders] = useState<Set<string>>(() => new Set());
  const [providerChoices, setProviderChoices] = useState<Record<string, string>>({});
  const [selectionOverrides, setSelectionOverrides] = useState<Record<string, boolean>>({});
  const [syncResultCount, setSyncResultCount] = useState(0);

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

  const openSyncPreview = useCallback(async () => {
    setSyncOpen(true);
    setSyncState("loading");
    setSyncError(null);
    setSyncPreview(null);
    setSyncSearch("");
    setSelectedProviders(new Set());
    setProviderChoices({});
    setSelectionOverrides({});
    try {
      const result = await previewModelsDevPriceSync();
      setSyncPreview(result);
      setSelectedProviders(new Set(result.providers.map((provider) => provider.id)));
      setSyncState("ready");
    } catch (error) {
      setSyncError(error instanceof Error ? error.message : String(error));
      setSyncState("error");
    }
  }, []);

  const retrySyncPreview = useCallback(() => {
    void openSyncPreview();
  }, [openSyncPreview]);

  const candidateGroups = useMemo<ModelCandidateGroup[]>(() => {
    if (!syncPreview) return [];
    const query = syncSearch.trim().toLocaleLowerCase();
    const groups = new Map<string, ModelsDevPriceCandidate[]>();
    for (const candidate of syncPreview.candidates) {
      if (!selectedProviders.has(candidate.providerId)) continue;
      const existing = groups.get(candidate.model) ?? [];
      existing.push(candidate);
      groups.set(candidate.model, existing);
    }
    return Array.from(groups, ([model, candidates]) => ({ model, candidates }))
      .filter(
        ({ model, candidates }) =>
          !query ||
          model.toLocaleLowerCase().includes(query) ||
          candidates.some(
            (candidate) =>
              candidate.name.toLocaleLowerCase().includes(query) ||
              candidate.providerName.toLocaleLowerCase().includes(query) ||
              candidate.providerId.toLocaleLowerCase().includes(query),
          ),
      )
      .sort((a, b) => a.model.localeCompare(b.model));
  }, [selectedProviders, syncPreview, syncSearch]);

  const selectedSyncEntries = useMemo(() => {
    const entries: PricingEntry[] = [];
    for (const group of candidateGroups) {
      const local = pricesByModel.get(group.model);
      const selectedProvider = providerChoices[group.model];
      const candidate =
        group.candidates.find((item) => item.providerId === selectedProvider) ??
        (group.candidates.length === 1 ? group.candidates[0] : undefined);
      if (!candidate || !candidate.importable || pricesEqual(local, candidate)) continue;
      const defaultSelected = group.candidates.length === 1 || Boolean(selectedProvider);
      const checked =
        selectionOverrides[group.model] ?? (defaultSelected && local?.source !== "custom");
      if (!checked) continue;
      entries.push({
        model: candidate.model,
        inputPer1m: candidate.inputPer1m ?? 0,
        outputPer1m: candidate.outputPer1m ?? 0,
        cacheInputPer1m: candidate.cacheReadPer1m,
        cacheReadPer1m: candidate.cacheReadPer1m,
        cacheWritePer1m: candidate.cacheWritePer1m,
        reasoningPer1m: candidate.reasoningPer1m,
        source: "models.dev",
      });
    }
    return entries;
  }, [candidateGroups, pricesByModel, providerChoices, selectionOverrides]);

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

  const applySelectedPrices = useCallback(async () => {
    if (selectedSyncEntries.length === 0) return;
    setSyncState("applying");
    setSyncError(null);
    try {
      const result = await applyModelsDevPriceSync(selectedSyncEntries);
      setSyncResultCount(selectedSyncEntries.length);
      setSyncState("applied");
      setSettings((current) =>
        current
          ? {
              ...current,
              pricing: result,
              proxy: {
                ...current.proxy,
                models: Array.from(
                  new Set([
                    ...current.proxy.models,
                    ...selectedSyncEntries.map((entry) => entry.model),
                  ]),
                ).sort((a, b) => a.localeCompare(b)),
              },
            }
          : current,
      );
    } catch (error) {
      setSyncError(error instanceof Error ? error.message : String(error));
      setSyncState("ready");
    }
  }, [selectedSyncEntries]);

  const closeSyncDialog = useCallback(
    (open: boolean) => {
      setSyncOpen(open);
      if (!open && syncState === "applied") {
        setSyncPreview(null);
      }
    },
    [syncState],
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
          <Button type="button" onClick={() => void openSyncPreview()}>
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

      <Dialog open={syncOpen} onOpenChange={closeSyncDialog}>
        <DialogContent className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden desktop:w-[min(78rem,calc(100vw-2rem))]">
          <div className="flex min-h-0 flex-col px-4 pb-4 pt-4 desktop:px-6 desktop:pt-5">
            <div className="flex items-start justify-between gap-4">
              <DialogHeader className="min-w-0">
                <DialogTitle>{t("system.models.syncTitle")}</DialogTitle>
                <DialogDescription>{t("system.models.syncDescription")}</DialogDescription>
              </DialogHeader>
              <DialogCloseIcon aria-label={t("system.models.close")} />
            </div>

            {syncState === "loading" ? (
              <div
                className="flex min-h-48 items-center justify-center gap-3 text-sm text-base-content/70"
                role="status"
              >
                <span className="loading loading-spinner loading-sm" aria-hidden />
                {t("system.models.fetching")}
              </div>
            ) : null}
            {syncState === "error" ? (
              <div className="space-y-4 py-5">
                <Alert variant="error" role="alert">
                  {syncError ?? t("system.models.fetchFailed")}
                </Alert>
                <Button type="button" variant="secondary" onClick={retrySyncPreview}>
                  <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                  {t("system.models.retry")}
                </Button>
              </div>
            ) : null}
            {(syncState === "ready" || syncState === "applying") && syncPreview ? (
              <>
                {syncError ? (
                  <Alert variant="error" role="alert" className="mt-3">
                    {syncError}
                  </Alert>
                ) : null}
                <div className="mt-4 grid gap-3 desktop:grid-cols-[minmax(0,1fr)_18rem]">
                  <label className="relative block">
                    <AppIcon
                      name="magnify"
                      className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-base-content/55"
                      aria-hidden
                    />
                    <Input
                      value={syncSearch}
                      className="pl-9"
                      placeholder={t("system.models.search")}
                      aria-label={t("system.models.search")}
                      onChange={(event) => setSyncSearch(event.target.value)}
                    />
                  </label>
                  <details className="group relative min-w-0">
                    <summary className="flex h-10 cursor-pointer list-none items-center justify-between gap-2 rounded-md border border-base-300 bg-base-100 px-3 text-sm">
                      <span className="truncate">
                        {t("system.models.providersSelected", {
                          count: selectedProviders.size,
                          total: syncPreview.providers.length,
                        })}
                      </span>
                      <AppIcon
                        name="chevron-down"
                        className="h-4 w-4 shrink-0 text-base-content/65"
                        aria-hidden
                      />
                    </summary>
                    <div className="absolute right-0 top-11 z-10 w-full min-w-[17rem] space-y-2 rounded-lg border border-base-300 bg-base-100 p-3 shadow-lg">
                      <div className="flex gap-2 border-b border-base-300/70 pb-2">
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setSelectedProviders(
                              new Set(syncPreview.providers.map((provider) => provider.id)),
                            );
                            setProviderChoices({});
                            setSelectionOverrides({});
                          }}
                        >
                          {t("system.models.selectAll")}
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setSelectedProviders(new Set());
                            setProviderChoices({});
                            setSelectionOverrides({});
                          }}
                        >
                          {t("system.models.clearAll")}
                        </Button>
                      </div>
                      <div className="max-h-52 space-y-1 overflow-y-auto">
                        {syncPreview.providers.map((provider) => (
                          <label
                            key={provider.id}
                            className="flex min-h-9 items-center gap-2 rounded px-1 text-sm hover:bg-base-200/60"
                          >
                            <input
                              type="checkbox"
                              checked={selectedProviders.has(provider.id)}
                              className="checkbox checkbox-sm"
                              onChange={(event) => {
                                const next = new Set(selectedProviders);
                                if (event.target.checked) next.add(provider.id);
                                else next.delete(provider.id);
                                setSelectedProviders(next);
                                setProviderChoices({});
                                setSelectionOverrides({});
                              }}
                            />
                            <span className="min-w-0 flex-1 truncate">{provider.name}</span>
                            <span className="text-xs text-base-content/55">{provider.id}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </details>
                </div>

                <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-xs text-base-content/65">
                  <span>
                    {t("system.models.fetchSummary", {
                      providers: syncPreview.providerCount,
                      models: syncPreview.candidateCount,
                    })}
                  </span>
                  <span>
                    {t("system.models.selectedCount", { count: selectedSyncEntries.length })}
                  </span>
                </div>

                <div className="mt-3 min-h-0 flex-1 overflow-y-auto border-y border-base-300/70">
                  {candidateGroups.length === 0 ? (
                    <div className="px-2 py-10 text-center text-sm text-base-content/65">
                      {t("system.models.noMatches")}
                    </div>
                  ) : (
                    <div className="divide-y divide-base-300/60">
                      {candidateGroups.map((group) => {
                        const local = pricesByModel.get(group.model);
                        const selectedProvider = providerChoices[group.model];
                        const candidate =
                          group.candidates.find((item) => item.providerId === selectedProvider) ??
                          (group.candidates.length === 1 ? group.candidates[0] : undefined);
                        const isDuplicate = group.candidates.length > 1;
                        const changed = candidate ? !pricesEqual(local, candidate) : false;
                        const defaultSelected = Boolean(
                          candidate && (group.candidates.length === 1 || selectedProvider),
                        );
                        const checked =
                          candidate && candidate.importable && changed
                            ? (selectionOverrides[group.model] ??
                              (defaultSelected && local?.source !== "custom"))
                            : false;
                        return (
                          <article
                            key={group.model}
                            className="grid gap-3 px-2 py-3 desktop:grid-cols-[minmax(13rem,1.2fr)_minmax(17rem,1.4fr)_minmax(17rem,1.4fr)_8rem] desktop:items-start"
                          >
                            <div className="min-w-0">
                              <div className="break-all font-mono text-[13px] font-medium">
                                {group.model}
                              </div>
                              <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-base-content/65">
                                {!local ? <span>{t("system.models.newModel")}</span> : null}
                                {local?.source === "custom" ? (
                                  <span>{t("system.models.manualPrice")}</span>
                                ) : null}
                                {!changed && local ? (
                                  <span>{t("system.models.noPriceChange")}</span>
                                ) : null}
                              </div>
                              {isDuplicate ? (
                                <SelectField
                                  className="mt-2"
                                  triggerClassName="h-9 min-w-0 rounded-md border border-base-300 px-2 text-sm"
                                  value={selectedProvider ?? ""}
                                  placeholder={t("system.models.chooseProviderPlaceholder")}
                                  aria-label={t("system.models.chooseProviderFor", {
                                    model: group.model,
                                  })}
                                  options={group.candidates.map((item) => ({
                                    value: item.providerId,
                                    label: `${item.providerName} (${item.providerId})`,
                                  }))}
                                  onValueChange={(providerId) => {
                                    setProviderChoices((current) => ({
                                      ...current,
                                      [group.model]: providerId,
                                    }));
                                    setSelectionOverrides((current) => {
                                      const next = { ...current };
                                      delete next[group.model];
                                      return next;
                                    });
                                  }}
                                />
                              ) : candidate ? (
                                <div className="mt-1 text-xs text-base-content/65">
                                  {candidate.providerName}
                                </div>
                              ) : null}
                              {candidate?.docUrl ? (
                                <a
                                  className="mt-2 inline-block text-xs text-primary underline-offset-2 hover:underline"
                                  href={candidate.docUrl}
                                  target="_blank"
                                  rel="noreferrer"
                                >
                                  {t("system.models.providerDocs")}
                                </a>
                              ) : null}
                            </div>

                            <div className="min-w-0">
                              <div className="mb-1 text-xs font-medium text-base-content/60">
                                {t("system.models.localPrice")}
                              </div>
                              <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs desktop:grid-cols-1">
                                {PRICE_FIELDS.map(([field, label]) => (
                                  <div key={field} className="flex min-w-0 justify-between gap-2">
                                    <span className="truncate text-base-content/60">
                                      {t(`settings.pricing.columns.${label}`)}
                                    </span>
                                    <span className="shrink-0 tabular-nums">
                                      {priceText(localPriceFields(local)[field])}
                                    </span>
                                  </div>
                                ))}
                              </div>
                            </div>

                            <div className="min-w-0">
                              <div className="mb-1 text-xs font-medium text-base-content/60">
                                {t("system.models.candidatePrice")}
                              </div>
                              {candidate ? (
                                <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs desktop:grid-cols-1">
                                  {PRICE_FIELDS.map(([field, label]) => (
                                    <div key={field} className="flex min-w-0 justify-between gap-2">
                                      <span className="truncate text-base-content/60">
                                        {t(`settings.pricing.columns.${label}`)}
                                      </span>
                                      <span className="shrink-0 tabular-nums">
                                        {priceText(candidatePriceFields(candidate)[field])}
                                      </span>
                                    </div>
                                  ))}
                                </div>
                              ) : (
                                <span className="text-xs text-base-content/55">
                                  {t("system.models.chooseProviderFirst")}
                                </span>
                              )}
                              {candidate?.unsupportedDimensions.length ? (
                                <p className="mt-2 break-words text-xs text-warning">
                                  {t("system.models.unsupported", {
                                    fields: candidate.unsupportedDimensions.join(", "),
                                  })}
                                </p>
                              ) : null}
                              {candidate && !candidate.importable ? (
                                <p className="mt-2 text-xs text-base-content/65">
                                  {t("system.models.notImportable")}
                                </p>
                              ) : null}
                            </div>

                            <div className="flex items-center justify-between gap-3 border-t border-base-300/50 pt-2 desktop:justify-end desktop:border-0 desktop:pt-0">
                              <span className="text-xs text-base-content/70">
                                {t("system.models.syncThisPrice")}
                              </span>
                              <input
                                type="checkbox"
                                className="checkbox checkbox-sm"
                                checked={Boolean(checked)}
                                disabled={
                                  !candidate ||
                                  !candidate.importable ||
                                  !changed ||
                                  syncState === "applying"
                                }
                                aria-label={t("system.models.syncModelPrice", {
                                  model: group.model,
                                })}
                                onChange={(event) =>
                                  setSelectionOverrides((current) => ({
                                    ...current,
                                    [group.model]: event.target.checked,
                                  }))
                                }
                              />
                            </div>
                          </article>
                        );
                      })}
                    </div>
                  )}
                </div>

                <DialogFooter className="mt-3 items-center desktop:justify-between">
                  <span className="text-xs text-base-content/65">
                    {t("system.models.sourceLabel")}
                  </span>
                  <div className="flex gap-2">
                    <Button type="button" variant="secondary" onClick={() => setSyncOpen(false)}>
                      {t("system.models.cancel")}
                    </Button>
                    <Button
                      type="button"
                      disabled={selectedSyncEntries.length === 0 || syncState === "applying"}
                      onClick={() => void applySelectedPrices()}
                    >
                      {syncState === "applying"
                        ? t("system.models.syncing")
                        : t("system.models.syncSelected", { count: selectedSyncEntries.length })}
                    </Button>
                  </div>
                </DialogFooter>
              </>
            ) : null}
            {syncState === "applied" ? (
              <div className="space-y-4 py-6">
                <Alert role="status">
                  {t("system.models.syncSuccess", { count: syncResultCount })}
                </Alert>
                <div className="flex justify-end">
                  <Button type="button" onClick={() => setSyncOpen(false)}>
                    {t("system.models.done")}
                  </Button>
                </div>
              </div>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
