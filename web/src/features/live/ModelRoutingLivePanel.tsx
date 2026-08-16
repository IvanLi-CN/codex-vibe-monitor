import { useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { useTranslation } from "../../i18n";
import type {
  ModelRoutingLiveAccount,
  ModelRoutingLiveResponse,
  ModelRoutingLiveWindow,
  ModelRoutingTimelineRecord,
} from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";
import { ModelIdentity } from "../shared/ModelIdentity";

function formatBeijing(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    timeZone: "Asia/Shanghai",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function statusTone(state: string): "success" | "warning" | "secondary" {
  if (state === "available") return "success";
  if (state === "cooling_down") return "warning";
  return "secondary";
}

function statusLabel(state: string, t: (key: string) => string) {
  const key = `live.routing.states.${state}`;
  const translated = t(key);
  return translated === key ? state : translated;
}

function attemptLabel(
  record: ModelRoutingTimelineRecord,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (record.kind === "event") return t("live.routing.record.event");
  if ((record.sameAccountRetryIndex ?? 0) > 0) {
    return t("live.routing.record.retry", { index: record.sameAccountRetryIndex ?? 0 });
  }
  return t("live.routing.record.attempt");
}

function AccountRow({
  account,
  model,
  onOpen,
}: {
  account: ModelRoutingLiveAccount;
  model: string;
  onOpen: (accountId: number, model: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-t border-base-300/60 px-3 py-2 text-left transition hover:bg-base-200/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      onClick={() => onOpen(account.accountId, model)}
      data-testid={`model-routing-account-${account.accountId}-${model}`}
    >
      <span className="min-w-0">
        <span className="block truncate text-sm font-medium text-base-content">
          {account.accountDisplayName}
        </span>
        <span className="block truncate text-xs text-base-content/65">
          {account.accountGroupName || t("live.routing.account.ungrouped")}
        </span>
      </span>
      <Chip tone={statusTone(account.state)}>{statusLabel(account.state, t)}</Chip>
      <span className="text-xs tabular-nums text-base-content/65">
        {account.cacheConcurrencyLimit != null
          ? t("live.routing.account.limit", { limit: account.cacheConcurrencyLimit })
          : t("live.routing.account.unlimited")}
      </span>
    </button>
  );
}

function RecordRow({
  record,
  expanded,
  onToggle,
  onOpenAccount,
  onOpenInvocation,
}: {
  record: ModelRoutingTimelineRecord;
  expanded: boolean;
  onToggle: () => void;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
}) {
  const { t } = useTranslation();
  const audit = record.routingSelectionAudit;
  return (
    <div
      className="border-t border-base-300/60 px-3 py-2"
      data-testid={`model-routing-record-${record.id}`}
    >
      <div className="flex min-w-0 items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          aria-label={
            expanded ? t("live.routing.record.collapse") : t("live.routing.record.expand")
          }
          onClick={onToggle}
        >
          <AppIcon
            name={expanded ? "chevron-up" : "chevron-down"}
            className="h-4 w-4"
            aria-hidden
          />
        </Button>
        <button
          type="button"
          className="min-w-0 flex-1 truncate text-left text-sm font-medium text-base-content hover:underline"
          onClick={() => onOpenAccount(record.accountId, record.model)}
        >
          {record.accountDisplayName}
        </button>
        <Chip tone={record.status === "success" ? "success" : "secondary"}>
          {attemptLabel(record, t)}
        </Chip>
        <span className="hidden shrink-0 text-xs tabular-nums text-base-content/65 sm:inline">
          {formatBeijing(record.occurredAt)}
        </span>
        {record.invokeId ? (
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-7 w-7 shrink-0"
            title={t("live.routing.record.openInvocation")}
            aria-label={t("live.routing.record.openInvocation")}
            onClick={() => onOpenInvocation(record.invokeId ?? "")}
          >
            <AppIcon name="chevron-right-circle" className="h-4 w-4" aria-hidden />
          </Button>
        ) : null}
      </div>
      <div className="mt-1 pl-9 text-xs tabular-nums text-base-content/65 sm:hidden">
        {formatBeijing(record.occurredAt)}
      </div>
      {expanded ? (
        <div className="mt-2 grid gap-2 rounded-md bg-base-200/60 px-3 py-2 pl-9 text-xs text-base-content/75 sm:grid-cols-2">
          <div>
            <span className="font-semibold text-base-content">
              {t("live.routing.record.reason")}
            </span>{" "}
            {record.reasonCode || record.action || t("live.routing.record.unknown")}
          </div>
          <div>
            <span className="font-semibold text-base-content">
              {t("live.routing.record.result")}
            </span>{" "}
            {record.httpStatus
              ? `HTTP ${record.httpStatus}`
              : record.status || t("live.routing.record.unknown")}
            {record.totalLatencyMs != null ? ` · ${Math.round(record.totalLatencyMs)} ms` : ""}
          </div>
          {audit ? (
            <div className="sm:col-span-2">
              <span className="font-semibold text-base-content">
                {t("live.routing.record.comparison")}
              </span>{" "}
              {audit.winnerReasonCode} ·{" "}
              {t("live.routing.record.eligible", { count: audit.eligibleCandidateCount })}
              {audit.comparedAccountName
                ? ` · ${t("live.routing.record.compared", { account: audit.comparedAccountName })}`
                : ""}
            </div>
          ) : null}
          {record.modelRouteStateBefore || record.modelRouteStateAfter ? (
            <div className="sm:col-span-2">
              <span className="font-semibold text-base-content">
                {t("live.routing.record.transition")}
              </span>{" "}
              {record.modelRouteStateBefore || "-"} → {record.modelRouteStateAfter || "-"}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
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
  const [expandedRecords, setExpandedRecords] = useState<Set<string>>(new Set());
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
            <h2 className="section-title">{t("live.routing.title")}</h2>
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
            <SegmentedControl aria-label={t("live.routing.windowLabel")}>
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
              <div
                key={group.model}
                className="surface-subtle overflow-hidden rounded-lg"
                data-testid={`model-routing-model-group-${group.model}`}
              >
                <div className="flex items-center justify-between gap-3 px-3 py-2">
                  <ModelIdentity
                    model={group.model}
                    textClassName="truncate font-mono text-sm font-semibold"
                  />
                  <div className="flex shrink-0 items-center gap-3 text-xs tabular-nums text-base-content/65">
                    <span>{t("live.routing.accountsCount", { count: group.accounts.length })}</span>
                    <span>
                      {t("live.routing.modelRecordsCount", { count: modelRecords.length })}
                    </span>
                  </div>
                </div>
                {group.accounts.map((account) => (
                  <AccountRow
                    key={account.accountId}
                    account={account}
                    model={group.model}
                    onOpen={onOpenAccount}
                  />
                ))}
                <div
                  className="border-t border-base-300/60"
                  data-testid={`model-routing-model-records-${group.model}`}
                >
                  <div className="flex items-center justify-between gap-3 px-3 py-2">
                    <h3 className="text-xs font-semibold text-base-content/75">
                      {t("live.routing.modelRecordsTitle", { model: group.model })}
                    </h3>
                    <span className="text-xs tabular-nums text-base-content/65">
                      {modelRecords.length}/100
                    </span>
                  </div>
                  {modelRecords.length === 0 ? (
                    <p className="border-t border-base-300/60 px-3 py-3 text-sm text-base-content/70">
                      {t("live.routing.modelRecordsEmpty")}
                    </p>
                  ) : (
                    modelRecords.map((record) => (
                      <RecordRow
                        key={record.id}
                        record={record}
                        expanded={expandedRecords.has(record.id)}
                        onToggle={() =>
                          setExpandedRecords((current) => {
                            const next = new Set(current);
                            if (next.has(record.id)) next.delete(record.id);
                            else next.add(record.id);
                            return next;
                          })
                        }
                        onOpenAccount={onOpenAccount}
                        onOpenInvocation={onOpenInvocation}
                      />
                    ))
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
