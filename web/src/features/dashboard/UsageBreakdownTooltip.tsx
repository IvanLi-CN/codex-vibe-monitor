import type { ReactNode } from "react";
import type { UsageBreakdown, UsageBreakdownModel } from "../../lib/api";

export type UsageBreakdownKind = "cost" | "tokens";

export interface UsageBreakdownTooltipProps {
  title: string;
  breakdown?: UsageBreakdown | null;
  kind: UsageBreakdownKind;
  formatNumber: (value: number) => string;
  formatRatio: (value: number | null) => string;
  formatCurrency: (value: number) => string;
  labels: {
    total: string;
    model: string;
    cacheWrite: string;
    cacheRead: string;
    cacheHitTokens: string;
    cacheHitRate: string;
    output: string;
    input: string;
    reasoning: string;
    unknown: string;
    unavailable: string;
    tokenUnavailable: string;
    unknownModel: string;
    reasoningEffort: string;
    unspecifiedEffort: string;
    effortNone: string;
    effortMinimal: string;
    effortLow: string;
    effortMedium: string;
    effortHigh: string;
    effortXhigh: string;
  };
}

interface BreakdownTableRow {
  key: string;
  label: ReactNode;
  values?: string[];
  unavailable?: string;
}

interface BreakdownTableColumn {
  label: string;
}

function modelLabel(model: string, unknownModel: string) {
  return model === "unknown" ? unknownModel : model;
}

function effortLabel(
  effort: string | null | undefined,
  labels: UsageBreakdownTooltipProps["labels"],
) {
  const normalized = effort?.trim().toLowerCase();
  if (!normalized) return labels.unspecifiedEffort;
  return (
    {
      none: labels.effortNone,
      minimal: labels.effortMinimal,
      low: labels.effortLow,
      medium: labels.effortMedium,
      high: labels.effortHigh,
      xhigh: labels.effortXhigh,
    }[normalized] ??
    effort?.trim() ??
    labels.unspecifiedEffort
  );
}

function groupKey(model: UsageBreakdownModel) {
  return `${model.model}\u0000${model.reasoningEffort?.trim() ?? ""}`;
}

function groupLabel(model: UsageBreakdownModel, labels: UsageBreakdownTooltipProps["labels"]) {
  const modelName = modelLabel(model.model, labels.unknownModel);
  const effort = effortLabel(model.reasoningEffort, labels);
  return (
    <span
      className="flex min-w-0 flex-col gap-0.5"
      aria-label={`${modelName}, ${labels.reasoningEffort}: ${effort}`}
    >
      <span className="break-all font-medium text-base-content/80">{modelName}</span>
      <span className="break-words text-[9px] font-normal leading-3 text-base-content/58 sm:text-[10px]">
        {labels.reasoningEffort}: {effort}
      </span>
    </span>
  );
}

