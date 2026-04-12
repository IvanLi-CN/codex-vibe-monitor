import { useEffect, useMemo } from "react";
import { AppIcon } from "./AppIcon";
import { ConcurrencyLimitSlider } from "./ConcurrencyLimitSlider";
import type { ForwardProxyBindingNode } from "../lib/api";
import { apiConcurrencyLimitToSliderValue } from "../lib/concurrencyLimit";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { ForwardProxyRequestTrendChart } from "./ForwardProxyRequestTrendChart";
import { SelectField } from "./ui/select-field";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";
import { Switch } from "./ui/switch";

type GroupProxyOption = ForwardProxyBindingNode & {
  identityHint?: string;
  missing?: boolean;
};

interface UpstreamAccountGroupNoteDialogProps {
  open: boolean;
  container?: HTMLElement | null;
  groupName: string;
  note: string;
  concurrencyLimit?: number;
  busy?: boolean;
  error?: string | null;
  existing: boolean;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  availableProxyNodes?: ForwardProxyBindingNode[];
  onNoteChange: (value: string) => void;
  onConcurrencyLimitChange?: (value: number) => void;
  onBoundProxyKeysChange?: (value: string[]) => void;
  onNodeShuntEnabledChange?: (value: boolean) => void;
  onUpstream429RetryEnabledChange?: (value: boolean) => void;
  onUpstream429MaxRetriesChange?: (value: number) => void;
  onClose: () => void;
  onSave: () => void;
  title: string;
  existingDescription: string;
  draftDescription: string;
  noteLabel: string;
  notePlaceholder: string;
  concurrencyLimitLabel?: string;
  concurrencyLimitHint?: string;
  concurrencyLimitCurrentLabel?: string;
  concurrencyLimitUnlimitedLabel?: string;
  cancelLabel: string;
  saveLabel: string;
  closeLabel: string;
  existingBadgeLabel: string;
  draftBadgeLabel: string;
  nodeShuntLabel?: string;
  nodeShuntHint?: string;
  nodeShuntToggleLabel?: string;
  nodeShuntWarning?: string;
  upstream429RetryLabel?: string;
  upstream429RetryHint?: string;
  upstream429RetryToggleLabel?: string;
  upstream429RetryCountLabel?: string;
  upstream429RetryCountOptions?: Array<{
    value: number;
    label: string;
  }>;
  proxyBindingsLabel?: string;
  proxyBindingsHint?: string;
  proxyBindingsAutomaticLabel?: string;
  proxyBindingsLoadingLabel?: string;
  proxyBindingsEmptyLabel?: string;
  proxyBindingsMissingLabel?: string;
  proxyBindingsUnavailableLabel?: string;
  proxyBindingsCatalogKind?: "ready-empty" | "ready-with-data" | "loading" | "missing" | "deferred";
  proxyBindingsCatalogFreshness?: "fresh" | "stale" | "missing" | "deferred";
  proxyBindingsChartLabel?: string;
  proxyBindingsChartSuccessLabel?: string;
  proxyBindingsChartFailureLabel?: string;
  proxyBindingsChartEmptyLabel?: string;
  proxyBindingsChartTotalLabel?: string;
  proxyBindingsChartAriaLabel?: string;
  proxyBindingsChartInteractionHint?: string;
  proxyBindingsChartLocaleTag?: string;
}

