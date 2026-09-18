import { useEffect, useMemo, useState } from "react";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import { useTranslation } from "../../i18n";
import {
  type FetchUpstreamAccountActionEventsQuery,
  type ForwardProxyBindingNode,
  fetchUpstreamAccountActionEvents,
  type UpstreamAccountActionEvent,
} from "../../lib/api";
import {
  type ChipTone,
  type MaintenanceKind,
  MaintenanceRecordsView,
  type MaintenanceRecordsViewProps,
  type SelectOption,
  type Translator,
} from "./MaintenanceRecords.view";

type FilterModel = Pick<
  MaintenanceRecordsViewProps,
  | "accountFilter"
  | "kindFilter"
  | "groupFilter"
  | "proxyKeyFilter"
  | "resultFilter"
  | "page"
  | "pageSize"
  | "proxyOptions"
  | "kindOptions"
  | "resultOptions"
  | "onAccountFilterChange"
  | "onKindFilterChange"
  | "onGroupFilterChange"
  | "onProxyFilterChange"
  | "onResultFilterChange"
  | "onPageSizeChange"
  | "onPreviousPage"
  | "onNextPage"
  | "resetFilters"
> & {
  query: FetchUpstreamAccountActionEventsQuery;
  proxyNodeByKey: Map<string, ForwardProxyBindingNode>;
};

function useMaintenanceFilters(
  t: Translator,
  forwardProxyNodes: ForwardProxyBindingNode[],
): FilterModel {
  const [accountFilter, setAccountFilter] = useState("");
  const [kindFilter, setKindFilter] = useState<MaintenanceKind>("");
  const [groupFilter, setGroupFilter] = useState("");
  const [proxyKeyFilter, setProxyKeyFilter] = useState("");
  const [resultFilter, setResultFilter] = useState("");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const query = useMemo<FetchUpstreamAccountActionEventsQuery>(
    () => ({
      account: accountFilter.trim() || undefined,
      kind: kindFilter || undefined,
      group: kindFilter === "api_key_codex" ? undefined : groupFilter.trim() || undefined,
      proxyKey: proxyKeyFilter.trim() || undefined,
      result: resultFilter.trim() || undefined,
      page,
      pageSize,
    }),
    [accountFilter, groupFilter, kindFilter, page, pageSize, proxyKeyFilter, resultFilter],
  );
  const options = useMaintenanceOptions(t, forwardProxyNodes);
  const resetFilters = () => {
    setAccountFilter("");
    setKindFilter("");
    setGroupFilter("");
    setProxyKeyFilter("");
    setResultFilter("");
    setPage(1);
  };
  return {
    accountFilter,
    kindFilter,
    groupFilter,
    proxyKeyFilter,
    resultFilter,
    page,
    pageSize,
    ...options,
    query,
    onAccountFilterChange: (value) => {
      setAccountFilter(value);
      setPage(1);
    },
    onKindFilterChange: (value) => {
      setKindFilter(value);
      if (value === "api_key_codex") setGroupFilter("");
      setPage(1);
    },
    onGroupFilterChange: (value) => {
      setGroupFilter(value);
      setPage(1);
    },
    onProxyFilterChange: (value) => {
      setProxyKeyFilter(value);
      setPage(1);
    },
    onResultFilterChange: (value) => {
      setResultFilter(value);
      setPage(1);
    },
    onPageSizeChange: (value) => {
      setPageSize(value);
      setPage(1);
    },
    onPreviousPage: () => setPage((current) => Math.max(1, current - 1)),
    onNextPage: () => setPage((current) => current + 1),
    resetFilters,
  };
}

