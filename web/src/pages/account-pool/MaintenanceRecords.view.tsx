import type { ReactNode } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { Tooltip } from "../../components/ui/tooltip";
import { AppIcon } from "../../features/shared/AppIcon";
import { ListBodyState } from "../../features/shared/ListBodyState";
import type { TranslationValues } from "../../i18n";
import type { UpstreamAccountActionEvent } from "../../lib/api";
import { cn } from "../../lib/utils";

export type ChipTone = "secondary" | "success" | "info" | "warning" | "error";
export type MaintenanceKind = "" | "oauth_codex" | "api_key_codex";
export type Translator = (key: string, options?: TranslationValues) => string;
export type SelectOption = { value: string; label: string };
export type OccurredAt = { time: string; date: string };

export type MaintenanceRecordsViewProps = {
  t: Translator;
  accountFilter: string;
  kindFilter: MaintenanceKind;
  groupFilter: string;
  proxyKeyFilter: string;
  resultFilter: string;
  page: number;
  pageSize: number;
  pageCount: number;
  total: number;
  loading: boolean;
  error: string | null;
  events: UpstreamAccountActionEvent[];
  isInitialLoading: boolean;
  isInitialError: boolean;
  proxyOptions: SelectOption[];
  kindOptions: SelectOption[];
  resultOptions: SelectOption[];
  onAccountFilterChange: (value: string) => void;
  onKindFilterChange: (value: MaintenanceKind) => void;
  onGroupFilterChange: (value: string) => void;
  onProxyFilterChange: (value: string) => void;
  onResultFilterChange: (value: string) => void;
  onPageSizeChange: (value: number) => void;
  onPreviousPage: () => void;
  onNextPage: () => void;
  resetFilters: () => void;
  actionLabel: (action?: string | null) => string | null;
  resultLabel: (result?: string | null) => string | null;
  actionVariant: (action?: string | null) => ChipTone;
  resultVariant: (result?: string | null) => Exclude<ChipTone, "info">;
  descriptionLabel: (event: UpstreamAccountActionEvent) => string;
  egressIpLabel: (event: UpstreamAccountActionEvent) => string;
  formatOccurredAt: (value: string) => OccurredAt;
};

function renderTruncatedValue(value: ReactNode, tooltip: ReactNode, className?: string) {
  return (
    <Tooltip
      content={tooltip}
      side="top"
      contentClassName="max-w-[28rem] break-words leading-5"
      className="min-w-0 max-w-full overflow-hidden whitespace-nowrap align-baseline"
      triggerProps={{ className: "min-w-0 max-w-full overflow-hidden whitespace-nowrap" }}
    >
      <span className={cn("block min-w-0 max-w-full truncate whitespace-nowrap", className)}>
        {value}
      </span>
    </Tooltip>
  );
}

type EventRowsProps = {
  eventId: number;
  occurredAt: OccurredAt;
  eventActionLabel: string;
  eventResultLabel: string;
  accountLabel: string;
  groupLabel: string;
  proxyLabel: string;
  ipLabel: string;
  eventDescriptionLabel: string;
  actionTone: ChipTone;
  resultTone: Exclude<ChipTone, "info">;
};

