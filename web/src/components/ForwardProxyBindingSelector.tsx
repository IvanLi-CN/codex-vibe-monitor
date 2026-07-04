import { useMemo } from "react";
import { AppIcon } from "./AppIcon";
import { ForwardProxyRequestTrendChart } from "./ForwardProxyRequestTrendChart";
import type { ForwardProxyBindingNode } from "../lib/api";
import { cn } from "../lib/utils";
import {
  canonicalizeForwardProxyBindingKeys,
  normalizeForwardProxyBindingKeys,
  resolveForwardProxyBindingOptions,
} from "./forwardProxyBindingSelectorUtils";

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

function toggleForwardProxyBindingKey(
  keys: string[],
  target: string,
): string[] {
  if (keys.includes(target)) {
    return keys.filter((key) => key !== target);
  }
  return [...keys, target];
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
  catalogKind?:
    | "ready-empty"
    | "ready-with-data"
    | "loading"
    | "missing"
    | "deferred";
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
    () =>
      resolveForwardProxyBindingOptions(canonicalSelectedKeys, availableProxyNodes),
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
      {showAutomaticNotice && canonicalSelectedKeys.length === 0 ? (
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

      {options.length > 0 ? (
        <div
          className={cn(
            "min-h-0 max-h-[min(26rem,45dvh)] overflow-y-auto rounded-xl pr-1",
            scrollRegionClassName,
          )}
          data-testid="proxy-binding-options-scroll-region"
        >
          <div className="grid gap-2">
            {options.map((node) => {
              const selected = canonicalSelectedKeys.includes(node.key);
              const optionDisabled = disabled || (!selected && !node.selectable);
              const badgeLabel = node.missing
                ? (labels?.missing ?? "Missing")
                : !node.selectable
                  ? (labels?.unavailable ?? "Unavailable")
                  : null;
              return (
                <button
                  key={node.key}
                  type="button"
                  disabled={optionDisabled}
                  onClick={() => {
                    if (!onChange) return;
                    onChange(
                      toggleForwardProxyBindingKey(
                        canonicalSelectedKeys,
                        node.key,
                      ),
                    );
                  }}
                  className={cn(
                    "grid gap-2 rounded-xl border px-3 py-2 text-left transition-colors sm:grid-cols-[minmax(0,1fr)_15.5rem] sm:items-center sm:gap-3",
                    selected
                      ? "border-primary/45 bg-primary/10"
                      : "border-base-300/80 bg-base-100/75",
                    optionDisabled
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
                      <span
                        className="block min-w-0 truncate text-sm font-medium text-base-content"
                        title={node.displayName}
                      >
                        {node.displayName}
                      </span>
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
                            {labels?.penalized ?? "Penalized"}
                          </span>
                        ) : null}
                      </div>
                    </div>
                  </div>
                  <ProxyOptionTrafficChart
                    node={node}
                    scaleMax={chartScaleMax}
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
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}
