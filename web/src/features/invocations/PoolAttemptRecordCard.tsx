import type { ReactNode } from "react";
import { Chip } from "../../components/ui/chip";
import type { TranslationKey } from "../../i18n";
import type {
  ApiPoolUpstreamRequestAttempt,
  PoolRoutingSelectionAudit,
  PoolRoutingSelectionScoreSnapshot,
} from "../../lib/api";
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

function resolveOriginalModel(attempt: ApiPoolUpstreamRequestAttempt) {
  return formatOptionalText(attempt.requestModel ?? attempt.model);
}

function resolveUpstreamRequestModel(attempt: ApiPoolUpstreamRequestAttempt, t: Translator) {
  const upstreamRequestModel = attempt.upstreamRequestModel?.trim();
  if (upstreamRequestModel) return upstreamRequestModel;
  if (attempt.modelMappingPattern?.trim()) return t("table.poolAttempts.modelNotSent");
  return resolveOriginalModel(attempt);
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
  variant: "primary" | "secondary" | "warning" | "info";
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
        variant: "primary",
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

function routingSelectionWinnerLabel(audit: PoolRoutingSelectionAudit, t: Translator) {
  if (!audit.selectedScore) {
    return t("table.poolAttempts.routingDecision.auditUnavailable", {
      account: audit.selectedAccountName,
      comparedAccount: audit.comparedAccountName ?? FALLBACK_CELL,
    });
  }
  const key =
    `table.poolAttempts.routingDecision.winnerReasons.${audit.winnerReasonCode}` as TranslationKey;
  const translated = t(key, {
    account: audit.selectedAccountName,
    comparedAccount: audit.comparedAccountName ?? FALLBACK_CELL,
  });
  return translated === key
    ? t("table.poolAttempts.routingDecision.winnerReasons.unknown", {
        account: audit.selectedAccountName,
        comparedAccount: audit.comparedAccountName ?? FALLBACK_CELL,
      })
    : translated;
}

function routingSelectionScoreLabel(
  account: string,
  score: PoolRoutingSelectionScoreSnapshot,
  t: Translator,
) {
  return t("table.poolAttempts.routingDecision.score", {
    account,
    modelPenalty: score.modelRoutePenalty,
    modelPenaltyCode: score.modelRoutePenaltyCode,
    routeFailurePenalty: score.routeBindingFailurePenalty,
    priority: score.routingPriorityRank,
    capacityLane: score.capacityLane,
    dispatchState: score.dispatchState,
    effectiveLoad: score.effectiveLoad,
    scarcityScore: score.scarcityScore,
  });
}

function routingSelectionExclusionLabel(account: string, reasonCode: string, t: Translator) {
  const key = `table.poolAttempts.routingDecision.exclusionReasons.${reasonCode}` as TranslationKey;
  const translated = t(key, { account });
  return translated === key
    ? t("table.poolAttempts.routingDecision.exclusionReasons.unknown", { account })
    : translated;
}

function routingSelectionHandoffLabel(audit: PoolRoutingSelectionAudit, t: Translator) {
  const admission = audit.handoffAdmission;
  if (!admission) return null;
  const decisionKey =
    `live.routing.record.handoffDecisions.${admission.decision}` as TranslationKey;
  const phaseKey = `live.routing.record.handoffPhases.${admission.phase}` as TranslationKey;
  const decision = t(decisionKey) === decisionKey ? admission.decision : t(decisionKey);
  const phase = t(phaseKey) === phaseKey ? admission.phase : t(phaseKey);
  const triggerKey =
    `live.routing.record.handoffTriggers.${admission.trigger ?? ""}` as TranslationKey;
  const trigger = admission.trigger
    ? t(triggerKey) === triggerKey
      ? admission.trigger
      : t(triggerKey)
    : null;
  return t(
    trigger
      ? "live.routing.record.handoffAdmissionValueWithTrigger"
      : "live.routing.record.handoffAdmissionValue",
    {
      decision,
      phase,
      count: admission.verificationSuccessCount,
      ...(trigger ? { trigger } : {}),
    },
  );
}

function PoolAttemptRoutingAudit({
  audit,
  t,
}: {
  audit: PoolRoutingSelectionAudit;
  t: Translator;
}) {
  return (
    <div
      className="mt-2 space-y-1 rounded border border-info/25 bg-info/5 p-2 text-xs text-base-content/72"
      data-testid="pool-attempt-routing-selection-audit"
    >
      <p className="font-medium text-base-content">
        {t("table.poolAttempts.routingDecision.summary", {
          account: audit.selectedAccountName,
          count: audit.eligibleCandidateCount,
        })}
      </p>
      <p>{routingSelectionWinnerLabel(audit, t)}</p>
      {routingSelectionHandoffLabel(audit, t) ? (
        <p>
          {t("live.routing.record.handoffAdmission")}: {routingSelectionHandoffLabel(audit, t)}
        </p>
      ) : null}
      {audit.selectedScore ? (
        <p data-testid="pool-attempt-routing-selection-score">
          {routingSelectionScoreLabel(audit.selectedAccountName, audit.selectedScore, t)}
        </p>
      ) : null}
      {audit.comparedScore && audit.comparedAccountName ? (
        <p>{routingSelectionScoreLabel(audit.comparedAccountName, audit.comparedScore, t)}</p>
      ) : null}
      {audit.excludedCandidates.map((candidate) => (
        <p key={`${candidate.accountId}-${candidate.reasonCode}`}>
          {routingSelectionExclusionLabel(candidate.accountName, candidate.reasonCode, t)}
        </p>
      ))}
    </div>
  );
}

function PoolAttemptMetric({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start gap-2">
      <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">{label}</span>
      <span className="min-w-0 break-all font-mono">{children}</span>
    </div>
  );
}

function PoolAttemptIdentityMetrics({
  attempt,
  proxyDisplay,
  originalModel,
  upstreamRequestModel,
  t,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  proxyDisplay: PoolAttemptProxyDisplay;
  originalModel: string;
  upstreamRequestModel: string;
  t: Translator;
}) {
  return (
    <>
      <PoolAttemptMetric label={t("table.poolAttempts.retry")}>
        {attempt.sameAccountRetryIndex}/{attempt.distinctAccountIndex}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.proxy")}>
        <span
          className={cn(
            "block truncate whitespace-nowrap",
            proxyDisplay.resolved ? "font-medium" : "",
          )}
          title={proxyDisplay.title}
          data-testid="pool-attempt-proxy-value"
        >
          {proxyDisplay.value}
        </span>
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.upstreamHttpStatus")}>
        {formatOptionalStatusCode(attempt.httpStatus)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.downstreamHttpStatus")}>
        {formatOptionalStatusCode(attempt.downstreamHttpStatus)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.failureKind")}>
        {formatOptionalText(attempt.failureKind)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.originalModel")}>
        {originalModel}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.upstreamRequestModel")}>
        {upstreamRequestModel}
      </PoolAttemptMetric>
      {attempt.modelMappingPattern?.trim() ? (
        <PoolAttemptMetric label={t("table.poolAttempts.modelMappingPattern")}>
          {attempt.modelMappingPattern.trim()}
        </PoolAttemptMetric>
      ) : null}
    </>
  );
}

function PoolAttemptTimingMetrics({
  attempt,
  t,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  t: Translator;
}) {
  return (
    <>
      <PoolAttemptMetric label={t("table.poolAttempts.connectLatency")}>
        {formatMilliseconds(attempt.connectLatencyMs)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.firstByteLatency")}>
        {formatMilliseconds(attempt.firstByteLatencyMs)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.streamLatency")}>
        {formatMilliseconds(attempt.streamLatencyMs)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.startedAt")}>
        {formatDetailTimestamp(attempt.startedAt)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.finishedAt")}>
        {formatDetailTimestamp(attempt.finishedAt)}
      </PoolAttemptMetric>
      <PoolAttemptMetric label={t("table.poolAttempts.upstreamRequestId")}>
        {formatOptionalText(attempt.upstreamRequestId)}
      </PoolAttemptMetric>
    </>
  );
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
  const originalModel = resolveOriginalModel(attempt);
  const upstreamRequestModel = resolveUpstreamRequestModel(attempt, t);

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
        <Chip tone={statusMeta.variant}>{t(statusMeta.key)}</Chip>
        {!isPoolAttemptTerminal(attempt) ? (
          <Chip tone={phaseMeta.variant} data-testid="pool-attempt-phase-badge">
            {t(phaseMeta.key)}
          </Chip>
        ) : null}
        <span className="font-mono text-xs text-base-content/70">#{attempt.attemptIndex}</span>
        <span className="font-mono text-xs text-info">{attempt.attemptId}</span>
        <span className="text-sm font-medium">{accountLabel}</span>
      </div>
      {summarySupplement ? <div className="mt-2">{summarySupplement}</div> : null}
      {attempt.routingSelectionAudit ? (
        <PoolAttemptRoutingAudit audit={attempt.routingSelectionAudit} t={t} />
      ) : null}
      <div className="mt-2 grid gap-2 text-sm md:grid-cols-2 xl:grid-cols-3">
        <PoolAttemptIdentityMetrics
          attempt={attempt}
          proxyDisplay={proxyDisplay}
          originalModel={originalModel}
          upstreamRequestModel={upstreamRequestModel}
          t={t}
        />
        <PoolAttemptTimingMetrics attempt={attempt} t={t} />
      </div>
      {children}
    </div>
  );
}
