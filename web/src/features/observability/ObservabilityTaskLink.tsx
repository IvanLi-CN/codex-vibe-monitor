import { type JSX, useEffect, useState } from "react";
import { useTranslation } from "../../i18n";
import { fetchObservabilityCapabilities } from "../../lib/api";
import { grafanaLink } from "../../lib/observabilityLinks";
export function ObservabilityTaskLink({ taskKey }: { taskKey: string }): JSX.Element {
  const { t } = useTranslation();
  const [url, setUrl] = useState<string | undefined>();
  useEffect(() => {
    let active = true;
    setUrl(undefined);
    void fetchObservabilityCapabilities()
      .then((value) => {
        if (active && value.grafanaPublicUrl)
          setUrl(grafanaLink(value.grafanaPublicUrl, "cvm-runtime", taskKey));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [taskKey]);
  return (
    <div className="rounded-xl border border-base-300 p-4" data-testid="task-observability-link">
      {url ? (
        <a href={url} target="_blank" rel="noopener noreferrer" className="text-primary">
          {t("system.observability.dashboard.runtime")} ↗
        </a>
      ) : (
        <p className="text-sm text-base-content/65">{t("system.observability.unconfigured")}</p>
      )}
    </div>
  );
}
