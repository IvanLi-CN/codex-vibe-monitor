import { useEffect, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import {
  fetchUpstreamAccountModelRoutingEvents,
  type ModelRoutingHistoryResponse,
  type ModelRoutingState,
  type ModelRoutingTimelineRecord,
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

function routeTone(state: string): "success" | "warning" | "secondary" {
  if (state === "available") return "success";
  if (state === "cooling_down") return "warning";
  return "secondary";
}

function routeStateLabel(state: string, t: (key: string) => string) {
  const key = `accountPool.upstreamAccounts.modelRouting.states.${state}`;
  const translated = t(key);
  return translated === key
    ? t("accountPool.upstreamAccounts.modelRouting.history.unknown")
    : translated;
}

function routeProtocolLabel(value: string | null | undefined, t: (key: string) => string) {
  if (!value) return t("accountPool.upstreamAccounts.modelRouting.history.unknown");
  const candidates = [
    `accountPool.upstreamAccounts.modelRouting.history.reasons.${value}`,
    `accountPool.upstreamAccounts.modelRouting.failureKinds.${value}`,
    `accountPool.upstreamAccounts.modelRouting.states.${value}`,
    `accountPool.upstreamAccounts.modelRouting.priorities.${value}`,
    `accountPool.upstreamAccounts.modelRouting.history.results.${value}`,
  ];
  for (const key of candidates) {
    const translated = t(key);
    if (translated !== key) return translated;
  }
  return t("accountPool.upstreamAccounts.modelRouting.history.unknown");
}

function HistoryRecord({ record }: { record: ModelRoutingTimelineRecord }) {
  const { t } = useTranslation();
  const summary = routeProtocolLabel(
    record.reasonCode || record.action || record.failureKind || record.status,
    t,
  );
  return (
    <div className="grid grid-cols-[8.25rem_minmax(0,1fr)_auto] gap-2 border-t border-base-300/60 px-3 py-2 text-xs">
      <span className="tabular-nums text-base-content/65">{formatBeijing(record.occurredAt)}</span>
      <span className="min-w-0 truncate text-base-content/80" title={summary}>
        {record.kind === "event"
          ? t("accountPool.upstreamAccounts.modelRouting.history.event")
          : (record.sameAccountRetryIndex ?? 0) > 0
            ? t("accountPool.upstreamAccounts.modelRouting.history.retry", {
                index: record.sameAccountRetryIndex ?? 0,
              })
            : t("accountPool.upstreamAccounts.modelRouting.history.attempt")}
        {" · "}
        {summary}
      </span>
      <span className="tabular-nums text-base-content/65">
        {record.httpStatus
          ? `HTTP ${record.httpStatus}`
          : record.totalLatencyMs
            ? `${Math.round(record.totalLatencyMs)} ms`
            : ""}
      </span>
    </div>
  );
}

function ModelRoutingHistory({ accountId, model }: { accountId?: number; model: string }) {
  const { t } = useTranslation();
  const [response, setResponse] = useState<ModelRoutingHistoryResponse | null>(null);
  const [loading, setLoading] = useState(Boolean(accountId));
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!accountId) {
      setLoading(false);
      setResponse(null);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    setResponse(null);
    void fetchUpstreamAccountModelRoutingEvents(accountId, {
      model,
      pageSize: 50,
      signal: controller.signal,
    })
      .then((next) => {
        if (!controller.signal.aborted) setResponse(next);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [accountId, model]);

  const loadMore = () => {
    if (!accountId || !response?.nextCursor || loadingMore) return;
    setLoadingMore(true);
    void fetchUpstreamAccountModelRoutingEvents(accountId, {
      model,
      cursor: response.nextCursor,
      pageSize: 50,
    })
      .then((next) =>
        setResponse((current) =>
          current
            ? { items: [...current.items, ...next.items], nextCursor: next.nextCursor }
            : next,
        ),
      )
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setLoadingMore(false));
  };

  if (!accountId) {
    return (
      <p className="px-3 py-2 text-xs text-base-content/65">
        {t("accountPool.upstreamAccounts.modelRouting.history.unavailable")}
      </p>
    );
  }
  if (loading)
    return (
      <p className="px-3 py-2 text-xs text-base-content/65">
        {t("accountPool.upstreamAccounts.modelRouting.history.loading")}
      </p>
    );
  if (error) return <p className="px-3 py-2 text-xs tone-ink-warning">{error}</p>;
  return (
    <div className="border-t border-base-300/60 bg-base-200/40">
      {response?.items.length ? (
        response.items.map((record) => <HistoryRecord key={record.id} record={record} />)
      ) : (
        <p className="border-t border-base-300/60 px-3 py-2 text-xs text-base-content/65">
          {t("accountPool.upstreamAccounts.modelRouting.history.empty")}
        </p>
      )}
      {response?.nextCursor ? (
        <div className="border-t border-base-300/60 px-3 py-2">
          <Button type="button" size="sm" variant="ghost" disabled={loadingMore} onClick={loadMore}>
            {loadingMore
              ? t("accountPool.upstreamAccounts.modelRouting.history.loadingMore")
              : t("accountPool.upstreamAccounts.modelRouting.history.loadMore")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

export interface ModelRoutingHealthPanelProps {
  accountId?: number;
  states: ModelRoutingState[];
  error?: string | null;
  resettingModel?: string | null;
  writesEnabled: boolean;
  initialExpandedModel?: string | null;
  onReset: (model: string) => void;
}

export function ModelRoutingHealthPanel({
  accountId,
  states,
  error,
  resettingModel = null,
  writesEnabled,
  initialExpandedModel = null,
  onReset,
}: ModelRoutingHealthPanelProps) {
  const { t } = useTranslation();
  const [expandedModel, setExpandedModel] = useState<string | null>(initialExpandedModel);

  useEffect(() => {
    if (initialExpandedModel && states.some((route) => route.model === initialExpandedModel)) {
      setExpandedModel(initialExpandedModel);
    }
  }, [initialExpandedModel, states]);

  return (
    <Card data-testid="upstream-account-model-routing-panel">
      <CardHeader className="gap-1 px-4 py-3">
        <CardTitle className="text-base">
          {t("accountPool.upstreamAccounts.modelRouting.title")}
        </CardTitle>
        <CardDescription className="text-xs leading-4 text-base-content/72">
          {t("accountPool.upstreamAccounts.modelRouting.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-2 px-4 pb-4">
        {error ? (
          <Alert
            variant="default"
            className="border-error/45 bg-error/10"
            data-testid="model-routing-error"
          >
            <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            <span>{error}</span>
          </Alert>
        ) : null}
        {states.length === 0 ? (
          <p className="text-sm text-base-content/70">
            {t("accountPool.upstreamAccounts.modelRouting.empty")}
          </p>
        ) : (
          <div className="overflow-hidden rounded-lg border border-base-300/70">
            {states.map((route) => {
              const expanded = expandedModel === route.model;
              const protection = route.probeRequired
                ? t("accountPool.upstreamAccounts.modelRouting.cacheProbe")
                : route.cacheConcurrencyLimit != null
                  ? t("accountPool.upstreamAccounts.modelRouting.cacheLimitCompact", {
                      limit: route.cacheConcurrencyLimit,
                      recoveryLimit: route.cacheRecoveryLimit ?? "-",
                      streak: route.cacheLowHitStreak ?? 0,
                      cooldown: route.cacheCooldownLevel ?? 0,
                      hitRate: route.cacheLastHitRatePercent ?? "-",
                    })
                  : t("accountPool.upstreamAccounts.modelRouting.cacheNormal");
              const failureSummary =
                route.lastFailureKind || route.failureCount > 0
                  ? route.lastFailureKind
                    ? routeProtocolLabel(route.lastFailureKind, t)
                    : t("accountPool.upstreamAccounts.modelRouting.history.failureCount", {
                        count: route.failureCount,
                      })
                  : null;
              return (
                <div key={route.model} className="bg-base-100">
                  <div className="grid grid-cols-[minmax(0,1fr)_auto_auto_auto] items-center gap-2 px-3 py-2">
                    <button
                      type="button"
                      className="min-w-0 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                      onClick={() => setExpandedModel(expanded ? null : route.model)}
                      aria-expanded={expanded}
                    >
                      <ModelIdentity
                        model={route.model}
                        className="max-w-full justify-start"
                        textClassName="truncate font-mono text-sm font-semibold"
                      />
                      <span className="mt-0.5 block truncate text-xs text-base-content/65">
                        {[
                          protection,
                          failureSummary,
                          formatBeijing(route.changedAt ?? route.lastSeenAt),
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </button>
                    <Chip tone={routeTone(route.state)}>{routeStateLabel(route.state, t)}</Chip>
                    {route.cooldownUntil ? (
                      <span className="hidden text-xs tabular-nums tone-ink-warning md:inline">
                        {formatBeijing(route.cooldownUntil)}
                      </span>
                    ) : null}
                    {route.state !== "available" ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={!writesEnabled || resettingModel === route.model}
                        onClick={() => onReset(route.model)}
                        data-testid={`model-routing-reset-${route.model}`}
                      >
                        {resettingModel === route.model
                          ? t("accountPool.upstreamAccounts.modelRouting.resetting")
                          : t("accountPool.upstreamAccounts.modelRouting.reset")}
                      </Button>
                    ) : null}
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-7 w-7"
                      aria-label={
                        expanded
                          ? t("accountPool.upstreamAccounts.modelRouting.collapse")
                          : t("accountPool.upstreamAccounts.modelRouting.expand")
                      }
                      onClick={() => setExpandedModel(expanded ? null : route.model)}
                    >
                      <AppIcon
                        name={expanded ? "chevron-up" : "chevron-down"}
                        className="h-4 w-4"
                        aria-hidden
                      />
                    </Button>
                  </div>
                  {expanded ? (
                    <ModelRoutingHistory accountId={accountId} model={route.model} />
                  ) : null}
                </div>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
