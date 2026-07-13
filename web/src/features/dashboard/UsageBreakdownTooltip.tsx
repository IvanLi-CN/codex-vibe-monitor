import type { ReactNode } from "react";
import type { UsageBreakdown, UsageBreakdownModel } from "../../lib/api";

type UsageCostBreakdown = NonNullable<UsageBreakdown["costs"]>;

export interface UsageBreakdownTooltipProps {
  title: string;
  breakdown: UsageBreakdown;
  formatNumber: (value: number) => string;
  formatRatio: (value: number | null) => string;
  formatCurrency: (value: number) => string;
  labels: {
    total: string;
    model: string;
    cacheWrite: string;
    cacheRead: string;
    cacheHitRate: string;
    output: string;
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

type UsageBreakdownItem = Pick<
  UsageBreakdown,
  "cacheWriteTokens" | "cacheReadTokens" | "outputTokens" | "costs"
>;

interface BreakdownTableRow {
  key: string;
  label: ReactNode;
  values: BreakdownTableValue[];
}

interface BreakdownTableValue {
  key: string;
  content: ReactNode;
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
    <span className="flex min-w-0 flex-col gap-0.5">
      <span className="break-all font-normal text-base-content/80">{modelName}</span>
      <span className="break-words text-[8px] font-normal leading-3 text-base-content/58 sm:text-[10px]">
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
}: {
  title: string;
  columns: readonly BreakdownTableColumn[];
  rows: readonly BreakdownTableRow[];
  modelLabel: string;
}) {
  return (
    <table className="w-full table-fixed border-collapse text-[8px] leading-3 sm:text-[10px] sm:leading-4">
      <caption className="sr-only">{title}</caption>
      <thead className="border-y border-base-300/50 bg-base-200/45 text-[8px] font-semibold text-base-content/58 sm:text-[9px]">
        <tr>
          <th scope="col" className="w-[38%] px-1.5 py-1.5 text-left font-semibold sm:w-[30%]">
            {modelColumnLabel}
          </th>
          {columns.map((column) => (
            <th
              key={column.label}
              scope="col"
              className="border-l border-base-300/30 px-0.5 py-1.5 text-right font-semibold break-words"
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
              className="px-1.5 py-1.5 text-left font-normal text-base-content/76 break-all"
            >
              {row.label}
            </th>
            {row.values.map((value) => (
              <td
                key={`${row.key}:${value.key}`}
                className="border-l border-base-300/30 px-0.5 py-1.5 text-right font-mono font-normal tabular-nums"
              >
                {value.content}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function totalTokens(
  item: Pick<UsageBreakdownItem, "cacheWriteTokens" | "cacheReadTokens" | "outputTokens">,
) {
  return (
    Math.max(item.cacheWriteTokens, 0) +
    Math.max(item.cacheReadTokens, 0) +
    Math.max(item.outputTokens, 0)
  );
}

function totalCost(costs: UsageCostBreakdown | null | undefined) {
  if (!costs) return null;
  return (
    costs.input +
    costs.cacheWrite +
    costs.cacheRead +
    costs.output +
    costs.reasoning +
    costs.unknown
  );
}

function isHistoricalCostOnly(costs: UsageCostBreakdown | null | undefined) {
  if (!costs) return false;
  return (
    costs.unknown !== 0 &&
    costs.input === 0 &&
    costs.cacheWrite === 0 &&
    costs.cacheRead === 0 &&
    costs.output === 0 &&
    costs.reasoning === 0
  );
}

function cacheWriteCost(costs: UsageCostBreakdown | null | undefined) {
  if (!costs || isHistoricalCostOnly(costs)) return null;
  return costs.input + costs.cacheWrite;
}

function cacheReadCost(costs: UsageCostBreakdown | null | undefined) {
  if (!costs || isHistoricalCostOnly(costs)) return null;
  return costs.cacheRead;
}

function outputCost(costs: UsageCostBreakdown | null | undefined) {
  if (!costs || isHistoricalCostOnly(costs)) return null;
  return costs.output + costs.reasoning;
}

function displayCurrency(value: number | null, formatCurrency: (value: number) => string) {
  return value == null ? "—" : formatCurrency(value);
}

function UsageAndCostValue({
  tokenCount,
  cost,
  formatNumber,
  formatCurrency,
}: {
  tokenCount: number;
  cost: number | null;
  formatNumber: (value: number) => string;
  formatCurrency: (value: number) => string;
}) {
  const tokenText = formatNumber(tokenCount);
  const costText = displayCurrency(cost, formatCurrency);
  return (
    <span className="flex min-w-0 flex-col items-end gap-0.5 whitespace-nowrap">
      <span className="text-base-content">{tokenText}</span>
      <span className="text-base-content/62">{costText}</span>
    </span>
  );
}

function UsageValueWithPlaceholder({ value }: { value: string }) {
  return (
    <span className="flex min-w-0 flex-col items-end gap-0.5 whitespace-nowrap">
      <span className="text-base-content">{value}</span>
      <span aria-hidden="true" className="block h-3 sm:h-4" />
    </span>
  );
}

function cacheHitRate(
  item: Pick<UsageBreakdownItem, "cacheWriteTokens" | "cacheReadTokens" | "outputTokens">,
) {
  const rowTotalTokens = totalTokens(item);
  return rowTotalTokens > 0 ? Math.max(item.cacheReadTokens, 0) / rowTotalTokens : null;
}

function UsageBreakdownTable({
  title,
  breakdown,
  models,
  formatNumber,
  formatRatio,
  formatCurrency,
  labels,
}: UsageBreakdownTooltipProps & { models: UsageBreakdown["models"] }) {
  const columns = [
    { label: labels.cacheWrite },
    { label: labels.cacheRead },
    { label: labels.cacheHitRate },
    { label: labels.output },
    { label: labels.total },
  ];
  const rowFor = (key: string, label: ReactNode, item: UsageBreakdownItem): BreakdownTableRow => ({
    key,
    label,
    values: [
      {
        key: "cache-write",
        content: (
          <UsageAndCostValue
            tokenCount={item.cacheWriteTokens}
            cost={cacheWriteCost(item.costs)}
            formatNumber={formatNumber}
            formatCurrency={formatCurrency}
          />
        ),
      },
      {
        key: "cache-read",
        content: (
          <UsageAndCostValue
            tokenCount={item.cacheReadTokens}
            cost={cacheReadCost(item.costs)}
            formatNumber={formatNumber}
            formatCurrency={formatCurrency}
          />
        ),
      },
      {
        key: "cache-hit-rate",
        content: <UsageValueWithPlaceholder value={formatRatio(cacheHitRate(item))} />,
      },
      {
        key: "output",
        content: (
          <UsageAndCostValue
            tokenCount={item.outputTokens}
            cost={outputCost(item.costs)}
            formatNumber={formatNumber}
            formatCurrency={formatCurrency}
          />
        ),
      },
      {
        key: "total",
        content: (
          <UsageAndCostValue
            tokenCount={totalTokens(item)}
            cost={totalCost(item.costs)}
            formatNumber={formatNumber}
            formatCurrency={formatCurrency}
          />
        ),
      },
    ],
  });

  return (
    <BreakdownTable
      title={title}
      modelLabel={labels.model}
      columns={columns}
      rows={[
        rowFor("total", labels.total, breakdown),
        ...models.map((model) => rowFor(groupKey(model), groupLabel(model, labels), model)),
      ]}
    />
  );
}

export function UsageBreakdownTooltip({
  title,
  breakdown,
  formatNumber,
  formatRatio,
  formatCurrency,
  labels,
}: UsageBreakdownTooltipProps) {
  const models = [...breakdown.models]
    .filter(
      (model) =>
        model.costs != null ||
        model.cacheWriteTokens > 0 ||
        model.cacheReadTokens > 0 ||
        model.outputTokens > 0,
    )
    .sort((left, right) => {
      const tokenDifference = totalTokens(right) - totalTokens(left);
      const costDifference = (totalCost(right.costs) ?? 0) - (totalCost(left.costs) ?? 0);
      return (
        tokenDifference ||
        costDifference ||
        left.model.localeCompare(right.model) ||
        groupKey(left).localeCompare(groupKey(right))
      );
    });

  return (
    <div data-testid="usage-breakdown-tooltip" className="space-y-1.5">
      <div className="px-0.5 text-[11px] font-semibold leading-4 text-base-content/72">{title}</div>
      <UsageBreakdownTable
        title={title}
        breakdown={breakdown}
        models={models}
        formatNumber={formatNumber}
        formatRatio={formatRatio}
        formatCurrency={formatCurrency}
        labels={labels}
      />
    </div>
  );
}
