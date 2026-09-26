import { useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import { fetchSystemStatus, type SystemStatusResponse } from "../../lib/api";

const REFRESH_INTERVAL_MS = 60_000;
const RETENTION_RECOVERY_STAGES = new Set([
  "preparing",
  "publishing",
  "finalizing",
  "prepared_reconcile",
  "orphan_sweep",
  "legacy_reconcile",
  "status_refresh",
]);

function formatBytes(value: number | null | undefined, unknownLabel = "Unknown"): string {
  if (value == null || !Number.isFinite(value)) return unknownLabel;
  if (value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let current = value;
  let index = 0;
  while (current >= 1024 && index < units.length - 1) {
    current /= 1024;
    index += 1;
  }
  return `${current >= 10 || index === 0 ? current.toFixed(0) : current.toFixed(1)} ${units[index]}`;
}

function formatGib(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 GiB";
  const gib = value / 1024 ** 3;
  return `${gib >= 10 || Number.isInteger(gib) ? gib.toFixed(0) : gib.toFixed(1)} GiB`;
}

function formatRecoveryTimestamp(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "-";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

type MetricCellProps = {
  title: string;
  value: string;
  hint?: string;
  tone?: "default" | "primary" | "secondary";
  badge?: string;
  badges?: string[];
};

function MetricCell({ title, value, hint, tone = "default", badge, badges }: MetricCellProps) {
  const toneClass =
    tone === "primary"
      ? "text-primary"
      : tone === "secondary"
        ? "text-secondary"
        : "text-base-content";
  return (
    <div className="metric-cell h-full">
      <div className="flex flex-wrap items-center gap-2">
        <div className="metric-label normal-case tracking-normal">{title}</div>
        {[badge, ...(badges ?? [])].filter(Boolean).map((label) => (
          <Chip
            key={label}
            size="compact"
            tone="secondary"
            className="px-2 text-[11px] font-semibold"
          >
            {label}
          </Chip>
        ))}
      </div>
      <div className={`metric-value mt-2 text-2xl tabular-nums sm:text-3xl ${toneClass}`}>
        {value}
      </div>
      {hint ? (
        <div className="mt-2 text-xs leading-relaxed text-base-content/65">{hint}</div>
      ) : null}
    </div>
  );
}

type BreakdownRowProps = {
  label: string;
  value: string;
  hint?: string;
  valueTitle?: string;
  badge?: string;
  badges?: string[];
};

function BreakdownRow({ label, value, hint, valueTitle, badge, badges }: BreakdownRowProps) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-lg border border-base-300/70 bg-base-100/50 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-semibold text-base-content">{label}</div>
        {hint ? (
          <div className="mt-1 text-xs leading-relaxed text-base-content/65">{hint}</div>
        ) : null}
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        {[badge, ...(badges ?? [])].filter(Boolean).map((label) => (
          <Chip
            key={label}
            size="compact"
            tone="secondary"
            className="px-2 text-[11px] font-semibold"
          >
            {label}
          </Chip>
        ))}
        <div
          className={`text-right text-lg font-semibold tabular-nums text-base-content sm:text-xl ${
            valueTitle ? "max-w-[55%] break-words" : "shrink-0"
          }`}
          title={valueTitle}
        >
          {value}
        </div>
      </div>
    </div>
  );
}

type PairedMetricProps = {
  title: string;
  testId?: string;
  badge?: string;
  badges?: string[];
  summary?: string;
  bytesLabel: string;
  bytesValue: string;
  countLabel: string;
  countValue: string;
  bytesHint?: string;
  countHint?: string;
  tone?: "default" | "secondary";
};

function PairedMetric({
  title,
  testId,
  badge,
  badges,
  summary,
  bytesLabel,
  bytesValue,
  countLabel,
  countValue,
  bytesHint,
  countHint,
  tone = "default",
}: PairedMetricProps) {
  return (
    <div
      className="rounded-xl border border-base-300/75 bg-base-100/60 px-4 py-4"
      data-testid={testId}
    >
      <div className="flex flex-wrap items-center gap-2">
        <div className="text-sm font-semibold text-base-content">{title}</div>
        {[badge, ...(badges ?? [])].filter(Boolean).map((label) => (
          <Chip
            key={label}
            size="compact"
            tone="secondary"
            className="px-2 text-[11px] font-semibold"
          >
            {label}
          </Chip>
        ))}
      </div>
      {summary ? (
        <p className="mt-2 max-w-[44ch] text-xs leading-relaxed text-base-content/65">{summary}</p>
      ) : null}
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <MetricCell title={bytesLabel} value={bytesValue} hint={bytesHint} tone={tone} />
        <MetricCell title={countLabel} value={countValue} hint={countHint} />
      </div>
    </div>
  );
}

type OverviewPanelProps = {
  status: SystemStatusResponse;
  t: ReturnType<typeof useTranslation>["t"];
};

function OverviewPanel({ status, t }: OverviewPanelProps) {
  const rawMetricsState = status.rawMetricsHealth.state;
  const rawMetricsMessage =
    rawMetricsState === "ready"
      ? t("system.status.rawMetrics.ready")
      : rawMetricsState === "deferred"
        ? t("system.status.rawMetrics.deferred")
        : rawMetricsState === "error"
          ? t("system.status.rawMetrics.error")
          : rawMetricsState === "unknown"
            ? t("system.status.rawMetrics.unknown")
            : rawMetricsState === "preparing"
              ? t("system.status.rawMetrics.preparing")
              : t("system.status.rawMetrics.unknown");
  const rawBodiesBytes = rawMetricsState === "ready" ? status.rawBodies.bytes : null;
  const requestRawBodiesBytes = rawMetricsState === "ready" ? status.requestRawBodies.bytes : null;
  const responseRawBodiesBytes =
    rawMetricsState === "ready" ? status.responseRawBodies.bytes : null;
  const rawCoverage =
    rawBodiesBytes == null
      ? "unknown"
      : status.rawMetricsHealth.physicalCoverage === "complete"
        ? "verified"
        : "limited";
  const rawCoverageLabel = t(`system.status.storage.${rawCoverage}`);
  const unknownBytesLabel = t("system.status.storage.unknown");
  const projectDiskBytes =
    rawBodiesBytes == null || status.archivedBodies.bytes == null
      ? null
      : status.archivedBodies.bytes +
        rawBodiesBytes +
        status.databaseBytes +
        status.otherFilesBytes;

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-status-overview">
      <div className="surface-panel-body gap-5">
        <div className="section-heading">
          <h3 className="section-title">{t("system.status.sections.diskOverviewTitle")}</h3>
        </div>

        <div className="rounded-xl border border-primary/20 bg-primary/8 px-5 py-5">
          <div className="flex flex-wrap items-center gap-2">
            <div className="text-sm font-semibold text-primary">
              {t("system.status.summary.projectDiskLabel")}
            </div>
            <Chip size="compact" tone="secondary" className="px-2 text-[11px] font-semibold">
              {rawCoverageLabel}
            </Chip>
          </div>
          <div className="mt-2 text-4xl font-semibold tracking-tight tabular-nums text-base-content sm:text-5xl">
            {formatBytes(projectDiskBytes, unknownBytesLabel)}
          </div>
          <p
            className="mt-3 max-w-full text-sm leading-relaxed text-base-content/72"
            data-testid="system-status-project-disk-formula"
          >
            {t("system.status.summary.projectDiskHint")}
          </p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <BreakdownRow
            label={t("system.status.breakdown.rawPayloadBytes")}
            value={formatBytes(rawBodiesBytes, unknownBytesLabel)}
            hint={t("system.status.breakdown.rawPayloadBytesHint")}
            badge={rawCoverageLabel}
          />
          <BreakdownRow
            label={t("system.status.breakdown.archiveBytes")}
            value={formatBytes(status.archivedBodies.bytes, unknownBytesLabel)}
            hint={t("system.status.breakdown.archiveBytesHint")}
          />
          <BreakdownRow
            label={t("system.status.breakdown.databaseBytes")}
            value={formatBytes(status.databaseBytes)}
            hint={t("system.status.breakdown.databaseBytesHint")}
          />
          <BreakdownRow
            label={t("system.status.breakdown.otherFilesBytes")}
            value={formatBytes(status.otherFilesBytes)}
            hint={t("system.status.breakdown.otherFilesBytesHint")}
          />
        </div>

        <div className="rounded-xl border border-base-300/75 bg-base-100/58 px-5 py-5">
          <div className="section-heading">
            <h3 className="section-title">{t("system.status.sections.rawPayloadFocusTitle")}</h3>
            <p className="section-description max-w-[70ch]">
              {t("system.status.sections.rawPayloadFocusDescription")}
            </p>
          </div>
          <div className="mt-5 grid gap-3 xl:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] xl:items-start">
            <MetricCell
              title={t("system.status.cards.rawBodiesBytes")}
              value={formatBytes(rawBodiesBytes, unknownBytesLabel)}
              hint={t("system.status.cards.rawBodiesBytesHint")}
              tone="primary"
              badge={t("system.status.metric.unionBadge")}
              badges={[rawCoverageLabel]}
            />
            <div className="grid gap-3 xl:grid-cols-2">
              <PairedMetric
                title={t("system.status.cards.requestRawBodiesBytes")}
                testId="system-status-request-raw-breakdown"
                badge={t("system.status.metric.splitBadge")}
                summary={t("system.status.cards.requestRawBodiesSplitHint")}
                bytesLabel={t("system.status.metric.bytesLabel")}
                bytesValue={formatBytes(requestRawBodiesBytes, unknownBytesLabel)}
                countLabel={t("system.status.metric.countLabel")}
                countValue={status.requestRawBodies.count.toLocaleString()}
                tone="secondary"
                badges={[rawCoverageLabel]}
              />
              <PairedMetric
                title={t("system.status.cards.responseRawBodiesBytes")}
                testId="system-status-response-raw-breakdown"
                badge={t("system.status.metric.splitBadge")}
                summary={t("system.status.cards.responseRawBodiesSplitHint")}
                bytesLabel={t("system.status.metric.bytesLabel")}
                bytesValue={formatBytes(responseRawBodiesBytes, unknownBytesLabel)}
                countLabel={t("system.status.metric.countLabel")}
                countValue={status.responseRawBodies.count.toLocaleString()}
                badges={[rawCoverageLabel]}
              />
            </div>
          </div>
          <div className="mt-3">
            <Alert variant="info">{t("system.status.rawPayloadDefinition")}</Alert>
          </div>
          <div className="mt-3" data-testid="system-status-raw-metrics-health">
            <Alert variant={rawMetricsState === "ready" ? "success" : "info"}>
              {rawMetricsMessage}
            </Alert>
          </div>
        </div>
      </div>
    </section>
  );
}

