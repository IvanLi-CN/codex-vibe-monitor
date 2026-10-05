import { useEffect, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import { fetchObservabilityCapabilities, type ObservabilityCapabilities } from "../../lib/api";
import { usePageObservation } from "../../lib/browserObservability";
import { grafanaLink } from "../../lib/observabilityLinks";

export default function SystemPerformancePage() {
  const { t } = useTranslation();
  const [capabilities, setCapabilities] = useState<ObservabilityCapabilities | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    let active = true;
    void fetchObservabilityCapabilities()
      .then((value) => {
        if (active) setCapabilities(value);
      })
      .catch(() => {
        if (active) setError(true);
      });
    return () => {
      active = false;
    };
  }, []);
  usePageObservation("system", capabilities);
  const destinations = [
    ["cvm-overview", "overview"],
    ["cvm-proxy", "proxy"],
    ["cvm-sqlite", "sqlite"],
    ["cvm-runtime", "runtime"],
    ["cvm-web", "web"],
  ] as const;
  return (
    <section className="surface-panel overflow-hidden" data-testid="system-observability-page">
      <div className="surface-panel-body gap-6">
        <div className="section-heading">
          <h2 className="section-title text-2xl">{t("system.observability.title")}</h2>
          <p className="section-description max-w-3xl">{t("system.observability.description")}</p>
        </div>
        {error ? (
          <Alert variant="error" role="alert">
            {t("system.observability.error")}
          </Alert>
        ) : !capabilities ? (
          <p role="status">{t("system.observability.loading")}</p>
        ) : (
          <>
            <div className="flex flex-wrap gap-3">
              <Chip>{t(`system.observability.state.${capabilities.state}`)}</Chip>
              <Chip>{t("system.observability.connectivityUnknown")}</Chip>
            </div>
            {capabilities.grafanaPublicUrl ? (
              <nav aria-label="Grafana" className="grid gap-3 sm:grid-cols-2">
                {destinations.map(([uid, key]) => (
                  <a
                    key={uid}
                    className="rounded-xl border border-base-300 p-4 text-primary transition-colors hover:bg-primary/10"
                    href={grafanaLink(capabilities.grafanaPublicUrl ?? "", uid) ?? "#"}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    {t(`system.observability.dashboard.${key}`)}
                    <span aria-hidden="true" className="ml-2">
                      ↗
                    </span>
                  </a>
                ))}
              </nav>
            ) : (
              <Alert>{t("system.observability.unconfigured")}</Alert>
            )}
            <p className="text-sm text-base-content/65">{t("system.observability.historyNote")}</p>
          </>
        )}
      </div>
    </section>
  );
}
