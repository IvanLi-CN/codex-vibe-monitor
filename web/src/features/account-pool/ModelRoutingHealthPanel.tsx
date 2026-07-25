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
      <p className="text-xs font-semibold uppercase tracking-[0.08em] text-base-content/55">
        {label}
      </p>
      <div className="mt-1 text-sm text-base-content/85">{children}</div>
    </div>
  );
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
        <CardDescription>
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
          <p className="text-sm leading-6 text-base-content/68">
            {t("accountPool.upstreamAccounts.modelRouting.empty")}
          </p>
        ) : (
          states.map((route) => {
            const isAvailable = route.state === "available";
            const isCooling = route.state === "cooling_down";
            return (
              <div
                key={route.model}
                className="surface-subtle grid gap-3 rounded-[1rem] p-4 md:grid-cols-[minmax(0,1.4fr)_repeat(3,minmax(0,1fr))_auto] md:items-center"
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-sm font-semibold text-base-content">
                    {route.model}
                  </p>
                  <p className="mt-1 text-xs text-base-content/60">
                    {t("accountPool.upstreamAccounts.modelRouting.lastSeen")}:{" "}
                    {formatDateTime(route.lastSeenAt)}
                  </p>
                </div>
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
                <div className="flex flex-wrap items-center justify-start gap-2 md:justify-end">
                  {route.cooldownUntil ? (
                    <span className="text-xs text-warning">
                      {t("accountPool.upstreamAccounts.modelRouting.recoveryAt")}:{" "}
                      {formatDateTime(route.cooldownUntil)}
                    </span>
                  ) : null}
                  {!isAvailable ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
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
                {!isAvailable && route.failureCount > 0 ? (
                  <div className="grid gap-2 sm:grid-cols-3 md:col-span-full">
                    <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.failures")}>
                      {route.failureCount}
                    </DetailValue>
                    <DetailValue label={t("accountPool.upstreamAccounts.modelRouting.failureKind")}>
                      {route.lastFailureKind ?? "-"}
                    </DetailValue>
                    <DetailValue
                      label={t("accountPool.upstreamAccounts.modelRouting.lastFailureAt")}
                    >
                      {formatDateTime(route.lastFailureAt)}
                    </DetailValue>
                  </div>
                ) : null}
                {route.lastFailureMessage ? (
                  <p className="break-words text-xs leading-5 text-base-content/65 md:col-span-full">
                    {t("accountPool.upstreamAccounts.modelRouting.failure")}:{" "}
                    {route.lastFailureMessage}
                  </p>
                ) : null}
              </div>
            );
          })
        )}
      </CardContent>
    </Card>
  );
}
