import { useMemo } from "react";
import { Alert } from "../../components/ui/alert";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type {
  ForwardProxyLiveNode,
  ForwardProxyLiveStatsResponse,
  ForwardProxyWeightBucket,
  ForwardProxyWindowStats,
} from "../../lib/api";
import { cn } from "../../lib/utils";
import {
  formatWeight,
  forwardProxyWindowColumns,
  forwardProxyWindowHeaderClassNames,
  ProxyTrendCells,
  WindowCell,
} from "./ForwardProxyWeightTrendCell";

interface ForwardProxyLiveTableProps {
  stats: ForwardProxyLiveStatsResponse | null;
  isLoading: boolean;
  error?: string | null;
}

interface ForwardProxyLiveTableRowProps {
  node: ForwardProxyLiveNode;
  windows: Array<{ labelKey: string; value: ForwardProxyWindowStats }>;
  total24h: { success: number; failure: number };
  weightBuckets: ForwardProxyWeightBucket[];
  requestBucketScaleMax: number;
  weightTrendScale: { minValue: number; maxValue: number };
  localeTag: string;
  requestTooltipLabels: { success: string; failure: string; total: string };
  weightTooltipLabels: {
    samples: string;
    min: string;
    max: string;
    avg: string;
    last: string;
  };
  requestTrendAriaLabel: string;
  weightTrendAriaLabel: string;
  chartInteractionHint: string;
  t: (key: string, values?: Record<string, string | number>) => string;
}

function ForwardProxyLiveTableRow({
  node,
  windows,
  total24h,
  weightBuckets,
  requestBucketScaleMax,
  weightTrendScale,
  localeTag,
  requestTooltipLabels,
  weightTooltipLabels,
  requestTrendAriaLabel,
  weightTrendAriaLabel,
  chartInteractionHint,
  t,
}: ForwardProxyLiveTableRowProps) {
  return (
    <tr
      key={node.key}
      className={cn("transition-colors hover:bg-primary/6", node.penalized && "bg-warning/8")}
    >
      <td className="max-w-0 px-2 py-2 align-middle sm:px-3 sm:py-3">
        <div className="min-w-0">
          <div className="truncate whitespace-nowrap text-sm font-medium" title={node.displayName}>
            {node.displayName}
          </div>
          <div className="mt-1 text-[11px] text-base-content/65">
            {t("live.proxy.table.successShort", { count: total24h.success })}
            {" / "}
            {t("live.proxy.table.failureShort", { count: total24h.failure })}
          </div>
          <div className="mt-0.5 text-[11px] text-base-content/58">
            {t("live.proxy.table.currentWeight", { value: formatWeight(node.weight) })}
          </div>
        </div>
      </td>
      {windows.map((window, index) => (
        <td
          key={`${node.key}-${window.labelKey}`}
          className={cn(
            "px-1 py-2 text-center align-middle sm:px-2 sm:py-3",
            index === 1 && "hidden md:table-cell",
            index === 2 && "hidden md:table-cell",
            index === 3 && "hidden lg:table-cell",
            index === 4 && "hidden lg:table-cell",
          )}
        >
          <WindowCell value={window.value} />
        </td>
      ))}
      <ProxyTrendCells
        node={node}
        weightBuckets={weightBuckets}
        requestBucketScaleMax={requestBucketScaleMax}
        weightTrendScale={weightTrendScale}
        localeTag={localeTag}
        requestTooltipLabels={requestTooltipLabels}
        weightTooltipLabels={weightTooltipLabels}
        requestTrendAriaLabel={requestTrendAriaLabel}
        weightTrendAriaLabel={weightTrendAriaLabel}
        chartInteractionHint={chartInteractionHint}
      />
    </tr>
  );
}

function sumLast24h(node: ForwardProxyLiveNode) {
  return node.last24h.reduce(
    (acc, bucket) => {
      acc.success += bucket.successCount;
      acc.failure += bucket.failureCount;
      return acc;
    },
    { success: 0, failure: 0 },
  );
}

function resolveWeightBuckets(node: ForwardProxyLiveNode): ForwardProxyWeightBucket[] {
  if (node.weight24h.length > 0) return node.weight24h;
  if (node.last24h.length === 0) return [];
  return node.last24h.map((bucket) => ({
    bucketStart: bucket.bucketStart,
    bucketEnd: bucket.bucketEnd,
    sampleCount: 0,
    minWeight: node.weight,
    maxWeight: node.weight,
    avgWeight: node.weight,
    lastWeight: node.weight,
  }));
}