function MaintenanceEventRows({
  occurredAt,
  eventActionLabel,
  eventResultLabel,
  accountLabel,
  groupLabel,
  proxyLabel,
  ipLabel,
  eventDescriptionLabel,
  actionTone,
  resultTone,
}: EventRowsProps) {
  return (
    <>
      <tr className="border-t border-base-300/60 align-baseline first:border-t-0">
        <td className="whitespace-nowrap px-3 pb-0.5 pt-3 align-baseline text-xs tabular-nums text-base-content/72">
          {renderTruncatedValue(
            occurredAt.time,
            occurredAt.time,
            "font-mono text-[12px] font-semibold leading-4 text-base-content",
          )}
        </td>
        <td className="min-w-0 px-3 pb-0.5 pt-3 align-baseline">
          {renderTruncatedValue(
            accountLabel,
            accountLabel,
            "font-medium leading-4 text-base-content",
          )}
        </td>
        <td className="min-w-0 px-3 pb-0.5 pt-3 align-baseline">
          {renderTruncatedValue(proxyLabel, proxyLabel, "font-medium leading-4 text-base-content")}
        </td>
        <td className="max-w-[18rem] px-3 pb-0.5 pt-3 align-baseline">
          <Tooltip
            content={eventActionLabel}
            side="top"
            contentClassName="max-w-[28rem] break-words leading-5"
            className="max-w-full whitespace-nowrap align-baseline"
            triggerProps={{ className: "max-w-full whitespace-nowrap" }}
          >
            <Chip
              data-maintenance-event-badge="true"
              tone={actionTone}
              className="w-fit max-w-none whitespace-nowrap px-2 py-0 text-[11px] font-semibold leading-5"
            >
              {eventActionLabel}
            </Chip>
          </Tooltip>
        </td>
        <td className="min-w-0 px-3 pb-0.5 pt-3 align-baseline">
          <Tooltip
            content={eventResultLabel}
            side="top"
            contentClassName="max-w-[28rem] break-words leading-5"
            className="max-w-full whitespace-nowrap align-baseline"
            triggerProps={{ className: "max-w-full whitespace-nowrap" }}
          >
            <Chip
              data-maintenance-event-badge="true"
              tone={resultTone}
              className="w-fit max-w-none whitespace-nowrap px-2 py-0 text-[11px] font-semibold leading-5"
            >
              {eventResultLabel}
            </Chip>
          </Tooltip>
        </td>
      </tr>
      <tr>
        <td className="whitespace-nowrap px-3 pb-3 pt-0 align-baseline font-mono text-[11px] leading-4 tabular-nums text-base-content/55">
          {renderTruncatedValue(
            occurredAt.date,
            occurredAt.date,
            "font-mono text-[11px] leading-4 tabular-nums text-base-content/55",
          )}
        </td>
        <td className="min-w-0 px-3 pb-3 pt-0 align-baseline">
          {renderTruncatedValue(groupLabel, groupLabel, "text-xs leading-4 text-base-content/60")}
        </td>
        <td className="min-w-0 px-3 pb-3 pt-0 align-baseline">
          {renderTruncatedValue(
            ipLabel,
            ipLabel,
            "font-mono text-xs leading-4 tabular-nums text-base-content/60",
          )}
        </td>
        <td
          className="min-w-0 px-3 pb-3 pt-0 align-baseline text-xs leading-4 text-base-content/65"
          colSpan={2}
        >
          {renderTruncatedValue(
            eventDescriptionLabel,
            eventDescriptionLabel,
            "text-xs leading-4 text-base-content/65",
          )}
        </td>
      </tr>
    </>
  );
}

function EventLabels({
  event,
  model,
}: {
  event: UpstreamAccountActionEvent;
  model: MaintenanceRecordsViewProps;
}) {
  const { t, actionLabel, resultLabel, descriptionLabel, egressIpLabel, formatOccurredAt } = model;
  const accountLabel =
    event.accountDisplayName ?? t("accountPool.upstreamAccounts.maintenanceEvents.unknownAccount");
  const groupLabel =
    event.accountGroupName ?? t("accountPool.upstreamAccounts.maintenanceEvents.unknownGroup");
  const proxyLabel =
    event.forwardProxyDisplayName ??
    event.forwardProxyKey ??
    t("accountPool.upstreamAccounts.maintenanceEvents.unknownProxy");
  return {
    occurredAt: formatOccurredAt(event.occurredAt),
    eventActionLabel: actionLabel(event.action) ?? event.action,
    eventResultLabel: resultLabel(event.result) ?? event.result ?? "-",
    accountLabel,
    groupLabel,
    proxyLabel,
    ipLabel: egressIpLabel(event),
    eventDescriptionLabel: descriptionLabel(event),
  };
}

function MaintenanceMobileEvent({
  event,
  model,
}: {
  event: UpstreamAccountActionEvent;
  model: MaintenanceRecordsViewProps;
}) {
  const labels = EventLabels({ event, model });
  return (
    <article
      key={`mobile-${event.id}`}
      className="rounded-xl border border-base-300/80 bg-base-100/72 px-4 py-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-medium text-base-content">{labels.accountLabel}</div>
          <div className="mt-1 text-xs text-base-content/65">{labels.groupLabel}</div>
        </div>
        <div className="text-right">
          <div className="font-mono text-sm font-semibold text-base-content">
            {labels.occurredAt.time}
          </div>
          <div className="mt-1 text-xs text-base-content/60">{labels.occurredAt.date}</div>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Chip tone={model.actionVariant(event.action)}>{labels.eventActionLabel}</Chip>
        <Chip tone={model.resultVariant(event.result)}>{labels.eventResultLabel}</Chip>
      </div>
      <dl className="mt-4 grid grid-cols-1 gap-3 text-sm">
        <div className="rounded-lg border border-base-300/70 bg-base-100/70 px-3 py-2.5">
          <dt className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/58">
            {model.t("accountPool.upstreamAccounts.maintenanceEvents.columns.proxy")}
          </dt>
          <dd className="mt-1 text-base-content/78">{labels.proxyLabel}</dd>
        </div>
        <div className="rounded-lg border border-base-300/70 bg-base-100/70 px-3 py-2.5">
          <dt className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/58">
            IP
          </dt>
          <dd className="mt-1 font-mono text-xs text-base-content/72">{labels.ipLabel}</dd>
        </div>
      </dl>
      <div className="mt-4 rounded-lg border border-base-300/70 bg-base-100/70 px-3 py-2.5 text-sm leading-6 text-base-content/72">
        {labels.eventDescriptionLabel}
      </div>
    </article>
  );
}

