import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Chip, type ChipTone } from "../../components/ui/chip";
import { Switch } from "../../components/ui/switch";
import { ListBodyState } from "../../features/shared/ListBodyState";
import { useTranslation } from "../../i18n";
import {
  fetchPromptCacheMaterializationStatus,
  type PromptCacheMaterializationRun,
  type PromptCacheMaterializationStatus,
  updatePromptCacheMaterializationControl,
} from "../../lib/api";

const REFRESH_INTERVAL_MS = 5_000;

function formatNumber(value: number | undefined): string {
  return value == null || !Number.isFinite(value) ? "-" : Math.round(value).toLocaleString();
}

function formatDuration(value: number | undefined): string {
  if (value == null || !Number.isFinite(value)) return "-";
  if (value < 1_000) return `${String(Math.round(value))} ms`;
  if (value < 60_000) return `${(value / 1_000).toFixed(1)} s`;
  const minutes = Math.floor(value / 60_000);
  const seconds = Math.round((value % 60_000) / 1_000);
  return `${String(minutes)}m ${String(seconds)}s`;
}

function formatTimestamp(value: string | undefined): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "-";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function statusTone(status: string): ChipTone {
  switch (status) {
    case "success":
      return "success";
    case "deferred":
      return "warning";
    case "failed":
      return "error";
    case "source_unavailable":
      return "warning";
    case "disabled":
      return "neutral";
    default:
      return "info";
  }
}

function phaseKey(phase: string): string {
  return [
    "identity_backfill",
    "identity_reconciliation",
    "stats_rebuild",
    "queue_drain",
    "complete",
  ].includes(phase)
    ? phase
    : "unknown";
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="metric-cell h-full">
      <div className="metric-label normal-case tracking-normal">{label}</div>
      <div className="metric-value mt-2 text-2xl tabular-nums text-base-content sm:text-3xl">
        {value}
      </div>
      {hint ? (
        <div className="mt-2 text-xs leading-relaxed text-base-content/60">{hint}</div>
      ) : null}
    </div>
  );
}

