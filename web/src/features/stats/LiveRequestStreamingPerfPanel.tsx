import { Alert } from "../../components/ui/alert";
import { useTranslation } from "../../i18n";
import type { LiveRequestStreamingCohortStats, LiveRequestStreamingPerf } from "../../lib/api";

const MIN_SUCCESS_SAMPLES = 200;

export interface LiveRequestStreamingPerfPanelProps {
  data: LiveRequestStreamingPerf | null;
  isLoading?: boolean;
  error?: string | null;
}

function findCohort(
  cohorts: LiveRequestStreamingCohortStats[],
  cohort: string,
  transportMode: string,
): LiveRequestStreamingCohortStats | undefined {
  return cohorts.find((item) => item.cohort === cohort && item.transportMode === transportMode);
}

function formatMs(value: number | undefined): string {
  return value == null || !Number.isFinite(value) ? "-" : `${Math.round(value)} ms`;
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatBenefit(control: number | undefined, treatment: number | undefined): string {
  if (
    control == null ||
    treatment == null ||
    !Number.isFinite(control) ||
    !Number.isFinite(treatment)
  ) {
    return "-";
  }
  const difference = control - treatment;
  const relative = control === 0 ? 0 : (difference / control) * 100;
  return `${difference >= 0 ? "+" : ""}${Math.round(difference)} ms (${relative >= 0 ? "+" : ""}${relative.toFixed(1)}%)`;
}

function CohortColumn({
  label,
  cohort,
}: {
  label: string;
  cohort?: LiveRequestStreamingCohortStats;
}) {
  const { t } = useTranslation();
  const sampleCount = cohort?.successSampleCount ?? 0;
  const sampleSufficient = cohort?.sufficientSamples ?? false;
  return (
    <div className="min-w-0 border-l border-base-300/70 pl-4 first:border-l-0 first:pl-0">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <div className="text-sm font-semibold text-base-content">{label}</div>
        <div className="font-mono text-xs text-base-content/65">n={sampleCount}</div>
      </div>
      {!sampleSufficient ? (
        <div className="mt-1 text-xs text-warning">
          {t("stats.liveRequestStreaming.insufficient", {
            count: sampleCount,
            minimum: MIN_SUCCESS_SAMPLES,
          })}
        </div>
      ) : null}
      <dl className="mt-3 grid grid-cols-1 gap-2 text-sm">
        <Metric
          label={t("stats.liveRequestStreaming.firstResponse")}
          value={formatMs(cohort?.firstResponseByteTotalMs?.p50Ms)}
        />
        <Metric
          label={t("stats.liveRequestStreaming.firstToken")}
          value={formatMs(cohort?.firstTokenMs?.p50Ms)}
        />
        <Metric
          label={t("stats.liveRequestStreaming.overlap")}
          value={formatMs(cohort?.requestUpstreamOverlapMs?.p50Ms)}
        />
        <Metric
          label={t("stats.liveRequestStreaming.retryRisk")}
          value={formatPercent(cohort?.fallbackOrRetryRate ?? 0)}
        />
      </dl>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-base-content/65">{label}</dt>
      <dd className="font-mono text-base-content">{value}</dd>
    </div>
  );
}

export function LiveRequestStreamingPerfPanel({
  data,
  isLoading = false,
  error = null,
}: LiveRequestStreamingPerfPanelProps) {
  const { t } = useTranslation();
  const control = findCohort(data?.cohorts ?? [], "control", "buffered");
  const treatment = findCohort(data?.cohorts ?? [], "treatment", "live_first");

  return (
    <section className="surface-panel" data-testid="live-request-streaming-perf-panel">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h3 className="section-title">{t("stats.liveRequestStreaming.title")}</h3>
            <p className="section-description">{t("stats.liveRequestStreaming.subtitle")}</p>
          </div>
          <div className="font-mono text-xs text-base-content/65">
            {t("stats.liveRequestStreaming.coverage", {
              rate: formatPercent(data?.coverage ?? 0),
              count: data?.measuredInvocationCount ?? 0,
              total: data?.responseInvocationCount ?? 0,
            })}
          </div>
        </div>
        {error ? <Alert variant="error">{error}</Alert> : null}
        {isLoading ? (
          <div
            className="h-32 animate-pulse bg-base-200/60"
            role="status"
            aria-label={t("chart.loading")}
          />
        ) : (
          <>
            <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
              <CohortColumn label={t("stats.liveRequestStreaming.control")} cohort={control} />
              <CohortColumn label={t("stats.liveRequestStreaming.treatment")} cohort={treatment} />
            </div>
            <dl className="grid grid-cols-1 gap-2 border-t border-base-300/70 pt-3 text-sm sm:grid-cols-3">
              <Metric
                label={t("stats.liveRequestStreaming.responseBenefit")}
                value={formatBenefit(
                  control?.firstResponseByteTotalMs?.p50Ms,
                  treatment?.firstResponseByteTotalMs?.p50Ms,
                )}
              />
              <Metric
                label={t("stats.liveRequestStreaming.tokenBenefit")}
                value={formatBenefit(control?.firstTokenMs?.p50Ms, treatment?.firstTokenMs?.p50Ms)}
              />
              <Metric
                label={t("stats.liveRequestStreaming.overlapBenefit")}
                value={formatBenefit(
                  control?.requestUpstreamOverlapMs?.p50Ms,
                  treatment?.requestUpstreamOverlapMs?.p50Ms,
                )}
              />
            </dl>
          </>
        )}
      </div>
    </section>
  );
}
