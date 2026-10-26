import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Chip, type ChipTone } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type {
  StorageMigrationControlAction,
  StorageMigrationModuleProgress,
  StorageMigrationResponse,
  StorageMigrationState,
  StorageMigrationTableProgress,
} from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";

export interface StorageMigrationStatusCardProps {
  status: StorageMigrationResponse;
  error?: string | null;
  isActionPending?: boolean;
  onAction?: (action: StorageMigrationControlAction) => void;
}

function formatNumber(value: number | null, unknownLabel: string): string {
  return value == null || !Number.isFinite(value)
    ? unknownLabel
    : Math.round(value).toLocaleString();
}

function formatBytes(value: number | null, unknownLabel: string): string {
  if (value == null || !Number.isFinite(value)) return unknownLabel;
  if (value === 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let current = value;
  let index = 0;
  while (current >= 1024 && index < units.length - 1) {
    current /= 1024;
    index += 1;
  }
  return `${Number.isInteger(current) || index === 0 ? current.toFixed(0) : current.toFixed(1)} ${units[index]}`;
}

function formatDuration(value: number | null, unknownLabel: string): string {
  if (value == null || !Number.isFinite(value)) return unknownLabel;
  if (value < 1_000) return `${Math.round(value)} ms`;
  if (value < 60_000) return `${(value / 1_000).toFixed(1)} s`;
  const minutes = Math.floor(value / 60_000);
  const seconds = Math.round((value % 60_000) / 1_000);
  return `${minutes}m ${seconds}s`;
}

function formatTimestamp(value: string | null, unknownLabel: string): string {
  if (!value) return unknownLabel;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return unknownLabel;
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function formatCoverage(value: number | null, unknownLabel: string): string {
  return value == null || !Number.isFinite(value) ? unknownLabel : `${value.toFixed(1)}%`;
}

function stateTone(state: StorageMigrationState): ChipTone {
  switch (state) {
    case "postgres":
    case "ready":
      return "success";
    case "paused":
      return "warning";
    case "blocked":
    case "recovering":
    case "failed":
      return "error";
    case "switching":
    case "catching_up":
      return "info";
    default:
      return "secondary";
  }
}

function stateLabel(
  state: StorageMigrationState,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  return t(`system.storageMigration.states.${state}`);
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="metric-cell h-full">
      <div className="metric-label normal-case tracking-normal">{label}</div>
      <div className="mt-2 text-xl font-semibold tabular-nums text-base-content sm:text-2xl">
        {value}
      </div>
      {hint ? (
        <div className="mt-2 text-xs leading-relaxed text-base-content/60">{hint}</div>
      ) : null}
    </div>
  );
}

function TableRow({
  table,
  unknownLabel,
  t,
}: {
  table: StorageMigrationTableProgress;
  unknownLabel: string;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  return (
    <tr className="border-t border-base-300/65 text-sm">
      <td className="whitespace-nowrap py-3 pr-4 font-medium text-base-content">
        {table.tableName}
      </td>
      <td className="whitespace-nowrap py-3 pr-4 text-xs font-semibold uppercase tracking-wide text-base-content/70">
        {stateLabel(table.state, t)}
      </td>
      <td className="whitespace-nowrap py-3 pr-4 tabular-nums">
        {formatNumber(table.committedRows, unknownLabel)}
        <span className="ml-1 text-xs text-base-content/50">
          / {formatNumber(table.sourceRows, unknownLabel)}
        </span>
      </td>
      <td className="whitespace-nowrap py-3 pr-4 tabular-nums">
        {formatBytes(table.committedBytes, unknownLabel)}
        <span className="ml-1 text-xs text-base-content/50">
          / {formatBytes(table.sourceBytes, unknownLabel)}
        </span>
      </td>
      <td className="whitespace-nowrap py-3 pr-4 tabular-nums">
        {formatNumber(table.verifiedChunks, unknownLabel)}
        <span className="ml-1 text-xs text-base-content/50">
          / {formatNumber(table.dirtyChunks, unknownLabel)}{" "}
          {t("system.storageMigration.tables.chunks")}
        </span>
      </td>
      <td className="max-w-[15rem] truncate py-3 font-mono text-xs text-base-content/55">
        <div>{table.scanCursor ?? unknownLabel}</div>
        <div className="mt-1 text-[0.7rem] text-base-content/45">
          {t("system.storageMigration.tables.scanBound", {
            bound: formatNumber(table.scanBound, unknownLabel),
          })}
        </div>
      </td>
      <td className="whitespace-nowrap py-3 text-xs tabular-nums text-base-content/65">
        {formatNumber(table.indexPreparation.prepared, unknownLabel)} /{" "}
        {formatNumber(table.indexPreparation.total, unknownLabel)}
      </td>
    </tr>
  );
}

function ModuleRow({
  module,
  unknownLabel,
  t,
}: {
  module: StorageMigrationModuleProgress;
  unknownLabel: string;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  return (
    <div className="border-t border-base-300/65 py-3 first:border-t-0 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="text-sm font-semibold text-base-content">{module.module}</div>
        <Chip size="compact" tone={stateTone(module.state)}>
          {stateLabel(module.state, t)}
        </Chip>
      </div>
      <div className="mt-2 grid gap-2 text-xs text-base-content/60 sm:grid-cols-2 xl:grid-cols-4">
        <span>
          {t("system.storageMigration.modules.dependencies")}:{" "}
          {module.dependencyReady == null
            ? unknownLabel
            : module.dependencyReady
              ? t("system.storageMigration.modules.ready")
              : t("system.storageMigration.modules.pending")}
        </span>
        <span>
          {t("system.storageMigration.modules.unapplied")}:{" "}
          {formatNumber(module.unappliedChanges, unknownLabel)}
        </span>
        <span>
          {t("system.storageMigration.modules.proof")}:{" "}
          {formatCoverage(module.finalProofCoverage, unknownLabel)}
        </span>
        <span>
          {t("system.storageMigration.modules.switchAttempts")}: {module.switchAttemptCount}
          {module.switchResult ? ` · ${module.switchResult}` : ""}
        </span>
        <span>
          {t("system.storageMigration.modules.drain")}: {module.drainState}
        </span>
      </div>
    </div>
  );
}

export default function StorageMigrationStatusCard({
  status,
  error,
  isActionPending = false,
  onAction,
}: StorageMigrationStatusCardProps) {
  const { t } = useTranslation();
  const unknownLabel = t("system.storageMigration.unknown");
  const progress =
    status.progress.committedRows != null && status.progress.sourceRows
      ? Math.min(100, (status.progress.committedRows / status.progress.sourceRows) * 100)
      : null;
  const actionLabel = status.controls.lastAction
    ? t(`system.storageMigration.actions.${status.controls.lastAction}`)
    : null;

  return (
    <section className="surface-panel overflow-hidden" data-testid="storage-migration-status-card">
      <div className="surface-panel-body gap-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
          <div className="section-heading">
            <h2 className="section-title text-2xl">{t("system.storageMigration.title")}</h2>
            <p className="section-description max-w-3xl">
              {t("system.storageMigration.description")}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2" aria-live="polite">
            <Chip size="compact" tone={stateTone(status.state)}>
              {stateLabel(status.state, t)}
            </Chip>
            <Chip size="compact" tone={status.target.configured ? "success" : "warning"}>
              {status.target.configured
                ? t("system.storageMigration.target.configured")
                : t("system.storageMigration.target.unconfigured")}
            </Chip>
          </div>
        </div>

        {error ? (
          <Alert variant="error">{t("system.storageMigration.loadError", { error })}</Alert>
        ) : null}

        {onAction ? (
          <div className="flex flex-col gap-3 border-y border-base-300/70 py-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <div className="text-sm font-semibold text-base-content">
                {t("system.storageMigration.controls.title")}
              </div>
              <div className="mt-1 text-xs leading-relaxed text-base-content/60">
                {t("system.storageMigration.controls.description")}
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              {status.controls.canStart ? (
                <Button
                  type="button"
                  size="sm"
                  disabled={isActionPending}
                  onClick={() => onAction("start")}
                >
                  <AppIcon name="sync" className="mr-1.5 h-4 w-4" aria-hidden />
                  {t("system.storageMigration.actions.start")}
                </Button>
              ) : null}
              {status.controls.canPause ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={isActionPending}
                  onClick={() => onAction("pause")}
                >
                  <AppIcon name="close" className="mr-1.5 h-4 w-4" aria-hidden />
                  {t("system.storageMigration.actions.pause")}
                </Button>
              ) : null}
              {status.controls.canResume ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={isActionPending}
                  onClick={() => onAction("resume")}
                >
                  <AppIcon name="refresh" className="mr-1.5 h-4 w-4" aria-hidden />
                  {t("system.storageMigration.actions.resume")}
                </Button>
              ) : null}
            </div>
          </div>
        ) : null}

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <Metric
            label={t("system.storageMigration.metrics.phase")}
            value={stateLabel(status.progress.phase, t)}
            hint={`${t("system.storageMigration.metrics.module")}: ${status.module}`}
          />
          <Metric
            label={t("system.storageMigration.metrics.committed")}
            value={formatNumber(status.progress.committedRows, unknownLabel)}
            hint={`${t("system.storageMigration.metrics.source")}: ${formatNumber(status.progress.sourceRows, unknownLabel)}`}
          />
          <Metric
            label={t("system.storageMigration.metrics.bytes")}
            value={formatBytes(status.progress.committedBytes, unknownLabel)}
            hint={`${t("system.storageMigration.metrics.source")}: ${formatBytes(status.progress.sourceBytes, unknownLabel)}`}
          />
          <Metric
            label={t("system.storageMigration.metrics.eta")}
            value={
              status.progress.converged === false
                ? t("system.storageMigration.metrics.noEta")
                : formatDuration(status.progress.etaMs, unknownLabel)
            }
            hint={
              status.progress.converged === false
                ? t("system.storageMigration.metrics.nonConvergent")
                : t("system.storageMigration.metrics.noEstimate")
            }
          />
        </div>

        <div className="grid gap-4 border-t border-base-300/70 pt-5 lg:grid-cols-3">
          <div>
            <div className="metric-label">{t("system.storageMigration.authority.title")}</div>
            <div className="mt-2 flex items-center gap-2 text-sm font-semibold text-base-content">
              <AppIcon name="database-outline" className="h-4 w-4 text-primary" aria-hidden />
              {status.authority.backend} ·{" "}
              {t("system.storageMigration.authority.generation", {
                generation: status.authority.generation,
              })}
            </div>
            <p className="mt-1 text-xs text-base-content/55">
              {status.source.identity} → {status.target.identity}
            </p>
          </div>
          <div>
            <div className="metric-label">{t("system.storageMigration.backlog.title")}</div>
            <div className="mt-2 text-sm font-semibold tabular-nums text-base-content">
              {formatNumber(status.backlog.pendingKeys, unknownLabel)}
            </div>
            <p className="mt-1 text-xs text-base-content/55">
              {t("system.storageMigration.backlog.oldestChange", {
                age: formatDuration(status.backlog.oldestChangeAgeMs, unknownLabel),
              })}
            </p>
          </div>
          <div>
            <div className="metric-label">{t("system.storageMigration.switch.title")}</div>
            <div className="mt-2 text-sm font-semibold tabular-nums text-base-content">
              {status.switch.result ?? unknownLabel}
            </div>
            <p className="mt-1 text-xs text-base-content/55">
              {t("system.storageMigration.switch.budget", { budget: status.switch.budgetMs })} ·{" "}
              {t("system.storageMigration.switch.attempts", {
                count: status.switch.attemptCount,
              })}
            </p>
          </div>
        </div>

        {status.modules.length > 0 ? (
          <div className="border-t border-base-300/70 pt-5">
            <div className="flex flex-wrap items-end justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-base-content">
                  {t("system.storageMigration.modules.title")}
                </div>
                <div className="mt-1 text-xs text-base-content/55">
                  {t("system.storageMigration.modules.queue", {
                    queued: status.queuedModules.length,
                    completed: status.completedModules.length,
                  })}
                </div>
              </div>
              <div className="text-xs text-base-content/55">
                {t("system.storageMigration.heartbeat", {
                  at: formatTimestamp(status.heartbeat.at, unknownLabel),
                })}
              </div>
            </div>
            <div className="mt-4">
              {status.modules.map((module) => (
                <ModuleRow key={module.module} module={module} unknownLabel={unknownLabel} t={t} />
              ))}
            </div>
          </div>
        ) : null}

        {status.taskHolds.held || status.recovery.required || status.pressure.nonConvergent ? (
          <div className="grid gap-3 border-t border-base-300/70 pt-5 md:grid-cols-3">
            {status.taskHolds.held ? (
              <Alert variant="warning">
                <AppIcon
                  name="shield-key-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <span>
                  <strong>{t("system.storageMigration.hold.title")}</strong>
                  <span className="mt-1 block text-xs">
                    {status.taskHolds.reason ?? unknownLabel} · {status.taskHolds.drainState}
                    <span className="mt-1 block font-mono text-[0.7rem] text-base-content/55">
                      {status.taskHolds.lastSafeCheckpoint ?? unknownLabel}
                    </span>
                  </span>
                </span>
              </Alert>
            ) : null}
            {status.recovery.required ? (
              <Alert variant="error">
                <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <span>
                  <strong>{t("system.storageMigration.recovery.title")}</strong>
                  <span className="mt-1 block text-xs">
                    {status.recovery.reason ?? unknownLabel}
                  </span>
                </span>
              </Alert>
            ) : null}
            {status.pressure.nonConvergent ? (
              <Alert variant="warning">
                <AppIcon name="timer-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <span>
                  <strong>{t("system.storageMigration.pressure.nonConvergent")}</strong>
                  <span className="mt-1 block text-xs">
                    {formatNumber(status.progress.replayThroughputPerSec, unknownLabel)} /{" "}
                    {formatNumber(status.progress.incomingRatePerSec, unknownLabel)}{" "}
                    {t("system.storageMigration.pressure.rateUnit")}
                  </span>
                </span>
              </Alert>
            ) : null}
          </div>
        ) : null}

        <div className="border-t border-base-300/70 pt-5">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <div className="text-sm font-semibold text-base-content">
                {t("system.storageMigration.tables.title")}
              </div>
              <div className="mt-1 text-xs text-base-content/55">
                {progress == null
                  ? t("system.storageMigration.tables.progressUnknown")
                  : t("system.storageMigration.tables.progress", { percent: progress.toFixed(1) })}
              </div>
            </div>
            <div className="text-xs text-base-content/55">
              {t("system.storageMigration.updatedAt", {
                at: formatTimestamp(status.updatedAt, unknownLabel),
              })}
              {actionLabel ? ` · ${actionLabel}` : ""}
            </div>
          </div>
          {status.tables.length > 0 ? (
            <div className="mt-4 overflow-x-auto">
              <table className="w-full min-w-[62rem] text-left">
                <thead className="text-xs uppercase tracking-[0.12em] text-base-content/50">
                  <tr>
                    <th className="pb-2 pr-4 font-semibold">
                      {t("system.storageMigration.tables.table")}
                    </th>
                    <th className="pb-2 pr-4 font-semibold">
                      {t("system.storageMigration.tables.state")}
                    </th>
                    <th className="pb-2 pr-4 font-semibold">
                      {t("system.storageMigration.tables.rows")}
                    </th>
                    <th className="pb-2 pr-4 font-semibold">
                      {t("system.storageMigration.tables.bytes")}
                    </th>
                    <th className="pb-2 pr-4 font-semibold">
                      {t("system.storageMigration.tables.verify")}
                    </th>
                    <th className="pb-2 font-semibold">
                      {t("system.storageMigration.tables.cursor")}
                    </th>
                    <th className="pb-2 font-semibold">
                      {t("system.storageMigration.tables.indexes")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {status.tables.map((table) => (
                    <TableRow
                      key={table.tableName}
                      table={table}
                      unknownLabel={unknownLabel}
                      t={t}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <div className="mt-4 border-t border-base-300/65 py-8 text-sm text-base-content/60">
              {t("system.storageMigration.tables.empty")}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
