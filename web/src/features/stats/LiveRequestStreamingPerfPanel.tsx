import { Alert } from "../../components/ui/alert";
import { useTranslation } from "../../i18n";
import type {
  LiveRequestStreamingCohortStats,
  LiveRequestStreamingEvaluation,
  LiveRequestStreamingPerf,
  LiveRequestStreamingValuePercentiles,
} from "../../lib/api";

const MIN_SUCCESS_SAMPLES = 200;

export interface LiveRequestStreamingPerfPanelProps {
  data: LiveRequestStreamingPerf | null;
  evaluation?: LiveRequestStreamingEvaluation | null;
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

function formatPercent(value: number | undefined): string {
  return value == null || !Number.isFinite(value) ? "-" : `${(value * 100).toFixed(1)}%`;
}

function formatDistribution(
  value: LiveRequestStreamingValuePercentiles | undefined | null,
  suffix: string,
): string {
  if (!value) return "-";
  return `${Math.round(value.p50)} / ${Math.round(value.p90)} / ${Math.round(value.p99)} ${suffix}`;
}

function formatRatioDistribution(
  value: LiveRequestStreamingValuePercentiles | undefined | null,
): string {
  if (!value) return "-";
  return `${(value.p50 * 100).toFixed(1)} / ${(value.p90 * 100).toFixed(1)} / ${(value.p99 * 100).toFixed(1)}%`;
}

function formatBenefit(
  control: number | undefined,
  treatment: number | undefined,
  higherIsBetter = false,
): string {
  if (
    control == null ||
    treatment == null ||
    !Number.isFinite(control) ||
    !Number.isFinite(treatment)
  ) {
    return "-";
  }
  const difference = higherIsBetter ? treatment - control : control - treatment;
  const absolute = `${difference >= 0 ? "+" : ""}${Math.round(difference)} ms`;
  if (control === 0) return absolute;
  const relative = (difference / control) * 100;
  return `${absolute} (${relative >= 0 ? "+" : ""}${relative.toFixed(1)}%)`;
}

function formatEvaluationWindow(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatEvaluationInterval(
  value: LiveRequestStreamingEvaluation["metrics"]["firstResponse"],
): string {
  if (!value) return "-";
  return `${Math.round(value.p50DifferenceMs)} ms [${Math.round(value.lowerMs)}, ${Math.round(value.upperMs)}]`;
}

function formatRiskUpperBound(
  value: LiveRequestStreamingEvaluation["risk"]["firstAttemptFailure"],
): string {
  return value ? `${(value.upperBound * 100).toFixed(2)}%` : "-";
}

function evaluationStatusLabel(
  status: LiveRequestStreamingEvaluation["decision"]["status"],
  t: (key: string, values?: Record<string, string | number>) => string,
): string {
  switch (status) {
    case "recommend_keep":
      return t("stats.liveRequestStreaming.evaluationStatus.keep");
    case "recommend_remove":
      return t("stats.liveRequestStreaming.evaluationStatus.remove");
    case "insufficient_data":
      return t("stats.liveRequestStreaming.evaluationStatus.insufficient");
    case "review_required":
      return t("stats.liveRequestStreaming.evaluationStatus.review");
    default:
      return status;
  }
}

function EvaluationSummary({ evaluation }: { evaluation: LiveRequestStreamingEvaluation }) {
  const { t } = useTranslation();
  const statusClass =
    evaluation.decision.status === "recommend_keep"
      ? "text-success"
      : evaluation.decision.status === "recommend_remove"
        ? "text-error"
        : "text-warning";
  return (
    <div
      className="border-t border-base-300/70 pt-4"
      data-testid="live-request-streaming-evaluation"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-base-content">
            {t("stats.liveRequestStreaming.evaluationTitle")}
          </div>
          <div className="mt-1 text-xs text-base-content/60">
            {t("stats.liveRequestStreaming.evaluationWindow", {
              start: formatEvaluationWindow(evaluation.rangeStart),
              end: formatEvaluationWindow(evaluation.rangeEnd),
            })}
          </div>
        </div>
        <div className={`text-sm font-semibold ${statusClass}`}>
          {evaluationStatusLabel(evaluation.decision.status, t)}
        </div>
      </div>
      <dl className="mt-3 grid grid-cols-1 gap-2 text-sm sm:grid-cols-3">
        <Metric
          label={t("stats.liveRequestStreaming.evaluationAssignments")}
          value={String(evaluation.treatmentAssignmentCount)}
        />
        <Metric
          label={t("stats.liveRequestStreaming.evaluationActualRate")}
          value={`${evaluation.actualLiveFirstCount} (${formatPercent(evaluation.actualLiveFirstRate)})`}
        />
        <Metric
          label={t("stats.liveRequestStreaming.evaluationFallbacks")}
          value={String(evaluation.treatmentBufferedFallbackCount)}
        />
      </dl>
      <div className="mt-3 grid grid-cols-1 gap-2 text-xs text-base-content/70 sm:grid-cols-2">
        <div>
          <span className="font-semibold text-base-content/80">
            {t("stats.liveRequestStreaming.evaluationEvidence")}:{" "}
            {t("stats.liveRequestStreaming.evaluationMetric")}
          </span>
          <div className="mt-1 font-mono">
            {t("stats.liveRequestStreaming.evaluationResponse")}{" "}
            {formatEvaluationInterval(evaluation.metrics.firstResponse)} ·{" "}
            {t("stats.liveRequestStreaming.evaluationToken")}{" "}
            {formatEvaluationInterval(evaluation.metrics.firstToken)} ·{" "}
            {t("stats.liveRequestStreaming.evaluationOverlap")}{" "}
            {formatEvaluationInterval(evaluation.metrics.overlap)}
          </div>
        </div>
        <div>
          <span className="font-semibold text-base-content/80">
            {t("stats.liveRequestStreaming.evaluationRisk")}
          </span>
          <div className="mt-1 font-mono">
            {t("stats.liveRequestStreaming.evaluationFirstAttempt")}{" "}
            {formatRiskUpperBound(evaluation.risk.firstAttemptFailure)} ·{" "}
            {t("stats.liveRequestStreaming.evaluationFallbackRisk")}{" "}
            {formatRiskUpperBound(evaluation.risk.fallbackOrRetry)} ·{" "}
            {t("stats.liveRequestStreaming.evaluationCapture")}{" "}
            {formatRiskUpperBound(evaluation.risk.captureFailure)} ·{" "}
            {t("stats.liveRequestStreaming.evaluationDelivery")}{" "}
            {formatRiskUpperBound(evaluation.risk.ambiguousDelivery)}
          </div>
        </div>
      </div>
      <div className="mt-3 text-xs text-base-content/60">
        <span className="font-semibold text-base-content/80">
          {t("stats.liveRequestStreaming.evaluationReasonCodes")}:
        </span>{" "}
        <span className="font-mono">{evaluation.decision.reasonCodes.join(" · ") || "-"}</span>
      </div>
    </div>
  );
}

function CohortColumn({
  label,
  cohort,
  testId,
}: {
  label: string;
  cohort?: LiveRequestStreamingCohortStats;
  testId: string;
}) {
  const { t } = useTranslation();
  const successSampleCount = cohort?.successSampleCount ?? 0;
  const firstResponseSampleCount = cohort?.firstResponseByteSampleCount ?? successSampleCount;
  const firstTokenSampleCount = cohort?.firstTokenSampleCount ?? successSampleCount;
  const overlapSampleCount = cohort?.requestUpstreamOverlapSampleCount ?? successSampleCount;
  const fallbackReasons = Object.entries(cohort?.fallbackReasonCounts ?? {})
    .map(([reason, count]) => `${reason}: ${count}`)
    .join(" · ");
  const minimumMetricSampleCount = Math.min(
    firstResponseSampleCount,
    firstTokenSampleCount,
    overlapSampleCount,
  );
  const sampleSufficient = cohort?.sufficientSamples ?? false;
  return (
    <div
      className="min-w-0 border-l border-base-300/70 pl-4 first:border-l-0 first:pl-0"
      data-testid={testId}
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <div className="text-sm font-semibold text-base-content">{label}</div>
        <div className="font-mono text-xs text-base-content/65">
          n={cohort?.invocationCount ?? 0}
        </div>
      </div>
      {!sampleSufficient ? (
        <div className="mt-1 text-xs text-warning">
          {t("stats.liveRequestStreaming.metricCoverage", {
            count: minimumMetricSampleCount,
            minimum: MIN_SUCCESS_SAMPLES,
          })}
        </div>
      ) : null}
      <dl className="mt-3 grid grid-cols-1 gap-2 text-sm">
        <Metric
          label={t("stats.liveRequestStreaming.firstResponse")}
          value={formatMs(cohort?.firstResponseByteTotalMs?.p50Ms)}
          sampleCount={firstResponseSampleCount}
        />
        <Metric
          label={t("stats.liveRequestStreaming.firstToken")}
          value={formatMs(cohort?.firstTokenMs?.p50Ms)}
          sampleCount={firstTokenSampleCount}
        />
        <Metric
          label={t("stats.liveRequestStreaming.overlap")}
          value={formatMs(cohort?.requestUpstreamOverlapMs?.p50Ms)}
          sampleCount={overlapSampleCount}
        />
        <Metric
          label={t("stats.liveRequestStreaming.retryRisk")}
          value={formatPercent(cohort?.fallbackOrRetryRate)}
        />
      </dl>
      {fallbackReasons ? (
        <div className="mt-2 text-xs text-base-content/60">
          {t("stats.liveRequestStreaming.fallbackReasons")}: {fallbackReasons}
        </div>
      ) : null}
    </div>
  );
}

function Metric({
  label,
  value,
  sampleCount,
}: {
  label: string;
  value: string;
  sampleCount?: number;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="flex min-w-0 items-baseline gap-1 text-base-content/65">
        <span>{label}</span>
        {sampleCount != null ? (
          <span className="font-mono text-xs text-base-content/45">n={sampleCount}</span>
        ) : null}
      </dt>
      <dd className="font-mono text-base-content">{value}</dd>
    </div>
  );
}

export function LiveRequestStreamingPerfPanel({
  data,
  evaluation = null,
  isLoading = false,
  error = null,
}: LiveRequestStreamingPerfPanelProps) {
  const { t } = useTranslation();
  const control = findCohort(data?.cohorts ?? [], "control", "buffered");
  const treatment = findCohort(data?.cohorts ?? [], "treatment", "live_first");
  const treatmentFallback = findCohort(data?.cohorts ?? [], "treatment", "buffered");
  const metricHasEnoughSamples = (
    cohort: LiveRequestStreamingCohortStats | undefined,
    metric:
      | "firstResponseByteSampleCount"
      | "firstTokenSampleCount"
      | "requestUpstreamOverlapSampleCount",
  ) => (cohort?.[metric] ?? cohort?.successSampleCount ?? 0) >= MIN_SUCCESS_SAMPLES;
  const responseBenefitReady =
    metricHasEnoughSamples(control, "firstResponseByteSampleCount") &&
    metricHasEnoughSamples(treatment, "firstResponseByteSampleCount");
  const tokenBenefitReady =
    metricHasEnoughSamples(control, "firstTokenSampleCount") &&
    metricHasEnoughSamples(treatment, "firstTokenSampleCount");
  const overlapBenefitReady =
    metricHasEnoughSamples(control, "requestUpstreamOverlapSampleCount") &&
    metricHasEnoughSamples(treatment, "requestUpstreamOverlapSampleCount");
  const routeFinalization = data?.routeFinalization;
  const routeFactors = Object.entries(routeFinalization?.dependencyFactorCounts ?? {})
    .map(([factor, count]) => `${factor}: ${count}`)
    .join(" · ");
  const routeOutcomes = Object.entries(routeFinalization?.outcomeCounts ?? {})
    .map(([outcome, count]) => `${outcome}: ${count}`)
    .join(" · ");
  const noEffectiveLiveFirst = (treatment?.invocationCount ?? 0) === 0;
  const treatmentFallbackCount = treatmentFallback?.invocationCount ?? 0;
  const cohortGridClass = treatmentFallback
    ? "grid grid-cols-1 gap-5 lg:grid-cols-3"
    : "grid grid-cols-1 gap-5 lg:grid-cols-2";

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
            {noEffectiveLiveFirst ? (
              <Alert variant="warning">
                {t("stats.liveRequestStreaming.noEffectiveTreatment", {
                  count: treatmentFallbackCount,
                })}
              </Alert>
            ) : treatmentFallbackCount > 0 ? (
              <Alert variant="warning">
                {t("stats.liveRequestStreaming.treatmentFallbackNotice", {
                  count: treatmentFallbackCount,
                })}
              </Alert>
            ) : null}
            <div className={cohortGridClass}>
              <CohortColumn
                label={t("stats.liveRequestStreaming.control")}
                cohort={control}
                testId="live-request-streaming-cohort-control"
              />
              <CohortColumn
                label={t("stats.liveRequestStreaming.treatment")}
                cohort={treatment}
                testId="live-request-streaming-cohort-treatment"
              />
              {treatmentFallback ? (
                <CohortColumn
                  label={t("stats.liveRequestStreaming.treatmentFallback")}
                  cohort={treatmentFallback}
                  testId="live-request-streaming-cohort-treatment-fallback"
                />
              ) : null}
            </div>
            <dl className="grid grid-cols-1 gap-2 border-t border-base-300/70 pt-3 text-sm sm:grid-cols-3">
              <Metric
                label={t("stats.liveRequestStreaming.responseBenefit")}
                value={
                  responseBenefitReady
                    ? formatBenefit(
                        control?.firstResponseByteTotalMs?.p50Ms,
                        treatment?.firstResponseByteTotalMs?.p50Ms,
                      )
                    : "-"
                }
              />
              <Metric
                label={t("stats.liveRequestStreaming.tokenBenefit")}
                value={
                  tokenBenefitReady
                    ? formatBenefit(control?.firstTokenMs?.p50Ms, treatment?.firstTokenMs?.p50Ms)
                    : "-"
                }
              />
              <Metric
                label={t("stats.liveRequestStreaming.overlapBenefit")}
                value={
                  overlapBenefitReady
                    ? formatBenefit(
                        control?.requestUpstreamOverlapMs?.p50Ms,
                        treatment?.requestUpstreamOverlapMs?.p50Ms,
                        true,
                      )
                    : "-"
                }
              />
            </dl>
            <div className="border-t border-base-300/70 pt-3">
              <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <div className="text-sm font-semibold text-base-content">
                  {t("stats.liveRequestStreaming.routeFinalization")}
                </div>
                <div className="font-mono text-xs text-base-content/65">
                  n={routeFinalization?.sampleCount ?? 0}
                </div>
              </div>
              {!routeFinalization?.sufficientSamples ? (
                <div className="mt-1 text-xs text-warning">
                  {t("stats.liveRequestStreaming.insufficient", {
                    count: routeFinalization?.sampleCount ?? 0,
                    minimum: MIN_SUCCESS_SAMPLES,
                  })}
                </div>
              ) : null}
              <dl className="mt-3 grid grid-cols-1 gap-2 text-sm sm:grid-cols-2 xl:grid-cols-3">
                <Metric
                  label={t("stats.liveRequestStreaming.routeRawBytes")}
                  value={formatDistribution(routeFinalization?.rawBytes, "B")}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeLogicalBytes")}
                  value={formatDistribution(routeFinalization?.logicalBytes, "B")}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeRawRatio")}
                  value={formatRatioDistribution(routeFinalization?.rawRatio)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeLogicalRatio")}
                  value={formatRatioDistribution(routeFinalization?.logicalRatio)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeFinalizationMs")}
                  value={formatMs(routeFinalization?.finalizationMs?.p50Ms)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeEofBuffered")}
                  value={formatPercent(routeFinalization?.eofFinalizedRate)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeConservativeBuffered")}
                  value={formatPercent(routeFinalization?.conservativeBufferedRate)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeCacheHit")}
                  value={formatPercent(routeFinalization?.hotCacheHitRate)}
                />
                <Metric
                  label={t("stats.liveRequestStreaming.routeColdLoad")}
                  value={formatPercent(routeFinalization?.coldLoadRate)}
                />
              </dl>
              {routeOutcomes ? (
                <div className="mt-2 text-xs text-base-content/60">
                  {t("stats.liveRequestStreaming.routeOutcomes")}: {routeOutcomes}
                </div>
              ) : null}
              {routeFactors ? (
                <div className="mt-2 text-xs text-base-content/60">{routeFactors}</div>
              ) : null}
            </div>
            {evaluation ? <EvaluationSummary evaluation={evaluation} /> : null}
          </>
        )}
      </div>
    </section>
  );
}