type MetricSectionProps = {
  title: string;
  description: string;
  metrics: MetricCellProps[];
  testId: string;
};

function MetricSection({ title, description, metrics, testId }: MetricSectionProps) {
  return (
    <section className="surface-panel overflow-hidden" data-testid={testId}>
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h3 className="section-title">{title}</h3>
          <p className="section-description max-w-[65ch]">{description}</p>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {metrics.map((metric) => (
            <MetricCell key={metric.title} {...metric} />
          ))}
        </div>
      </div>
    </section>
  );
}

function ProjectionHealthSection({ status, t }: OverviewPanelProps) {
  const { terminal, longTerm } = status.projectionHealth;
  const consumerLabel = (state: string) =>
    state === "healthy"
      ? t("system.status.projection.healthy")
      : state === "deferred"
        ? t("system.status.projection.deferred")
        : state === "repairing"
          ? t("system.status.projection.repairing")
          : state === "dirty_last_good"
            ? t("system.status.projection.lastGood")
            : t("system.status.projection.preparing");
  const age = (value?: number) => (value == null ? "-" : `${Math.round(value / 1000)} s`);

  return (
    <section
      className="surface-panel overflow-hidden"
      data-testid="system-status-projection-health"
    >
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h3 className="section-title">{t("system.status.projection.title")}</h3>
          <p className="section-description max-w-[72ch]">
            {t("system.status.projection.description")}
          </p>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <MetricCell
            title={t("system.status.projection.terminal")}
            value={consumerLabel(terminal.state)}
            hint={t("system.status.projection.terminalHint", {
              count: terminal.pendingEventCount.toLocaleString(),
            })}
            tone={terminal.state === "healthy" ? "primary" : "secondary"}
          />
          <MetricCell
            title={t("system.status.projection.longTerm")}
            value={consumerLabel(longTerm.state)}
            hint={t("system.status.projection.longTermHint", {
              count: longTerm.dirtyBucketCount.toLocaleString(),
            })}
            tone={longTerm.state === "healthy" ? "primary" : "secondary"}
          />
        </div>
        <details className="rounded-lg border border-base-300/70 bg-base-100/50 px-4 py-3">
          <summary className="cursor-pointer text-sm font-semibold text-base-content">
            {t("system.status.projection.details")}
          </summary>
          <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            <BreakdownRow
              label={t("system.status.projection.cursorLag")}
              value={longTerm.cursorLag.toLocaleString()}
            />
            <BreakdownRow
              label={t("system.status.projection.dirtyBuckets")}
              value={longTerm.dirtyBucketCount.toLocaleString()}
            />
            <BreakdownRow
              label={t("system.status.projection.lastFlush")}
              value={age(longTerm.lastFlushAgeMs)}
              hint={
                longTerm.lastFlushElapsedMs == null
                  ? undefined
                  : `${longTerm.lastFlushElapsedMs.toLocaleString()} ms`
              }
            />
            <BreakdownRow
              label={t("system.status.projection.deferReason")}
              value={longTerm.lastDeferReason ?? terminal.lastDeferReason ?? "-"}
            />
          </div>
        </details>
      </div>
    </section>
  );
}

