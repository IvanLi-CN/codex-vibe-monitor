import { useMemo } from "react";
import { Chip } from "../../components/ui/chip";
import type { ForwardProxyBindingNode } from "../../lib/api";
import { ForwardProxyRequestTrendChart } from "./ForwardProxyRequestTrendChart";

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

export function ProxyOptionTrafficChart({
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
  const buckets = useMemo(() => (Array.isArray(node.last24h) ? node.last24h : []), [node.last24h]);
  const totals = useMemo(() => sumProxyTraffic(node), [node]);
  const windowBadgeLabel = useMemo(() => {
    if (/24/.test(label)) return "24H";
    return label;
  }, [label]);

  return (
    <div className="flex w-full flex-col justify-center gap-0.5 sm:min-w-[15.5rem] sm:max-w-[15.5rem] sm:self-center">
      <div className="flex h-4 items-center justify-between gap-2">
        <Chip
          size="micro"
          tone="secondary"
          role="img"
          className="h-4 min-w-[2.25rem] justify-center px-1.5 text-[9px] uppercase tracking-[0.12em]"
          title={label}
          aria-label={label}
        >
          {windowBadgeLabel}
        </Chip>
        <div className="flex items-center gap-1.5 text-[10px] font-semibold leading-none tabular-nums">
          <span
            role="img"
            className="inline-flex items-center gap-1 text-success"
            aria-label={`${successLabel} ${totals.success}`}
            title={`${successLabel} ${totals.success}`}
          >
            <span className="h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
            <span>{totals.success}</span>
          </span>
          <span
            role="img"
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
