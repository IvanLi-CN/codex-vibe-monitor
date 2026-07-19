import type { ReactNode } from "react";
import { Badge } from "../../components/ui/badge";
import type { TranslationKey } from "../../i18n";
import type { ApiPoolUpstreamRequestAttempt } from "../../lib/api";
import { cn } from "../../lib/utils";

const FALLBACK_CELL = "—";

type Translator = (key: TranslationKey, values?: Record<string, string | number>) => string;

export interface PoolAttemptProxyDisplay {
  value: string;
  title: string;
  resolved: boolean;
}

function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : FALLBACK_CELL;
}

function formatOptionalStatusCode(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return String(Math.trunc(value));
}

function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return `${value.toFixed(1)} ms`;
}

function formatDetailTimestamp(value: string | null | undefined) {
  const normalized = value?.trim();
  if (!normalized) return FALLBACK_CELL;

  const parsed = new Date(normalized);
  if (Number.isNaN(parsed.getTime())) return normalized;

  return parsed.toISOString().replace(".000Z", "Z").replace("T", " ");
}

function formatPoolAttemptAccountLabel(attempt: ApiPoolUpstreamRequestAttempt) {
  const accountName = attempt.upstreamAccountName?.trim();
  if (accountName) return accountName;
  if (typeof attempt.upstreamAccountId === "number" && Number.isFinite(attempt.upstreamAccountId)) {
    return `#${Math.trunc(attempt.upstreamAccountId)}`;
  }
  return FALLBACK_CELL;
}

function poolAttemptStatusMeta(status: string | null | undefined): {
  variant: "success" | "warning" | "error" | "secondary";
  key: TranslationKey;
} {
  switch (status?.trim().toLowerCase()) {
    case "pending":
      return { variant: "warning", key: "table.poolAttempts.status.pending" };
    case "success":
      return { variant: "success", key: "table.poolAttempts.status.success" };
    case "http_failure":
      return { variant: "error", key: "table.poolAttempts.status.httpFailure" };
    case "transport_failure":
      return {
        variant: "warning",
        key: "table.poolAttempts.status.transportFailure",
      };
    case "budget_exhausted_final":
      return {
        variant: "warning",
        key: "table.poolAttempts.status.budgetExhaustedFinal",
      };
    default:
      return { variant: "secondary", key: "table.poolAttempts.status.unknown" };
  }
}

function resolvePoolAttemptPhase(attempt: ApiPoolUpstreamRequestAttempt) {
  const explicitPhase = attempt.phase?.trim().toLowerCase();
  if (explicitPhase) return explicitPhase;

  const normalizedStatus = attempt.status?.trim().toLowerCase();
  if (normalizedStatus === "pending") return "sending_request";
  if (normalizedStatus === "success") return "completed";
  return "failed";
}

function poolAttemptPhaseMeta(phase: string | null | undefined): {
  variant: "default" | "secondary" | "warning" | "info";
  key: TranslationKey;
} {
  switch (phase?.trim().toLowerCase()) {
    case "connecting":
      return {
        variant: "secondary",
        key: "table.poolAttempts.phase.connecting",
      };
    case "sending_request":
      return {
        variant: "default",
        key: "table.poolAttempts.phase.sendingRequest",
      };
    case "waiting_first_byte":
      return {
        variant: "warning",
        key: "table.poolAttempts.phase.waitingFirstByte",
      };
    case "streaming_response":
      return {
        variant: "info",
        key: "table.poolAttempts.phase.streamingResponse",
      };
    case "completed":
      return {
        variant: "secondary",
        key: "table.poolAttempts.phase.completed",
      };
    case "failed":
      return { variant: "secondary", key: "table.poolAttempts.phase.failed" };
    default:
      return { variant: "secondary", key: "table.poolAttempts.phase.unknown" };
  }
}

function isPoolAttemptTerminal(attempt: ApiPoolUpstreamRequestAttempt) {
  if (attempt.finishedAt?.trim()) return true;
  return attempt.status.trim().toLowerCase() !== "pending";
}

export function PoolAttemptRecordCard({
  attempt,
  proxyDisplay,
  isFocused = false,
  t,
  className,
  summarySupplement,
  children,
  testId,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  proxyDisplay: PoolAttemptProxyDisplay;
  isFocused?: boolean;
  t: Translator;
  className?: string;
  summarySupplement?: ReactNode;
  children?: ReactNode;
  testId?: string;
}) {
  const statusMeta = poolAttemptStatusMeta(attempt.status);
  const phase = resolvePoolAttemptPhase(attempt);
  const phaseMeta = poolAttemptPhaseMeta(phase);
  const accountLabel = formatPoolAttemptAccountLabel(attempt);
  const httpStatusValue = formatOptionalStatusCode(attempt.httpStatus);
  const downstreamHttpStatusValue = formatOptionalStatusCode(attempt.downstreamHttpStatus);

  return (
    <div
      className={cn(
        "rounded-lg border bg-base-100/70 p-3",
        isFocused
          ? "border-primary/45 bg-primary/8 ring-1 ring-inset ring-primary/35"
          : "border-base-300/70",
        className,
      )}
      data-testid={testId}
      data-attempt-id={attempt.attemptId}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={statusMeta.variant}>{t(statusMeta.key)}</Badge>
        {!isPoolAttemptTerminal(attempt) ? (
          <Badge variant={phaseMeta.variant} data-testid="pool-attempt-phase-badge">
            {t(phaseMeta.key)}
          </Badge>
        ) : null}
        <span className="font-mono text-xs text-base-content/70">#{attempt.attemptIndex}</span>
        <span className="font-mono text-xs text-info">{attempt.attemptId}</span>
        <span className="text-sm font-medium">{accountLabel}</span>
      </div>
      {summarySupplement ? <div className="mt-2">{summarySupplement}</div> : null}
      <div className="mt-2 grid gap-2 text-sm md:grid-cols-2 xl:grid-cols-3">
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.retry")}
          </span>
          <span className="font-mono">
            {attempt.sameAccountRetryIndex}/{attempt.distinctAccountIndex}
          </span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.proxy")}
          </span>
          <span
            className={cn(
              "min-w-0 truncate whitespace-nowrap",
              proxyDisplay.resolved ? "font-medium" : "font-mono",
            )}
            title={proxyDisplay.title}
            data-testid="pool-attempt-proxy-value"
          >
            {proxyDisplay.value}
          </span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.upstreamHttpStatus")}
          </span>
          <span className="font-mono">{httpStatusValue}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.downstreamHttpStatus")}
          </span>
          <span className="font-mono">{downstreamHttpStatusValue}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.failureKind")}
          </span>
          <span className="break-all font-mono">{formatOptionalText(attempt.failureKind)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.connectLatency")}
          </span>
          <span className="font-mono">{formatMilliseconds(attempt.connectLatencyMs)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.firstByteLatency")}
          </span>
          <span className="font-mono">{formatMilliseconds(attempt.firstByteLatencyMs)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.streamLatency")}
          </span>
          <span className="font-mono">{formatMilliseconds(attempt.streamLatencyMs)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.startedAt")}
          </span>
          <span className="font-mono">{formatDetailTimestamp(attempt.startedAt)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.finishedAt")}
          </span>
          <span className="font-mono">{formatDetailTimestamp(attempt.finishedAt)}</span>
        </div>
        <div className="flex items-start gap-2">
          <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
            {t("table.poolAttempts.upstreamRequestId")}
          </span>
          <span className="break-all font-mono">
            {formatOptionalText(attempt.upstreamRequestId)}
          </span>
        </div>
      </div>
      {children}
    </div>
  );
}