function RuntimePressureHealthSection({ status, t }: OverviewPanelProps) {
  const health = status.runtimePressureHealth;
  const state = health?.state ?? "unknown";
  const stateLabel = t(`system.status.runtimePressure.states.${state}`);
  const alertVariant = state === "healthy" ? "success" : state === "unknown" ? "info" : "warning";
  const eventBus = health?.eventBus;
  const backfill = health?.backfill;
  const retention = health?.retentionWriteHealth;
  const eventBusState = eventBus?.state ?? "unknown";
  const backfillState = backfill?.state ?? "unknown";
  const recovery = health?.retentionRecovery;
  const rawOrphanSweep = health?.rawOrphanSweep;
  const rawCapture = health?.rawCapture;
  const rawCaptureState = ["capturing", "storage_suppressed", "unknown"].includes(
    rawCapture?.state ?? "unknown",
  )
    ? (rawCapture?.state ?? "unknown")
    : "unknown";
  const rawCaptureReason = [
    "inventory_unready",
    "raw_store_limit",
    "filesystem_low",
    "both",
  ].includes(rawCapture?.reason ?? "")
    ? rawCapture?.reason
    : undefined;
  const rawCaptureInventoryState = ["ready", "preparing", "resetting"].includes(
    rawCapture?.inventoryState ?? "",
  )
    ? (rawCapture?.inventoryState ?? "unknown")
    : "unknown";
  const rawCaptureStateLabel = t(
    `system.status.runtimePressure.rawCapture.states.${rawCaptureState}`,
  );
  const rawCaptureReasonLabel = rawCaptureReason
    ? t(`system.status.runtimePressure.rawCapture.reasons.${rawCaptureReason}`)
    : rawCaptureState === "capturing"
      ? t("system.status.runtimePressure.rawCapture.reasons.none")
      : t("system.status.runtimePressure.additiveUnknown");
  const rawCaptureInventoryLabel = t(
    `system.status.runtimePressure.rawCapture.inventoryStates.${rawCaptureInventoryState}`,
  );
  const recoveryStageLabel = (stage?: string | null) =>
    stage && RETENTION_RECOVERY_STAGES.has(stage)
      ? t(`system.status.runtimePressure.retentionRecovery.stages.${stage}`)
      : t("system.status.runtimePressure.states.unknown");
  const recoveryDeferReason = ["sqlite_pressure", "retry_backoff"].includes(
    recovery?.deferReason ?? "",
  )
    ? recovery?.deferReason
    : undefined;
  const recoveryDeferReasonLabel = recoveryDeferReason
    ? t(`system.status.runtimePressure.retentionRecovery.deferReasons.${recoveryDeferReason}`)
    : undefined;
  const recoveryHints = recovery
    ? [
        recovery.nextRetryAt
          ? t("system.status.runtimePressure.retentionRecovery.retryHint", {
              retry: formatRecoveryTimestamp(recovery.nextRetryAt),
            })
          : undefined,
        recovery.failureFingerprint
          ? t("system.status.runtimePressure.retentionRecovery.failureHint", {
              stage: recoveryStageLabel(recovery.failureStage),
              fingerprint: recovery.failureFingerprint,
            })
          : undefined,
        recoveryDeferReasonLabel
          ? t("system.status.runtimePressure.retentionRecovery.deferHint", {
              reason: recoveryDeferReasonLabel,
            })
          : undefined,
        recovery.consecutiveFailureCount != null
          ? t("system.status.runtimePressure.retentionRecovery.failureCountHint", {
              count: recovery.consecutiveFailureCount.toLocaleString(),
            })
          : undefined,
      ]
        .filter((hint): hint is string => hint != null)
        .join(" · ")
    : "";
  const rawOrphanSweepState = ["idle", "scanning", "deferred", "degraded"].includes(
    rawOrphanSweep?.state ?? "",
  )
    ? (rawOrphanSweep?.state ?? "unknown")
    : "unknown";
  const rawOrphanSweepDeferReason = ["sqlite_pressure", "retry_backoff"].includes(
    rawOrphanSweep?.deferReason ?? "",
  )
    ? rawOrphanSweep?.deferReason
    : undefined;
  const rawOrphanSweepHints = rawOrphanSweep
    ? [
        rawOrphanSweep.lastProgressAt
          ? t("system.status.runtimePressure.rawOrphanSweep.progressHint", {
              progress: formatRecoveryTimestamp(rawOrphanSweep.lastProgressAt),
            })
          : undefined,
        rawOrphanSweep.nextRetryAt
          ? t("system.status.runtimePressure.retentionRecovery.retryHint", {
              retry: formatRecoveryTimestamp(rawOrphanSweep.nextRetryAt),
            })
          : undefined,
        rawOrphanSweepDeferReason
          ? t("system.status.runtimePressure.rawOrphanSweep.deferHint", {
              reason: t(
                `system.status.runtimePressure.retentionRecovery.deferReasons.${rawOrphanSweepDeferReason}`,
              ),
            })
          : undefined,
        rawOrphanSweep.failureFingerprint
          ? t("system.status.runtimePressure.rawOrphanSweep.failureHint", {
              fingerprint: rawOrphanSweep.failureFingerprint,
            })
          : undefined,
      ]
        .filter((hint): hint is string => hint != null)
        .join(" · ")
    : "";
  const slices = health?.dashboardProjection.sliceCounters;
  const deliveryTopics = health
    ? [
        health.delivery.activity,
        health.delivery.summary,
        health.delivery.networkTimeseries,
        health.delivery.networkRecent,
      ]
    : [];
  const delivery = deliveryTopics.reduce(
    (total, topic) => ({
      serializationCount: total.serializationCount + topic.serializationCount,
      frameBytesCount: total.frameBytesCount + topic.frameBytesCount,
      laggedCount: total.laggedCount + topic.laggedCount,
      skippedCount: total.skippedCount + topic.skippedCount,
    }),
    { serializationCount: 0, frameBytesCount: 0, laggedCount: 0, skippedCount: 0 },
  );
  const hotTopics = health?.dashboardHotTopics;
  const hotTopicRows = hotTopics
    ? ([
        ["activity", hotTopics.activity],
        ["summary", hotTopics.summary],
        ["networkTimeseries", hotTopics.networkTimeseries],
        ["networkRecent", hotTopics.networkRecent],
        ["workingConversations", hotTopics.workingConversations],
        ["parallelWork", hotTopics.parallelWork],
        ["timeseries", hotTopics.timeseries],
      ] as const)
    : [];

  return (
    <section
      className="surface-panel overflow-hidden"
      data-testid="system-status-runtime-pressure-health"
    >
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h3 className="section-title">{t("system.status.runtimePressure.title")}</h3>
          <p className="section-description max-w-[72ch]">
            {t("system.status.runtimePressure.description")}
          </p>
        </div>
        <Alert variant={alertVariant}>
          {t("system.status.runtimePressure.summary", { state: stateLabel })}
        </Alert>
        <div
          className="border-y border-base-300/70 py-3"
          data-testid="system-status-dashboard-hot-topics"
        >
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <h4 className="text-sm font-semibold text-base-content">
              {t("system.status.runtimePressure.hotTopics.title")}
            </h4>
            <span className="text-xs font-medium text-base-content/70">
              {t(`system.status.runtimePressure.states.${hotTopics?.state ?? "unknown"}`)}
            </span>
          </div>
          {hotTopicRows.length > 0 ? (
            <div className="mt-3 divide-y divide-base-300/60">
              {hotTopicRows.map(([name, topic]) => (
                <div
                  className="grid gap-1 py-2 text-xs sm:grid-cols-[minmax(10rem,1fr)_auto] sm:items-center"
                  data-testid={`system-status-hot-topic-${name}`}
                  key={name}
                >
                  <div>
                    <span className="font-medium text-base-content">
                      {t(`system.status.runtimePressure.hotTopics.${name}`)}
                    </span>
                    <span className="ml-2 text-base-content/55">{topic.topicClass}</span>
                  </div>
                  <div className="flex flex-wrap gap-x-3 gap-y-1 text-base-content/70 sm:justify-end">
                    <span>{t(`system.status.runtimePressure.states.${topic.state}`)}</span>
                    <span>sub {topic.activeSubscriberCount.toLocaleString()}</span>
                    <span>build {topic.builderCount.toLocaleString()}</span>
                    <span>DB {topic.livePathDbReadCount.toLocaleString()}</span>
                    <span>fallback {topic.genericFallbackBuildCount.toLocaleString()}</span>
                    <span>cadence {topic.cadenceMissCount.toLocaleString()}</span>
                    <span>churn {topic.reconnectChurnCount.toLocaleString()}</span>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="mt-2 text-xs text-base-content/60">
              {t("system.status.runtimePressure.additiveUnknown")}
            </p>
          )}
        </div>
        <span className="sr-only" aria-live="polite" aria-atomic="true">
          {t("system.status.runtimePressure.rawCapture.statusAnnouncement", {
            state: rawCaptureStateLabel,
            reason: rawCaptureReasonLabel,
            inventory: rawCaptureInventoryLabel,
          })}
        </span>
        <span
          className="sr-only"
          data-testid="system-status-retention-recovery-live"
          aria-live="polite"
          aria-atomic="true"
        >
          {[
            t(`system.status.runtimePressure.states.${recovery?.state ?? "unknown"}`),
            recoveryStageLabel(recovery?.stage),
            recoveryHints || t("system.status.runtimePressure.additiveUnknown"),
          ].join(" · ")}
        </span>
        {health ? (
          <details className="rounded-lg border border-base-300/70 bg-base-100/50 px-4 py-3">
            <summary className="cursor-pointer text-sm font-semibold text-base-content">
              {t("system.status.runtimePressure.details")}
            </summary>
            <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <BreakdownRow
                label={t("system.status.runtimePressure.rssAnon")}
                value={formatBytes(health.process.rssAnonBytes)}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.swap")}
                value={formatBytes(health.process.swapBytes)}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.managed")}
                value={formatBytes(health.process.managedBytes)}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.unattributed")}
                value={formatBytes(health.process.unattributedAnonBytes)}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.writerQueue")}
                value={`${health.writerAccounting.pendingDepth.toLocaleString()} / ${formatBytes(health.writerAccounting.pendingBytes)}`}
                hint={health.writerAccounting.degradedReason}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.dashboardProducer")}
                value={health.dashboardProjection.producerState}
                hint={
                  health.dashboardProjection.lastDeferReason ??
                  health.dashboardProjection.degradedReason
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.liveDbReads")}
                value={health.dashboardProjection.livePathDbReadCount.toLocaleString()}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.cadenceMisses")}
                value={
                  slices
                    ? [slices.current, slices.network, slices.terminal]
                        .reduce((count, slice) => count + slice.cadenceMissCount, 0)
                        .toLocaleString()
                    : "-"
                }
                hint={
                  slices
                    ? `current ${slices.current.cadenceMissCount.toLocaleString()} / network ${slices.network.cadenceMissCount.toLocaleString()} / terminal ${slices.terminal.cadenceMissCount.toLocaleString()}`
                    : undefined
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.deliveryFrames")}
                value={`${delivery.serializationCount.toLocaleString()} / ${formatBytes(delivery.frameBytesCount)}`}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.deliveryLag")}
                value={`${delivery.laggedCount.toLocaleString()} / ${delivery.skippedCount.toLocaleString()}`}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.eventBus")}
                value={t(`system.status.runtimePressure.states.${eventBusState}`)}
                hint={
                  eventBus
                    ? t("system.status.runtimePressure.eventBusHint", {
                        published: eventBus.publishedCount.toLocaleString(),
                        processed: eventBus.processedEventCount.toLocaleString(),
                        coalesced: eventBus.coalescedEventCount.toLocaleString(),
                        topicWork: eventBus.topicWorkCount.toLocaleString(),
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.eventBusLag")}
                value={
                  eventBus
                    ? `${eventBus.routerLaggedCount.toLocaleString()} / ${eventBus.cursorRecoveryCount.toLocaleString()}`
                    : t("system.status.runtimePressure.states.unknown")
                }
                hint={
                  eventBus
                    ? t("system.status.runtimePressure.eventBusLagHint", {
                        gaps: eventBus.routerGapCount.toLocaleString(),
                        clones: eventBus.businessPayloadCloneCount.toLocaleString(),
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.backfill")}
                value={t(`system.status.runtimePressure.states.${backfillState}`)}
                hint={
                  backfill
                    ? t("system.status.runtimePressure.backfillHint", {
                        wakes: backfill.wakeCount.toLocaleString(),
                        due: backfill.dueDispatchCount.toLocaleString(),
                        deferred: backfill.deferredTaskCount.toLocaleString(),
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.backfillSuppression")}
                value={
                  backfill
                    ? `${backfill.noopSuppressedCount.toLocaleString()} / ${backfill.pressureDeferCount.toLocaleString()}`
                    : t("system.status.runtimePressure.states.unknown")
                }
                hint={
                  backfill
                    ? t("system.status.runtimePressure.backfillSuppressionHint", {
                        failed: backfill.failedTaskCount.toLocaleString(),
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.writerGate")}
                value={
                  health.proxySqliteWriteCoordinator
                    ? `${health.proxySqliteWriteCoordinator.p1WaiterCount.toLocaleString()} / ${health.proxySqliteWriteCoordinator.interactiveWaiterCount.toLocaleString()} / ${health.proxySqliteWriteCoordinator.p2WaiterCount.toLocaleString()} / ${health.proxySqliteWriteCoordinator.maintenanceWaiterCount.toLocaleString()}`
                    : t("system.status.runtimePressure.states.unknown")
                }
                hint={health.writerAccounting.p2WakeReason}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.retentionWrite")}
                value={t(`system.status.runtimePressure.states.${retention?.state ?? "unknown"}`)}
                hint={retention?.deferReason ?? retention?.lastError ?? retention?.operation}
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.retentionTransaction")}
                value={
                  retention
                    ? `${retention.batchRows.toLocaleString()} / ${formatBytes(retention.estimatedBytes)}`
                    : t("system.status.runtimePressure.states.unknown")
                }
                hint={
                  retention
                    ? t("system.status.runtimePressure.retentionTransactionHint", {
                        wait: retention.lockWaitMs.toLocaleString(),
                        execute: retention.executeMs.toLocaleString(),
                        commit: retention.commitMs.toLocaleString(),
                        reference:
                          retention.rawReferenceCheckMs == null
                            ? "-"
                            : `${retention.rawReferenceCheckMs.toLocaleString()}ms`,
                        fairness: retention.admissionMode ?? "-",
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <BreakdownRow
                label={t("system.status.runtimePressure.retentionBudget")}
                value={
                  retention
                    ? `${retention.budgetBreachCount.toLocaleString()} / ${retention.candidateRemainingHint.toLocaleString()}`
                    : t("system.status.runtimePressure.states.unknown")
                }
                hint={
                  retention
                    ? t("system.status.runtimePressure.retentionBudgetHint", {
                        prepare: retention.prepareElapsedMs.toLocaleString(),
                        starvation: retention.starvationAgeMs?.toLocaleString() ?? "-",
                        p1: retention.p1WaiterCount.toLocaleString(),
                      })
                    : t("system.status.runtimePressure.additiveUnknown")
                }
              />
              <div
                className="col-span-full border-t border-base-300/60 pt-3"
                data-testid="system-status-retention-recovery"
              >
                <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
                  <h4 className="text-sm font-semibold text-base-content">
                    {t("system.status.runtimePressure.retentionRecovery.title")}
                  </h4>
                  <span className="text-xs font-medium text-base-content/70">
                    {t(`system.status.runtimePressure.states.${recovery?.state ?? "unknown"}`)}
                  </span>
                </div>
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <BreakdownRow
                    label={t("system.status.runtimePressure.retentionRecovery.stage")}
                    value={recoveryStageLabel(recovery?.stage)}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.retentionRecovery.backlog")}
                    value={
                      recovery?.expiredBacklogCount != null
                        ? recovery.expiredBacklogCount.toLocaleString()
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    hint={
                      recovery?.oldestBacklogAgeSecs != null
                        ? t("system.status.runtimePressure.retentionRecovery.backlogHint", {
                            age: recovery.oldestBacklogAgeSecs.toLocaleString(),
                          })
                        : t("system.status.runtimePressure.additiveUnknown")
                    }
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.retentionRecovery.prepared")}
                    value={`${recovery?.preparedCount?.toLocaleString() ?? t("system.status.runtimePressure.states.unknown")} / ${recovery?.quarantinedCount?.toLocaleString() ?? t("system.status.runtimePressure.states.unknown")}`}
                    hint={t("system.status.runtimePressure.retentionRecovery.preparedHint")}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.retentionRecovery.progress")}
                    value={formatRecoveryTimestamp(recovery?.lastProgressAt)}
                    valueTitle={recovery?.lastProgressAt}
                    hint={recoveryHints || t("system.status.runtimePressure.additiveUnknown")}
                  />
                </div>
              </div>
              <div
                className="col-span-full border-t border-base-300/60 pt-3"
                data-testid="system-status-raw-orphan-sweep"
              >
                <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
                  <h4 className="text-sm font-semibold text-base-content">
                    {t("system.status.runtimePressure.rawOrphanSweep.title")}
                  </h4>
                  <span className="text-xs font-medium text-base-content/70">
                    {t(
                      `system.status.runtimePressure.rawOrphanSweep.states.${rawOrphanSweepState}`,
                    )}
                  </span>
                </div>
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawOrphanSweep.inspected")}
                    value={
                      rawOrphanSweep?.inspectedEntries != null
                        ? rawOrphanSweep.inspectedEntries.toLocaleString()
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    hint={t("system.status.runtimePressure.rawOrphanSweep.inspectedHint")}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawOrphanSweep.referenced")}
                    value={
                      rawOrphanSweep?.referencedSkipped != null
                        ? rawOrphanSweep.referencedSkipped.toLocaleString()
                        : t("system.status.runtimePressure.states.unknown")
                    }
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawOrphanSweep.quarantined")}
                    value={
                      rawOrphanSweep?.quarantined != null
                        ? rawOrphanSweep.quarantined.toLocaleString()
                        : t("system.status.runtimePressure.states.unknown")
                    }
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawOrphanSweep.removed")}
                    value={
                      rawOrphanSweep?.removed != null
                        ? rawOrphanSweep.removed.toLocaleString()
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    hint={rawOrphanSweepHints || t("system.status.runtimePressure.additiveUnknown")}
                  />
                </div>
              </div>
              <div
                className="col-span-full border-t border-base-300/60 pt-3"
                data-testid="system-status-raw-capture"
              >
                <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
                  <h4 className="text-sm font-semibold text-base-content">
                    {t("system.status.runtimePressure.rawCapture.title")}
                  </h4>
                  <span className="text-xs font-medium text-base-content/70">
                    {rawCaptureStateLabel}
                  </span>
                </div>
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.state")}
                    value={t(`system.status.runtimePressure.rawCapture.states.${rawCaptureState}`)}
                    hint={rawCaptureReasonLabel}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.inventory")}
                    value={t(
                      `system.status.runtimePressure.rawCapture.inventoryStates.${rawCaptureInventoryState}`,
                    )}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.rawBytes")}
                    value={
                      rawCapture?.rawBytes != null
                        ? formatBytes(rawCapture.rawBytes)
                        : t("system.status.runtimePressure.states.unknown")
                    }
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.availableBytes")}
                    value={
                      rawCapture?.availableBytes != null
                        ? formatBytes(rawCapture.availableBytes)
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    hint={
                      rawCapture?.reservedBytes != null
                        ? `${t("system.status.runtimePressure.rawCapture.reservedBytes")}: ${formatBytes(rawCapture.reservedBytes)}`
                        : t("system.status.runtimePressure.additiveUnknown")
                    }
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.rawWatermark")}
                    value={
                      rawCapture?.rawCloseBytes != null && rawCapture?.rawResumeBytes != null
                        ? `${t("system.status.runtimePressure.rawCapture.watermarkClose")}: ${formatGib(rawCapture.rawCloseBytes)} · ${t("system.status.runtimePressure.rawCapture.watermarkResume")}: ${formatGib(rawCapture.rawResumeBytes)}`
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    valueTitle={t("system.status.runtimePressure.rawCapture.rawWatermark")}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.filesystemWatermark")}
                    value={
                      rawCapture?.availableCloseBytes != null &&
                      rawCapture?.availableResumeBytes != null
                        ? `${t("system.status.runtimePressure.rawCapture.watermarkClose")}: ${formatGib(rawCapture.availableCloseBytes)} · ${t("system.status.runtimePressure.rawCapture.watermarkResume")}: ${formatGib(rawCapture.availableResumeBytes)}`
                        : t("system.status.runtimePressure.states.unknown")
                    }
                    valueTitle={t("system.status.runtimePressure.rawCapture.filesystemWatermark")}
                  />
                  <BreakdownRow
                    label={t("system.status.runtimePressure.rawCapture.backlog")}
                    value={
                      rawCapture?.backlogNonGrowing == null
                        ? t("system.status.runtimePressure.rawCapture.backlogUnknown")
                        : rawCapture.backlogNonGrowing
                          ? t("system.status.runtimePressure.rawCapture.backlogStable")
                          : t("system.status.runtimePressure.rawCapture.backlogGrowingKnown")
                    }
                  />
                </div>
              </div>
              <BreakdownRow
                label={t("system.status.runtimePressure.allocatorArenas")}
                value={health.allocator.mallocArenaMax}
              />
            </div>
          </details>
        ) : null}
      </div>
    </section>
  );
}

export default function SystemStatusPage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<SystemStatusResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);

  useEffect(() => {
    let active = true;

    const load = async (background: boolean) => {
      if (!background) {
        setIsLoading(true);
      } else {
        setIsRefreshing(true);
      }
      const complete = () => {
        setIsLoading(false);
        setIsRefreshing(false);
      };
      try {
        const next = await fetchSystemStatus();
        if (!active) {
          complete();
          return;
        }
        setStatus(next);
        setError(null);
      } catch (err) {
        if (!active) {
          complete();
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      }
      complete();
    };

    void load(false);
    const timer = window.setInterval(() => {
      void load(true);
    }, REFRESH_INTERVAL_MS);

    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);

  const sections = useMemo(() => {
    if (!status) return null;

    return {
      databaseMetrics: [
        {
          title: t("system.status.cards.liveInvocationsCount"),
          value: status.liveInvocationsCount.toLocaleString(),
          hint: t("system.status.cards.liveInvocationsCountHint"),
          tone: "primary" as const,
        },
        {
          title: t("system.status.cards.successCount"),
          value: status.successCount.toLocaleString(),
          hint: t("system.status.cards.successCountHint"),
        },
        {
          title: t("system.status.cards.nonSuccessCount"),
          value: status.nonSuccessCount.toLocaleString(),
          hint: t("system.status.cards.nonSuccessCountHint"),
        },
        {
          title: t("system.status.cards.completedArchiveBatchesCount"),
          value: status.completedArchiveBatchesCount.toLocaleString(),
          hint: t("system.status.cards.completedArchiveBatchesCountHint"),
        },
      ],
      archiveMetrics: [
        {
          title: t("system.status.cards.archivedBodiesCount"),
          value: status.archivedBodies.count.toLocaleString(),
          hint: t("system.status.cards.archivedBodiesCountHint"),
        },
        {
          title: t("system.status.cards.archivedBodiesBytes"),
          value: formatBytes(status.archivedBodies.bytes, t("system.status.storage.unknown")),
          hint: t("system.status.cards.archivedBodiesBytesHint"),
          tone: "secondary" as const,
        },
        {
          title: t("system.status.cards.rawBodiesCount"),
          value: status.rawBodies.count.toLocaleString(),
          hint: t("system.status.cards.rawBodiesCountHint"),
        },
      ],
    };
  }, [status, t]);

  return (
    <div className="space-y-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
            <div className="section-heading">
              <h2 className="section-title text-2xl">{t("system.status.title")}</h2>
              <p className="section-description max-w-3xl">{t("system.status.description")}</p>
            </div>
            <div className="flex flex-wrap items-center gap-2 text-xs text-base-content/65">
              <span>{isRefreshing ? t("system.status.refreshing") : t("system.status.idle")}</span>
              <span>
                {status
                  ? t("system.status.lastRefreshed", { at: status.refreshedAt })
                  : t("system.status.lastRefreshedEmpty")}
              </span>
            </div>
          </div>

          {error && <Alert variant="error">{t("system.status.loadError", { error })}</Alert>}
          {isLoading && !status ? <Alert variant="info">{t("system.status.loading")}</Alert> : null}
          {status ? (
            <div className="space-y-4" data-testid="system-status-layout">
              <OverviewPanel status={status} t={t} />
              <RuntimePressureHealthSection status={status} t={t} />
              <ProjectionHealthSection status={status} t={t} />
              <div className="grid gap-4 xl:grid-cols-2" data-testid="system-status-sections">
                <MetricSection
                  testId="system-status-records-section"
                  title={t("system.status.sections.databaseRecordsTitle")}
                  description={t("system.status.sections.databaseRecordsDescription")}
                  metrics={sections?.databaseMetrics ?? []}
                />
                <MetricSection
                  testId="system-status-archive-section"
                  title={t("system.status.sections.archiveLogicalTitle")}
                  description={t("system.status.sections.archiveLogicalDescription")}
                  metrics={sections?.archiveMetrics ?? []}
                />
              </div>
            </div>
          ) : null}
          <Alert variant="info">{t("system.status.definition")}</Alert>
        </div>
      </section>
    </div>
  );
}
