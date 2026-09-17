import { Alert } from "../components/ui/alert";
import { SelectField } from "../components/ui/select-field";
import { ErrorReasonPieChart } from "../features/stats/ErrorReasonPieChart";
import { LongTermStatsSection } from "../features/stats/LongTermStatsSection";
import { ParallelWorkStatsSection } from "../features/stats/ParallelWorkStatsSection";
import { StatsCards } from "../features/stats/StatsCards";
import { SuccessFailureChart } from "../features/stats/SuccessFailureChart";
import { TimeseriesChart } from "../features/stats/TimeseriesChart";
import { useTranslation } from "../i18n";
import type {
  ErrorDistributionResponse,
  FailureScope,
  FailureSummaryResponse,
  ParallelWorkStatsResponse,
  StatsResponse,
  TimeseriesResponse,
} from "../lib/api";

type SelectOption = { value: string; label: string };

interface StatsSummarySectionProps {
  range: string;
  rangeOptions: SelectOption[];
  onRangeChange: (value: string) => void;
  bucket: string;
  bucketOptions: SelectOption[];
  onBucketChange: (value: string) => void;
  summary: StatsResponse | null;
  summaryLoading: boolean;
  summaryError: string | null;
}

function StatsSummarySection({
  range,
  rangeOptions,
  onRangeChange,
  bucket,
  bucketOptions,
  onBucketChange,
  summary,
  summaryLoading,
  summaryError,
}: StatsSummarySectionProps) {
  const { t } = useTranslation();
  return (
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
              onValueChange={onRangeChange}
              triggerClassName="w-full sm:min-w-[8.5rem]"
              data-testid="stats-range-select-trigger"
              aria-label={t("stats.subtitle")}
            />
            <SelectField
              className="w-full sm:w-auto"
              options={bucketOptions}
              value={bucket}
              onValueChange={onBucketChange}
              triggerClassName="w-full sm:min-w-[7rem]"
              data-testid="stats-bucket-select-trigger"
              aria-label={t("stats.trendTitle")}
            />
          </div>
        </div>
        <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
      </div>
    </section>
  );
}

interface StatsTrendSectionsProps {
  timeseries: TimeseriesResponse | null;
  isLoading: boolean;
  error: string | null;
}

function StatsTrendSections({ timeseries, isLoading, error }: StatsTrendSectionsProps) {
  const { t } = useTranslation();
  const chartProps = {
    points: timeseries?.points ?? [],
    isLoading,
    bucketSeconds: timeseries?.bucketSeconds,
  };
  return (
    <>
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t("stats.trendTitle")}</h3>
          </div>
          {error ? <Alert variant="error">{error}</Alert> : <TimeseriesChart {...chartProps} />}
        </div>
      </section>
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t("stats.successFailureTitle")}</h3>
          </div>
          {error ? <Alert variant="error">{error}</Alert> : <SuccessFailureChart {...chartProps} />}
        </div>
      </section>
    </>
  );
}

interface StatsErrorSectionProps {
  errors: ErrorDistributionResponse | null;
  errorsLoading: boolean;
  errorsError: string | null;
  failureSummary: FailureSummaryResponse | null;
  failureSummaryLoading: boolean;
  failureSummaryError: string | null;
  errorScope: FailureScope;
  scopeOptions: SelectOption[];
  onScopeChange: (value: string) => void;
}

function StatsErrorSection({
  errors,
  errorsLoading,
  errorsError,
  failureSummary,
  failureSummaryLoading,
  failureSummaryError,
  errorScope,
  scopeOptions,
  onScopeChange,
}: StatsErrorSectionProps) {
  const { t } = useTranslation();
  const metrics = [
    ["service", "text-error", failureSummary?.serviceFailureCount ?? 0],
    ["client", "text-warning", failureSummary?.clientFailureCount ?? 0],
    ["abort", "text-info", failureSummary?.clientAbortCount ?? 0],
    ["actionable", "text-secondary", failureSummary?.actionableFailureCount ?? 0],
  ] as const;
  return (
    <section className="surface-panel">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
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
          <SelectField
            label={t("stats.errors.scope.label")}
            className="w-full min-[769px]:max-w-[14rem]"
            options={scopeOptions}
            value={errorScope}
            onValueChange={onScopeChange}
            data-testid="stats-error-scope-select-trigger"
            aria-label={t("stats.errors.scope.label")}
          />
        </div>
        <div className="metric-grid w-full grid-cols-1 sm:grid-cols-4">
          {metrics.map(([key, tone, value]) => (
            <div className="metric-cell" key={key}>
              <div className="metric-label">{t(`stats.errors.summary.${key}` as const)}</div>
              <div className={`metric-value ${tone} text-2xl`}>
                {failureSummaryLoading ? "—" : value}
              </div>
            </div>
          ))}
        </div>
        {errorsError ? (
          <Alert variant="error">{errorsError}</Alert>
        ) : (
          <ErrorReasonPieChart items={errors?.items ?? []} isLoading={errorsLoading} />
        )}
      </div>
    </section>
  );
}

interface StatsPageSectionsProps {
  summary: StatsResponse | null;
  summaryLoading: boolean;
  summaryError: string | null;
  range: string;
  rangeOptions: SelectOption[];
  onRangeChange: (value: string) => void;
  effectiveBucket: string;
  bucketOptions: SelectOption[];
  onBucketChange: (value: string) => void;
  timeseries: TimeseriesResponse | null;
  timeseriesLoading: boolean;
  timeseriesError: string | null;
  parallelWorkStats: ParallelWorkStatsResponse | null;
  parallelWorkLoading: boolean;
  parallelWorkError: string | null;
  errors: ErrorDistributionResponse | null;
  errorsLoading: boolean;
  errorsError: string | null;
  failureSummary: FailureSummaryResponse | null;
  failureSummaryLoading: boolean;
  failureSummaryError: string | null;
  errorScope: FailureScope;
  scopeOptions: SelectOption[];
  onScopeChange: (value: string) => void;
}

export function StatsPageSections(props: StatsPageSectionsProps) {
  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <StatsSummarySection
        range={props.range}
        rangeOptions={props.rangeOptions}
        onRangeChange={props.onRangeChange}
        bucket={props.effectiveBucket}
        bucketOptions={props.bucketOptions}
        onBucketChange={props.onBucketChange}
        summary={props.summary}
        summaryLoading={props.summaryLoading}
        summaryError={props.summaryError}
      />
      <StatsTrendSections
        timeseries={props.timeseries}
        isLoading={props.timeseriesLoading}
        error={props.timeseriesError}
      />
      <ParallelWorkStatsSection
        stats={props.parallelWorkStats}
        isLoading={props.parallelWorkLoading}
        error={props.parallelWorkError}
      />
      <StatsErrorSection
        errors={props.errors}
        errorsLoading={props.errorsLoading}
        errorsError={props.errorsError}
        failureSummary={props.failureSummary}
        failureSummaryLoading={props.failureSummaryLoading}
        failureSummaryError={props.failureSummaryError}
        errorScope={props.errorScope}
        scopeOptions={props.scopeOptions}
        onScopeChange={props.onScopeChange}
      />
      <LongTermStatsSection />
    </div>
  );
}