export function ForwardProxyLiveTable({ stats, isLoading, error }: ForwardProxyLiveTableProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const weightTrendAriaLabel = t("live.proxy.table.weightTrendAria");
  const requestTrendAriaLabel = t("live.proxy.table.requestTrendAria");
  const chartInteractionHint = t("live.chart.tooltip.instructions");
  const requestTooltipLabels = useMemo(
    () => ({
      success: t("stats.cards.success"),
      failure: t("stats.cards.failures"),
      total: t("live.proxy.table.requestTooltip.total"),
    }),
    [t],
  );
  const weightTooltipLabels = useMemo(
    () => ({
      samples: t("live.proxy.table.weightTooltip.samples"),
      min: t("live.proxy.table.weightTooltip.min"),
      max: t("live.proxy.table.weightTooltip.max"),
      avg: t("live.proxy.table.weightTooltip.avg"),
      last: t("live.proxy.table.weightTooltip.last"),
    }),
    [t],
  );

  const { rowData, requestBucketScaleMax, weightTrendScale } = useMemo(() => {
    const rows = (stats?.nodes ?? []).map((node) => {
      const weightBuckets = resolveWeightBuckets(node);
      return {
        node,
        windows: forwardProxyWindowColumns.map((column) => ({
          labelKey: column.labelKey,
          value: column.selectStats(node.stats),
        })),
        total24h: sumLast24h(node),
        weightBuckets,
      };
    });
    const requestBucketScaleMax = Math.max(
      ...rows.flatMap(({ node }) =>
        node.last24h.map((bucket) => bucket.successCount + bucket.failureCount),
      ),
      0,
    );
    const hasRealWeightHistory = rows.some(({ node }) => node.weight24h.length > 0);
    const allWeightValues = (
      hasRealWeightHistory
        ? rows.flatMap(({ node }) => node.weight24h)
        : rows.flatMap(({ weightBuckets }) => weightBuckets)
    ).flatMap((bucket) => [bucket.minWeight, bucket.maxWeight, bucket.lastWeight]);
    const minValue = Math.min(...allWeightValues, 0);
    const maxValue = Math.max(...allWeightValues, 0);
    const padding = Math.max((maxValue - minValue) * 0.08, 0.2);
    return {
      rowData: rows,
      requestBucketScaleMax,
      weightTrendScale: {
        minValue: minValue - padding,
        maxValue: maxValue + padding,
      },
    };
  }, [stats]);

  if (isLoading && !stats) {
    return (
      <div className="flex min-h-[240px] items-center justify-center rounded-2xl border border-base-300/75 bg-base-100/55">
        <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
      </div>
    );
  }

  if (error) {
    return <Alert variant="error">{t("table.loadError", { error })}</Alert>;
  }

  if (!stats || stats.nodes.length === 0) {
    return <Alert>{t("live.proxy.table.empty")}</Alert>;
  }

  return (
    <div className="overflow-x-auto rounded-2xl border border-base-300/75 bg-base-100/55">
      <table className="w-full min-w-[1180px] table-fixed text-xs sm:min-w-[1260px] lg:min-w-0">
        <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
          <tr>
            <th className="w-[18%] px-2 py-2 text-left font-semibold sm:w-[30%] sm:px-3 sm:py-3 md:w-[18%] lg:w-[21%]">
              {t("live.proxy.table.proxy")}
            </th>
            {forwardProxyWindowColumns.map((column, index) => (
              <th key={column.labelKey} className={forwardProxyWindowHeaderClassNames[index]}>
                {t(column.labelKey)}
              </th>
            ))}
            <th className="w-[24%] px-2 py-2 text-left font-semibold sm:w-[29%] sm:px-3 sm:py-3 md:w-[31%] lg:w-[21%]">
              {t("live.proxy.table.trend24h")}
            </th>
            <th className="w-[20%] px-2 py-2 text-left font-semibold sm:w-[28%] sm:px-3 sm:py-3 md:w-[27%] lg:w-[18%]">
              {t("live.proxy.table.weightTrend24h")}
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-base-300/65">
          {rowData.map((row) => (
            <ForwardProxyLiveTableRow
              key={row.node.key}
              {...row}
              requestBucketScaleMax={requestBucketScaleMax}
              weightTrendScale={weightTrendScale}
              requestTooltipLabels={requestTooltipLabels}
              weightTooltipLabels={weightTooltipLabels}
              requestTrendAriaLabel={requestTrendAriaLabel}
              weightTrendAriaLabel={weightTrendAriaLabel}
              chartInteractionHint={chartInteractionHint}
              localeTag={localeTag}
              t={t}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}
