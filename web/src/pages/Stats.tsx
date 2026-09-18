import { useEffect, useMemo, useState } from "react";
import { useErrorDistribution } from "../hooks/useErrorDistribution";
import { useFailureSummary } from "../hooks/useFailureSummary";
import { useParallelWorkStats } from "../hooks/useParallelWorkStats";
import { useSummary } from "../hooks/useStats";
import { useTimeseries } from "../hooks/useTimeseries";
import { useTranslation } from "../i18n";
import type { FailureScope } from "../lib/api";
import { StatsPageSections } from "./StatsPageSections";
import { RANGE_OPTIONS, resolveStatsBucketOptions, resolveStatsBucketValue } from "./stats-options";

function useStatsBucketSelection(range: (typeof RANGE_OPTIONS)[number]["value"]) {
  const [bucket, setBucket] = useState("15m");
  const requestedOptions = useMemo(() => resolveStatsBucketOptions(range), [range]);
  const requestedBucket = useMemo(
    () => resolveStatsBucketValue(bucket, requestedOptions),
    [bucket, requestedOptions],
  );
  const {
    data: timeseries,
    isLoading,
    error,
  } = useTimeseries(range, { bucket: requestedBucket, preferServerAggregation: true });
  const rawOptions = useMemo(
    () => resolveStatsBucketOptions(range, timeseries?.availableBuckets),
    [range, timeseries?.availableBuckets],
  );
  const effectiveBucket = useMemo(
    () =>
      rawOptions.some((option) => option.value === requestedBucket)
        ? requestedBucket
        : resolveStatsBucketValue(timeseries?.effectiveBucket ?? requestedBucket, rawOptions),
    [rawOptions, requestedBucket, timeseries?.effectiveBucket],
  );
  useEffect(() => {
    if (bucket !== effectiveBucket) setBucket(effectiveBucket);
  }, [bucket, effectiveBucket]);
  return {
    timeseries,
    timeseriesLoading: isLoading,
    timeseriesError: error,
    rawOptions,
    effectiveBucket,
    setBucket,
  };
}

export default function StatsPage() {
  const { t } = useTranslation();
  const [range, setRange] = useState<(typeof RANGE_OPTIONS)[number]["value"]>("today");
  const [errorScope, setErrorScope] = useState<FailureScope>("service");

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  );

  const { summary, isLoading: summaryLoading, error: summaryError } = useSummary(range);

  const { timeseries, timeseriesLoading, timeseriesError, rawOptions, effectiveBucket, setBucket } =
    useStatsBucketSelection(range);
  const bucketOptions = useMemo(
    () => rawOptions.map((option) => ({ ...option, label: t(option.labelKey) })),
    [rawOptions, t],
  );

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

  const scopeOptions = useMemo(
    () => [
      { value: "service", label: t("stats.errors.scope.service") },
      { value: "client", label: t("stats.errors.scope.client") },
      { value: "abort", label: t("stats.errors.scope.abort") },
      { value: "all", label: t("stats.errors.scope.all") },
    ],
    [t],
  );

  return (
    <StatsPageSections
      summary={summary}
      summaryLoading={summaryLoading}
      summaryError={summaryError}
      range={range}
      rangeOptions={rangeOptions}
      onRangeChange={(value) => setRange(value as typeof range)}
      effectiveBucket={effectiveBucket}
      bucketOptions={bucketOptions}
      onBucketChange={setBucket}
      timeseries={timeseries}
      timeseriesLoading={timeseriesLoading}
      timeseriesError={timeseriesError}
      parallelWorkStats={parallelWorkStats}
      parallelWorkLoading={parallelWorkLoading}
      parallelWorkError={parallelWorkError}
      errors={errors}
      errorsLoading={errorsLoading}
      errorsError={errorsError}
      failureSummary={failureSummary}
      failureSummaryLoading={failureSummaryLoading}
      failureSummaryError={failureSummaryError}
      errorScope={errorScope}
      scopeOptions={scopeOptions}
      onScopeChange={(value) => setErrorScope(value as FailureScope)}
    />
  );
}
