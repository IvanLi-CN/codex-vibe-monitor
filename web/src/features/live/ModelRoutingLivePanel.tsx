import { useMemo } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { useTranslation } from "../../i18n";
import type { ModelRoutingLiveResponse, ModelRoutingLiveWindow } from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";
import { ModelRoutingGantt } from "./ModelRoutingGantt";

export interface ModelRoutingLivePanelProps {
  data?: ModelRoutingLiveResponse | null;
  isLoading: boolean;
  error?: string | null;
  window: ModelRoutingLiveWindow;
  onWindowChange: (value: ModelRoutingLiveWindow) => void;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
  onRefresh: () => void;
}

export function ModelRoutingLivePanel({
  data,
  isLoading,
  error,
  window,
  onWindowChange,
  onOpenAccount,
  onOpenInvocation,
  onRefresh,
}: ModelRoutingLivePanelProps) {
  const { t } = useTranslation();
  const records = data?.records ?? [];
  const groups = useMemo(() => {
    const currentGroups = data?.groups ?? [];
    const knownGroupModels = new Set(currentGroups.map((group) => group.model));
    const recordOnlyGroups = Array.from(new Set(records.map((record) => record.model)))
      .filter((modelName) => !knownGroupModels.has(modelName))
      .sort()
      .map((modelName) => ({ model: modelName, accounts: [] }));
    return [...currentGroups, ...recordOnlyGroups];
  }, [data?.groups, records]);

  return (
    <section className="surface-panel" data-testid="model-routing-live-panel">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-col gap-3 desktop:flex-row desktop:items-start desktop:justify-between">
          <div className="section-heading">
            <h1 className="section-title">{t("live.routing.title")}</h1>
            <p className="section-description">{t("live.routing.description")}</p>
          </div>
          <div
            className="flex min-w-0 flex-wrap items-end gap-2 desktop:justify-end"
            data-testid="model-routing-live-controls"
          >
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="shrink-0"
              onClick={onRefresh}
              disabled={isLoading}
            >
              <AppIcon name="refresh" className="mr-1.5 h-4 w-4" aria-hidden />
              {t("live.routing.refresh")}
            </Button>
            <SegmentedControl role="group" aria-label={t("live.routing.windowLabel")}>
              {(["15m", "1h", "6h", "24h"] as ModelRoutingLiveWindow[]).map((value) => (
                <SegmentedControlItem
                  key={value}
                  active={window === value}
                  onClick={() => onWindowChange(value)}
                >
                  {value}
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          </div>
        </div>
        {error ? (
          <Alert variant="warning">
            <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            <span>{error}</span>
          </Alert>
        ) : null}
        {isLoading && !data ? (
          <p className="text-sm text-base-content/70">{t("live.routing.loading")}</p>
        ) : null}
        {!isLoading && !error && groups.length === 0 ? (
          <p className="text-sm text-base-content/70">{t("live.routing.empty")}</p>
        ) : null}
        {groups.length > 0 ? (
          <ModelRoutingGantt
            groups={groups}
            records={records}
            generatedAt={data?.generatedAt}
            window={window}
            onOpenAccount={onOpenAccount}
            onOpenInvocation={onOpenInvocation}
          />
        ) : null}
      </div>
    </section>
  );
}
