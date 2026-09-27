import type { ReactNode } from "react";
import { useTranslation } from "../../i18n";
import type { UsageBreakdown, UsageBreakdownModel } from "../../lib/api";
import { cn } from "../../lib/utils";
import { ModelIdentity } from "../shared/ModelIdentity";
import {
  ModelBreakdownMetricHeader,
  ModelBreakdownMetricSortButton,
  ModelBreakdownModelHeader,
  ModelBreakdownModelSortButton,
  ModelBreakdownModeToggle,
} from "./DashboardModelBreakdownControls";
import type { DashboardModelBreakdownSort } from "./dashboardModelBreakdown";
import {
  groupUsageBreakdownModels,
  sortForMode,
  sortUsageBreakdownModels,
  useDashboardModelBreakdownMode,
  useDashboardModelBreakdownSort,
} from "./dashboardModelBreakdown";
import { ModelPerformanceModelIdentity } from "./ModelPerformanceModelIdentity";

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
  };
}

type UsageBreakdownItem = Pick<
  UsageBreakdown,
  "cacheWriteTokens" | "cacheReadTokens" | "outputTokens" | "costs"
>;

interface BreakdownTableRow {
  key: string;
  label: ReactNode;
  compactLabel?: ReactNode;
  values: BreakdownTableValue[];
}

interface BreakdownTableValue {
  key: string;
  content: ReactNode;
}

type BreakdownTableColumn =
  | { key: "cache-hit-rate" | "total"; label: string; sortable: true }
  | { key: "cache-write" | "cache-read" | "output"; label: string; sortable: false };

function modelLabel(model: string, unknownModel: string) {
  return model === "unknown" ? unknownModel : model;
}

function groupKey(model: UsageBreakdownModel) {
  return `${model.model}\u0000${model.reasoningEffort?.trim() ?? ""}`;
}

function groupLabel(
  model: UsageBreakdownModel,
  labels: UsageBreakdownTooltipProps["labels"],
  detailed: boolean,
  compact = false,
) {
  const modelName = modelLabel(model.model, labels.unknownModel);

  if (detailed) {
    return (
      <ModelPerformanceModelIdentity
        model={modelName}
        effortValue={model.reasoningEffort}
        className="max-w-full"
        modelClassName="text-[12px] leading-4"
      />
    );
  }

  return (
    <span className="flex min-w-0 flex-col gap-0.5">
      {model.model === "unknown" ? (
        <span
          className={cn(
            "truncate font-normal text-base-content/80",
            compact && "text-[12px] leading-4",
          )}
        >
          {modelName}
        </span>
      ) : (
        <span
          className={cn("flex min-w-0 max-w-full items-center gap-1.5", compact && "gap-1")}
          title={modelName}
        >
          <span aria-hidden="true">
            <ModelIdentity
              model={modelName}
              className={compact ? "h-4 w-4" : "h-5 w-5"}
              iconClassName={compact ? "h-3.5 w-3.5" : "h-4 w-4"}
            />
          </span>
          <span
            className={cn(
              "min-w-0 truncate font-mono font-normal text-base-content/80",
              compact && "text-[12px] leading-4",
            )}
          >
            {modelName}
          </span>
        </span>
      )}
    </span>
  );
}