function useMaintenanceOptions(t: Translator, forwardProxyNodes: ForwardProxyBindingNode[]) {
  const proxyOptions = useMemo<SelectOption[]>(
    () => [
      { value: "", label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.allNodes") },
      ...forwardProxyNodes.map((node) => ({ value: node.key, label: node.displayName })),
    ],
    [forwardProxyNodes, t],
  );
  const kindOptions = useMemo<SelectOption[]>(
    () => [
      { value: "", label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.allTypes") },
      {
        value: "oauth_codex",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.pool"),
      },
      {
        value: "api_key_codex",
        label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.transit"),
      },
    ],
    [t],
  );
  const resultOptions = useMemo<SelectOption[]>(
    () => [
      { value: "", label: t("accountPool.upstreamAccounts.maintenanceEvents.filters.allResults") },
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
  return { proxyOptions, kindOptions, resultOptions, proxyNodeByKey };
}

function useMaintenanceEvents(query: FetchUpstreamAccountActionEventsQuery) {
  const [events, setEvents] = useState<UpstreamAccountActionEvent[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void fetchUpstreamAccountActionEvents(query)
      .then((response) => {
        if (controller.signal.aborted) return;
        setEvents(response.items);
        setTotal(response.total);
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setEvents((currentEvents) => (currentEvents.length > 0 ? currentEvents : []));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [query]);
  return { events, total, loading, error };
}

function humanizeAction(value: string) {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatOccurredAt(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return { time: value, date: value };
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

function useMaintenanceLabels(t: Translator, proxyNodeByKey: Map<string, ForwardProxyBindingNode>) {
  const actionLabel = (action?: string | null) => {
    if (!action) return null;
    const key = `accountPool.upstreamAccounts.maintenanceEvents.actions.${action}`;
    const translated = t(key);
    if (translated !== key) return translated;
    const latestKey = `accountPool.upstreamAccounts.latestAction.actions.${action}`;
    const latestTranslated = t(latestKey);
    return latestTranslated === latestKey ? humanizeAction(action) : latestTranslated;
  };
  const resultLabel = (result?: string | null) => {
    if (!result) return null;
    const key = `accountPool.upstreamAccounts.maintenanceEvents.results.${result}`;
    const translated = t(key);
    return translated === key ? result : translated;
  };
  const reasonLabel = (reason?: string | null) => {
    if (!reason) return null;
    const key = `accountPool.upstreamAccounts.latestAction.reasons.${reason}`;
    const translated = t(key);
    return translated === key ? reason : translated;
  };
  const actionVariant = (action?: string | null): ChipTone => {
    if (!action) return "secondary";
    if (action.includes("succeeded") || action.includes("recovered")) return "success";
    if (action.includes("deferred") || action.includes("cooldown") || action.includes("blocked"))
      return "warning";
    if (action.includes("failed") || action.includes("failure") || action.includes("unavailable"))
      return "error";
    return action.includes("updated") ? "info" : "secondary";
  };
  const resultVariant = (result?: string | null): Exclude<ChipTone, "info"> =>
    (({ success: "success", failed: "error", deferred: "warning" })[result ?? ""] ??
      "secondary") as Exclude<ChipTone, "info">;
  const descriptionLabel = (event: UpstreamAccountActionEvent) =>
    describeEvent(t, event, reasonLabel);
  const egressIpLabel = (event: UpstreamAccountActionEvent) => {
    if (event.forwardProxyEgressIp) return event.forwardProxyEgressIp;
    const liveNode = event.forwardProxyKey ? proxyNodeByKey.get(event.forwardProxyKey) : null;
    if (liveNode?.egressIp) return liveNode.egressIp;
    if (liveNode?.selectable)
      return t("accountPool.upstreamAccounts.maintenanceEvents.egressIpPending");
    return t("accountPool.upstreamAccounts.maintenanceEvents.noEgressIp");
  };
  return {
    actionLabel,
    resultLabel,
    actionVariant,
    resultVariant,
    descriptionLabel,
    egressIpLabel,
  };
}

function describeEvent(
  t: Translator,
  event: UpstreamAccountActionEvent,
  reasonLabel: (reason?: string | null) => string | null,
) {
  if (event.reasonCode === "egress_throttled") {
    const retryAfter = event.reasonMessage?.match(/another\s+(\d+)\s+seconds/i)?.[1];
    const proxy =
      event.forwardProxyDisplayName ??
      event.forwardProxyKey ??
      t("accountPool.upstreamAccounts.maintenanceEvents.unknownProxy");
    return retryAfter
      ? t(
          "accountPool.upstreamAccounts.maintenanceEvents.descriptions.egressThrottledWithSeconds",
          { proxy, seconds: retryAfter },
        )
      : t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.egressThrottled", { proxy });
  }
  const known = { sync_ok: "syncOk", upstream_http_429: "upstream429", sync_error: "syncError" }[
    event.reasonCode ?? ""
  ];
  if (known) return t(`accountPool.upstreamAccounts.maintenanceEvents.descriptions.${known}`);
  if (event.action === "status_change_suppressed") {
    const parts = [
      reasonLabel(event.reasonCode) ?? event.reasonCode,
      Number.isFinite(event.httpStatus ?? NaN) ? `HTTP ${event.httpStatus}` : null,
      event.reasonMessage ?? event.resultDescription,
    ].filter((value): value is string => Boolean(value));
    return parts.join(" · ") || t("accountPool.upstreamAccounts.maintenanceEvents.noDescription");
  }
  return event.httpStatus != null
    ? t("accountPool.upstreamAccounts.maintenanceEvents.descriptions.httpStatus", {
        status: event.httpStatus,
      })
    : (event.resultDescription ??
        event.reasonMessage ??
        t("accountPool.upstreamAccounts.maintenanceEvents.noDescription"));
}

export default function MaintenanceRecordsPage() {
  const { t } = useTranslation();
  const { forwardProxyNodes = [] } = useUpstreamAccounts({ includeAll: true });
  const filters = useMaintenanceFilters(t, forwardProxyNodes);
  const events = useMaintenanceEvents(filters.query);
  const labels = useMaintenanceLabels(t, filters.proxyNodeByKey);
  const pageCount = Math.max(1, Math.ceil(events.total / Math.max(filters.pageSize, 1)));
  const model: MaintenanceRecordsViewProps = {
    t,
    ...filters,
    ...events,
    ...labels,
    pageCount,
    isInitialLoading: events.loading && events.events.length === 0,
    isInitialError: Boolean(events.error) && events.events.length === 0,
    formatOccurredAt,
    onNextPage: () => filters.onNextPage(),
  };
  return <MaintenanceRecordsView model={model} />;
}
