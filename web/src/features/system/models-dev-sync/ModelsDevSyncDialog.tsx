import { useVirtualizer } from "@tanstack/react-virtual";
import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../../components/ui/alert";
import { Button } from "../../../components/ui/button";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../../components/ui/dialog";
import { Input } from "../../../components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../../components/ui/popover";
import { SelectField } from "../../../components/ui/select-field";
import { Switch } from "../../../components/ui/switch";
import { useTranslation } from "../../../i18n";
import {
  applyModelsDevPriceSync,
  type ModelsDevSyncPreview,
  type ModelsDevSyncProvider,
  type PricingEntry,
  previewModelsDevPriceSync,
} from "../../../lib/api";
import { AppIcon } from "../../shared/AppIcon";
import {
  applySelectionChanges,
  buildModelCandidateGroups,
  candidatePriceFields,
  createApplicablePriceEntries,
  filterModelCandidateGroups,
  localPriceFields,
  type ModelCandidateGroup,
  modelPricesEqual,
  modelProviderKey,
  PRICE_FIELDS,
  priceDifferenceFields,
  quoteProviderChoicesByModel,
  resolveModelCandidate,
  selectionMemoryByKey,
} from "./selection";
import { useModelsDevSyncMemory } from "./useModelsDevSyncMemory";

type DialogState = "loading" | "ready" | "error" | "applying" | "applied";

interface ModelsDevSyncDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  returnFocusRef: RefObject<HTMLButtonElement | null>;
  pricesByModel: ReadonlyMap<string, PricingEntry>;
  onPricesApplied(
    pricing: Awaited<ReturnType<typeof applyModelsDevPriceSync>>,
    entries: PricingEntry[],
  ): void;
}

type Translate = ReturnType<typeof useTranslation>["t"];

function formatPrice(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 12,
  }).format(value);
}

interface ModelCandidateRowProps {
  group: ModelCandidateGroup;
  local: PricingEntry | undefined;
  candidate: ModelCandidateGroup["candidates"][number] | undefined;
  rememberedProviderUnavailable: boolean;
  selected: boolean;
  isNew: boolean;
  applying: boolean;
  scrollElement: HTMLElement | null;
  onViewed(model: string): void;
  onSelectionChange(model: string, providerId: string, selected: boolean): void;
  onQuoteProviderChange(model: string, providerId: string): void;
  t: Translate;
}