function MaintenanceDesktopEvents({
  model,
  emptyState,
}: {
  model: MaintenanceRecordsViewProps;
  emptyState: ReactNode;
}) {
  const { events, t, isInitialLoading, isInitialError, actionVariant, resultVariant } = model;
  return (
    <div
      className={cn(
        isInitialLoading || isInitialError ? "hidden" : "hidden overflow-x-auto min-[769px]:block",
      )}
    >
      <table className="min-w-[60rem] table-fixed divide-y divide-base-300/70 text-sm lg:min-w-full">
        <thead className="bg-base-100/80">
          <tr className="text-left text-xs font-semibold uppercase tracking-[0.12em] text-base-content/55">
            <th className="w-[8rem] px-3 py-2.5">
              {t("accountPool.upstreamAccounts.maintenanceEvents.columns.time")}
            </th>
            <th className="w-[15rem] px-3 py-2.5">
              {t("accountPool.upstreamAccounts.maintenanceEvents.columns.account")}
            </th>
            <th className="w-[15rem] px-3 py-2.5">
              {t("accountPool.upstreamAccounts.maintenanceEvents.columns.proxy")}
            </th>
            <th className="px-3 py-2.5">
              {t("accountPool.upstreamAccounts.maintenanceEvents.columns.action")}
            </th>
            <th className="w-[7rem] px-3 py-2.5">
              {t("accountPool.upstreamAccounts.maintenanceEvents.columns.result")}
            </th>
          </tr>
        </thead>
        <tbody>
          {events.length === 0 ? (
            <tr>
              <td className="px-4 py-8 text-center text-sm text-base-content/60" colSpan={5}>
                {emptyState}
              </td>
            </tr>
          ) : (
            events.map((event) => {
              const labels = EventLabels({ event, model });
              return (
                <MaintenanceEventRows
                  key={event.id}
                  eventId={event.id}
                  {...labels}
                  actionTone={actionVariant(event.action)}
                  resultTone={resultVariant(event.result)}
                />
              );
            })
          )}
        </tbody>
      </table>
    </div>
  );
}

function MaintenanceEvents({ model }: { model: MaintenanceRecordsViewProps }) {
  const { events, t, isInitialLoading, isInitialError } = model;
  const emptyState = (
    <ListBodyState
      variant="empty"
      title={t("accountPool.upstreamAccounts.maintenanceEvents.empty")}
      testId="maintenance-records-empty"
    />
  );
  return (
    <div className="overflow-hidden rounded-[1rem] border border-base-300/80 bg-base-100/70">
      {isInitialLoading ? (
        <div className="p-4">
          <ListBodyState
            variant="loading"
            title={t("accountPool.upstreamAccounts.loadingTitle")}
            testId="maintenance-records-loading"
          />
        </div>
      ) : null}
      {isInitialError ? (
        <div className="p-4">
          <ListBodyState
            variant="error"
            title={t("accountPool.upstreamAccounts.maintenanceEvents.loadError")}
            description={model.error}
            testId="maintenance-records-error"
          />
        </div>
      ) : null}
      {model.loading && events.length > 0 ? (
        <Alert variant="info" className="m-4" data-testid="maintenance-records-refreshing">
          <AppIcon name="loading" className="mt-0.5 h-4 w-4 shrink-0 animate-spin" aria-hidden />
          <div>{t("accountPool.upstreamAccounts.loadingTitle")}</div>
        </Alert>
      ) : null}
      <div
        className={cn(
          "space-y-3 min-[769px]:hidden",
          isInitialLoading || isInitialError ? "hidden" : null,
        )}
      >
        {events.length === 0 ? (
          <div className="rounded-xl border border-base-300/80 bg-base-100/72 px-4 py-8 text-center text-sm text-base-content/60">
            {emptyState}
          </div>
        ) : (
          events.map((event) => (
            <MaintenanceMobileEvent key={event.id} event={event} model={model} />
          ))
        )}
      </div>
      <MaintenanceDesktopEvents model={model} emptyState={emptyState} />
    </div>
  );
}

