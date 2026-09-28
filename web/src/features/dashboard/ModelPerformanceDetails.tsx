import type { ReactNode } from "react";
import { useTranslation } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { cn } from "../../lib/utils";
import { ModelIdentity } from "../shared/ModelIdentity";
import {
  ModelBreakdownMetricGridButton,
  ModelBreakdownMetricHeader,
  ModelBreakdownModelHeader,
  ModelBreakdownModelSortButton,
  ModelBreakdownModeToggle,
} from "./DashboardModelBreakdownControls";
import {
  sortForMode,
  sortModelPerformanceModels,
  useDashboardModelBreakdownMode,
  useDashboardModelBreakdownSort,
} from "./dashboardModelBreakdown";
import { ModelPerformanceModelIdentity } from "./ModelPerformanceModelIdentity";

const METRIC_CELL_KEYS = [
  "tpm",
  "streaming-rate",
  "response",
  "first-byte",
  "wall-clock-duration",
  "cumulative-duration",
  "parallelism",
] as const;

const METRIC_LABEL_KEYS = [
  "tpm",
  "streamingRate",
  "response",
  "firstByte",
  "wallClockDuration",
  "cumulativeDuration",
  "parallelism",
] as const;

export interface ModelPerformanceDetailsProps {
  title: string;
  performance: ModelPerformance;
  presentation?: "tooltip" | "drawer";
}

function formatNumber(value: number, localeTag: string, maximumFractionDigits = 0) {
  return new Intl.NumberFormat(localeTag, { maximumFractionDigits }).format(value);
}

function formatDuration(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return "—";
  const milliseconds = Math.max(0, value);
  if (milliseconds < 1_000) return `${formatNumber(milliseconds, localeTag, 0)} ms`;
  if (milliseconds < 60_000) return `${formatNumber(milliseconds / 1_000, localeTag, 2)} s`;
  if (milliseconds < 3_600_000) return `${formatNumber(milliseconds / 60_000, localeTag, 1)} min`;
  let hours = Math.floor(milliseconds / 3_600_000);
  let minutes = Math.round((milliseconds % 3_600_000) / 60_000);
  if (minutes === 60) {
    hours += 1;
    minutes = 0;
  }
  return minutes > 0 ? `${hours} h ${minutes} min` : `${hours} h`;
}

function formatParallelism(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value) || value <= 0) return "—";
  return `x${new Intl.NumberFormat(localeTag, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value)}`;
}

