import { useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { useTranslation } from "../../i18n";
import { fetchSystemStatus, type SystemStatusResponse } from "../../lib/api";
import {
  formatBytes,
  MetricSection,
  OverviewPanel,
  ProjectionHealthSection,
  RuntimePressureHealthSection,
} from "./SystemStatusPanels";

const REFRESH_INTERVAL_MS = 60_000;

function useSystemStatus() {
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

  return { status, error, isLoading, isRefreshing };
}

export default function SystemStatusPage() {
  const { t } = useTranslation();
  const { status, error, isLoading, isRefreshing } = useSystemStatus();

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
          value: formatBytes(status.archivedBodies.bytes),
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