function BreakdownTable({
  title,
  columns,
  rows,
  modelLabel: modelColumnLabel,
  modelWidth,
}: {
  title: string;
  columns: readonly BreakdownTableColumn[];
  rows: readonly BreakdownTableRow[];
  modelLabel: string;
  modelWidth: string;
}) {
  const dense = columns.length >= 4;
  return (
    <table
      className={`w-full table-fixed border-collapse ${dense ? "text-[8px] leading-3 sm:text-[10px] sm:leading-4" : "text-[10px] leading-4 sm:text-[11px]"}`}
    >
      <caption className="sr-only">{title}</caption>
      <thead
        className={`border-y border-base-300/50 bg-base-200/45 font-semibold text-base-content/58 ${dense ? "text-[8px] sm:text-[9px]" : "text-[9px] sm:text-[10px]"}`}
      >
        <tr>
          <th
            scope="col"
            className="px-1.5 py-1.5 text-left font-semibold"
            style={{ width: modelWidth }}
          >
            {modelColumnLabel}
          </th>
          {columns.map((column) => (
            <th
              key={column.label}
              scope="col"
              className={`${dense ? "px-0.5" : "px-1"} border-l border-base-300/30 py-1.5 text-right font-semibold break-words`}
            >
              {column.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, rowIndex) => (
          <tr
            key={row.key}
            className={
              rowIndex === 0
                ? "border-b border-base-300/50 bg-base-100/45"
                : "border-b border-base-300/30 last:border-b-0"
            }
          >
            <th
              scope="row"
              className="px-1.5 py-1.5 text-left font-medium text-base-content/76 break-all"
            >
              {row.label}
            </th>
            {row.unavailable ? (
              <td colSpan={columns.length} className="px-1.5 py-1.5 text-left text-base-content/62">
                {row.unavailable}
              </td>
            ) : (
              row.values?.map((value, columnIndex) => (
                <td
                  key={`${row.key}:${columnIndex}`}
                  className={`${dense ? "px-0.5" : "px-1"} border-l border-base-300/30 py-1.5 text-right font-mono font-semibold text-base-content tabular-nums whitespace-nowrap`}
                >
                  {value}
                </td>
              ))
            )}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function CostBreakdownTable({
  title,
  breakdown,
  models,
  formatCurrency,
  labels,
}: Pick<UsageBreakdownTooltipProps, "formatCurrency" | "labels"> & {
  title: string;
  breakdown?: UsageBreakdown | null;
  models: UsageBreakdown["models"];
}) {
  const columns = [
    { label: labels.input, key: "input" },
    { label: labels.cacheWrite, key: "cacheWrite" },
    { label: labels.cacheRead, key: "cacheRead" },
    { label: labels.output, key: "output" },
    { label: labels.reasoning, key: "reasoning" },
    ...((breakdown?.costs?.unknown ?? 0) !== 0 ||
    models.some((model) => (model.costs?.unknown ?? 0) !== 0)
      ? [{ label: labels.unknown, key: "unknown" as const }]
      : []),
  ] as const;
  const rowFor = (
    key: string,
    label: ReactNode,
    costs: UsageBreakdown["costs"],
  ): BreakdownTableRow => {
    if (!costs) return { key, label, unavailable: labels.unavailable };
    return {
      key,
      label,
      values: columns.map(({ key }) => (costs[key] === 0 ? "-" : formatCurrency(costs[key]))),
    };
  };

  return (
    <BreakdownTable
      title={title}
      modelLabel={labels.model}
      modelWidth={columns.length === 6 ? "20%" : "22%"}
      columns={columns}
      rows={[
        rowFor("total", labels.total, breakdown?.costs),
        ...models.map((model) => rowFor(groupKey(model), groupLabel(model, labels), model.costs)),
      ]}
    />
  );
}

function TokenBreakdownTable({
  title,
  breakdown,
  models,
  formatNumber,
  formatRatio,
  labels,
}: Pick<UsageBreakdownTooltipProps, "formatNumber" | "formatRatio" | "labels"> & {
  title: string;
  breakdown?: UsageBreakdown | null;
  models: UsageBreakdown["models"];
}) {
  const columns = [
    { label: labels.cacheWrite },
    { label: labels.cacheHitTokens },
    { label: labels.cacheHitRate },
    { label: labels.output },
  ];
  const rowFor = (
    key: string,
    label: ReactNode,
    item: Pick<UsageBreakdown, "cacheWriteTokens" | "cacheReadTokens" | "outputTokens">,
  ): BreakdownTableRow => ({
    key,
    label,
    values: [
      formatNumber(item.cacheWriteTokens),
      formatNumber(item.cacheReadTokens),
      formatRatio(cacheHitRate(item)),
      formatNumber(item.outputTokens),
    ],
  });

  return (
    <BreakdownTable
      title={title}
      modelLabel={labels.model}
      modelWidth="28%"
      columns={columns}
      rows={
        breakdown
          ? [
              rowFor("total", labels.total, breakdown),
              ...models.map((model) => rowFor(groupKey(model), groupLabel(model, labels), model)),
            ]
          : [{ key: "total", label: labels.total, unavailable: labels.tokenUnavailable }]
      }
    />
  );
}

function cacheHitRate(
  item: Pick<UsageBreakdown, "cacheWriteTokens" | "cacheReadTokens" | "outputTokens">,
) {
  const cacheWriteTokens = Math.max(item.cacheWriteTokens, 0);
  const cacheReadTokens = Math.max(item.cacheReadTokens, 0);
  const outputTokens = Math.max(item.outputTokens, 0);
  const totalTokens = cacheWriteTokens + cacheReadTokens + outputTokens;
  return totalTokens > 0 ? cacheReadTokens / totalTokens : null;
}

export function UsageBreakdownTooltip({
  title,
  breakdown,
  kind,
  formatNumber,
  formatRatio,
  formatCurrency,
  labels,
}: UsageBreakdownTooltipProps) {
  const models = [...(breakdown?.models ?? [])]
    .filter((model) => {
      if (kind === "tokens") {
        return model.cacheWriteTokens > 0 || model.cacheReadTokens > 0 || model.outputTokens > 0;
      }
      return (
        model.costs != null ||
        model.cacheWriteTokens > 0 ||
        model.cacheReadTokens > 0 ||
        model.outputTokens > 0
      );
    })
    .sort((left, right) => {
      const leftValue =
        kind === "tokens"
          ? left.cacheWriteTokens + left.cacheReadTokens + left.outputTokens
          : (left.costs?.input ?? 0) +
            (left.costs?.cacheWrite ?? 0) +
            (left.costs?.cacheRead ?? 0) +
            (left.costs?.output ?? 0) +
            (left.costs?.reasoning ?? 0) +
            (left.costs?.unknown ?? 0);
      const rightValue =
        kind === "tokens"
          ? right.cacheWriteTokens + right.cacheReadTokens + right.outputTokens
          : (right.costs?.input ?? 0) +
            (right.costs?.cacheWrite ?? 0) +
            (right.costs?.cacheRead ?? 0) +
            (right.costs?.output ?? 0) +
            (right.costs?.reasoning ?? 0) +
            (right.costs?.unknown ?? 0);
      return (
        rightValue - leftValue ||
        left.model.localeCompare(right.model) ||
        groupKey(left).localeCompare(groupKey(right))
      );
    });

  return (
    <div data-testid={`usage-breakdown-tooltip-${kind}`} className="space-y-1.5">
      <div className="px-0.5 text-[11px] font-semibold leading-4 text-base-content/72">{title}</div>
      {kind === "tokens" ? (
        <TokenBreakdownTable
          title={title}
          breakdown={breakdown}
          models={models}
          formatNumber={formatNumber}
          formatRatio={formatRatio}
          labels={labels}
        />
      ) : (
        <CostBreakdownTable
          title={title}
          breakdown={breakdown}
          models={models}
          formatCurrency={formatCurrency}
          labels={labels}
        />
      )}
    </div>
  );
}
