import type { ReactNode } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { useTranslation } from "../../i18n";
import type { ModelRoutingState } from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";
import { ModelIdentity } from "../shared/ModelIdentity";

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function DetailValue({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs font-semibold uppercase tracking-[0.06em] text-base-content/75">
        {label}
      </dt>
      <dd className="mt-0.5 text-sm leading-5 text-base-content/85">{children}</dd>
    </div>
  );
}

function failureKindLabel(kind: string | null | undefined, t: (key: string) => string) {
  if (!kind) return "-";
  const key = `accountPool.upstreamAccounts.modelRouting.failureKinds.${kind}`;
  const translated = t(key);
  if (translated !== key) return translated;
  const reasonKey = `accountPool.upstreamAccounts.latestAction.reasons.${kind}`;
  const reason = t(reasonKey);
  return reason === reasonKey ? t("accountPool.upstreamAccounts.latestAction.unknown") : reason;
}

export interface ModelRoutingHealthPanelProps {
  states: ModelRoutingState[];
  error?: string | null;
  resettingModel?: string | null;
  writesEnabled: boolean;
  onReset: (model: string) => void;
}

export function ModelRoutingHealthPanel({
  states,
  error,
  resettingModel = null,
  writesEnabled,
  onReset,
}: ModelRoutingHealthPanelProps) {
  const { t } = useTranslation();
  return (
    <Card data-testid="upstream-account-model-routing-panel">
      <CardHeader>
        <CardTitle>{t("accountPool.upstreamAccounts.modelRouting.title")}</CardTitle>
        <CardDescription className="text-base-content/80">
          {t("accountPool.upstreamAccounts.modelRouting.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">
        {error ? (
          <Alert variant="warning" data-testid="model-routing-error">
            <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            <span>{error}</span>
          </Alert>
        ) : null}
        {states.length === 0 ? (
          <p className="text-sm leading-6 text-base-content/80">
            {t("accountPool.upstreamAccounts.modelRouting.empty")}
          </p>
        ) : (
          states.map((route) => {
            const isAvailable = route.state === "available";
            const isCooling = route.state === "cooling_down";
            return (
              <div
                key={route.model}
                className="surface-subtle grid gap-2 rounded-lg px-3 py-2.5 lg:grid-cols-[minmax(0,1.4fr)_minmax(7rem,.8fr)_minmax(7rem,.8fr)_minmax(11rem,1fr)_minmax(13rem,1.1fr)] lg:items-center"
              >
                <div className="min-w-0">
                  <ModelIdentity
                    model={route.model}
                    className="max-w-full justify-start"
                    textClassName="truncate font-mono text-sm font-semibold leading-5 text-base-content"
                  />
                  <p className="mt-0.5 text-xs leading-4 text-base-content/72">
                    {t("accountPool.upstreamAccounts.modelRouting.lastSeen")}:{" "}
                    {formatDateTime(route.lastSeenAt)}
                  </p>
                </div>
                <dl className="contents">
                  <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.state")}>
                    <Badge variant={isAvailable ? "success" : isCooling ? "warning" : "secondary"}>
                      {t(`accountPool.upstreamAccounts.modelRouting.states.${route.state}`)}
                    </Badge>
                  </DetailValue>
                  <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.priority")}>
                    {t(`accountPool.upstreamAccounts.modelRouting.priorities.${route.priority}`)}
                  </DetailValue>
                  <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.changedAt")}>
                    {formatDateTime(route.changedAt)}
                  </DetailValue>
                </dl>
                <div className="flex min-w-0 flex-wrap items-center justify-start gap-1.5 lg:justify-end">
                  {route.cooldownUntil ? (
                    <span className="text-xs leading-4 tabular-nums tone-ink-warning">
                      {t("accountPool.upstreamAccounts.modelRouting.recoveryAt")}:{" "}
                      {formatDateTime(route.cooldownUntil)}
                    </span>
                  ) : null}
                  {!isAvailable ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="min-h-11 lg:min-h-0"
                      disabled={!writesEnabled || resettingModel === route.model}
                      onClick={() => onReset(route.model)}
                      data-testid={`model-routing-reset-${route.model}`}
                    >
                      {resettingModel === route.model
                        ? t("accountPool.upstreamAccounts.modelRouting.resetting")
                        : t("accountPool.upstreamAccounts.modelRouting.reset")}
                    </Button>
                  ) : null}
                </div>
                {!isAvailable && (route.failureCount > 0 || route.lastFailureMessage) ? (
                  <div className="mt-1 grid gap-x-3 gap-y-2 border-t border-base-300/60 pt-2.5 sm:grid-cols-2 lg:col-span-full lg:grid-cols-[minmax(0,1.4fr)_minmax(7rem,.8fr)_minmax(7rem,.8fr)_minmax(11rem,1fr)_minmax(13rem,1.1fr)]">
                    <dl className="contents">
                      <div className="min-w-0 sm:col-span-2 lg:col-span-2">
                        <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.failure")}>
                          <p className="break-words text-sm leading-5 text-base-content/85">
                            {failureKindLabel(route.lastFailureKind, t)}
                          </p>
                        </DetailValue>
                      </div>
                      <div className="min-w-0 lg:col-start-3">
                        <DetailValue
                          label={t("accountPool.upstreamAccounts.modelRouting.failures")}
                        >
                          <span className="font-mono tabular-nums">{route.failureCount}</span>
                        </DetailValue>
                      </div>
                      <div className="min-w-0 lg:col-start-4">
                        <DetailValue
                          label={t("accountPool.upstreamAccounts.modelRouting.failureKind")}
                        >
                          <span className="break-all font-mono text-xs">
                            {failureKindLabel(route.lastFailureKind, t)}
                          </span>
                        </DetailValue>
                      </div>
                      <div className="min-w-0 lg:col-start-5">
                        <DetailValue
                          label={t("accountPool.upstreamAccounts.modelRouting.lastFailureAt")}
                        >
                          <span className="tabular-nums">
                            {formatDateTime(route.lastFailureAt)}
                          </span>
                        </DetailValue>
                      </div>
                    </dl>
                  </div>
                ) : null}
              </div>
            );
          })
        )}
      </CardContent>
    </Card>
  );
}