function ModelCandidateRow({
  group,
  local,
  candidate,
  rememberedProviderUnavailable,
  selected,
  isNew,
  applying,
  scrollElement,
  onViewed,
  onSelectionChange,
  onQuoteProviderChange,
  t,
}: ModelCandidateRowProps) {
  const rowRef = useRef<HTMLElement | null>(null);
  const differences = useMemo(
    () => (candidate ? priceDifferenceFields(local, candidate) : new Set()),
    [candidate, local],
  );
  useEffect(() => {
    if (
      !isNew ||
      !scrollElement ||
      !rowRef.current ||
      typeof IntersectionObserver === "undefined"
    ) {
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting && entry.intersectionRatio > 0)) {
          onViewed(group.model);
          observer.disconnect();
        }
      },
      { root: scrollElement, threshold: 0.01 },
    );
    observer.observe(rowRef.current);
    return () => observer.disconnect();
  }, [group.model, isNew, onViewed, scrollElement]);

  return (
    <article
      ref={rowRef}
      className="grid gap-3 border-b border-base-300/60 px-2 py-3 desktop:grid-cols-[minmax(13rem,1.2fr)_minmax(17rem,1.4fr)_minmax(17rem,1.4fr)_8rem] desktop:items-start"
    >
      <div className="min-w-0">
        <div className="break-all font-mono text-[13px] font-medium">
          {group.model}
          {isNew ? (
            <span
              className="ml-2 inline-block h-2 w-2 rounded-full bg-info align-middle"
              title={t("system.models.newModel")}
              aria-label={t("system.models.newModelAccessible")}
              role="img"
            />
          ) : null}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-base-content/65">
          {!local ? <span>{t("system.models.locallyUnpriced")}</span> : null}
          {local?.source === "custom" ? <span>{t("system.models.manualPrice")}</span> : null}
          {candidate && modelPricesEqual(local, candidate) && local ? (
            <span>{t("system.models.noPriceChange")}</span>
          ) : null}
          {candidate?.status ? (
            <span className="text-base-content/55">
              {t("system.models.sourceStatus", { status: candidate.status })}
            </span>
          ) : null}
        </div>
        {group.candidates.length > 1 || (candidate === undefined && group.candidates.length > 0) ? (
          <SelectField
            className="mt-2"
            options={group.candidates.map((item) => ({
              value: item.providerId,
              label: `${item.providerName} (${item.providerId})`,
            }))}
            value={candidate?.providerId ?? ""}
            placeholder={t("system.models.chooseProviderPlaceholder")}
            aria-label={t("system.models.chooseProviderFor", {
              model: group.model,
            })}
            triggerClassName="h-9"
            disabled={applying}
            onValueChange={(providerId) => onQuoteProviderChange(group.model, providerId)}
          />
        ) : candidate ? (
          <div className="mt-1 text-xs text-base-content/65">{candidate.providerName}</div>
        ) : null}
        {rememberedProviderUnavailable ? (
          <div className="mt-2 text-xs text-warning" role="status">
            {t("system.models.rememberedProviderUnavailable")}
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

      <PriceColumn title={t("system.models.localPrice")} values={localPriceFields(local)} />

      <div className="min-w-0">
        <div className="mb-1 text-xs font-medium text-base-content/60">
          {t("system.models.candidatePrice")}
        </div>
        {candidate ? (
          <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs desktop:grid-cols-1">
            {PRICE_FIELDS.map(([field, label]) => {
              const changed = differences.has(field);
              return (
                <div key={field} className="flex min-w-0 justify-between gap-2">
                  <span className="truncate text-base-content/60">
                    {t(`settings.pricing.columns.${label}`)}
                  </span>
                  <span
                    className={
                      changed
                        ? "shrink-0 rounded-sm border border-warning/40 bg-warning/10 px-1 font-semibold text-warning tabular-nums"
                        : "shrink-0 tabular-nums"
                    }
                  >
                    {formatPrice(candidatePriceFields(candidate)[field])}
                    {changed ? ` · ${t("system.models.priceChanged")}` : ""}
                  </span>
                </div>
              );
            })}
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
          <p className="mt-2 text-xs text-base-content/65">{t("system.models.notImportable")}</p>
        ) : null}
      </div>

      <label className="flex items-center justify-between gap-3 border-t border-base-300/50 pt-2 text-xs text-base-content/70 desktop:justify-end desktop:border-0 desktop:pt-0">
        <span>{t("system.models.syncThisPrice")}</span>
        <input
          type="checkbox"
          className="checkbox checkbox-sm"
          checked={selected}
          disabled={!candidate?.importable || applying}
          aria-label={t("system.models.syncModelPrice", { model: group.model })}
          onChange={(event) =>
            candidate && onSelectionChange(group.model, candidate.providerId, event.target.checked)
          }
        />
      </label>
    </article>
  );
}

function PriceColumn({
  title,
  values,
}: {
  title: string;
  values: ReturnType<typeof localPriceFields>;
}) {
  const { t } = useTranslation();
  return (
    <div className="min-w-0">
      <div className="mb-1 text-xs font-medium text-base-content/60">{title}</div>
      <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs desktop:grid-cols-1">
        {PRICE_FIELDS.map(([field, label]) => (
          <div key={field} className="flex min-w-0 justify-between gap-2">
            <span className="truncate text-base-content/60">
              {t(`settings.pricing.columns.${label}`)}
            </span>
            <span className="shrink-0 tabular-nums">{formatPrice(values[field])}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function ProviderPicker({
  providers,
  selected,
  disabled,
  onToggle,
  onSearchAction,
  t,
}: {
  providers: ModelsDevSyncProvider[];
  selected: ReadonlySet<string>;
  disabled: boolean;
  onToggle(providerId: string, checked: boolean): void;
  onSearchAction(providerIds: string[], selected: boolean): void;
  t: Translate;
}) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);
  const [search, setSearch] = useState("");
  const visibleProviders = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return providers;
    return providers.filter(
      (provider) =>
        provider.id.toLocaleLowerCase().includes(query) ||
        provider.name.toLocaleLowerCase().includes(query),
    );
  }, [providers, search]);

  return (
    <Popover
      open={open && !disabled}
      onOpenChange={(nextOpen) => {
        if (!disabled) setOpen(nextOpen);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="secondary"
          className="h-10 w-full justify-between font-normal"
          aria-label={t("system.models.providerPicker")}
          disabled={disabled}
        >
          <span className="truncate">
            {t("system.models.providersSelected", {
              count: selected.size,
              total: providers.length,
            })}
          </span>
          <AppIcon name="chevron-down" className="ml-2 h-4 w-4 shrink-0" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        container={null}
        align="end"
        side="bottom"
        collisionPadding={12}
        className="z-[82] flex max-h-[min(30rem,var(--radix-popover-content-available-height))] w-[min(20rem,calc(100vw-1.5rem))] flex-col overflow-hidden p-2"
        aria-label={t("system.models.providerPicker")}
      >
        <Input
          value={search}
          placeholder={t("system.models.providerSearch")}
          aria-label={t("system.models.providerSearch")}
          onChange={(event) => setSearch(event.target.value)}
        />
        <div className="flex items-center gap-2 border-b border-base-300/70 py-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={disabled}
            onClick={() =>
              onSearchAction(
                visibleProviders.map((provider) => provider.id),
                true,
              )
            }
          >
            {t("system.models.selectAll")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={disabled}
            onClick={() =>
              onSearchAction(
                visibleProviders.map((provider) => provider.id),
                false,
              )
            }
          >
            {t("system.models.clearAll")}
          </Button>
        </div>
        <fieldset
          disabled={disabled}
          className="min-h-0 max-h-[min(22rem,calc(100dvh-8rem))] flex-1 overflow-y-auto"
        >
          <legend className="sr-only">{t("system.models.providerPicker")}</legend>
          {visibleProviders.length ? (
            visibleProviders.map((provider) => (
              <label
                key={provider.id}
                className="flex min-h-9 items-center gap-2 rounded px-1 text-sm hover:bg-base-200/60"
              >
                <input
                  type="checkbox"
                  className="checkbox checkbox-sm"
                  checked={selected.has(provider.id)}
                  onChange={(event) => onToggle(provider.id, event.target.checked)}
                />
                <span className="min-w-0 flex-1 truncate">{provider.name}</span>
                <span className="text-xs text-base-content/55">{provider.id}</span>
              </label>
            ))
          ) : (
            <p className="py-5 text-center text-sm text-base-content/65">
              {t("system.models.noProvidersMatch")}
            </p>
          )}
        </fieldset>
      </PopoverContent>
    </Popover>
  );
}

export function ModelsDevSyncDialog({
  open,
  onOpenChange,
  returnFocusRef,
  pricesByModel,
  onPricesApplied,
}: ModelsDevSyncDialogProps) {
  const { t } = useTranslation();
  const [dialogState, setDialogState] = useState<DialogState>("loading");
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncPreview, setSyncPreview] = useState<ModelsDevSyncPreview | null>(null);
  const {
    memory: syncMemory,
    error: memoryError,
    isSaving: memorySaving,
    queueMemoryPatch,
    capturePersistedRevision,
    restoreFromServer,
    retry: retryMemorySave,
  } = useModelsDevSyncMemory();
  const [currentReviewNewModels, setCurrentReviewNewModels] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [selectedProviders, setSelectedProviders] = useState<ReadonlySet<string>>(new Set());
  const [modelSearch, setModelSearch] = useState("");
  const [showDeprecated, setShowDeprecated] = useState(false);
  const [resultCount, setResultCount] = useState(0);
  const [scrollElement, setScrollElement] = useState<HTMLElement | null>(null);
  const viewAcknowledgedRef = useRef(new Set<string>());
  const previewRequestIdRef = useRef(0);
  const previewAbortControllerRef = useRef<AbortController | null>(null);
  const applyRequestIdRef = useRef(0);
  const applyingRef = useRef(false);
  const mountedRef = useRef(false);

  const invalidatePreviewRequest = useCallback(() => {
    previewRequestIdRef.current += 1;
    previewAbortControllerRef.current?.abort();
    previewAbortControllerRef.current = null;
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      invalidatePreviewRequest();
      applyRequestIdRef.current += 1;
      applyingRef.current = false;
    };
  }, [invalidatePreviewRequest]);

  const loadPreview = useCallback(async () => {
    previewAbortControllerRef.current?.abort();
    const abortController = new AbortController();
    const requestId = ++previewRequestIdRef.current;
    const persistedRevisionAtRequestStart = capturePersistedRevision();
    previewAbortControllerRef.current = abortController;
    setDialogState("loading");
    setSyncError(null);
    setModelSearch("");
    setShowDeprecated(false);
    viewAcknowledgedRef.current.clear();
    try {
      const result = await previewModelsDevPriceSync({
        signal: abortController.signal,
      });
      if (
        !mountedRef.current ||
        abortController.signal.aborted ||
        requestId !== previewRequestIdRef.current
      ) {
        return;
      }
      const restoredSyncState = restoreFromServer(
        result.syncState,
        persistedRevisionAtRequestStart,
      );
      setSyncPreview(result);
      setCurrentReviewNewModels(new Set(restoredSyncState.unviewedModelIds));
      setSelectedProviders(
        new Set(
          restoredSyncState.providerSelectionInitialized
            ? restoredSyncState.providerSelections
                .filter((selection) => selection.selected)
                .map((selection) => selection.providerId)
            : result.providers.map((provider) => provider.id),
        ),
      );
      setDialogState("ready");
    } catch (error) {
      if (
        !mountedRef.current ||
        abortController.signal.aborted ||
        requestId !== previewRequestIdRef.current
      ) {
        return;
      }
      setSyncError(error instanceof Error ? error.message : String(error));
      setDialogState("error");
    } finally {
      if (requestId === previewRequestIdRef.current) {
        previewAbortControllerRef.current = null;
      }
    }
  }, [capturePersistedRevision, restoreFromServer]);

  useEffect(() => {
    if (!open) {
      invalidatePreviewRequest();
      return;
    }
    void loadPreview();
    return invalidatePreviewRequest;
  }, [invalidatePreviewRequest, loadPreview, open]);

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) {
        if (applyingRef.current) return;
        invalidatePreviewRequest();
      }
      onOpenChange(nextOpen);
    },
    [invalidatePreviewRequest, onOpenChange],
  );

  const selectionByKey = useMemo(() => selectionMemoryByKey(syncMemory), [syncMemory]);
  const quoteChoices = useMemo(() => quoteProviderChoicesByModel(syncMemory), [syncMemory]);
  const candidateGroups = useMemo(
    () =>
      buildModelCandidateGroups(syncPreview?.candidates ?? [], selectedProviders, showDeprecated),
    [selectedProviders, showDeprecated, syncPreview],
  );
  const visibleGroups = useMemo(
    () => filterModelCandidateGroups(candidateGroups, modelSearch),
    [candidateGroups, modelSearch],
  );
  const applicableEntries = useMemo(
    () =>
      createApplicablePriceEntries(candidateGroups, pricesByModel, selectionByKey, quoteChoices),
    [candidateGroups, pricesByModel, quoteChoices, selectionByKey],
  );
  const visibleModelIds = useMemo(
    () => new Set(visibleGroups.map((group) => group.model)),
    [visibleGroups],
  );
  const hiddenApplyCount = applicableEntries.reduce(
    (count, entry) => count + Number(!visibleModelIds.has(entry.model)),
    0,
  );
  const virtualizer = useVirtualizer({
    count: visibleGroups.length,
    getScrollElement: () => scrollElement,
    estimateSize: () => 220,
    initialRect: { width: 960, height: 520 },
    overscan: 8,
    getItemKey: (index) => visibleGroups[index]?.model ?? index,
  });

  const acknowledgeViewed = useCallback(
    (model: string) => {
      if (!currentReviewNewModels.has(model) || viewAcknowledgedRef.current.has(model)) return;
      viewAcknowledgedRef.current.add(model);
      queueMemoryPatch({ viewedModelIds: [model] });
    },
    [currentReviewNewModels, queueMemoryPatch],
  );

  const saveSelection = useCallback(
    (model: string, providerId: string, selected: boolean) => {
      queueMemoryPatch({ modelSelections: [{ model, providerId, selected }] });
    },
    [queueMemoryPatch],
  );

  const saveQuoteProvider = useCallback(
    (model: string, providerId: string) => {
      queueMemoryPatch({ quoteProviderChoices: [{ model, providerId }] });
    },
    [queueMemoryPatch],
  );

  const updateProviderSelection = useCallback(
    (providerIds: string[], selected: boolean) => {
      if (!providerIds.length) return;
      const next = new Set(selectedProviders);
      providerIds.forEach((providerId) => {
        if (selected) next.add(providerId);
        else next.delete(providerId);
      });
      setSelectedProviders(next);
      queueMemoryPatch({
        providerSelections: providerIds.map((providerId) => ({
          providerId,
          selected,
        })),
      });
    },
    [queueMemoryPatch, selectedProviders],
  );

  const bulkChange = useCallback(
    (value: boolean | "invert") => {
      const changes = visibleGroups.flatMap((group) => {
        const candidate = resolveModelCandidate(group, quoteChoices[group.model]);
        return candidate?.importable
          ? [{ model: group.model, providerId: candidate.providerId }]
          : [];
      });
      if (!changes.length) return;
      const next = applySelectionChanges(selectionByKey, changes, value);
      const modelSelections = changes.map(({ model, providerId }) => ({
        model,
        providerId,
        selected: next[modelProviderKey(model, providerId)],
      }));
      queueMemoryPatch({ modelSelections });
    },
    [queueMemoryPatch, quoteChoices, selectionByKey, visibleGroups],
  );

  const applySelected = useCallback(async () => {
    if (!applicableEntries.length || applyingRef.current) return;
    applyingRef.current = true;
    const requestId = ++applyRequestIdRef.current;
    setDialogState("applying");
    setSyncError(null);
    try {
      const pricing = await applyModelsDevPriceSync(applicableEntries);
      if (!mountedRef.current || requestId !== applyRequestIdRef.current) return;
      setResultCount(applicableEntries.length);
      onPricesApplied(pricing, applicableEntries);
      setDialogState("applied");
    } catch (error) {
      if (!mountedRef.current || requestId !== applyRequestIdRef.current) return;
      setSyncError(error instanceof Error ? error.message : String(error));
      setDialogState("ready");
    } finally {
      if (requestId === applyRequestIdRef.current) applyingRef.current = false;
    }
  }, [applicableEntries, onPricesApplied]);

  const providers = syncPreview?.providers ?? [];
  const virtualRows = virtualizer.getVirtualItems();

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden desktop:min-h-[min(35rem,calc(100dvh-1.25rem))] desktop:w-[min(78rem,calc(100vw-2rem))]"
        onCloseAutoFocus={(event) => {
          if (!returnFocusRef.current) return;
          event.preventDefault();
          returnFocusRef.current.focus();
        }}
      >
        <div className="flex min-h-0 flex-1 flex-col px-4 pb-4 pt-4 desktop:px-6 desktop:pt-5">
          <div className="flex shrink-0 items-start justify-between gap-4">
            <DialogHeader className="min-w-0">
              <DialogTitle>{t("system.models.syncTitle")}</DialogTitle>
              <DialogDescription>{t("system.models.syncDescription")}</DialogDescription>
            </DialogHeader>
            <DialogCloseIcon
              aria-label={t("system.models.close")}
              disabled={dialogState === "applying"}
            />
          </div>

          {dialogState === "loading" ? (
            <div
              className="flex min-h-48 flex-1 items-center justify-center gap-3 text-sm text-base-content/70"
              role="status"
            >
              <span className="loading loading-spinner loading-sm" aria-hidden />
              {t("system.models.fetching")}
            </div>
          ) : null}

          {dialogState === "error" ? (
            <div className="py-5">
              <Alert variant="error" role="alert" className="items-center">
                <span className="min-w-0 flex-1 break-words">
                  {syncError ?? t("system.models.fetchFailed")}
                </span>
                <Button
                  type="button"
                  size="xs"
                  variant="destructive"
                  className="shrink-0"
                  onClick={() => void loadPreview()}
                >
                  <AppIcon name="refresh" className="mr-1.5 h-3.5 w-3.5" aria-hidden />
                  {t("system.models.retry")}
                </Button>
              </Alert>
            </div>
          ) : null}

          {memoryError ? (
            <Alert variant="error" role="alert" className="mt-3 shrink-0 items-center">
              <AppIcon name="alert-circle-outline" className="h-4 w-4 shrink-0" aria-hidden />
              <span className="min-w-0 flex-1 break-words">
                {t("system.models.memorySaveFailed")}
              </span>
              <Button
                type="button"
                size="xs"
                variant="destructive"
                className="shrink-0"
                onClick={() => void retryMemorySave()}
              >
                <AppIcon name="refresh" className="mr-1.5 h-3.5 w-3.5" aria-hidden />
                {t("system.models.retry")}
              </Button>
            </Alert>
          ) : memorySaving ? (
            <div className="mt-2 text-xs text-base-content/65" role="status">
              {t("system.models.memorySaving")}
            </div>
          ) : null}

          {(dialogState === "ready" || dialogState === "applying") && syncPreview ? (
            <>
              {syncError ? (
                <Alert variant="error" role="alert" className="mt-3 shrink-0">
                  {syncError}
                </Alert>
              ) : null}
              <div className="mt-4 grid shrink-0 gap-3 desktop:grid-cols-[minmax(0,1fr)_18rem]">
                <label className="relative block">
                  <AppIcon
                    name="magnify"
                    className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-base-content/55"
                    aria-hidden
                  />
                  <Input
                    value={modelSearch}
                    className="pl-9"
                    placeholder={t("system.models.search")}
                    aria-label={t("system.models.search")}
                    onChange={(event) => setModelSearch(event.target.value)}
                  />
                </label>
                <ProviderPicker
                  providers={providers}
                  selected={selectedProviders}
                  disabled={dialogState === "applying"}
                  onToggle={(providerId, checked) => updateProviderSelection([providerId], checked)}
                  onSearchAction={updateProviderSelection}
                  t={t}
                />
              </div>

              <div
                data-testid="models-sync-toolbar"
                className="mt-2 grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-y border-base-300/70 bg-base-200/30 px-2 py-1 text-xs text-base-content/65 desktop:grid-cols-[auto_minmax(0,1fr)_auto] desktop:gap-x-4"
              >
                <span className="col-start-1 row-start-1 shrink-0">
                  {t("system.models.fetchSummary", {
                    providers: syncPreview.providerCount,
                    models: syncPreview.candidateCount,
                  })}
                </span>
                <fieldset
                  className="col-start-1 row-start-2 m-0 flex min-w-0 items-center gap-x-1 border-0 p-0 desktop:col-start-2 desktop:row-start-1 desktop:gap-x-2"
                  aria-label={t("system.models.bulkScope", {
                    count: visibleGroups.length,
                  })}
                >
                  <span className="shrink-0 text-base-content/55">
                    {t("system.models.bulkScopeCompact", {
                      count: visibleGroups.length,
                    })}
                  </span>
                  <div className="flex shrink-0 items-center gap-0.5">
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-1.5 text-xs desktop:h-8 desktop:px-3 desktop:text-sm"
                      aria-label={t("system.models.selectAll")}
                      title={t("system.models.selectAll")}
                      disabled={dialogState === "applying"}
                      onClick={() => bulkChange(true)}
                    >
                      <span className="desktop:hidden">{t("system.models.selectAllCompact")}</span>
                      <span className="hidden desktop:inline">{t("system.models.selectAll")}</span>
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-1.5 text-xs desktop:h-8 desktop:px-3 desktop:text-sm"
                      aria-label={t("system.models.invertSelection")}
                      title={t("system.models.invertSelection")}
                      disabled={dialogState === "applying"}
                      onClick={() => bulkChange("invert")}
                    >
                      <span className="desktop:hidden">
                        {t("system.models.invertSelectionCompact")}
                      </span>
                      <span className="hidden desktop:inline">
                        {t("system.models.invertSelection")}
                      </span>
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-1.5 text-xs desktop:h-8 desktop:px-3 desktop:text-sm"
                      aria-label={t("system.models.selectNone")}
                      title={t("system.models.selectNone")}
                      disabled={dialogState === "applying"}
                      onClick={() => bulkChange(false)}
                    >
                      <span className="desktop:hidden">{t("system.models.selectNoneCompact")}</span>
                      <span className="hidden desktop:inline">{t("system.models.selectNone")}</span>
                    </Button>
                  </div>
                </fieldset>
                <div className="contents desktop:col-start-3 desktop:row-start-1 desktop:flex desktop:items-center desktop:justify-end desktop:gap-x-3">
                  <span className="col-start-2 row-start-2 w-full min-w-0 self-center text-right text-base-content/80 desktop:col-auto desktop:row-auto desktop:w-auto desktop:whitespace-nowrap">
                    {t("system.models.applyCount", {
                      count: applicableEntries.length,
                    })}
                    {hiddenApplyCount > 0
                      ? ` · ${t("system.models.searchHiddenApplyCount", { count: hiddenApplyCount })}`
                      : ""}
                  </span>
                  <div
                    className="col-start-2 row-start-1 flex shrink-0 items-center justify-self-end gap-x-2 desktop:col-auto desktop:row-auto"
                    title={t("system.models.showDeprecated")}
                  >
                    <Switch
                      checked={showDeprecated}
                      onCheckedChange={setShowDeprecated}
                      aria-label={t("system.models.showDeprecated")}
                      disabled={dialogState === "applying"}
                    />
                    <span aria-hidden="true">
                      <span className="desktop:hidden">
                        {t("system.models.showDeprecatedCompact")}
                      </span>
                      <span className="hidden desktop:inline">
                        {t("system.models.showDeprecated")}
                      </span>
                    </span>
                  </div>
                </div>
              </div>

              <section
                ref={setScrollElement}
                className="mt-2 min-h-0 flex-1 overflow-y-auto border-b border-base-300/70"
                aria-label={t("system.models.reviewRows")}
              >
                {visibleGroups.length === 0 ? (
                  <div className="px-2 py-10 text-center text-sm text-base-content/65">
                    {t("system.models.noMatches")}
                  </div>
                ) : (
                  <div
                    className="relative w-full"
                    style={{ height: `${virtualizer.getTotalSize()}px` }}
                  >
                    {virtualRows.map((virtualRow) => {
                      const group = visibleGroups[virtualRow.index];
                      if (!group) return null;
                      const candidate = resolveModelCandidate(group, quoteChoices[group.model]);
                      const local = pricesByModel.get(group.model);
                      return (
                        <div
                          key={virtualRow.key}
                          data-index={virtualRow.index}
                          ref={virtualizer.measureElement}
                          className="absolute left-0 top-0 w-full"
                          style={{
                            transform: `translateY(${virtualRow.start}px)`,
                          }}
                        >
                          <ModelCandidateRow
                            group={group}
                            local={local}
                            candidate={candidate}
                            rememberedProviderUnavailable={
                              quoteChoices[group.model] !== undefined && candidate === undefined
                            }
                            selected={Boolean(
                              candidate &&
                                selectionByKey[modelProviderKey(group.model, candidate.providerId)],
                            )}
                            isNew={currentReviewNewModels.has(group.model)}
                            applying={dialogState === "applying"}
                            scrollElement={scrollElement}
                            onViewed={acknowledgeViewed}
                            onSelectionChange={saveSelection}
                            onQuoteProviderChange={saveQuoteProvider}
                            t={t}
                          />
                        </div>
                      );
                    })}
                  </div>
                )}
              </section>

              <DialogFooter
                className="mt-3 shrink-0 items-center desktop:justify-between"
                data-testid="models-sync-dialog-footer"
              >
                <span className="text-xs text-base-content/65">
                  {t("system.models.sourceLabel")}
                </span>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    disabled={dialogState === "applying"}
                    onClick={() => handleOpenChange(false)}
                  >
                    {t("system.models.cancel")}
                  </Button>
                  <Button
                    type="button"
                    disabled={applicableEntries.length === 0 || dialogState === "applying"}
                    onClick={() => void applySelected()}
                  >
                    {dialogState === "applying"
                      ? t("system.models.syncing")
                      : t("system.models.syncSelected", {
                          count: applicableEntries.length,
                        })}
                  </Button>
                </div>
              </DialogFooter>
            </>
          ) : null}

          {dialogState === "applied" ? (
            <div className="space-y-4 py-6">
              <Alert role="status">{t("system.models.syncSuccess", { count: resultCount })}</Alert>
              <div className="flex justify-end">
                <Button type="button" onClick={() => handleOpenChange(false)}>
                  {t("system.models.done")}
                </Button>
              </div>
            </div>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