function RunRow({
  run,
  t,
}: {
  run: PromptCacheMaterializationRun;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  const result = t(`system.promptCache.runs.status.${run.status}`);
  return (
    <div className="grid gap-3 border-t border-base-300/70 py-4 text-sm lg:grid-cols-[minmax(10rem,1.2fr)_minmax(5rem,0.7fr)_minmax(5rem,0.7fr)_minmax(5rem,0.7fr)_minmax(8rem,1fr)] lg:items-center">
      <div className="min-w-0">
        <div className="font-medium text-base-content">{formatTimestamp(run.startedAt)}</div>
        <div className="mt-1 text-xs text-base-content/55">
          {t(`system.promptCache.phases.${phaseKey(run.phase)}`)}
        </div>
      </div>
      <div className="flex justify-between gap-2 lg:block">
        <span className="text-xs text-base-content/55 lg:hidden">
          {t("system.promptCache.runs.duration")}
        </span>
        <span className="tabular-nums">{formatDuration(run.durationMs)}</span>
      </div>
      <div className="flex justify-between gap-2 lg:block">
        <span className="text-xs text-base-content/55 lg:hidden">
          {t("system.promptCache.runs.processed")}
        </span>
        <span className="tabular-nums">{formatNumber(run.scanned)}</span>
      </div>
      <div className="flex justify-between gap-2 lg:block">
        <span className="text-xs text-base-content/55 lg:hidden">
          {t("system.promptCache.runs.updated")}
        </span>
        <span className="tabular-nums">{formatNumber(run.updated)}</span>
      </div>
      <div className="flex min-w-0 items-center justify-between gap-2 lg:block">
        <span className="text-xs text-base-content/55 lg:hidden">
          {t("system.promptCache.runs.result")}
        </span>
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-2 lg:justify-start">
          <Chip size="compact" tone={statusTone(run.status)}>
            {result}
          </Chip>
          {run.deferReason ? (
            <span className="max-w-[18rem] truncate text-xs text-warning">
              {t("system.promptCache.runs.deferReason", {
                reason: run.deferReason,
              })}
            </span>
          ) : null}
          {run.error ? (
            <span className="max-w-[18rem] truncate text-xs text-error" title={run.error}>
              {t("system.promptCache.runs.error", { error: run.error })}
            </span>
          ) : null}
        </div>
        <div className="mt-1 text-xs text-base-content/55">
          {t("system.promptCache.runs.batchSummary", {
            count: run.batchCount,
            max: run.maxBatchSize,
            elapsed: formatDuration(run.batchElapsedMs),
          })}
        </div>
      </div>
    </div>
  );
}

export default function SystemPromptCachePage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<PromptCacheMaterializationStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);

  const refresh = useCallback(async (signal?: AbortSignal) => {
    const next = await fetchPromptCacheMaterializationStatus(signal);
    setStatus(next);
    setError(null);
    setIsLoading(false);
  }, []);

  useEffect(() => {
    let active = true;
    const controller = new AbortController();
    const load = async () => {
      try {
        await refresh(controller.signal);
      } catch (cause) {
        if (!active || controller.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        setIsLoading(false);
      }
    };
    void load();
    const interval = window.setInterval(() => {
      void load();
    }, REFRESH_INTERVAL_MS);
    return () => {
      active = false;
      controller.abort();
      window.clearInterval(interval);
    };
  }, [refresh]);

  const handleEnabledChange = async (enabled: boolean) => {
    setIsSaving(true);
    try {
      const next = await updatePromptCacheMaterializationControl(enabled);
      setStatus(next);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSaving(false);
    }
  };

  const progress = useMemo(
    () => Math.max(0, Math.min(100, status?.progressPercent ?? 0)),
    [status?.progressPercent],
  );
  const phaseLabel = status ? t(`system.promptCache.phases.${phaseKey(status.phase)}`) : "-";
  const statusLabel = status
    ? t(`system.promptCache.runs.status.${status.lastStatus}`)
    : t("system.promptCache.runs.status.unknown");

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-prompt-cache">
      <div className="surface-panel-body gap-5">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="section-heading">
            <h2 className="section-title text-2xl">{t("system.promptCache.title")}</h2>
            <p className="section-description max-w-3xl">{t("system.promptCache.description")}</p>
          </div>
          <div className="flex items-center gap-3 text-xs text-base-content/55">
            <span>{t("system.promptCache.refreshing")}</span>
            {status ? (
              <Chip size="compact" tone={status.enabled ? "success" : "neutral"}>
                {t(status.enabled ? "system.promptCache.enabled" : "system.promptCache.disabled")}
              </Chip>
            ) : null}
          </div>
        </div>

        {error && status ? (
          <Alert variant="error">{t("system.promptCache.loadError", { error })}</Alert>
        ) : null}

        {isLoading && !status ? (
          <ListBodyState
            variant="loading"
            title={t("system.promptCache.loading")}
            testId="system-prompt-cache-loading"
          />
        ) : error && !status ? (
          <ListBodyState
            variant="error"
            title={t("system.promptCache.loadError", { error })}
            testId="system-prompt-cache-error"
          />
        ) : status ? (
          <>
            <div className="flex flex-col gap-4 rounded-xl border border-base-300/75 bg-base-100/55 p-4 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-sm font-semibold text-base-content">
                  {t("system.promptCache.control")}
                </div>
                <div className="mt-1 text-xs leading-relaxed text-base-content/60">
                  {t(
                    status.enabled
                      ? "system.promptCache.pauseHint"
                      : "system.promptCache.resumeHint",
                  )}
                </div>
              </div>
              <label className="flex shrink-0 items-center gap-3 text-sm font-medium text-base-content">
                <span>
                  {status.enabled
                    ? t("system.promptCache.enabled")
                    : t("system.promptCache.disabled")}
                </span>
                <Switch
                  checked={status.enabled}
                  disabled={isSaving}
                  onCheckedChange={(enabled) => void handleEnabledChange(enabled)}
                  aria-label={t("system.promptCache.control")}
                />
              </label>
            </div>

            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <Metric
                label={t("system.promptCache.metrics.total")}
                value={formatNumber(status.totalKeys)}
                hint={phaseLabel}
              />
              <Metric
                label={t("system.promptCache.metrics.completed")}
                value={formatNumber(status.completedKeys)}
                hint={`${formatNumber(status.queuePending)} ${t("system.promptCache.metrics.queue")}`}
              />
              <Metric
                label={t("system.promptCache.metrics.progress")}
                value={`${progress.toFixed(1)}%`}
                hint={
                  formatNumber(status.sourceMaxInvocationId) +
                  " " +
                  t("system.promptCache.metrics.source")
                }
              />
              <Metric
                label={t("system.promptCache.metrics.eta")}
                value={
                  status.estimatedRemainingMs == null
                    ? "-"
                    : status.estimatedRemainingMs === 0
                      ? t("system.promptCache.phases.complete")
                      : formatDuration(status.estimatedRemainingMs)
                }
                hint={status.suspensionReason ?? statusLabel}
              />
            </div>

            <div className="rounded-xl border border-base-300/75 bg-base-100/55 p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="text-sm font-semibold text-base-content">{phaseLabel}</div>
                <div className="text-xs text-base-content/55">
                  {t("system.promptCache.metrics.updatedAt")}: {formatTimestamp(status.updatedAt)}
                </div>
              </div>
              <div
                className="mt-4 h-2 overflow-hidden rounded-full bg-base-300/70"
                role="progressbar"
                aria-valuemax={100}
                aria-valuemin={0}
                aria-valuenow={progress}
                aria-label={t("system.promptCache.metrics.progress")}
              >
                <div
                  className="h-full rounded-full bg-primary transition-[width] duration-500"
                  style={{ width: `${String(progress)}%` }}
                />
              </div>
              <div className="mt-3 flex flex-wrap justify-between gap-3 text-xs text-base-content/60">
                <span>
                  {formatNumber(status.completedKeys)} / {formatNumber(status.totalKeys)}
                </span>
                <span>
                  {t("system.promptCache.metrics.lastStatus")}: {statusLabel}
                </span>
              </div>
            </div>

            <div className="rounded-xl border border-base-300/75 bg-base-100/55 px-4">
              <div className="flex flex-col gap-1 py-4 sm:flex-row sm:items-center sm:justify-between">
                <h3 className="text-base font-semibold text-base-content">
                  {t("system.promptCache.runs.title")}
                </h3>
                <div className="text-xs text-base-content/55">
                  {status.lastFinishedAt
                    ? formatTimestamp(status.lastFinishedAt)
                    : t("system.promptCache.metrics.updatedAt")}
                </div>
              </div>
              {status.recentRuns.length > 0 ? (
                <>
                  <div className="hidden border-y border-base-300/70 py-2 text-xs font-semibold uppercase tracking-[0.12em] text-base-content/55 lg:grid lg:grid-cols-[minmax(10rem,1.2fr)_minmax(5rem,0.7fr)_minmax(5rem,0.7fr)_minmax(5rem,0.7fr)_minmax(8rem,1fr)] lg:gap-3">
                    <span>{t("system.promptCache.runs.started")}</span>
                    <span>{t("system.promptCache.runs.duration")}</span>
                    <span>{t("system.promptCache.runs.processed")}</span>
                    <span>{t("system.promptCache.runs.updated")}</span>
                    <span>{t("system.promptCache.runs.result")}</span>
                  </div>
                  {status.recentRuns.map((run) => (
                    <RunRow key={run.id} run={run} t={t} />
                  ))}
                </>
              ) : (
                <div className="border-t border-base-300/70 py-8 text-sm text-base-content/60">
                  {t("system.promptCache.runs.empty")}
                </div>
              )}
            </div>
          </>
        ) : null}
      </div>
    </section>
  );
}