function MaintenanceFilters({ model }: { model: MaintenanceRecordsViewProps }) {
  const { t, kindFilter } = model;
  return (
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
      <label className="field min-w-0">
        <span className="field-label">
          {t("accountPool.upstreamAccounts.maintenanceEvents.filters.account")}
        </span>
        <Input
          value={model.accountFilter}
          onChange={(event) => model.onAccountFilterChange(event.target.value)}
          placeholder={t(
            "accountPool.upstreamAccounts.maintenanceEvents.filters.accountPlaceholder",
          )}
          className="h-11 border-base-300/90 bg-base-100 lg:h-10"
        />
      </label>
      <label className="field min-w-0">
        <span className="field-label">
          {t("accountPool.upstreamAccounts.maintenanceEvents.filters.type")}
        </span>
        <SelectField
          size="sm"
          value={kindFilter}
          options={model.kindOptions}
          triggerClassName="h-11 border-base-300/90 bg-base-100 lg:h-10"
          onValueChange={(value) => model.onKindFilterChange(value as MaintenanceKind)}
        />
      </label>
      {kindFilter !== "api_key_codex" ? (
        <label className="field min-w-0">
          <span className="field-label">
            {t("accountPool.upstreamAccounts.maintenanceEvents.filters.group")}
          </span>
          <Input
            value={model.groupFilter}
            onChange={(event) => model.onGroupFilterChange(event.target.value)}
            placeholder={t(
              "accountPool.upstreamAccounts.maintenanceEvents.filters.groupPlaceholder",
            )}
            className="h-11 border-base-300/90 bg-base-100 lg:h-10"
          />
        </label>
      ) : null}
      <label className="field min-w-0">
        <span className="field-label">
          {t("accountPool.upstreamAccounts.maintenanceEvents.filters.node")}
        </span>
        <SelectField
          size="sm"
          value={model.proxyKeyFilter}
          options={model.proxyOptions}
          triggerClassName="h-11 border-base-300/90 bg-base-100 lg:h-10"
          onValueChange={model.onProxyFilterChange}
        />
      </label>
      <label className="field min-w-0">
        <span className="field-label">
          {t("accountPool.upstreamAccounts.maintenanceEvents.filters.result")}
        </span>
        <SelectField
          size="sm"
          value={model.resultFilter}
          options={model.resultOptions}
          triggerClassName="h-11 border-base-300/90 bg-base-100 lg:h-10"
          onValueChange={model.onResultFilterChange}
        />
      </label>
    </div>
  );
}

function MaintenancePagination({ model }: { model: MaintenanceRecordsViewProps }) {
  const { t, page, pageCount, total, loading } = model;
  return (
    <div className="flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-end sm:justify-between">
      <div className="text-sm text-base-content/70">
        {t("accountPool.upstreamAccounts.pagination.summary", { page, pageCount, total })}
      </div>
      <div className="flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-center">
        <div className="flex items-center justify-between gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2 sm:justify-start">
          <span className="text-sm font-medium text-base-content/65">
            {t("accountPool.upstreamAccounts.pagination.pageSize")}
          </span>
          <SelectField
            className="w-[7rem] min-w-[7rem]"
            value={String(model.pageSize)}
            options={[20, 50, 100].map((value) => ({ value: String(value), label: String(value) }))}
            size="sm"
            triggerClassName="h-11 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm lg:h-10"
            aria-label={t("accountPool.upstreamAccounts.pagination.pageSize")}
            onValueChange={(value) => model.onPageSizeChange(Number(value))}
          />
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-11 rounded-xl px-4 lg:h-10"
            onClick={model.onPreviousPage}
            disabled={loading || page <= 1}
          >
            {t("accountPool.upstreamAccounts.pagination.previous")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-11 rounded-xl px-4 lg:h-10"
            onClick={model.onNextPage}
            disabled={loading || page >= pageCount}
          >
            {t("accountPool.upstreamAccounts.pagination.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function MaintenanceRecordsView({ model }: { model: MaintenanceRecordsViewProps }) {
  const { t, error, events } = model;
  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
          <div className="section-heading">
            <h2 className="section-title">
              {t("accountPool.upstreamAccounts.maintenanceEvents.title")}
            </h2>
            <p className="section-description">
              {t("accountPool.upstreamAccounts.maintenanceEvents.description")}
            </p>
          </div>
          <Tooltip
            content={t("accountPool.upstreamAccounts.maintenanceEvents.resetFilters")}
            side="left"
          >
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-11 w-11 rounded-xl lg:h-10 lg:w-10"
              aria-label={t("accountPool.upstreamAccounts.maintenanceEvents.resetFilters")}
              onClick={model.resetFilters}
            >
              <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
            </Button>
          </Tooltip>
        </div>
        <MaintenanceFilters model={model} />
        {error && events.length > 0 ? (
          <Alert variant="error">
            <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
            <div>{error}</div>
          </Alert>
        ) : null}
        <MaintenanceEvents model={model} />
        <MaintenancePagination model={model} />
      </div>
    </section>
  );
}
