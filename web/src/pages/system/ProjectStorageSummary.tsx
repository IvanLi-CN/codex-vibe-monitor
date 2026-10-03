import { Alert } from "../../components/ui/alert";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type { SystemStorageResponse } from "../../lib/api";

export type ProjectStorageSummaryProps = {
  storage: SystemStorageResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  error: string | null;
};

function formatStorageBytes(value: number | null, unknownLabel: string): string {
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

function formatTimestamp(value: string): string {
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

function storageReasonLabel(reason: string, t: ReturnType<typeof useTranslation>["t"]): string {
  const keys: Record<string, string> = {
    unsupported_platform: "system.status.storage.reasons.unsupportedPlatform",
    resource_limit: "system.status.storage.reasons.resourceLimit",
    permission_denied: "system.status.storage.reasons.permissionDenied",
    io_error: "system.status.storage.reasons.ioError",
    path_resolution: "system.status.storage.reasons.pathResolution",
    overflow: "system.status.storage.reasons.overflow",
    worker_failed: "system.status.storage.reasons.workerFailed",
    not_found: "system.status.storage.reasons.notFound",
    cancelled: "system.status.storage.reasons.cancelled",
  };
  const key = keys[reason];
  return key ? t(key) : t("system.status.storage.reasons.generic");
}

export default function ProjectStorageSummary({
  storage,
  isLoading,
  isRefreshing,
  error,
}: ProjectStorageSummaryProps) {
  const { t } = useTranslation();
  const state = storage?.state ?? "unknown";
  const sampledAt = storage?.sampledAt ? formatTimestamp(storage.sampledAt) : null;

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-storage-summary">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h3 className="section-title">{t("system.status.summary.projectDiskLabel")}</h3>
            <p className="section-description max-w-[70ch]">
              {t("system.status.summary.projectDiskHint")}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2" aria-live="polite">
            <Chip size="compact" tone="secondary" className="px-2 text-[11px] font-semibold">
              {t(`system.status.storage.states.${state}`)}
            </Chip>
            {storage?.scanInProgress ? (
              <Chip size="compact" tone="secondary" className="px-2 text-[11px] font-semibold">
                {t("system.status.storage.scanning")}
              </Chip>
            ) : null}
            {storage?.stale ? (
              <Chip size="compact" tone="secondary" className="px-2 text-[11px] font-semibold">
                {t("system.status.storage.stale")}
              </Chip>
            ) : null}
          </div>
        </div>
        <div className="rounded-lg border border-primary/20 bg-primary/8 px-5 py-5">
          <div className="text-4xl font-semibold tabular-nums text-base-content sm:text-5xl">
            {formatStorageBytes(storage?.totalBytes ?? null, t("system.status.storage.unknown"))}
          </div>
          <div className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-xs text-base-content/65">
            <span>
              {sampledAt
                ? t("system.status.storage.sampledAt", { at: sampledAt })
                : isLoading
                  ? t("system.status.storage.waiting")
                  : t("system.status.storage.notSampled")}
            </span>
            {isRefreshing ? <span>{t("system.status.storage.refreshing")}</span> : null}
          </div>
          {storage?.reason ? (
            <p className="mt-2 text-xs leading-relaxed text-base-content/65">
              {storageReasonLabel(storage.reason, t)}
            </p>
          ) : null}
        </div>
        {error ? (
          <Alert variant="error">{t("system.status.storage.loadError", { error })}</Alert>
        ) : null}
      </div>
    </section>
  );
}
