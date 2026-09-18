import { useMemo } from "react";
import { Chip } from "../../components/ui/chip";
import type { ForwardProxyBindingNode } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import {
  canonicalizeForwardProxyBindingKeys,
  normalizeForwardProxyBindingKeys,
  resolveForwardProxyBindingOptions,
} from "./forwardProxyBindingSelectorUtils";
import { ProxyOptionTrafficChart } from "./ProxyOptionTrafficChart";

export type ForwardProxyBindingSelectorLabels = {
  automatic?: string;
  loading?: string;
  empty?: string;
  missing?: string;
  unavailable?: string;
  unavailableSelectionWarning?: string;
  penalized?: string;
  chartLabel?: string;
  chartSuccess?: string;
  chartFailure?: string;
  chartEmpty?: string;
  chartTotal?: string;
  chartAriaLabel?: string;
  chartInteractionHint?: string;
  chartLocaleTag?: string;
};

function toggleForwardProxyBindingKey(keys: string[], target: string): string[] {
  if (keys.includes(target)) {
    return keys.filter((key) => key !== target);
  }
  return [...keys, target];
}

function ForwardProxyBindingNotices({
  labels,
  loading,
  showAutomaticNotice,
  showEmpty,
  showUnavailableSelectionWarning,
  selectedKeyCount,
}: {
  labels?: ForwardProxyBindingSelectorLabels;
  loading: boolean;
  showAutomaticNotice: boolean;
  showEmpty: boolean;
  showUnavailableSelectionWarning: boolean;
  selectedKeyCount: number;
}) {
  return (
    <>
      {showAutomaticNotice && selectedKeyCount === 0 ? (
        <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
          {labels?.automatic ?? "No nodes bound. This group uses automatic routing."}
        </div>
      ) : null}
      {showUnavailableSelectionWarning ? (
        <div className="rounded-xl border border-warning/35 bg-warning/10 px-3 py-2 text-xs text-warning">
          {labels?.unavailableSelectionWarning ??
            "Select at least one available proxy node or clear bindings before saving."}
        </div>
      ) : null}
      {loading ? (
        <div
          className="flex items-center gap-2 rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65"
          data-testid="proxy-binding-options-loading"
        >
          <AppIcon name="loading" className="h-4 w-4 animate-spin" aria-hidden />
          <span>{labels?.loading ?? "Loading proxy nodes..."}</span>
        </div>
      ) : null}
      {showEmpty ? (
        <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
          {labels?.empty ?? "No proxy nodes available."}
        </div>
      ) : null}
    </>
  );
}

