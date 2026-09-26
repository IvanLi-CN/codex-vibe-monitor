import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { useTranslation } from "../../i18n";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import type {
  DashboardModelBreakdownMode,
  DashboardModelBreakdownSort,
  DashboardSortDirection,
} from "./dashboardModelBreakdown";
import {
  modelSortRuleDirection,
  nextModelSortRule,
  useDashboardModelBreakdownMode,
} from "./dashboardModelBreakdown";

export function ModelBreakdownModeToggle() {
  const { t } = useTranslation();
  const [mode, setMode] = useDashboardModelBreakdownMode();
  const options: Array<{ value: DashboardModelBreakdownMode; label: string }> = [
    { value: "simple", label: t("dashboard.modelBreakdown.simple") },
    { value: "detailed", label: t("dashboard.modelBreakdown.detailed") },
  ];

  return (
    <SegmentedControl
      size="compact"
      role="tablist"
      aria-label={t("dashboard.modelBreakdown.modeLabel")}
      data-testid="dashboard-model-breakdown-mode-toggle"
      className="shrink-0"
    >
      {options.map((option) => (
        <SegmentedControlItem
          key={option.value}
          role="tab"
          aria-selected={mode === option.value}
          active={mode === option.value}
          onClick={() => setMode(option.value)}
          data-testid={`dashboard-model-breakdown-mode-${option.value}`}
        >
          {option.label}
        </SegmentedControlItem>
      ))}
    </SegmentedControl>
  );
}

export function ModelBreakdownSortIndicator({
  active,
  direction,
}: {
  active: boolean;
  direction: DashboardSortDirection;
}) {
  return (
    <AppIcon
      name={
        active && direction === "asc"
          ? "arrow-up-bold"
          : active
            ? "arrow-down-bold"
            : "sort-variant"
      }
      className="h-3.5 w-3.5 shrink-0"
      aria-hidden="true"
    />
  );
}

function modelSortLabel(
  t: ReturnType<typeof useTranslation>["t"],
  sort: DashboardModelBreakdownSort,
) {
  if (sort.column === "model") {
    const key = `dashboard.modelBreakdown.sort.${sort.rule}`;
    return t(key);
  }
  return t("dashboard.modelBreakdown.sort.name-asc");
}

export function ModelBreakdownModelSortButton({
  label,
  sort,
  onSort,
  simple,
  className,
}: {
  label: string;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  simple: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const active = sort.column === "model";
  const direction = active ? modelSortRuleDirection(sort.rule) : "desc";
  const current = active ? modelSortLabel(t, sort) : label;

  return (
    <button
      type="button"
      className={cn(
        "inline-flex max-w-full items-center gap-1 whitespace-nowrap text-left hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
        className,
      )}
      aria-label={t("dashboard.modelBreakdown.sort.modelAria", { current })}
      title={t("dashboard.modelBreakdown.sort.modelAria", { current })}
      onClick={() =>
        onSort({
          column: "model",
          rule: nextModelSortRule(active ? sort.rule : null, simple),
        })
      }
      data-testid="dashboard-model-breakdown-model-sort"
    >
      <span className="truncate">{label}</span>
      <ModelBreakdownSortIndicator active={active} direction={direction} />
    </button>
  );
}

export function ModelBreakdownModelHeader({
  label,
  sort,
  onSort,
  simple,
  className,
}: {
  label: string;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  simple: boolean;
  className?: string;
}) {
  const active = sort.column === "model";
  const direction = active ? modelSortRuleDirection(sort.rule) : "desc";
  return (
    <th
      scope="col"
      className={cn("w-[19%] whitespace-nowrap px-2 py-2 text-left font-semibold", className)}
      aria-sort={active ? (direction === "asc" ? "ascending" : "descending") : "none"}
    >
      <ModelBreakdownModelSortButton label={label} sort={sort} onSort={onSort} simple={simple} />
    </th>
  );
}

export function ModelBreakdownMetricSortButton({
  label,
  column,
  sort,
  onSort,
  className,
}: {
  label: string;
  column: Exclude<DashboardModelBreakdownSort["column"], "model">;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const active = sort.column === column;
  const direction = active ? sort.direction : "desc";
  return (
    <button
      type="button"
      className={cn(
        "inline-flex max-w-full items-center justify-end gap-1 whitespace-nowrap text-right hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
        className,
      )}
      aria-label={t("dashboard.modelBreakdown.sort.metricAria", { metric: label, direction })}
      title={t("dashboard.modelBreakdown.sort.metricAria", { metric: label, direction })}
      onClick={() =>
        onSort({
          column,
          direction: active && sort.direction === "desc" ? "asc" : "desc",
        })
      }
      data-testid={`dashboard-model-breakdown-sort-${column}`}
    >
      <span>{label}</span>
      <ModelBreakdownSortIndicator active={active} direction={direction} />
    </button>
  );
}

export function ModelBreakdownMetricHeader({
  label,
  column,
  sort,
  onSort,
  className,
}: {
  label: string;
  column: Exclude<DashboardModelBreakdownSort["column"], "model">;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
  className?: string;
}) {
  const active = sort.column === column;
  const direction = active ? sort.direction : "desc";
  return (
    <th
      scope="col"
      className={cn(
        "border-l border-base-300/35 whitespace-nowrap px-1.5 py-2 text-right font-semibold",
        className,
      )}
      aria-sort={active ? (direction === "asc" ? "ascending" : "descending") : "none"}
    >
      <ModelBreakdownMetricSortButton label={label} column={column} sort={sort} onSort={onSort} />
    </th>
  );
}

/*
 * Non-table views, such as the compact performance drawer, still need a
 * direct sorting affordance without introducing a second dropdown control.
 */
export function ModelBreakdownMetricGridButton({
  label,
  column,
  sort,
  onSort,
}: {
  label: string;
  column: Exclude<DashboardModelBreakdownSort["column"], "model">;
  sort: DashboardModelBreakdownSort;
  onSort: (nextSort: DashboardModelBreakdownSort) => void;
}) {
  return (
    <ModelBreakdownMetricSortButton
      label={label}
      column={column}
      sort={sort}
      onSort={onSort}
      className="w-full justify-start text-left text-xs font-semibold text-base-content/60"
    />
  );
}
