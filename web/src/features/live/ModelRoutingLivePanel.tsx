import { useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { useTranslation } from "../../i18n";
import type {
  ModelRoutingLiveResponse,
  ModelRoutingLiveWindow,
  ModelRoutingTimelineRecord,
} from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";
import { ModelIdentity } from "../shared/ModelIdentity";
import { ModelRoutingGantt } from "./ModelRoutingGantt";

function statusLabel(state: string, t: (key: string) => string) {
  const key = `live.routing.states.${state}`;
  const translated = t(key);
  return translated === key ? state : translated;
}

export interface ModelRoutingLivePanelProps {
  data?: ModelRoutingLiveResponse | null;
  isLoading: boolean;
  error?: string | null;
  window: ModelRoutingLiveWindow;
  model?: string;
  state?: string;
  onWindowChange: (value: ModelRoutingLiveWindow) => void;
  onModelChange: (value: string) => void;
  onStateChange: (value: string) => void;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
  onRefresh: () => void;
}

export function ModelRoutingLivePanel({
  data,
  isLoading,
  error,
  window,
  model = "",
  state = "",
  onWindowChange,
  onModelChange,
  onStateChange,
  onOpenAccount,
  onOpenInvocation,
  onRefresh,
}: ModelRoutingLivePanelProps) {
  const { t } = useTranslation();
  const [knownModels, setKnownModels] = useState<string[]>([]);
  useEffect(() => {
    const nextModels = data?.groups.map((group) => group.model) ?? [];
    if (nextModels.length === 0) return;
    setKnownModels((current) => Array.from(new Set([...current, ...nextModels])).sort());
  }, [data]);
  const models = useMemo(() => knownModels, [knownModels]);
  const records = data?.records ?? [];
  const recordsByModel = useMemo(() => {
    const grouped = new Map<string, ModelRoutingTimelineRecord[]>();
    for (const record of records) {
      const current = grouped.get(record.model) ?? [];
      current.push(record);
      grouped.set(record.model, current);
    }
    return grouped;
  }, [records]);
  const groups = useMemo(() => {
    const currentGroups = data?.groups ?? [];
    const knownGroupModels = new Set(currentGroups.map((group) => group.model));
    const recordOnlyGroups = Array.from(recordsByModel.keys())
      .filter((modelName) => !knownGroupModels.has(modelName))
      .sort()
      .map((modelName) => ({ model: modelName, accounts: [] }));
    return [...currentGroups, ...recordOnlyGroups];
  }, [data?.groups, recordsByModel]);

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
            <SelectField
              label={t("live.routing.stateLabel")}
              name="modelRoutingState"
              size="sm"
              className="w-40"
              value={state}
              options={[
                { value: "", label: t("live.routing.allStates") },
                { value: "available", label: statusLabel("available", t) },
                { value: "degraded", label: statusLabel("degraded", t) },
                { value: "cooling_down", label: statusLabel("cooling_down", t) },
              ]}
              onValueChange={onStateChange}
            />
            <SelectField
              label={t("live.routing.modelLabel")}
              name="modelRoutingModel"
              size="sm"
              className="w-44"
              value={model}
              options={[
                { value: "", label: t("live.routing.allModels") },
                ...models.map((value) => ({ value, label: value })),
              ]}
              onValueChange={onModelChange}
            />
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
        <div className="grid gap-3">
          {groups.map((group) => {
            const modelRecords = recordsByModel.get(group.model) ?? [];
            return (
              <section
                key={group.model}
                className="surface-subtle overflow-hidden rounded-lg"
                data-testid={`model-routing-model-group-${group.model}`}
              >
                <div className="flex items-center justify-between gap-3 px-3 py-2">
                  <h2 className="min-w-0">
                    <ModelIdentity
                      model={group.model}
                      textClassName="truncate font-mono text-sm font-semibold"
                    />
                  </h2>
                  <div className="flex shrink-0 items-center gap-3 text-xs tabular-nums text-base-content/65">
                    <span>{t("live.routing.accountsCount", { count: group.accounts.length })}</span>
                    <span>
                      {t("live.routing.modelRecordsCount", { count: modelRecords.length })}
                    </span>
                  </div>
                </div>
                <ModelRoutingGantt
                  model={group.model}
                  accounts={group.accounts}
                  records={modelRecords}
                  generatedAt={data?.generatedAt}
                  window={window}
                  onOpenAccount={onOpenAccount}
                  onOpenInvocation={onOpenInvocation}
                />
              </section>
            );
          })}
        </div>
      </div>
    </section>
  );
}
