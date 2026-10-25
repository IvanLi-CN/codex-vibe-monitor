import { Alert } from "../../components/ui/alert";
import { useTranslation } from "../../i18n";
import type { StatsResponse } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AdaptiveMetricValue } from "../shared/AdaptiveMetricValue";
import { AppIcon } from "../shared/AppIcon";

interface StatsCardsProps {
  stats: StatsResponse | null;
  loading: boolean;
  error?: string | null;
  errorCode?: string | null;
}

function MetricValueSkeleton({ testId }: { testId: string }) {
  return (
    <span
      data-testid={testId}
      aria-hidden="true"
      className="block h-8 w-full max-w-[7.5rem] motion-safe:animate-pulse rounded bg-base-300/65"
    />
  );
}

export function StatsCards({ stats, loading, error, errorCode }: StatsCardsProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const summaryErrorMessage = error
    ? errorCode === "summary_recovery_pending"
      ? t("stats.cards.recoveryPending")
      : errorCode === "summary_recovery_failed"
        ? t("stats.cards.recoveryFailed")
        : t("stats.cards.loadError", { error })
    : null;
  const metricsLoading = loading || summaryErrorMessage != null;
  const metricsBusy = loading || errorCode === "summary_recovery_pending";

  const totalCalls = stats?.totalCount ?? 0;
  const successCount = stats?.successCount ?? 0;
  const failureCount = stats?.failureCount ?? 0;
  const totalCost = stats?.totalCost ?? 0;
  const totalTokens = stats?.totalTokens ?? 0;

  return (
    <div
      data-testid="stats-cards-state"
      className="relative"
      aria-busy={metricsBusy ? "true" : undefined}
    >
      <div
        data-testid="stats-cards-grid"
        aria-hidden={summaryErrorMessage != null ? "true" : undefined}
        inert={summaryErrorMessage != null ? true : undefined}
        className={cn(
          "metric-grid",
          summaryErrorMessage != null && "pointer-events-none select-none",
        )}
      >
        <div className="metric-cell">
          <div className="metric-label">{t("stats.cards.totalCalls")}</div>
          <div className="metric-value min-w-0 overflow-hidden text-primary">
            {metricsLoading ? (
              <MetricValueSkeleton testId="stats-card-value-total-calls-loading" />
            ) : (
              <AdaptiveMetricValue value={totalCalls} localeTag={localeTag} />
            )}
          </div>
        </div>
        <div className="metric-cell">
          <div className="metric-label">{t("stats.cards.success")}</div>
          <div className="metric-value min-w-0 overflow-hidden text-success">
            {metricsLoading ? (
              <MetricValueSkeleton testId="stats-card-value-success-loading" />
            ) : (
              <AdaptiveMetricValue value={successCount} localeTag={localeTag} />
            )}
          </div>
        </div>
        <div className="metric-cell">
          <div className="metric-label">{t("stats.cards.failures")}</div>
          <div className="metric-value min-w-0 overflow-hidden text-error">
            {metricsLoading ? (
              <MetricValueSkeleton testId="stats-card-value-failures-loading" />
            ) : (
              <AdaptiveMetricValue value={failureCount} localeTag={localeTag} />
            )}
          </div>
        </div>
        <div className="metric-cell">
          <div className="metric-label">{t("stats.cards.totalCost")}</div>
          <div className="metric-value min-w-0 overflow-hidden">
            {metricsLoading ? (
              <MetricValueSkeleton testId="stats-card-value-cost-loading" />
            ) : (
              <AdaptiveMetricValue value={totalCost} localeTag={localeTag} kind="currency" />
            )}
          </div>
        </div>
        <div className="metric-cell">
          <div className="metric-label">{t("stats.cards.totalTokens")}</div>
          <div className="metric-value min-w-0 overflow-hidden">
            {metricsLoading ? (
              <MetricValueSkeleton testId="stats-card-value-tokens-loading" />
            ) : (
              <AdaptiveMetricValue value={totalTokens} localeTag={localeTag} />
            )}
          </div>
        </div>
      </div>
      {summaryErrorMessage ? (
        <div
          data-testid="stats-cards-error-overlay"
          className="pointer-events-auto absolute inset-0 z-10 flex items-start justify-center rounded-[inherit] bg-base-100/38 px-4 pt-4 backdrop-blur-[3px] sm:items-center sm:pt-0"
        >
          <Alert
            role="alert"
            aria-live="assertive"
            variant="error"
            className="w-full max-w-[min(100%,36rem)] items-start shadow-[var(--summary-recovery-overlay-shadow)] backdrop-blur-xl"
          >
            <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
            <span>{summaryErrorMessage}</span>
          </Alert>
        </div>
      ) : null}
    </div>
  );
}