export function ForwardProxyBindingSelector({
  selectedKeys,
  availableProxyNodes,
  disabled = false,
  catalogKind,
  catalogFreshness,
  labels,
  onChange,
  showAutomaticNotice = true,
  showUnavailableSelectionWarning = false,
  className,
  scrollRegionClassName,
}: {
  selectedKeys: string[];
  availableProxyNodes?: ForwardProxyBindingNode[];
  disabled?: boolean;
  catalogKind?: "ready-empty" | "ready-with-data" | "loading" | "missing" | "deferred";
  catalogFreshness?: "fresh" | "stale" | "missing" | "deferred";
  labels?: ForwardProxyBindingSelectorLabels;
  onChange?: (value: string[]) => void;
  showAutomaticNotice?: boolean;
  showUnavailableSelectionWarning?: boolean;
  className?: string;
  scrollRegionClassName?: string;
}) {
  const canonicalSelectedKeys = useMemo(
    () =>
      canonicalizeForwardProxyBindingKeys(
        normalizeForwardProxyBindingKeys(selectedKeys),
        availableProxyNodes,
      ),
    [availableProxyNodes, selectedKeys],
  );
  const options = useMemo(
    () => resolveForwardProxyBindingOptions(canonicalSelectedKeys, availableProxyNodes),
    [availableProxyNodes, canonicalSelectedKeys],
  );
  const chartScaleMax = useMemo(
    () =>
      Math.max(
        ...options.flatMap((node) =>
          (Array.isArray(node.last24h) ? node.last24h : []).map(
            (bucket) => bucket.successCount + bucket.failureCount,
          ),
        ),
        0,
      ),
    [options],
  );
  const loading =
    catalogKind === "loading" ||
    catalogKind === "missing" ||
    (catalogFreshness === "stale" && options.length === 0);
  const showEmpty = !loading && options.length === 0;

  return (
    <div className={cn("grid gap-3", className)}>
      <ForwardProxyBindingNotices
        labels={labels}
        loading={loading}
        showAutomaticNotice={showAutomaticNotice}
        showEmpty={showEmpty}
        showUnavailableSelectionWarning={showUnavailableSelectionWarning}
        selectedKeyCount={canonicalSelectedKeys.length}
      />
      {options.length > 0 ? (
        <div
          className={cn(
            "min-h-0 max-h-[min(26rem,45dvh)] overflow-y-auto rounded-xl pr-1",
            scrollRegionClassName,
          )}
          data-testid="proxy-binding-options-scroll-region"
        >
          <div className="grid gap-2">
            {options.map((node) => (
              <ForwardProxyBindingOption
                key={node.key}
                node={node}
                selected={canonicalSelectedKeys.includes(node.key)}
                disabled={disabled}
                scaleMax={chartScaleMax}
                labels={labels}
                selectedKeys={canonicalSelectedKeys}
                onChange={onChange}
              />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ForwardProxyBindingOption({
  node,
  selected,
  disabled,
  scaleMax,
  labels,
  selectedKeys,
  onChange,
}: {
  node: NonNullable<ReturnType<typeof resolveForwardProxyBindingOptions>>[number];
  selected: boolean;
  disabled: boolean;
  scaleMax: number;
  labels?: ForwardProxyBindingSelectorLabels;
  selectedKeys: string[];
  onChange?: (value: string[]) => void;
}) {
  const optionDisabled = disabled || (!selected && !node.selectable);
  const badgeLabel = node.missing
    ? (labels?.missing ?? "Missing")
    : !node.selectable
      ? (labels?.unavailable ?? "Unavailable")
      : null;

  return (
    <button
      type="button"
      disabled={optionDisabled}
      onClick={() => {
        if (onChange) onChange(toggleForwardProxyBindingKey(selectedKeys, node.key));
      }}
      className={cn(
        "grid gap-2 rounded-xl border px-3 py-2 text-left transition-colors sm:grid-cols-[minmax(0,1fr)_15.5rem] sm:items-center sm:gap-3",
        selected ? "border-primary/45 bg-primary/10" : "border-base-300/80 bg-base-100/75",
        optionDisabled ? "cursor-not-allowed opacity-60" : "hover:border-primary/40",
      )}
    >
      <ForwardProxyBindingOptionIdentity
        node={node}
        selected={selected}
        badgeLabel={badgeLabel}
        penalizedLabel={labels?.penalized ?? "Penalized"}
      />
      <ProxyOptionTrafficChart
        node={node}
        scaleMax={scaleMax}
        label={labels?.chartLabel ?? "24h request trend"}
        successLabel={labels?.chartSuccess ?? "ok"}
        failureLabel={labels?.chartFailure ?? "fail"}
        emptyLabel={labels?.chartEmpty ?? "No 24h data"}
        totalLabel={labels?.chartTotal ?? "total"}
        ariaLabel={labels?.chartAriaLabel ?? "Last 24h request volume chart"}
        interactionHint={
          labels?.chartInteractionHint ??
          "Hover or tap for details. Focus the chart and use arrow keys to switch points."
        }
        localeTag={labels?.chartLocaleTag ?? "en-US"}
      />
    </button>
  );
}

function ForwardProxyBindingOptionIdentity({
  node,
  selected,
  badgeLabel,
  penalizedLabel,
}: {
  node: NonNullable<ReturnType<typeof resolveForwardProxyBindingOptions>>[number];
  selected: boolean;
  badgeLabel: string | null;
  penalizedLabel: string;
}) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-base-300/80 bg-base-100">
        {selected ? (
          <AppIcon name="check" className="h-3.5 w-3.5 text-primary" aria-hidden />
        ) : null}
      </div>
      <div className="min-w-0 flex-1">
        <span
          className="block min-w-0 truncate text-sm font-medium text-base-content"
          title={node.displayName}
        >
          {node.displayName}
        </span>
        <div className="mt-1 flex flex-wrap items-center gap-2">
          <Chip
            size="micro"
            tone="secondary"
            className="shrink-0 px-1.5 font-mono uppercase tracking-[0.08em]"
          >
            {node.protocolLabel}
          </Chip>
          {node.identityHint ? (
            <Chip
              size="micro"
              tone="neutral"
              className="shrink-0 px-1.5 font-mono tracking-[0.08em]"
              title={node.identityHint}
            >
              {node.identityHint}
            </Chip>
          ) : null}
          {badgeLabel ? (
            <Chip
              size="micro"
              tone={node.missing ? "error" : "warning"}
              className="shrink-0 px-2 uppercase tracking-[0.08em]"
            >
              {badgeLabel}
            </Chip>
          ) : null}
          {node.penalized ? (
            <Chip size="micro" tone="warning" className="shrink-0 px-2 uppercase tracking-[0.08em]">
              {penalizedLabel}
            </Chip>
          ) : null}
        </div>
      </div>
    </div>
  );
}
