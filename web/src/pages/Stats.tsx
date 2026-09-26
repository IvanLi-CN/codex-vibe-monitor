import { useEffect, useMemo, useState } from "react";
import { Alert } from "../components/ui/alert";
import { SelectField } from "../components/ui/select-field";
import { ErrorReasonDistribution } from "../features/stats/ErrorReasonDistribution";
import { LongTermStatsSection } from "../features/stats/LongTermStatsSection";
import { ParallelWorkStatsSection } from "../features/stats/ParallelWorkStatsSection";
import { StatsCards } from "../features/stats/StatsCards";
import { SuccessFailureChart } from "../features/stats/SuccessFailureChart";
import { TimeseriesChart } from "../features/stats/TimeseriesChart";
import { useErrorDistribution } from "../hooks/useErrorDistribution";
import { useFailureSummary } from "../hooks/useFailureSummary";
import { useParallelWorkStats } from "../hooks/useParallelWorkStats";
import { useSummary } from "../hooks/useStats";
import { useTimeseries } from "../hooks/useTimeseries";
import { useTranslation } from "../i18n";
import type { FailureScope } from "../lib/api";
import { RANGE_OPTIONS, resolveStatsBucketOptions, resolveStatsBucketValue } from "./stats-options";

export default function StatsPage() {
  const { locale, t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }),
    [localeTag],
  );
  const [range, setRange] = useState<(typeof RANGE_OPTIONS)[number]["value"]>("today");
  const [errorScope, setErrorScope] = useState<FailureScope>("service");
  const [bucket, setBucket] = useState<string>("15m");

  const requestedBucketOptions = useMemo(() => resolveStatsBucketOptions(range), [range]);
  const requestedBucket = useMemo(
    () => resolveStatsBucketValue(bucket, requestedBucketOptions),
    [bucket, requestedBucketOptions],
  );

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  );

  const { summary, isLoading: summaryLoading, error: summaryError } = useSummary(range);

  const {
    data: timeseries,
    isLoading: timeseriesLoading,
    error: timeseriesError,
  } = useTimeseries(range, {
    bucket: requestedBucket,
    preferServerAggregation: true,
  });

  const rawBucketOptions = useMemo(
    () => resolveStatsBucketOptions(range, timeseries?.availableBuckets),
    [range, timeseries?.availableBuckets],
  );
  const effectiveBucket = useMemo(() => {
    const serverBucket = timeseries?.effectiveBucket;
    const requestedStillAllowed = rawBucketOptions.some(
      (option) => option.value === requestedBucket,
    );
    // Keep the user's in-flight selection visible until the server explicitly narrows it.
    if (requestedStillAllowed) {
      return requestedBucket;
    }
    return resolveStatsBucketValue(serverBucket ?? requestedBucket, rawBucketOptions);
  }, [rawBucketOptions, requestedBucket, timeseries?.effectiveBucket]);
  const bucketOptions = useMemo(
    () => rawBucketOptions.map((option) => ({ ...option, label: t(option.labelKey) })),
    [rawBucketOptions, t],
  );

  // Keep internal bucket state in sync after the backend narrows unsupported options.
  useEffect(() => {
    if (bucket !== effectiveBucket) setBucket(effectiveBucket);
  }, [bucket, effectiveBucket]);

  const {
    data: errors,
    isLoading: errorsLoading,
    error: errorsError,
  } = useErrorDistribution(range, 8, errorScope);
  const {
    data: failureSummary,
    isLoading: failureSummaryLoading,
    error: failureSummaryError,
  } = useFailureSummary(range);
  const {
    data: parallelWorkStats,
    isLoading: parallelWorkLoading,
    error: parallelWorkError,
  } = useParallelWorkStats({ range, bucket: effectiveBucket });

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("stats.title")}</h2>
              <p className="section-description">{t("stats.subtitle")}</p>
            </div>
            <div className="flex w-full flex-col gap-3 sm:w-auto sm:flex-row sm:flex-wrap sm:items-center">
              <SelectField
                className="w-full sm:w-auto"
                options={rangeOptions}
                value={range}
                onValueChange={(value) => setRange(value as typeof range)}
                triggerClassName="w-full sm:min-w-[8.5rem]"
                data-testid="stats-range-select-trigger"
                aria-label={t("stats.subtitle")}
              />
              <SelectField
                className="w-full sm:w-auto"
                options={bucketOptions}
                value={effectiveBucket}
                onValueChange={setBucket}
                triggerClassName="w-full sm:min-w-[7rem]"
                data-testid="stats-bucket-select-trigger"
                aria-label={t("stats.trendTitle")}
              />
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t("stats.trendTitle")}</h3>
          </div>
          {timeseriesError ? (
            <Alert variant="error">{timeseriesError}</Alert>
          ) : (
            <TimeseriesChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t("stats.successFailureTitle")}</h3>
          </div>
          {timeseriesError ? (
            <Alert variant="error">{timeseriesError}</Alert>
          ) : (
            <SuccessFailureChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <ParallelWorkStatsSection
        stats={parallelWorkStats}
        isLoading={parallelWorkLoading}
        error={parallelWorkError}
      />

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="grid items-start gap-x-6 gap-y-3 xl:grid-cols-[minmax(14rem,0.9fr)_minmax(0,2fr)]">
            <div className="section-heading min-w-0">
              <h3 className="section-title">{t("stats.errors.title")}</h3>
              {failureSummaryError ? (
                <p className="section-description text-error">{failureSummaryError}</p>
              ) : (
                <p className="section-description">
                  {t("stats.errors.actionableRate", {
                    rate: `${((failureSummary?.actionableFailureRate ?? 0) * 100).toFixed(1)}%`,
                  })}
                </p>
              )}
            </div>
            <div className="grid min-w-0 grid-cols-2 gap-x-5 gap-y-3 md:grid-cols-4 xl:items-center">
              <div className="min-w-0">
                <div className="metric-label">{t("stats.errors.summary.service")}</div>
                <div className="mt-0.5 text-sm font-semibold tabular-nums text-error">
                  {failureSummaryLoading
                    ? "—"
                    : numberFormatter.format(failureSummary?.serviceFailureCount ?? 0)}
                </div>
              </div>
              <div className="min-w-0">
                <div className="metric-label">{t("stats.errors.summary.client")}</div>
                <div className="mt-0.5 text-sm font-semibold tabular-nums text-warning">
                  {failureSummaryLoading
                    ? "—"
                    : numberFormatter.format(failureSummary?.clientFailureCount ?? 0)}
                </div>
              </div>
              <div className="min-w-0">
                <div className="metric-label">{t("stats.errors.summary.abort")}</div>
                <div className="mt-0.5 text-sm font-semibold tabular-nums text-info">
                  {failureSummaryLoading
                    ? "—"
                    : numberFormatter.format(failureSummary?.clientAbortCount ?? 0)}
                </div>
              </div>
              <div className="min-w-0">
                <div className="metric-label">{t("stats.errors.summary.actionable")}</div>
                <div className="mt-0.5 text-sm font-semibold tabular-nums text-secondary">
                  {failureSummaryLoading
                    ? "—"
                    : numberFormatter.format(failureSummary?.actionableFailureCount ?? 0)}
                </div>
              </div>
            </div>
          </div>
          <ErrorReasonDistribution
            items={errors?.items ?? []}
            isLoading={errorsLoading}
            error={errorsError}
            scope={errorScope}
            onScopeChange={setErrorScope}
          />
        </div>
      </section>

      <LongTermStatsSection />
    </div>
  );
}
