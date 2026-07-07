import { Fragment, type ReactNode, useEffect, useMemo, useState } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import { ListBodyState } from "../../features/shared/ListBodyState";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { Tooltip } from "../../components/ui/tooltip";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import { useTranslation } from "../../i18n";
import {
  fetchUpstreamAccountActionEvents,
  type FetchUpstreamAccountActionEventsQuery,
  type UpstreamAccountActionEvent,
} from "../../lib/api";
import { cn } from "../../lib/utils";

type BadgeVariant = "secondary" | "success" | "info" | "warning" | "error";

const pageSizeOptions = [20, 50, 100].map((value) => ({
  value: String(value),
  label: String(value),
}));

function renderTruncatedValue(value: ReactNode, tooltip: ReactNode, className?: string) {
  return (
    <Tooltip
      content={tooltip}
      side="top"
      contentClassName="max-w-[28rem] break-words leading-5"
      className="min-w-0 max-w-full overflow-hidden whitespace-nowrap align-baseline"
      triggerProps={{
        className: "min-w-0 max-w-full overflow-hidden whitespace-nowrap",
      }}
    >
      <span className={cn("block min-w-0 max-w-full truncate whitespace-nowrap", className)}>
        {value}
      </span>
    </Tooltip>
  );
}

function formatOccurredAt(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return { time: value, date: value };
  }
  return {
    time: new Intl.DateTimeFormat("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(date),
    date: new Intl.DateTimeFormat("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).format(date),
  };
}

function humanizeAction(value: string) {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export default function MaintenanceRecordsPage() {
  const { t } = useTranslation();
  const [accountFilter, setAccountFilter] = useState("");
  const [groupFilter, setGroupFilter] = useState("");
  const [proxyKeyFilter, setProxyKeyFilter] = useState("");
  const [resultFilter, setResultFilter] = useState("");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [events, setEvents] = useState<UpstreamAccountActionEvent[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { forwardProxyNodes = [] } = useUpstreamAccounts({ includeAll: true });

  const query = useMemo<FetchUpstreamAccountActionEventsQuery>(
    () => ({
      account: accountFilter.trim() || undefined,
      group: groupFilter.trim() || undefined,
      proxyKey: proxyKeyFilter.trim() || undefined,
      result: resultFilter.trim() || undefined,
      page,
      pageSize,
    }),
    [accountFilter, groupFilter, page, pageSize, proxyKeyFilter, resultFilter],
  );

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void fetchUpstreamAccountActionEvents(query)
      .then((response) => {
        if (controller.signal.aborted) return;
        setEvents(response.items);
        setTotal(response.total);
        setPage(response.page);
        setPageSize(response.pageSize);
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setEvents((currentEvents) => {
          if (currentEvents.length > 0) return currentEvents;
          setTotal(0);
          return [];
        });
      })
      .finally(() => {
        if (controller.signal.aborted) return;
        setLoading(false);
      });
    return () => {
      controller.abort();
    };
  }, [query]);

  const proxyOptions = useMemo(
    () => [
      {
        value: "",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.allNodes"),
      },
      ...forwardProxyNodes.map((node) => ({
        value: node.key,
        label: node.displayName,
      })),
    ],
    [forwardProxyNodes, t],
  );
  const resultOptions = useMemo(
    () => [
      {
        value: "",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.allResults"),
      },
      {
        value: "success",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.results.success"),
      },
      {
        value: "failed",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.results.failed"),
      },
      {
        value: "deferred",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.results.deferred"),
      },
    ],
    [t],
  );
  const proxyNodeByKey = useMemo(
    () => new Map(forwardProxyNodes.map((node) => [node.key, node])),
    [forwardProxyNodes],
  );
  const pageCount = Math.max(1, Math.ceil(total / Math.max(pageSize, 1)));
  const isInitialLoading = loading && events.length === 0;
  const isInitialError = Boolean(error) && events.length === 0;

  const resetFilters = () => {
    setAccountFilter("");
    setGroupFilter("");
    setProxyKeyFilter("");
    setResultFilter("");
    setPage(1);
  };
  const actionLabel = (action?: string | null) => {
    if (!action) return null;
    const key = `accountPool.upstreamAccounts.maintenanceEvents.actions.${action}`;
    const translated = t(key);
    if (translated !== key) return translated;
    const latestKey = `accountPool.upstreamAccounts.latestAction.actions.${action}`;
    const latestTranslated = t(latestKey);
    return latestTranslated === latestKey ? humanizeAction(action) : latestTranslated;
  };
  const reasonLabel = (reason?: string | null) => {
    if (!reason) return null;
    const key = `accountPool.upstreamAccounts.latestAction.reasons.${reason}`;
    const translated = t(key);
    return translated === key ? reason : translated;
  };
  const resultLabel = (result?: string | null) => {
    if (!result) return null;
    const key = `accountPool.upstreamAccounts.maintenanceEvents.results.${result}`;
    const translated = t(key);
    return translated === key ? result : translated;
  };
  const actionVariant = (action?: string | null): BadgeVariant => {
    if (!action) return "secondary";
    if (action.includes("succeeded") || action.includes("recovered")) return "success";
    if (action.includes("deferred") || action.includes("cooldown") || action.includes("blocked")) {
      return "warning";
    }
    if (action.includes("failed") || action.includes("failure") || action.includes("unavailable")) {
      return "error";
    }
    if (action.includes("updated")) return "info";
    return "secondary";
  };
  const resultVariant = (result?: string | null): Exclude<BadgeVariant, "info"> => {
    switch (result) {
      case "success":
        return "success";
      case "failed":
        return "error";
      case "deferred":
        return "warning";
      default:
        return "secondary";
    }
  };
  const descriptionLabel = (event: UpstreamAccountActionEvent) => {
    if (event.reasonCode === "egress_throttled") {
      const retryAfter = event.reasonMessage?.match(/another\s+(\d+)\s+seconds/i)?.[1];
      const proxyName =
        event.forwardProxyDisplayName ??
        event.forwardProxyKey ??
        t("accountPool.upstreamAccounts.maintenanceEvents.unknownProxy");
      return retryAfter
        ? t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.egressThrottledWithSeconds", {
            proxy: proxyName,
            seconds: retryAfter,
          })
        : t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.egressThrottled", {
            proxy: proxyName,
          });
    }
    if (event.reasonCode === "sync_ok") {
      return t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.syncOk");
    }
    if (event.reasonCode === "upstream_http_429") {
      return t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.upstream429");
    }
    if (event.reasonCode === "sync_error") {
      return t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.syncError");
    }
    if (event.action === "status_change_suppressed") {
      const parts = [
        reasonLabel(event.reasonCode) ?? event.reasonCode,
        Number.isFinite(event.httpStatus ?? NaN)
          ? `HTTP ${event.httpStatus}`
          : null,
        event.reasonMessage ?? event.resultDescription,
      ].filter((value): value is string => Boolean(value));
      return parts.join(" · ") || t("accountPool.upstreamAccounts.maintenanceEvents.noDescription");
    }
    if (event.httpStatus != null) {
      return t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.httpStatus", {
        status: event.httpStatus,
      });
    }
    return (
      event.resultDescription ??
      event.reasonMessage ??
      t("accountPool.upstreamAccounts.maintenanceEvents.noDescription")
    );
  };
  const egressIpLabel = (event: UpstreamAccountActionEvent) => {
    if (event.forwardProxyEgressIp) return event.forwardProxyEgressIp;
    const liveNode = event.forwardProxyKey ? proxyNodeByKey.get(event.forwardProxyKey) : null;
    if (liveNode?.egressIp) return liveNode.egressIp;
    if (liveNode?.selectable) {
      return t("accountPool.upstreamAccounts.maintenanceEvents.egressIpPending");
    }
    return t("accountPool.upstreamAccounts.maintenanceEvents.noEgressIp");
  };

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
              onClick={resetFilters}
            >
              <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
            </Button>
          </Tooltip>
        </div>

        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <label className="field min-w-0">
            <span className="field-label">
              {t("accountPool.upstreamAccounts.maintenanceEvents.filters.account")}
            </span>
            <Input
              value={accountFilter}
              onChange={(event) => {
                setAccountFilter(event.target.value);
                setPage(1);
              }}
              placeholder={t("accountPool.upstreamAccounts.maintenanceEvents.filters.accountPlaceholder")}
              className="h-11 border-base-300/90 bg-base-100 lg:h-10"
            />
          </label>
          <label className="field min-w-0">
            <span className="field-label">
              {t("accountPool.upstreamAccounts.maintenanceEvents.filters.group")}
            </span>
            <Input
              value={groupFilter}
              onChange={(event) => {
                setGroupFilter(event.target.value);
                setPage(1);
              }}
              placeholder={t("accountPool.upstreamAccounts.maintenanceEvents.filters.groupPlaceholder")}
              className="h-11 border-base-300/90 bg-base-100 lg:h-10"
            />
          </label>
          <label className="field min-w-0">
            <span className="field-label">
              {t("accountPool.upstreamAccounts.maintenanceEvents.filters.node")}
            </span>
            <SelectField
              size="sm"
              value={proxyKeyFilter}
              options={proxyOptions}
              triggerClassName="h-11 border-base-300/90 bg-base-100 lg:h-10"
              onValueChange={(value) => {
                setProxyKeyFilter(value);
                setPage(1);
              }}
            />
          </label>
          <label className="field min-w-0">
            <span className="field-label">
              {t("accountPool.upstreamAccounts.maintenanceEvents.filters.result")}
            </span>
            <SelectField
              size="sm"
              value={resultFilter}
              options={resultOptions}
              triggerClassName="h-11 border-base-300/90 bg-base-100 lg:h-10"
              onValueChange={(value) => {
                setResultFilter(value);
                setPage(1);
              }}
            />
          </label>
        </div>

        {error && events.length > 0 ? (
          <Alert variant="error">
            <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
            <div>{error}</div>
          </Alert>
        ) : null}

        <div className="overflow-hidden rounded-[1rem] border border-base-300/80 bg-base-100/70">
          {isInitialLoading ? (
            <div className="p-4">
              <ListBodyState
                variant="loading"
                title={t("accountPool.upstreamAccounts.loadingTitle")}
                testId="maintenance-records-loading"
              />
            </div>
          ) : isInitialError ? (
            <div className="p-4">
              <ListBodyState
                variant="error"
                title={t("accountPool.upstreamAccounts.maintenanceEvents.loadError")}
                description={error}
                testId="maintenance-records-error"
              />
            </div>
          ) : null}
          {loading && events.length > 0 ? (
            <Alert
              variant="info"
              className="m-4"
              data-testid="maintenance-records-refreshing"
            >
              <AppIcon name="loading" className="mt-0.5 h-4 w-4 shrink-0 animate-spin" aria-hidden />
              <div>{t("accountPool.upstreamAccounts.loadingTitle")}</div>
            </Alert>
          ) : null}
          <div className={isInitialLoading || isInitialError ? "hidden" : "overflow-x-auto"}>
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
                      <ListBodyState
                        variant="empty"
                        title={t("accountPool.upstreamAccounts.maintenanceEvents.empty")}
                        testId="maintenance-records-empty"
                      />
                    </td>
                  </tr>
                ) : (
                  events.map((event) => {
                    const occurredAt = formatOccurredAt(event.occurredAt);
                    const eventActionLabel = actionLabel(event.action) ?? event.action;
                    const eventResultLabel = resultLabel(event.result) ?? event.result ?? "-";
                    const accountLabel =
                      event.accountDisplayName ??
                      t("accountPool.upstreamAccounts.maintenanceEvents.unknownAccount");
                    const groupLabel =
                      event.accountGroupName ??
                      t("accountPool.upstreamAccounts.maintenanceEvents.unknownGroup");
                    const proxyLabel =
                      event.forwardProxyDisplayName ??
                      event.forwardProxyKey ??
                      t("accountPool.upstreamAccounts.maintenanceEvents.unknownProxy");
                    const ipLabel = egressIpLabel(event);
                    const eventDescriptionLabel = descriptionLabel(event);
                    return (
                      <Fragment key={event.id}>
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
                            {renderTruncatedValue(
                              proxyLabel,
                              proxyLabel,
                              "font-medium leading-4 text-base-content",
                            )}
                          </td>
                          <td className="max-w-[18rem] px-3 pb-0.5 pt-3 align-baseline">
                            <Tooltip
                              content={eventActionLabel}
                              side="top"
                              contentClassName="max-w-[28rem] break-words leading-5"
                              className="max-w-full whitespace-nowrap align-baseline"
                              triggerProps={{ className: "max-w-full whitespace-nowrap" }}
                            >
                              <Badge
                                data-maintenance-event-badge="true"
                                variant={actionVariant(event.action)}
                                className="w-fit max-w-none whitespace-nowrap px-2 py-0 text-[11px] font-semibold leading-5"
                              >
                                {eventActionLabel}
                              </Badge>
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
                              <Badge
                                data-maintenance-event-badge="true"
                                variant={resultVariant(event.result)}
                                className="w-fit max-w-none whitespace-nowrap px-2 py-0 text-[11px] font-semibold leading-5"
                              >
                                {eventResultLabel}
                              </Badge>
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
                          <td className="min-w-0 px-3 pb-3 pt-0 align-baseline text-xs leading-4 text-base-content/65" colSpan={2}>
                            {renderTruncatedValue(
                              eventDescriptionLabel,
                              eventDescriptionLabel,
                              "text-xs leading-4 text-base-content/65",
                            )}
                          </td>
                        </tr>
                      </Fragment>
                    );
                  })
                )}
              </tbody>
            </table>
          </div>
        </div>

        <div className="flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-end sm:justify-between">
          <div className="text-sm text-base-content/70">
            {t("accountPool.upstreamAccounts.pagination.summary", {
              page,
              pageCount,
              total,
            })}
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex items-center gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2">
              <span className="text-sm font-medium text-base-content/65">
                {t("accountPool.upstreamAccounts.pagination.pageSize")}
              </span>
              <SelectField
                className="min-w-[7rem]"
                value={String(pageSize)}
                options={pageSizeOptions}
                size="sm"
                triggerClassName="h-11 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm lg:h-10"
                aria-label={t("accountPool.upstreamAccounts.pagination.pageSize")}
                onValueChange={(value) => {
                  setPageSize(Number(value));
                  setPage(1);
                }}
              />
            </div>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-11 rounded-xl px-4 lg:h-10"
                onClick={() => setPage((current) => Math.max(1, current - 1))}
                disabled={loading || page <= 1}
              >
                {t("accountPool.upstreamAccounts.pagination.previous")}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-11 rounded-xl px-4 lg:h-10"
                onClick={() => setPage((current) => Math.min(pageCount, current + 1))}
                disabled={loading || page >= pageCount}
              >
                {t("accountPool.upstreamAccounts.pagination.next")}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