function BreakdownTable({
  title,
  columns,
  rows,
  modelLabel: modelColumnLabel,
  sort,
  onSort,
  simple,
}: {
  title: string;
  columns: readonly BreakdownTableColumn[];
  rows: readonly BreakdownTableRow[];
  modelLabel: string;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  simple: boolean;
}) {
  return (
    <>
      <div
        className="hidden overflow-x-hidden desktop:block"
        data-testid="usage-breakdown-table-scroll-region"
      >
        <table className="w-full table-fixed border-collapse text-[12px] leading-4">
          <caption className="sr-only">{title}</caption>
          <colgroup>
            <col className="w-[32%]" />
            {columns.map((column) => (
              <col key={column.key} className="w-[13.6%]" />
            ))}
          </colgroup>
          <thead className="border-y border-base-300/50 bg-base-200/45 text-[12px] font-semibold text-base-content/58">
            <tr>
              <ModelBreakdownModelHeader
                label={modelColumnLabel}
                sort={sort}
                onSort={onSort}
                simple={simple}
                className="px-1 py-1.5 text-[12px] leading-4"
              />
              {columns.map((column) =>
                column.sortable ? (
                  <ModelBreakdownMetricHeader
                    key={column.key}
                    label={column.label}
                    column={column.key}
                    sort={sort}
                    onSort={onSort}
                    className="px-1 py-1.5 text-[12px] leading-4"
                  />
                ) : (
                  <th
                    key={column.key}
                    scope="col"
                    className="min-w-0 border-l border-base-300/30 whitespace-nowrap px-1 py-1.5 text-right font-semibold"
                  >
                    {column.label}
                  </th>
                ),
              )}
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
                  className="min-w-0 max-w-full overflow-hidden px-2 py-1 text-left font-normal text-base-content/76 whitespace-nowrap"
                >
                  {row.label}
                </th>
                {row.values.map((value) => (
                  <td
                    key={`${row.key}:${value.key}`}
                    className="min-w-0 border-l border-base-300/30 px-1 py-1 text-right font-mono font-normal tabular-nums whitespace-nowrap"
                  >
                    {value.content}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="min-w-0 max-w-full desktop:hidden" data-testid="usage-breakdown-mobile-list">
        <div
          className="border-y border-base-300/50 bg-base-200/45 px-2 py-1.5"
          data-testid="usage-breakdown-mobile-controls"
        >
          <div
            className="flex min-w-0 flex-wrap items-center justify-between gap-x-2 gap-y-1"
            data-testid="usage-breakdown-mobile-sort-row"
          >
            <ModelBreakdownModelSortButton
              label={modelColumnLabel}
              sort={sort}
              onSort={onSort}
              simple={simple}
              className="min-w-0 flex-1 text-[12px] font-semibold text-base-content/58"
            />
            <div className="flex min-w-0 shrink-0 flex-wrap items-center justify-end gap-x-3 gap-y-1">
              {columns.map((column) =>
                column.sortable ? (
                  <ModelBreakdownMetricSortButton
                    key={column.key}
                    label={column.label}
                    column={column.key}
                    sort={sort}
                    onSort={onSort}
                    className="text-[12px] font-semibold text-base-content/58"
                  />
                ) : null,
              )}
            </div>
          </div>
        </div>
        <div className="divide-y divide-base-300/35">
          {rows.map((row, rowIndex) => (
            <section
              key={row.key}
              className={rowIndex === 0 ? "bg-base-100/45 px-2 py-1.5" : "px-2 py-1.5"}
            >
              <div className="min-w-0 max-w-full text-left font-normal text-[12px] leading-4 text-base-content/76">
                {row.compactLabel ?? row.label}
              </div>
              <dl className="mt-1 grid min-w-0 grid-cols-3 gap-x-2 gap-y-1.5">
                {columns.map((column) => {
                  const value = row.values.find((item) => item.key === column.key);
                  return (
                    <div
                      key={`${row.key}:${column.key}`}
                      className="min-w-0 border-t border-base-300/30 pt-1"
                    >
                      <dt className="min-w-0 truncate text-[12px] leading-4 text-base-content/58">
                        {column.label}
                      </dt>
                      <dd className="mt-0 min-w-0 text-right font-mono text-[12px] leading-4 font-normal tabular-nums">
                        {value?.content ?? "—"}
                      </dd>
                    </div>
                  );
                })}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </>
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
  sort,
  onSort,
  simple,
}: UsageBreakdownTooltipProps & {
  models: UsageBreakdown["models"];
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  simple: boolean;
}) {
  const columns = [
    { key: "cache-write" as const, label: labels.cacheWrite, sortable: false as const },
    { key: "cache-read" as const, label: labels.cacheRead, sortable: false as const },
    { key: "cache-hit-rate" as const, label: labels.cacheHitRate, sortable: true as const },
    { key: "output" as const, label: labels.output, sortable: false as const },
    { key: "total" as const, label: labels.total, sortable: true as const },
  ];
  const rowFor = (
    key: string,
    label: ReactNode,
    item: UsageBreakdownItem,
    compactLabel: ReactNode = label,
  ): BreakdownTableRow => ({
    key,
    label,
    compactLabel,
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
      sort={sort}
      onSort={onSort}
      simple={simple}
      rows={[
        rowFor("total", labels.total, breakdown),
        ...models.map((model) =>
          rowFor(
            groupKey(model),
            groupLabel(model, labels, !simple),
            model,
            groupLabel(model, labels, !simple, true),
          ),
        ),
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
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const [mode] = useDashboardModelBreakdownMode();
  const [sort, setSort] = useDashboardModelBreakdownSort("usage-breakdown");
  const effectiveSort = sortForMode(sort, mode);
  const groupedModels =
    mode === "simple" ? groupUsageBreakdownModels(breakdown.models) : breakdown.models;
  const models = sortUsageBreakdownModels(
    groupedModels.filter(
      (model) =>
        model.costs != null ||
        model.cacheWriteTokens > 0 ||
        model.cacheReadTokens > 0 ||
        model.outputTokens > 0,
    ),
    effectiveSort,
    localeTag,
  );

  return (
    <div data-testid="usage-breakdown-tooltip" className="min-w-0 max-w-full space-y-1.5">
      <div className="flex flex-wrap items-center justify-between gap-2 px-0.5">
        <div className="min-w-0 text-[12px] font-semibold leading-4 text-base-content/72">
          {title}
        </div>
        <div className="flex max-w-full flex-wrap items-center justify-end gap-2">
          <ModelBreakdownModeToggle />
        </div>
      </div>
      <UsageBreakdownTable
        title={title}
        breakdown={breakdown}
        models={models}
        formatNumber={formatNumber}
        formatRatio={formatRatio}
        formatCurrency={formatCurrency}
        labels={labels}
        sort={effectiveSort}
        onSort={setSort}
        simple={mode === "simple"}
      />
    </div>
  );
}