export function ModelPerformanceDetails({
  title,
  performance,
  presentation = "tooltip",
}: ModelPerformanceDetailsProps) {
  const { locale, t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const [mode] = useDashboardModelBreakdownMode();
  const [sort, setSort] = useDashboardModelBreakdownSort("model-performance");
  const effectiveSort = sortForMode(sort, mode);
  const modelRows = sortModelPerformanceModels(
    mode === "simple" ? (performance.modelGroups ?? performance.models) : performance.models,
    effectiveSort,
    localeTag,
  );
  const labels = {
    tpm: t("dashboard.modelPerformance.columns.tpm"),
    streamingRate: t("dashboard.modelPerformance.columns.streamingRate"),
    response: t("dashboard.modelPerformance.columns.response"),
    firstByte: t("dashboard.modelPerformance.columns.firstByte"),
    wallClockDuration: t("dashboard.modelPerformance.columns.wallClockDuration"),
    cumulativeDuration: t("dashboard.modelPerformance.columns.cumulativeDuration"),
    parallelism: t("dashboard.modelPerformance.columns.parallelism"),
  };
  const valuesFor = (metrics: ModelPerformance["total"]) => [
    formatNumber(metrics.tokensPerMinute, localeTag, 0),
    metrics.streamingResponseRate == null
      ? "—"
      : `${formatNumber(metrics.streamingResponseRate, localeTag, 2)} tok/s`,
    formatDuration(metrics.avgResponseMs, localeTag),
    formatDuration(metrics.avgFirstTokenMs, localeTag),
    formatDuration(metrics.wallClockUsageDurationMs, localeTag),
    formatDuration(metrics.cumulativeUsageDurationMs, localeTag),
    formatParallelism(metrics.parallelism, localeTag),
  ];

  if (!performance.available) {
    return (
      <div className="space-y-2" data-testid="model-performance-unavailable">
        <p className="font-semibold text-base-content">{title}</p>
        <p className="text-xs leading-5 text-base-content/70">
          {t("dashboard.modelPerformance.unavailable")}
        </p>
      </div>
    );
  }

  if (modelRows.length === 0) {
    return (
      <div className="space-y-2" data-testid="model-performance-empty">
        <p className="font-semibold text-base-content">{title}</p>
        <p className="text-xs leading-5 text-base-content/70">
          {t("dashboard.modelPerformance.empty")}
        </p>
      </div>
    );
  }

  if (presentation === "drawer") {
    return (
      <div className="space-y-4" data-testid="model-performance-drawer-content">
        <div className="space-y-2">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="min-w-0 flex-1 space-y-1.5">
              <p className="text-sm leading-6 text-base-content/70">
                {mode === "simple"
                  ? t("dashboard.modelPerformance.descriptionSimple")
                  : t("dashboard.modelPerformance.description")}
              </p>
              <p className="text-xs leading-5 text-base-content/58">
                {t("dashboard.modelPerformance.overlapNote")}
              </p>
            </div>
            <div className="flex max-w-full flex-wrap items-center justify-end gap-2">
              <ModelBreakdownModelSortButton
                label={
                  mode === "simple"
                    ? t("dashboard.modelPerformance.modelSimple")
                    : t("dashboard.modelPerformance.model")
                }
                sort={effectiveSort}
                onSort={setSort}
                simple={mode === "simple"}
              />
              <ModelBreakdownModeToggle />
            </div>
          </div>
        </div>
        <ModelPerformanceMetricGrid
          label={t("dashboard.modelPerformance.total")}
          values={valuesFor(performance.total)}
          labels={labels}
          sort={effectiveSort}
          onSort={setSort}
        />
        {modelRows.map((model) => (
          <section
            key={`${model.model}:${model.reasoningEffort ?? ""}`}
            className="border-t border-base-300/70 pt-3.5 first:border-t-0 first:pt-0"
          >
            <ModelPerformanceRowIdentity
              model={model.model}
              reasoningEffort={model.reasoningEffort}
              detailed={mode === "detailed"}
              className="w-full"
              modelClassName="text-sm font-semibold text-base-content"
              testId="model-performance-drawer-model-context"
            />
            <div className="mt-3">
              <ModelPerformanceMetricGrid
                values={valuesFor(model)}
                labels={labels}
                sort={effectiveSort}
                onSort={setSort}
              />
            </div>
          </section>
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-2" data-testid="model-performance-tooltip-content">
      <div>
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <p className="font-semibold text-base-content">{title}</p>
            <p className="mt-0.5 text-xs leading-4 text-base-content/65">
              {mode === "simple"
                ? t("dashboard.modelPerformance.descriptionSimple")
                : t("dashboard.modelPerformance.description")}
            </p>
            <p className="mt-1 text-xs leading-4 text-base-content/55">
              {t("dashboard.modelPerformance.overlapNote")}
            </p>
          </div>
          <div className="flex max-w-full flex-wrap items-center justify-end gap-2">
            <ModelBreakdownModeToggle />
          </div>
        </div>
      </div>
      <div
        className="max-h-[min(28rem,calc(100dvh-8rem))] overflow-x-hidden overflow-y-auto"
        data-testid="model-performance-table-scroll-region"
      >
        <table className="w-full table-fixed border-collapse text-xs leading-4">
          <caption className="sr-only">{title}</caption>
          <colgroup>
            <col className="w-[26%]" />
            <col span={7} />
          </colgroup>
          <thead className="border-y border-base-300/55 bg-base-200/55 text-xs text-base-content/60">
            <tr>
              <ModelBreakdownModelHeader
                label={
                  mode === "simple"
                    ? t("dashboard.modelPerformance.modelSimple")
                    : t("dashboard.modelPerformance.model")
                }
                sort={effectiveSort}
                onSort={setSort}
                simple={mode === "simple"}
              />
              <ModelBreakdownMetricHeader
                label={labels.tpm}
                column="tpm"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.streamingRate}
                column="streaming-rate"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.response}
                column="response"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.firstByte}
                column="first-byte"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.wallClockDuration}
                column="wall-clock-duration"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.cumulativeDuration}
                column="cumulative-duration"
                sort={effectiveSort}
                onSort={setSort}
              />
              <ModelBreakdownMetricHeader
                label={labels.parallelism}
                column="parallelism"
                sort={effectiveSort}
                onSort={setSort}
              />
            </tr>
          </thead>
          <tbody>
            <ModelPerformanceTableRow
              label={t("dashboard.modelPerformance.total")}
              values={valuesFor(performance.total)}
              emphasized
            />
            {modelRows.map((model) => (
              <ModelPerformanceTableRow
                key={`${model.model}:${model.reasoningEffort ?? ""}`}
                label={
                  <ModelPerformanceRowIdentity
                    model={model.model}
                    reasoningEffort={model.reasoningEffort}
                    detailed={mode === "detailed"}
                    className="w-full"
                    modelClassName="font-semibold text-base-content/85"
                    testId="model-performance-table-model-context"
                  />
                }
                values={valuesFor(model)}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ModelPerformanceRowIdentity({
  model,
  reasoningEffort,
  detailed,
  className,
  modelClassName,
  testId,
}: {
  model: string;
  reasoningEffort?: string | null;
  detailed: boolean;
  className?: string;
  modelClassName?: string;
  testId?: string;
}) {
  if (detailed) {
    return (
      <ModelPerformanceModelIdentity
        model={model}
        effortValue={reasoningEffort}
        className={className}
        modelClassName={modelClassName}
        testId={testId}
      />
    );
  }

  return (
    <span
      data-testid={testId}
      data-model-context-display="model-only"
      className={cn("flex min-w-0 max-w-full items-center gap-1.5", className)}
      title={model}
    >
      <span aria-hidden="true">
        <ModelIdentity
          model={model}
          className="h-5 w-5 max-w-full justify-start"
          iconClassName="h-3.5 w-3.5"
        />
      </span>
      <span className={cn("min-w-0 truncate font-mono", modelClassName)}>{model}</span>
    </span>
  );
}

function ModelPerformanceTableRow({
  label,
  values,
  emphasized = false,
}: {
  label: ReactNode;
  values: string[];
  emphasized?: boolean;
}) {
  return (
    <tr
      className={cn("border-b border-base-300/35 last:border-b-0", emphasized && "bg-base-100/50")}
    >
      <th
        scope="row"
        className="overflow-hidden px-2 py-1.5 text-left font-medium text-base-content/80 whitespace-nowrap"
      >
        {label}
      </th>
      {values.map((value, index) => (
        <td
          key={METRIC_CELL_KEYS[index]}
          className="overflow-hidden border-l border-base-300/30 px-1.5 py-1.5 text-right font-mono font-semibold text-ellipsis tabular-nums text-base-content whitespace-nowrap"
          title={value}
        >
          {value}
        </td>
      ))}
    </tr>
  );
}

function ModelPerformanceMetricGrid({
  label,
  values,
  labels,
  sort,
  onSort,
}: {
  label?: string;
  values: string[];
  labels: Record<string, string>;
  sort: Parameters<typeof ModelBreakdownMetricGridButton>[0]["sort"];
  onSort: Parameters<typeof ModelBreakdownMetricGridButton>[0]["onSort"];
}) {
  const entries = METRIC_CELL_KEYS.map((column, index) => ({
    column,
    label: labels[METRIC_LABEL_KEYS[index] ?? ""],
    value: values[index] ?? "—",
  }));
  return (
    <div className="space-y-2">
      {label ? <p className="text-xs font-semibold text-base-content/78">{label}</p> : null}
      <dl className="grid grid-cols-2 gap-x-5 gap-y-2.5">
        {entries.map((entry) => (
          <div key={entry.label} className="min-w-0">
            <dt className="text-xs leading-4 text-base-content/60">
              <ModelBreakdownMetricGridButton
                label={entry.label}
                column={entry.column}
                sort={sort}
                onSort={onSort}
              />
            </dt>
            <dd className="mt-0.5 truncate font-mono text-sm font-semibold tabular-nums text-base-content">
              {entry.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
