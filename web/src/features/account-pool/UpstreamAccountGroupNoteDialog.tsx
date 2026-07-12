import type { ReactNode } from "react";
import { useEffect, useId, useMemo, useState } from "react";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { Popover, PopoverContent, PopoverTrigger } from "../../components/ui/popover";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { Switch } from "../../components/ui/switch";
import type { ForwardProxyBindingNode } from "../../lib/api";
import { apiConcurrencyLimitToSliderValue } from "../../lib/concurrencyLimit";
import { cn } from "../../lib/utils";
import { ForwardProxyBindingSelector } from "../forward-proxy/ForwardProxyBindingSelector";
import {
  canonicalizeForwardProxyBindingKeys,
  hasSelectableForwardProxyBindingSelection,
  normalizeForwardProxyBindingKeys,
  resolveForwardProxyBindingOptions,
} from "../forward-proxy/forwardProxyBindingSelectorUtils";
import { AppIcon } from "../shared/AppIcon";
import { ConcurrencyLimitSlider } from "./ConcurrencyLimitSlider";
import { PolicyInlineOptionGroup } from "./PolicyInlineOptionGroup";

interface UpstreamAccountGroupNoteDialogProps {
  open: boolean;
  container?: HTMLElement | null;
  groupName: string;
  note: string;
  accountCount?: number;
  concurrencyLimit?: number;
  busy?: boolean;
  deleting?: boolean;
  saveDisabled?: boolean;
  error?: string | null;
  existing: boolean;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  availableProxyNodes?: ForwardProxyBindingNode[];
  onNoteChange: (value: string) => void;
  onConcurrencyLimitChange?: (value: number) => void;
  onBoundProxyKeysChange?: (value: string[]) => void;
  onNodeShuntEnabledChange?: (value: boolean) => void;
  onSingleAccountRotationEnabledChange?: (value: boolean) => void;
  onUpstream429RetryEnabledChange?: (value: boolean) => void;
  onUpstream429MaxRetriesChange?: (value: number) => void;
  onRoutingPolicyEdit?: () => void;
  routingPolicyEditor?: ReactNode;
  onClose: () => void;
  onSave: () => void;
  onDelete?: () => void;
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
  deleteLabel?: string;
  deleteDisabledHint?: string;
  closeLabel: string;
  existingBadgeLabel: string;
  draftBadgeLabel: string;
  infoTabLabel?: string;
  routingTabLabel?: string;
  proxyTabLabel?: string;
  accountCountLabel?: string;
  nodeShuntLabel?: string;
  nodeShuntHint?: string;
  nodeShuntToggleLabel?: string;
  nodeShuntWarning?: string;
  singleAccountRotationLabel?: string;
  singleAccountRotationHint?: string;
  singleAccountRotationToggleLabel?: string;
  upstream429RetryLabel?: string;
  upstream429RetryHint?: string;
  upstream429RetryToggleLabel?: string;
  upstream429RetryCountLabel?: string;
  upstream429RetryCountOptions?: Array<{
    value: number;
    label: string;
  }>;
  routingPolicyLabel?: string;
  routingPolicyHint?: string;
  routingPolicyEditLabel?: string;
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

type GroupSettingsTab = "info" | "routing" | "proxy";

function sameOrderedKeys(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function normalizeUpstream429MaxRetries(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

export function UpstreamAccountGroupNoteDialog({
  open,
  container,
  groupName,
  note,
  accountCount = 0,
  concurrencyLimit = apiConcurrencyLimitToSliderValue(0),
  busy = false,
  deleting = false,
  saveDisabled = false,
  error,
  existing,
  boundProxyKeys,
  nodeShuntEnabled = false,
  singleAccountRotationEnabled = false,
  upstream429RetryEnabled = false,
  upstream429MaxRetries = 0,
  availableProxyNodes,
  onNoteChange,
  onConcurrencyLimitChange = () => undefined,
  onBoundProxyKeysChange,
  onNodeShuntEnabledChange,
  onSingleAccountRotationEnabledChange,
  onUpstream429RetryEnabledChange,
  onUpstream429MaxRetriesChange,
  onRoutingPolicyEdit,
  routingPolicyEditor,
  onClose,
  onSave,
  onDelete,
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
  deleteLabel,
  deleteDisabledHint,
  closeLabel,
  existingBadgeLabel,
  draftBadgeLabel,
  infoTabLabel,
  routingTabLabel,
  proxyTabLabel,
  accountCountLabel,
  nodeShuntLabel,
  nodeShuntHint,
  nodeShuntToggleLabel,
  nodeShuntWarning,
  singleAccountRotationLabel,
  singleAccountRotationHint,
  singleAccountRotationToggleLabel,
  upstream429RetryLabel,
  upstream429RetryHint,
  upstream429RetryToggleLabel,
  upstream429RetryCountLabel,
  routingPolicyLabel,
  routingPolicyHint,
  routingPolicyEditLabel,
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
  const normalizedBoundProxyKeys = normalizeForwardProxyBindingKeys(boundProxyKeys);
  const normalizedNodeShuntEnabled = nodeShuntEnabled === true;
  const normalizedSingleAccountRotationEnabled = singleAccountRotationEnabled === true;
  const normalizedUpstream429RetryEnabled = upstream429RetryEnabled === true;
  const normalizedUpstream429MaxRetries = normalizeUpstream429MaxRetries(upstream429MaxRetries);
  const selectedRetryCount = normalizedUpstream429RetryEnabled
    ? Math.max(1, normalizedUpstream429MaxRetries || 1)
    : 0;
  const retryCountOptions = useMemo(
    () =>
      [0, 1, 2, 3, 4, 5].map((value) => ({
        value,
        label: String(value),
      })),
    [],
  );
  const handleUpstream429RetryCountChange = (value: number) => {
    const normalizedValue = normalizeUpstream429MaxRetries(value);
    onUpstream429RetryEnabledChange?.(normalizedValue > 0);
    onUpstream429MaxRetriesChange?.(normalizedValue);
  };
  const canonicalBoundProxyKeys = useMemo(
    () => canonicalizeForwardProxyBindingKeys(normalizedBoundProxyKeys, availableProxyNodes),
    [availableProxyNodes, normalizedBoundProxyKeys],
  );
  useEffect(() => {
    if (!open || !onBoundProxyKeysChange) return;
    if (sameOrderedKeys(canonicalBoundProxyKeys, normalizedBoundProxyKeys)) return;
    onBoundProxyKeysChange(canonicalBoundProxyKeys);
  }, [canonicalBoundProxyKeys, normalizedBoundProxyKeys, onBoundProxyKeysChange, open]);
  const proxyOptions = useMemo(() => {
    return resolveForwardProxyBindingOptions(canonicalBoundProxyKeys, availableProxyNodes);
  }, [availableProxyNodes, canonicalBoundProxyKeys]);
  const showProxyBindings =
    Boolean(onBoundProxyKeysChange) ||
    proxyOptions.length > 0 ||
    canonicalBoundProxyKeys.length > 0;
  const hasSelectableBoundProxySelection = hasSelectableForwardProxyBindingSelection(
    canonicalBoundProxyKeys,
    proxyOptions,
  );
  const blockingBindingSelection =
    showProxyBindings &&
    !normalizedNodeShuntEnabled &&
    canonicalBoundProxyKeys.length > 0 &&
    !hasSelectableBoundProxySelection;
  const blockingNodeShuntSelection =
    normalizedNodeShuntEnabled && canonicalBoundProxyKeys.length === 0;
  const showDelete = existing && onDelete != null;
  const deleteBlockedByMembers = accountCount > 0;
  const deleteBusyDisabled = busy;
  const deleteBlockedPopoverEnabled =
    showDelete && deleteBlockedByMembers && !deleteBusyDisabled && Boolean(deleteDisabledHint);
  const [deleteBlockedPopoverOpen, setDeleteBlockedPopoverOpen] = useState(false);
  const showNodeShuntSection =
    Boolean(onNodeShuntEnabledChange) || Boolean(nodeShuntLabel) || Boolean(nodeShuntHint);
  const showSingleAccountRotationSection =
    Boolean(onSingleAccountRotationEnabledChange) ||
    Boolean(singleAccountRotationLabel) ||
    Boolean(singleAccountRotationHint);
  const showUpstream429RetrySection =
    !routingPolicyEditor &&
    (Boolean(onUpstream429RetryEnabledChange) ||
      Boolean(onUpstream429MaxRetriesChange) ||
      Boolean(upstream429RetryLabel) ||
      Boolean(upstream429RetryHint));
  const tabsBaseId = useId();
  const [activeTab, setActiveTab] = useState<GroupSettingsTab>("info");
  const tabIds = {
    info: {
      tab: `${tabsBaseId}-info-tab`,
      panel: `${tabsBaseId}-info-panel`,
    },
    routing: {
      tab: `${tabsBaseId}-routing-tab`,
      panel: `${tabsBaseId}-routing-panel`,
    },
    proxy: {
      tab: `${tabsBaseId}-proxy-tab`,
      panel: `${tabsBaseId}-proxy-panel`,
    },
  } as const;

  useEffect(() => {
    if (!open || !deleteBlockedPopoverEnabled) {
      setDeleteBlockedPopoverOpen(false);
    }
  }, [deleteBlockedPopoverEnabled, open]);

  useEffect(() => {
    if (!open) {
      setActiveTab("info");
    }
  }, [open]);

  const handleDeleteClick = () => {
    if (deleteBlockedPopoverEnabled) {
      setDeleteBlockedPopoverOpen((current) => !current);
      return;
    }
    onDelete?.();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}
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
            <p className="text-sm font-semibold text-base-content">{groupName}</p>
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

            <SegmentedControl
              role="tablist"
              aria-label={title}
              className="w-full flex-wrap justify-start"
            >
              <SegmentedControlItem
                id={tabIds.info.tab}
                role="tab"
                active={activeTab === "info"}
                aria-selected={activeTab === "info"}
                aria-controls={tabIds.info.panel}
                onClick={() => setActiveTab("info")}
              >
                {infoTabLabel ?? "Group info"}
              </SegmentedControlItem>
              <SegmentedControlItem
                id={tabIds.routing.tab}
                role="tab"
                active={activeTab === "routing"}
                aria-selected={activeTab === "routing"}
                aria-controls={tabIds.routing.panel}
                onClick={() => setActiveTab("routing")}
              >
                {routingTabLabel ?? "Routing settings"}
              </SegmentedControlItem>
              <SegmentedControlItem
                id={tabIds.proxy.tab}
                role="tab"
                active={activeTab === "proxy"}
                aria-selected={activeTab === "proxy"}
                aria-controls={tabIds.proxy.panel}
                onClick={() => setActiveTab("proxy")}
              >
                {proxyTabLabel ?? "Proxy nodes"}
              </SegmentedControlItem>
            </SegmentedControl>

            <div
              id={tabIds.info.panel}
              role="tabpanel"
              aria-labelledby={tabIds.info.tab}
              hidden={activeTab !== "info"}
              className="space-y-4"
            >
              <div className="grid gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4 sm:grid-cols-2">
                <div>
                  <p className="text-xs font-semibold text-base-content/55">
                    {accountCountLabel ?? "Accounts"}
                  </p>
                  <p className="mt-1 text-sm font-semibold text-base-content">{accountCount}</p>
                </div>
                <div>
                  <p className="text-xs font-semibold text-base-content/55">
                    {existing ? existingBadgeLabel : draftBadgeLabel}
                  </p>
                  <p className="mt-1 break-all text-sm font-semibold text-base-content">
                    {groupName}
                  </p>
                </div>
              </div>
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
            </div>

            <div
              id={tabIds.routing.panel}
              role="tabpanel"
              aria-labelledby={tabIds.routing.tab}
              hidden={activeTab !== "routing"}
              className="space-y-4"
            >
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
              {showSingleAccountRotationSection ? (
                <section className="flex flex-col gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                  <div className="space-y-1">
                    <h3 className="text-sm font-semibold text-base-content">
                      {singleAccountRotationLabel ?? "Single-account rotation"}
                    </h3>
                    <p className="text-xs leading-5 text-base-content/68">
                      {singleAccountRotationHint ??
                        "Keep each conversation on its current account, then move only that conversation to the next candidate after the account finally returns 429."}
                    </p>
                  </div>

                  <div className="flex items-center justify-between gap-3 rounded-xl border border-base-300/80 bg-base-100/75 px-3 py-3">
                    <div className="min-w-0">
                      <p className="text-sm font-medium text-base-content">
                        {singleAccountRotationToggleLabel ?? "Bind conversations until final 429"}
                      </p>
                    </div>
                    <Switch
                      checked={normalizedSingleAccountRotationEnabled}
                      onCheckedChange={(checked) => onSingleAccountRotationEnabledChange?.(checked)}
                      disabled={busy || !onSingleAccountRotationEnabledChange}
                      aria-label={
                        singleAccountRotationToggleLabel ?? "Bind conversations until final 429"
                      }
                    />
                  </div>
                </section>
              ) : null}
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

                  <PolicyInlineOptionGroup<number>
                    ariaLabel={
                      upstream429RetryLabel ??
                      upstream429RetryToggleLabel ??
                      upstream429RetryCountLabel ??
                      "Upstream 429 retry"
                    }
                    value={selectedRetryCount}
                    disabled={
                      busy || (!onUpstream429RetryEnabledChange && !onUpstream429MaxRetriesChange)
                    }
                    options={retryCountOptions}
                    onChange={handleUpstream429RetryCountChange}
                  />
                </section>
              ) : null}
              {routingPolicyEditor ? (
                <section className="rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                  <div className="mb-4 space-y-1">
                    <h3 className="text-sm font-semibold text-base-content">
                      {routingPolicyLabel ?? "Routing policy"}
                    </h3>
                    <p className="text-xs leading-5 text-base-content/68">
                      {routingPolicyHint ??
                        "Customize priority, FAST mode, block-new-conversations, cut-in/cut-out, concurrency, and upstream 429 retry for this group."}
                    </p>
                  </div>
                  {routingPolicyEditor}
                </section>
              ) : null}
              {onRoutingPolicyEdit ? (
                <section className="flex items-center justify-between gap-4 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                  <div className="min-w-0 space-y-1">
                    <h3 className="text-sm font-semibold text-base-content">
                      {routingPolicyLabel ?? "Routing policy"}
                    </h3>
                    <p className="text-xs leading-5 text-base-content/68">
                      {routingPolicyHint ??
                        "Customize priority, FAST mode, block-new-conversations, cut-in/cut-out, concurrency, and upstream 429 retry for this group."}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    disabled={busy}
                    onClick={onRoutingPolicyEdit}
                  >
                    {routingPolicyEditLabel ?? "Edit policy"}
                  </Button>
                </section>
              ) : null}
            </div>

            <div
              id={tabIds.proxy.panel}
              role="tabpanel"
              aria-labelledby={tabIds.proxy.tab}
              hidden={activeTab !== "proxy"}
              className="space-y-4"
            >
              {showProxyBindings ? (
                <section className="flex min-h-0 flex-col gap-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
                  <div className="space-y-1">
                    <h3 className="text-sm font-semibold text-base-content">
                      {proxyBindingsLabel ?? "Bound proxy nodes"}
                    </h3>
                    <p className="text-xs leading-5 text-base-content/68">
                      {proxyBindingsHint ?? "Leave empty to use automatic routing."}
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
                      Select at least one available proxy node or clear bindings before saving.
                    </div>
                  ) : null}

                  <ForwardProxyBindingSelector
                    selectedKeys={canonicalBoundProxyKeys}
                    availableProxyNodes={availableProxyNodes}
                    disabled={busy}
                    catalogKind={proxyBindingsCatalogKind}
                    catalogFreshness={proxyBindingsCatalogFreshness}
                    showAutomaticNotice={false}
                    onChange={onBoundProxyKeysChange}
                    labels={{
                      loading: proxyBindingsLoadingLabel,
                      empty: proxyBindingsEmptyLabel,
                      missing: proxyBindingsMissingLabel,
                      unavailable: proxyBindingsUnavailableLabel,
                      chartLabel: proxyBindingsChartLabel,
                      chartSuccess: proxyBindingsChartSuccessLabel,
                      chartFailure: proxyBindingsChartFailureLabel,
                      chartEmpty: proxyBindingsChartEmptyLabel,
                      chartTotal: proxyBindingsChartTotalLabel,
                      chartAriaLabel: proxyBindingsChartAriaLabel,
                      chartInteractionHint: proxyBindingsChartInteractionHint,
                      chartLocaleTag: proxyBindingsChartLocaleTag,
                    }}
                  />
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
                      onCheckedChange={(checked) => onNodeShuntEnabledChange?.(checked)}
                      disabled={busy || !onNodeShuntEnabledChange}
                      aria-label={nodeShuntToggleLabel ?? "Enable node shunt strategy"}
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
        </div>

        <DialogFooter className="flex flex-col gap-3 border-t border-base-300/80 px-6 py-5 sm:flex-row sm:items-end sm:justify-between">
          <div className="flex min-w-0 items-end">
            {showDelete ? (
              <Popover
                open={deleteBlockedPopoverEnabled ? deleteBlockedPopoverOpen : false}
                onOpenChange={deleteBlockedPopoverEnabled ? setDeleteBlockedPopoverOpen : undefined}
              >
                <PopoverTrigger asChild>
                  <Button
                    type="button"
                    variant="destructive"
                    onClick={handleDeleteClick}
                    disabled={deleteBusyDisabled}
                    aria-disabled={deleteBlockedPopoverEnabled}
                    className={cn(
                      deleteBlockedPopoverEnabled
                        ? "opacity-50 focus-visible:ring-error/25"
                        : undefined,
                    )}
                  >
                    {deleting ? (
                      <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                    ) : (
                      <AppIcon name="trash-can-outline" className="mr-2 h-4 w-4" aria-hidden />
                    )}
                    {deleteLabel ?? "Delete group"}
                  </Button>
                </PopoverTrigger>
                {deleteBlockedPopoverEnabled ? (
                  <PopoverContent
                    side="top"
                    align="start"
                    sideOffset={12}
                    className="w-[min(22rem,calc(100vw-2rem))] rounded-2xl border-error/20 bg-base-100 px-4 py-3 shadow-xl"
                  >
                    <div className="flex items-start gap-3">
                      <div className="mt-0.5 rounded-full bg-error/10 p-1 text-error">
                        <AppIcon name="information-outline" className="h-4 w-4" aria-hidden />
                      </div>
                      <p className="text-sm leading-6 text-base-content/78">{deleteDisabledHint}</p>
                    </div>
                  </PopoverContent>
                ) : null}
              </Popover>
            ) : null}
          </div>
          <div className="flex w-full flex-col gap-2 sm:w-auto sm:flex-row sm:items-center">
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              <AppIcon name="close" className="mr-2 h-4 w-4" aria-hidden />
              {cancelLabel}
            </Button>
            <Button
              type="button"
              onClick={onSave}
              disabled={
                busy || saveDisabled || blockingBindingSelection || blockingNodeShuntSelection
              }
            >
              {busy && !deleting ? (
                <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
              ) : (
                <AppIcon
                  name={existing ? "content-save-outline" : "content-save-plus-outline"}
                  className="mr-2 h-4 w-4"
                  aria-hidden
                />
              )}
              {saveLabel}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