function normalizeBoundProxyKeys(values?: string[]): string[] {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(
      values.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
}

function canonicalizeBoundProxyKeys(
  values: string[],
  availableProxyNodes?: ForwardProxyBindingNode[],
): string[] {
  const keyAliases = new Map<string, string>();
  for (const node of Array.isArray(availableProxyNodes)
    ? availableProxyNodes
    : []) {
    keyAliases.set(node.key, node.key);
    for (const alias of normalizeBoundProxyKeys(node.aliasKeys)) {
      keyAliases.set(alias, node.key);
    }
  }
  return Array.from(
    new Set(values.map((value) => keyAliases.get(value) ?? value)),
  );
}

function sameOrderedKeys(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function toggleBoundProxyKey(keys: string[], target: string): string[] {
  if (keys.includes(target)) {
    return keys.filter((key) => key !== target);
  }
  return [...keys, target];
}

function buildMissingProxyOption(key: string): GroupProxyOption {
  const isDirect = key === "__direct__";
  return {
    key,
    source: isDirect ? "direct" : "missing",
    displayName: isDirect ? "Direct" : key,
    protocolLabel: isDirect ? "DIRECT" : "UNKNOWN",
    penalized: false,
    selectable: isDirect,
    last24h: [],
    missing: !isDirect,
  };
}

function buildProxyIdentityHint(key: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < key.length; index += 1) {
    hash ^= key.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `ID ${(hash >>> 0).toString(36).toUpperCase().slice(-6).padStart(6, "0")}`;
}

function shouldShowProxyIdentityHint(
  node: GroupProxyOption,
  duplicateDisplayName: boolean,
): boolean {
  if (node.missing || duplicateDisplayName) {
    return true;
  }
  return node.displayName.trim().length > 28;
}

function sumProxyTraffic(node: ForwardProxyBindingNode) {
  const buckets = Array.isArray(node.last24h) ? node.last24h : [];
  return buckets.reduce(
    (acc, bucket) => {
      acc.success += bucket.successCount;
      acc.failure += bucket.failureCount;
      return acc;
    },
    { success: 0, failure: 0 },
  );
}

function normalizeUpstream429MaxRetries(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

function ProxyOptionTrafficChart({
  node,
  scaleMax,
  label,
  successLabel,
  failureLabel,
  emptyLabel,
  totalLabel,
  ariaLabel,
  interactionHint,
  localeTag,
}: {
  node: ForwardProxyBindingNode;
  scaleMax: number;
  label: string;
  successLabel: string;
  failureLabel: string;
  emptyLabel: string;
  totalLabel: string;
  ariaLabel: string;
  interactionHint: string;
  localeTag: string;
}) {
  const buckets = useMemo(
    () => (Array.isArray(node.last24h) ? node.last24h : []),
    [node.last24h],
  );
  const totals = useMemo(() => sumProxyTraffic(node), [node]);
  const windowBadgeLabel = useMemo(() => {
    if (/24/.test(label)) return "24H";
    return label;
  }, [label]);

  return (
    <div className="flex w-full flex-col justify-center gap-0.5 sm:min-w-[15.5rem] sm:max-w-[15.5rem] sm:self-center">
      <div className="flex h-4 items-center justify-between gap-2">
        <span
          className="inline-flex h-4 min-w-[2.25rem] shrink-0 items-center justify-center rounded-md border border-base-300/80 bg-base-100/75 px-1.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-base-content/55 whitespace-nowrap"
          title={label}
          aria-label={label}
        >
          {windowBadgeLabel}
        </span>
        <div className="flex items-center gap-1.5 text-[10px] font-semibold leading-none tabular-nums">
          <span
            className="inline-flex items-center gap-1 text-success"
            aria-label={`${successLabel} ${totals.success}`}
            title={`${successLabel} ${totals.success}`}
          >
            <span className="h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
            <span>{totals.success}</span>
          </span>
          <span
            className="inline-flex items-center gap-1 text-error"
            aria-label={`${failureLabel} ${totals.failure}`}
            title={`${failureLabel} ${totals.failure}`}
          >
            <span className="h-1.5 w-1.5 rounded-full bg-error" aria-hidden />
            <span>{totals.failure}</span>
          </span>
        </div>
      </div>

      {buckets.length === 0 ? (
        <div className="mt-0.5 flex h-8 items-center justify-center rounded-xl border border-dashed border-base-300/80 bg-base-100/70 px-3 text-[11px] text-base-content/50">
          {emptyLabel}
        </div>
      ) : (
        <ForwardProxyRequestTrendChart
          buckets={buckets}
          scaleMax={scaleMax}
          localeTag={localeTag}
          tooltipLabels={{
            success: successLabel,
            failure: failureLabel,
            total: totalLabel,
          }}
          ariaLabel={`${node.displayName} ${ariaLabel}`}
          interactionHint={interactionHint}
          variant="dialog"
          className="mt-0.5"
          dataChartKind="proxy-binding-request-trend"
        />
      )}
    </div>
  );
}

export function UpstreamAccountGroupNoteDialog({
  open,
  container,
  groupName,
  note,
  concurrencyLimit = apiConcurrencyLimitToSliderValue(0),
  busy = false,
  error,
  existing,
  boundProxyKeys,
  nodeShuntEnabled = false,
  upstream429RetryEnabled = false,
  upstream429MaxRetries = 0,
  availableProxyNodes,
  onNoteChange,
  onConcurrencyLimitChange = () => undefined,
  onBoundProxyKeysChange,
  onNodeShuntEnabledChange,
  onUpstream429RetryEnabledChange,
  onUpstream429MaxRetriesChange,
  onClose,
  onSave,
  title,
  existingDescription,
  draftDescription,
  noteLabel,
  notePlaceholder,
  concurrencyLimitLabel,
  concurrencyLimitHint,
  concurrencyLimitCurrentLabel,
  concurrencyLimitUnlimitedLabel,
  cancelLabel,
  saveLabel,
  closeLabel,
  existingBadgeLabel,
  draftBadgeLabel,
  nodeShuntLabel,
  nodeShuntHint,
  nodeShuntToggleLabel,
  nodeShuntWarning,
  upstream429RetryLabel,
  upstream429RetryHint,
  upstream429RetryToggleLabel,
  upstream429RetryCountLabel,
  upstream429RetryCountOptions,
  proxyBindingsLabel,
  proxyBindingsHint,
  proxyBindingsAutomaticLabel,
  proxyBindingsLoadingLabel,
  proxyBindingsEmptyLabel,
  proxyBindingsMissingLabel,
  proxyBindingsUnavailableLabel,
  proxyBindingsCatalogKind,
  proxyBindingsCatalogFreshness,
  proxyBindingsChartLabel,
  proxyBindingsChartSuccessLabel,
  proxyBindingsChartFailureLabel,
  proxyBindingsChartEmptyLabel,
  proxyBindingsChartTotalLabel,
  proxyBindingsChartAriaLabel,
  proxyBindingsChartInteractionHint,
  proxyBindingsChartLocaleTag,
}: UpstreamAccountGroupNoteDialogProps) {
  const normalizedBoundProxyKeys = normalizeBoundProxyKeys(boundProxyKeys);
  const normalizedNodeShuntEnabled = nodeShuntEnabled === true;
  const normalizedUpstream429RetryEnabled = upstream429RetryEnabled === true;
  const normalizedUpstream429MaxRetries = normalizeUpstream429MaxRetries(
    upstream429MaxRetries,
  );
  const retryCountOptions = useMemo(() => {
    const fallback = [1, 2, 3, 4, 5].map((value) => ({
      value,
      label: value === 1 ? "1 retry" : `${value} retries`,
    }));
    if (
      !Array.isArray(upstream429RetryCountOptions) ||
      upstream429RetryCountOptions.length === 0
    ) {
      return fallback;
    }
    const options = upstream429RetryCountOptions
      .map((option) => ({
        value: normalizeUpstream429MaxRetries(option.value),
        label: option.label,
      }))
      .filter((option) => option.value > 0 && option.label.trim().length > 0);
    return options.length > 0 ? options : fallback;
  }, [upstream429RetryCountOptions]);
  const selectedRetryCount = useMemo(() => {
    if (
      retryCountOptions.some(
        (option) => option.value === normalizedUpstream429MaxRetries,
      )
    ) {
      return normalizedUpstream429MaxRetries;
    }
    return retryCountOptions[0]?.value ?? 1;
  }, [normalizedUpstream429MaxRetries, retryCountOptions]);
  const canonicalBoundProxyKeys = useMemo(
    () =>
      canonicalizeBoundProxyKeys(normalizedBoundProxyKeys, availableProxyNodes),
    [availableProxyNodes, normalizedBoundProxyKeys],
  );
  useEffect(() => {
    if (!open || !onBoundProxyKeysChange) return;
    if (sameOrderedKeys(canonicalBoundProxyKeys, normalizedBoundProxyKeys))
      return;
    onBoundProxyKeysChange(canonicalBoundProxyKeys);
  }, [
    canonicalBoundProxyKeys,
    normalizedBoundProxyKeys,
    onBoundProxyKeysChange,
    open,
  ]);
  const proxyOptions = useMemo(() => {
    const available = Array.isArray(availableProxyNodes)
      ? availableProxyNodes
          .filter(
            (node) =>
              node.source !== "missing" ||
              canonicalBoundProxyKeys.includes(node.key),
          )
          .map((node) => ({
            ...node,
            last24h: Array.isArray(node.last24h) ? node.last24h : [],
          }))
      : [];
    const availableByKey = new Map(available.map((node) => [node.key, node]));
    const options: GroupProxyOption[] = [...available];
    for (const key of canonicalBoundProxyKeys) {
      if (!availableByKey.has(key)) {
        options.push(buildMissingProxyOption(key));
      }
    }
    const displayNameCounts = new Map<string, number>();
    for (const node of options) {
      const normalizedDisplayName = node.displayName.trim();
      displayNameCounts.set(
        normalizedDisplayName,
        (displayNameCounts.get(normalizedDisplayName) ?? 0) + 1,
      );
    }
    return options.map((node) => {
      const duplicateDisplayName =
        (displayNameCounts.get(node.displayName.trim()) ?? 0) > 1;
      return {
        ...node,
        identityHint: shouldShowProxyIdentityHint(node, duplicateDisplayName)
          ? buildProxyIdentityHint(node.key)
          : undefined,
      };
    });
  }, [availableProxyNodes, canonicalBoundProxyKeys, proxyBindingsMissingLabel]);
  const proxyChartScaleMax = useMemo(
    () =>
      Math.max(
        ...proxyOptions.flatMap((node) =>
          (Array.isArray(node.last24h) ? node.last24h : []).map(
            (bucket) => bucket.successCount + bucket.failureCount,
          ),
        ),
        0,
      ),
    [proxyOptions],
  );
  const showProxyBindings =
    Boolean(onBoundProxyKeysChange) ||
    proxyOptions.length > 0 ||
    canonicalBoundProxyKeys.length > 0;
  const proxyBindingsLoading =
    proxyBindingsCatalogKind === "loading" ||
    proxyBindingsCatalogKind === "missing" ||
    (proxyBindingsCatalogFreshness === "stale" && proxyOptions.length === 0);
  const showProxyBindingsLoadingNotice = proxyBindingsLoading;
  const showProxyBindingsEmptyState =
    !proxyBindingsLoading && proxyOptions.length === 0;
  const showProxyBindingOptions = proxyOptions.length > 0;
  const hasSelectableBoundProxySelection =
    canonicalBoundProxyKeys.length === 0 ||
    canonicalBoundProxyKeys.some((key) =>
      proxyOptions.some((node) => node.key === key && node.selectable),
    );
  const blockingBindingSelection =
    showProxyBindings &&
    !normalizedNodeShuntEnabled &&
    canonicalBoundProxyKeys.length > 0 &&
    !hasSelectableBoundProxySelection;
  const blockingNodeShuntSelection =
    normalizedNodeShuntEnabled && canonicalBoundProxyKeys.length === 0;
  const showNodeShuntSection =
    Boolean(onNodeShuntEnabledChange) ||
    Boolean(nodeShuntLabel) ||
    Boolean(nodeShuntHint);
  const showUpstream429RetrySection =
    Boolean(onUpstream429RetryEnabledChange) ||
    Boolean(onUpstream429MaxRetriesChange) ||
    Boolean(upstream429RetryLabel) ||
    Boolean(upstream429RetryHint);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) =>
        !busy ? (nextOpen ? undefined : onClose()) : undefined
      }
    >
      <DialogContent
        container={container}
        className="flex max-h-[calc(100dvh-2rem)] flex-col overflow-hidden border-base-300 bg-base-100 p-0 sm:w-[min(44rem,calc(100vw-4rem))] sm:max-w-[44rem] sm:max-h-[calc(100dvh-4rem)]"
      >
        <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
          <DialogHeader className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <DialogTitle>{title}</DialogTitle>
              <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2.5 py-1 text-xs font-semibold text-base-content/70">
                {existing ? existingBadgeLabel : draftBadgeLabel}
              </span>
            </div>
            <DialogDescription>
              {existing ? existingDescription : draftDescription}
            </DialogDescription>
            <p className="text-sm font-semibold text-base-content">
              {groupName}
            </p>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <div className="space-y-4">
            {error ? (
              <div className="flex items-start gap-3 rounded-2xl border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
                <AppIcon
                  name="alert-circle-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div>{error}</div>
              </div>
            ) : null}

            <label className="field">
              <span className="field-label">{noteLabel}</span>
              <textarea
                className="min-h-32 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                value={note}
                placeholder={notePlaceholder}
                disabled={busy}
                onChange={(event) => onNoteChange(event.target.value)}
              />
            </label>

            <ConcurrencyLimitSlider
              value={concurrencyLimit}
              disabled={busy}
              title={concurrencyLimitLabel ?? "Concurrency limit"}
              description={
                concurrencyLimitHint ??
                "Use 1-30 to cap fresh assignments for this group. The last step means unlimited."
              }
              currentLabel={concurrencyLimitCurrentLabel ?? "Current"}
              unlimitedLabel={concurrencyLimitUnlimitedLabel ?? "Unlimited"}
              onChange={onConcurrencyLimitChange}
            />

            {showUpstream429RetrySection ? (
              <section className="flex flex-col gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                <div className="space-y-1">
                  <h3 className="text-sm font-semibold text-base-content">
                    {upstream429RetryLabel ?? "Upstream 429 retry"}
                  </h3>
                  <p className="text-xs leading-5 text-base-content/68">
                    {upstream429RetryHint ??
                      "Allow this group to keep the same account and retry after upstream 429 responses."}
                  </p>
                </div>

                <div className="flex items-center justify-between gap-3 rounded-xl border border-base-300/80 bg-base-100/75 px-3 py-3">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-base-content">
                      {upstream429RetryToggleLabel ??
                        "Retry the same account after upstream 429"}
                    </p>
                  </div>
                  <Switch
                    checked={normalizedUpstream429RetryEnabled}
                    onCheckedChange={(checked) =>
                      onUpstream429RetryEnabledChange?.(checked)
                    }
                    disabled={busy || !onUpstream429RetryEnabledChange}
                    aria-label={
                      upstream429RetryToggleLabel ??
                      "Retry the same account after upstream 429"
                    }
                  />
                </div>

                <SelectField
                  label={upstream429RetryCountLabel ?? "Retry count"}
                  value={String(selectedRetryCount)}
                  disabled={
                    busy ||
                    !normalizedUpstream429RetryEnabled ||
                    !onUpstream429MaxRetriesChange
                  }
                  options={retryCountOptions.map((option) => ({
                    value: String(option.value),
                    label: option.label,
                  }))}
                  onValueChange={(value) =>
                    onUpstream429MaxRetriesChange?.(
                      normalizeUpstream429MaxRetries(Number(value)),
                    )
                  }
                />
              </section>
            ) : null}

            {showProxyBindings ? (
              <section className="flex min-h-0 flex-col gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                <div className="space-y-1">
                  <h3 className="text-sm font-semibold text-base-content">
                    {proxyBindingsLabel ?? "Bound proxy nodes"}
                  </h3>
                  <p className="text-xs leading-5 text-base-content/68">
                    {proxyBindingsHint ??
                      "Leave empty to use automatic routing."}
                  </p>
                </div>

                {canonicalBoundProxyKeys.length === 0 ? (
                  <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
                    {proxyBindingsAutomaticLabel ??
                      "No nodes bound. This group uses automatic routing."}
                  </div>
                ) : null}

                {blockingBindingSelection ? (
                  <div className="rounded-xl border border-warning/35 bg-warning/10 px-3 py-2 text-xs text-warning">
                    Select at least one available proxy node or clear bindings
                    before saving.
                  </div>
                ) : null}

                {showProxyBindingsLoadingNotice ? (
                  <div
                    className="flex items-center gap-2 rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65"
                    data-testid="proxy-binding-options-loading"
                  >
                    <AppIcon
                      name="loading"
                      className="h-4 w-4 animate-spin"
                      aria-hidden
                    />
                    <span>
                      {proxyBindingsLoadingLabel ?? "Loading proxy nodes…"}
                    </span>
                  </div>
                ) : null}

                {showProxyBindingsEmptyState ? (
                  <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
                    {proxyBindingsEmptyLabel ?? "No proxy nodes available."}
                  </div>
                ) : null}

                {showProxyBindingOptions ? (
                  <div
                    className="min-h-0 max-h-[min(26rem,45dvh)] overflow-y-auto rounded-xl pr-1"
                    data-testid="proxy-binding-options-scroll-region"
                  >
                    <div className="grid gap-2">
                      {proxyOptions.map((node) => {
                        const selected = canonicalBoundProxyKeys.includes(
                          node.key,
                        );
                        const disabled =
                          busy || (!selected && !node.selectable);
                        const badgeLabel = node.missing
                          ? (proxyBindingsMissingLabel ?? "Missing")
                          : !node.selectable
                            ? (proxyBindingsUnavailableLabel ?? "Unavailable")
                            : null;
                        return (
                          <button
                            key={node.key}
                            type="button"
                            disabled={disabled}
                            onClick={() => {
                              if (!onBoundProxyKeysChange) return;
                              onBoundProxyKeysChange(
                                toggleBoundProxyKey(
                                  canonicalBoundProxyKeys,
                                  node.key,
                                ),
                              );
                            }}
                            className={cn(
                              "grid gap-2 rounded-xl border px-3 py-2 text-left transition-colors sm:grid-cols-[minmax(0,1fr)_15.5rem] sm:items-center sm:gap-3",
                              selected
                                ? "border-primary/45 bg-primary/10"
                                : "border-base-300/80 bg-base-100/75",
                              disabled
                                ? "cursor-not-allowed opacity-60"
                                : "hover:border-primary/40",
                            )}
                          >
                            <div className="flex min-w-0 flex-1 items-center gap-3">
                              <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-base-300/80 bg-base-100">
                                {selected ? (
                                  <AppIcon
                                    name="check"
                                    className="h-3.5 w-3.5 text-primary"
                                    aria-hidden
                                  />
                                ) : null}
                              </div>
                              <div className="min-w-0 flex-1">
                                <div className="min-w-0">
                                  <span
                                    className="block min-w-0 truncate text-sm font-medium text-base-content"
                                    title={node.displayName}
                                  >
                                    {node.displayName}
                                  </span>
                                </div>
                                <div className="mt-1 flex flex-wrap items-center gap-2">
                                  <span className="shrink-0 rounded-md border border-base-300/80 bg-base-200/65 px-1.5 py-0.5 text-[10px] font-mono font-semibold uppercase tracking-[0.08em] text-base-content/68">
                                    {node.protocolLabel}
                                  </span>
                                  {node.identityHint ? (
                                    <span
                                      className="shrink-0 rounded-md border border-base-300/80 bg-base-100/80 px-1.5 py-0.5 text-[10px] font-mono font-semibold tracking-[0.08em] text-base-content/55"
                                      title={node.identityHint}
                                    >
                                      {node.identityHint}
                                    </span>
                                  ) : null}
                                  {badgeLabel ? (
                                    <span className="shrink-0 rounded-full border border-base-300/80 bg-base-200/80 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/65">
                                      {badgeLabel}
                                    </span>
                                  ) : null}
                                  {node.penalized ? (
                                    <span className="shrink-0 rounded-full border border-warning/35 bg-warning/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-warning">
                                      Penalized
                                    </span>
                                  ) : null}
                                </div>
                              </div>
                            </div>
                            <ProxyOptionTrafficChart
                              node={node}
                              scaleMax={proxyChartScaleMax}
                              label={
                                proxyBindingsChartLabel ?? "24h request trend"
                              }
                              successLabel={
                                proxyBindingsChartSuccessLabel ?? "ok"
                              }
                              failureLabel={
                                proxyBindingsChartFailureLabel ?? "fail"
                              }
                              emptyLabel={
                                proxyBindingsChartEmptyLabel ?? "No 24h data"
                              }
                              totalLabel={
                                proxyBindingsChartTotalLabel ?? "total"
                              }
                              ariaLabel={
                                proxyBindingsChartAriaLabel ??
                                "Last 24h request volume chart"
                              }
                              interactionHint={
                                proxyBindingsChartInteractionHint ??
                                "Hover or tap for details. Focus the chart and use arrow keys to switch points."
                              }
                              localeTag={proxyBindingsChartLocaleTag ?? "en-US"}
                            />
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ) : null}
              </section>
            ) : null}

            {showNodeShuntSection ? (
              <section className="flex flex-col gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                <div className="space-y-1">
                  <h3 className="text-sm font-semibold text-base-content">
                    {nodeShuntLabel ?? "Node shunt strategy"}
                  </h3>
                  <p className="text-xs leading-5 text-base-content/68">
                    {nodeShuntHint ??
                      "Each selected node becomes an exclusive slot. If a group selects 3 nodes, the group can provide 3 upstream accounts at the same time."}
                  </p>
                </div>

                <div className="flex items-center justify-between gap-3 rounded-xl border border-base-300/80 bg-base-100/75 px-3 py-3">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-base-content">
                      {nodeShuntToggleLabel ?? "Enable node shunt strategy"}
                    </p>
                  </div>
                  <Switch
                    checked={normalizedNodeShuntEnabled}
                    onCheckedChange={(checked) =>
                      onNodeShuntEnabledChange?.(checked)
                    }
                    disabled={busy || !onNodeShuntEnabledChange}
                    aria-label={
                      nodeShuntToggleLabel ?? "Enable node shunt strategy"
                    }
                  />
                </div>

                {blockingNodeShuntSelection ? (
                  <div className="rounded-xl border border-warning/35 bg-warning/10 px-3 py-2 text-xs text-warning">
                    {nodeShuntWarning ??
                      "Enable this strategy only after binding at least one node (including Direct)."}
                  </div>
                ) : null}
              </section>
            ) : null}
          </div>
        </div>

        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button
            type="button"
            variant="ghost"
            onClick={onClose}
            disabled={busy}
          >
            {cancelLabel}
          </Button>
          <Button
            type="button"
            onClick={onSave}
            disabled={
              busy || blockingBindingSelection || blockingNodeShuntSelection
            }
          >
            {busy ? (
              <AppIcon
                name="loading"
                className="mr-2 h-4 w-4 animate-spin"
                aria-hidden
              />
            ) : null}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
