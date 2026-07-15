import type { ReactNode } from "react";
import { useTranslation } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { cn } from "../../lib/utils";

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

function effortLabel(
  effort: string | null | undefined,
  labels: Record<string, string>,
  unspecified: string,
) {
  const normalized = effort?.trim().toLowerCase();
  if (!normalized) return unspecified;
  return labels[normalized] ?? effort?.trim() ?? unspecified;
}

export function ModelPerformanceDetails({
  title,
  performance,
  presentation = "tooltip",
}: ModelPerformanceDetailsProps) {
  const { locale, t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const labels = {
    tpm: t("dashboard.modelPerformance.columns.tpm"),
    streamingRate: t("dashboard.modelPerformance.columns.streamingRate"),
    response: t("dashboard.modelPerformance.columns.response"),
    firstByte: t("dashboard.modelPerformance.columns.firstByte"),
    wallClockDuration: t("dashboard.modelPerformance.columns.wallClockDuration"),
    cumulativeDuration: t("dashboard.modelPerformance.columns.cumulativeDuration"),
    parallelism: t("dashboard.modelPerformance.columns.parallelism"),
  };
  const effortLabels = {
    none: t("dashboard.modelPerformance.effort.none"),
    minimal: t("dashboard.modelPerformance.effort.minimal"),
    low: t("dashboard.modelPerformance.effort.low"),
    medium: t("dashboard.modelPerformance.effort.medium"),
    high: t("dashboard.modelPerformance.effort.high"),
    xhigh: t("dashboard.modelPerformance.effort.xhigh"),
  };
  const valuesFor = (metrics: ModelPerformance["total"]) => [
    formatNumber(metrics.tokensPerMinute, localeTag, 0),
    metrics.streamingResponseRate == null
      ? "—"
      : `${formatNumber(metrics.streamingResponseRate, localeTag, 2)} tok/s`,
    formatDuration(metrics.avgResponseMs, localeTag),
    formatDuration(metrics.avgFirstResponseByteTotalMs, localeTag),
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

  if (performance.models.length === 0) {
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
        <div className="space-y-1.5">
          <p className="text-sm leading-6 text-base-content/70">
            {t("dashboard.modelPerformance.description")}
          </p>
          <p className="text-xs leading-5 text-base-content/58">
            {t("dashboard.modelPerformance.overlapNote")}
          </p>
        </div>
        <ModelPerformanceMetricGrid
          label={t("dashboard.modelPerformance.total")}
          values={valuesFor(performance.total)}
          labels={labels}
        />
        {performance.models.map((model) => (
          <section
            key={`${model.model}:${model.reasoningEffort ?? ""}`}
            className="border-t border-base-300/70 pt-3.5 first:border-t-0 first:pt-0"
          >
            <div className="min-w-0">
              <p className="break-all font-mono text-sm font-semibold text-base-content">
                {model.model}
              </p>
              <p className="mt-1 text-xs text-base-content/62">
                {t("dashboard.modelPerformance.reasoningEffort")}:{" "}
                {effortLabel(
                  model.reasoningEffort,
                  effortLabels,
                  t("dashboard.modelPerformance.effort.unspecified"),
                )}
              </p>
            </div>
            <div className="mt-3">
              <ModelPerformanceMetricGrid values={valuesFor(model)} labels={labels} />
            </div>
          </section>
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-2" data-testid="model-performance-tooltip-content">
      <div>
        <p className="font-semibold text-base-content">{title}</p>
        <p className="mt-0.5 text-[11px] leading-4 text-base-content/65">
          {t("dashboard.modelPerformance.description")}
        </p>
        <p className="mt-1 text-[11px] leading-4 text-base-content/55">
          {t("dashboard.modelPerformance.overlapNote")}
        </p>
      </div>
      <div className="max-h-[min(28rem,calc(100dvh-8rem))] overflow-auto">
        <table className="w-full min-w-[60rem] table-fixed border-collapse text-[10px] leading-4 sm:text-[11px]">
          <caption className="sr-only">{title}</caption>
          <thead className="border-y border-base-300/55 bg-base-200/55 text-[9px] text-base-content/60">
            <tr>
              <th scope="col" className="w-[19%] px-2 py-2 text-left font-semibold">
                {t("dashboard.modelPerformance.model")}
              </th>
              {[
                labels.tpm,
                labels.streamingRate,
                labels.response,
                labels.firstByte,
                labels.wallClockDuration,
                labels.cumulativeDuration,
                labels.parallelism,
              ].map((label) => (
                <th
                  key={label}
                  scope="col"
                  className="border-l border-base-300/35 px-1.5 py-2 text-right font-semibold"
                >
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            <ModelPerformanceTableRow
              label={t("dashboard.modelPerformance.total")}
              values={valuesFor(performance.total)}
              emphasized
            />
            {performance.models.map((model) => (
              <ModelPerformanceTableRow
                key={`${model.model}:${model.reasoningEffort ?? ""}`}
                label={
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="break-all font-mono font-semibold text-base-content/85">
                      {model.model}
                    </span>
                    <span className="text-[9px] font-normal leading-3 text-base-content/58">
                      {t("dashboard.modelPerformance.reasoningEffort")}:{" "}
                      {effortLabel(
                        model.reasoningEffort,
                        effortLabels,
                        t("dashboard.modelPerformance.effort.unspecified"),
                      )}
                    </span>
                  </span>
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
      <th scope="row" className="px-2 py-2 text-left font-medium text-base-content/80">
        {label}
      </th>
      {values.map((value, index) => (
        <td
          key={`${value}:${index}`}
          className="border-l border-base-300/30 px-1.5 py-2 text-right font-mono font-semibold tabular-nums text-base-content whitespace-nowrap"
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
}: {
  label?: string;
  values: string[];
  labels: Record<string, string>;
}) {
  const entries = Object.values(labels).map((metricLabel, index) => ({
    label: metricLabel,
    value: values[index] ?? "—",
  }));
  return (
    <div className="space-y-2">
      {label ? <p className="text-xs font-semibold text-base-content/78">{label}</p> : null}
      <dl className="grid grid-cols-2 gap-x-5 gap-y-2.5">
        {entries.map((entry) => (
          <div key={entry.label} className="min-w-0">
            <dt className="text-[11px] leading-4 text-base-content/60">{entry.label}</dt>
            <dd className="mt-0.5 truncate font-mono text-sm font-semibold tabular-nums text-base-content">
              {entry.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
