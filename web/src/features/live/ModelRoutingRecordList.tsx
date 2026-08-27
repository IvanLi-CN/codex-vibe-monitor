import { type ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type { ModelRoutingTimelineRecord } from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";
import { modelRoutingKey } from "./modelRoutingIds";

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

function recordKindLabel(
  record: ModelRoutingTimelineRecord,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (record.kind === "event") return t("live.routing.record.event");
  if ((record.sameAccountRetryIndex ?? 0) > 0) {
    return t("live.routing.record.retry", {
      index: record.sameAccountRetryIndex ?? 0,
    });
  }
  return t("live.routing.record.attempt");
}

function recordTone(record: ModelRoutingTimelineRecord): "success" | "warning" | "secondary" {
  if (record.status === "success" || (record.httpStatus != null && record.httpStatus < 400)) {
    return "success";
  }
  if (record.failureKind || (record.httpStatus != null && record.httpStatus >= 400)) {
    return "warning";
  }
  return "secondary";
}

function accountLabel(accountId: number, accountDisplayName?: string | null) {
  return accountDisplayName?.trim() || `API Key #${accountId}`;
}

function routeProtocolLabel(
  value: string | null | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  return routeProtocolLabelCandidates([value], t);
}

function routeProtocolLabelCandidates(
  values: Array<string | null | undefined>,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const candidates = [
    (value: string) => `accountPool.upstreamAccounts.modelRouting.history.reasons.${value}`,
    (value: string) => `accountPool.upstreamAccounts.modelRouting.failureKinds.${value}`,
    (value: string) => `accountPool.upstreamAccounts.modelRouting.states.${value}`,
    (value: string) => `accountPool.upstreamAccounts.modelRouting.priorities.${value}`,
    (value: string) => `accountPool.upstreamAccounts.modelRouting.history.results.${value}`,
    (value: string) => `accountPool.upstreamAccounts.latestAction.actions.${value}`,
  ];
  for (const value of values) {
    if (!value) continue;
    for (const keyForValue of candidates) {
      const key = keyForValue(value);
      const translated = t(key);
      if (translated !== key) return translated;
    }
  }
  return t("live.routing.record.unknown");
}

function routeSourceLabel(
  value: string | null | undefined,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!value) return t("live.routing.record.unknown");
  const key = `live.routing.record.sources.${value}`;
  const translated = t(key);
  return translated === key ? t("live.routing.record.unknown") : translated;
}

function handoffAdmissionLabel(
  admission: NonNullable<
    NonNullable<ModelRoutingTimelineRecord["routingSelectionAudit"]>["handoffAdmission"]
  >,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const decisionKey = `live.routing.record.handoffDecisions.${admission.decision}`;
  const phaseKey = `live.routing.record.handoffPhases.${admission.phase}`;
  const decision = t(decisionKey) === decisionKey ? admission.decision : t(decisionKey);
  const phase = t(phaseKey) === phaseKey ? admission.phase : t(phaseKey);
  return t("live.routing.record.handoffAdmissionValue", {
    decision,
    phase,
    count: admission.verificationSuccessCount,
  });
}

export function modelRoutingRecordsId(model: string) {
  return `model-routing-records-${modelRoutingKey(model)}`;
}

function DetailItem({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <span className="font-semibold text-base-content">{label}</span>{" "}
      <span className="break-words">{children}</span>
    </div>
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
  const reason = routeProtocolLabelCandidates(
    [record.reasonCode, record.action, record.failureKind],
    t,
  );
  const result = record.httpStatus
    ? `HTTP ${record.httpStatus}`
    : routeProtocolLabel(record.status, t);

  return (
    <div
      className="border-t border-base-300/60 px-2 py-1.5 sm:px-3"
      data-testid={`model-routing-record-${record.id}`}
    >
      <div className="flex min-w-0 items-center gap-1.5 sm:gap-2">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          aria-expanded={expanded}
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
          className="w-[5.75rem] shrink-0 truncate text-left text-xs font-semibold text-base-content hover:underline sm:w-28"
          onClick={() => onOpenAccount(record.accountId, record.model)}
        >
          {accountLabel(record.accountId, record.accountDisplayName)}
        </button>
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <Chip tone={recordTone(record)}>{recordKindLabel(record, t)}</Chip>
            <span className="truncate text-xs text-base-content/75" title={reason}>
              {reason}
            </span>
          </div>
          <div className="mt-0.5 truncate text-xs tabular-nums text-base-content/60 sm:hidden">
            {formatBeijing(record.occurredAt)} · {result}
            {record.totalLatencyMs != null ? ` · ${Math.round(record.totalLatencyMs)} ms` : ""}
          </div>
        </div>
        <span className="hidden shrink-0 text-xs tabular-nums text-base-content/65 sm:inline">
          {formatBeijing(record.occurredAt)}
        </span>
        <span className="hidden shrink-0 text-xs tabular-nums text-base-content/65 sm:inline">
          {result}
          {record.totalLatencyMs != null ? ` · ${Math.round(record.totalLatencyMs)} ms` : ""}
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
      {expanded ? (
        <div className="ml-8 mt-1.5 grid gap-x-4 gap-y-1.5 border-l border-base-300/70 bg-base-200/45 px-3 py-2 text-xs text-base-content/75 sm:grid-cols-2">
          <DetailItem label={t("live.routing.record.reason")}>{reason}</DetailItem>
          <DetailItem label={t("live.routing.record.result")}>
            {result}
            {record.totalLatencyMs != null ? ` · ${Math.round(record.totalLatencyMs)} ms` : ""}
          </DetailItem>
          {record.routingSource ? (
            <DetailItem label={t("live.routing.record.source")}>
              {routeSourceLabel(record.routingSource, t)}
            </DetailItem>
          ) : null}
          {record.modelRouteFailureCount != null ? (
            <DetailItem label={t("live.routing.record.failureCount")}>
              {record.modelRouteFailureCount}
            </DetailItem>
          ) : null}
          {audit ? (
            <div className="sm:col-span-2">
              <DetailItem label={t("live.routing.record.comparison")}>
                {routeProtocolLabel(audit.winnerReasonCode, t)} ·{" "}
                {t("live.routing.record.eligible", {
                  count: audit.eligibleCandidateCount,
                })}
                {audit.comparedAccountId != null
                  ? ` · ${
                      audit.comparedAccountName?.trim()
                        ? t("live.routing.record.comparedName", {
                            account: audit.comparedAccountName.trim(),
                          })
                        : t("live.routing.record.comparedId", {
                            accountId: audit.comparedAccountId,
                          })
                    }`
                  : ""}
                {audit.excludedCandidates.length > 0
                  ? ` · ${t("live.routing.record.excluded", {
                      count: audit.excludedCandidates.length,
                    })}: ${audit.excludedCandidates
                      .map(
                        (candidate) =>
                          `${accountLabel(candidate.accountId, candidate.accountName)} (${routeProtocolLabel(candidate.reasonCode, t)})`,
                      )
                      .join(", ")}`
                  : ""}
              </DetailItem>
            </div>
          ) : null}
          {audit?.handoffAdmission ? (
            <DetailItem label={t("live.routing.record.handoffAdmission")}>
              {handoffAdmissionLabel(audit.handoffAdmission, t)}
            </DetailItem>
          ) : null}
          {record.modelRouteStateBefore || record.modelRouteStateAfter ? (
            <DetailItem label={t("live.routing.record.transition")}>
              {routeProtocolLabel(record.modelRouteStateBefore, t)} →{" "}
              {routeProtocolLabel(record.modelRouteStateAfter, t)}
            </DetailItem>
          ) : null}
          {record.modelRoutePriorityBefore || record.modelRoutePriorityAfter ? (
            <DetailItem label={t("live.routing.record.priorityTransition")}>
              {routeProtocolLabel(record.modelRoutePriorityBefore, t)} →{" "}
              {routeProtocolLabel(record.modelRoutePriorityAfter, t)}
            </DetailItem>
          ) : null}
          {record.modelRouteCooldownUntil ? (
            <DetailItem label={t("live.routing.record.cooldownUntil")}>
              {formatBeijing(record.modelRouteCooldownUntil)}
            </DetailItem>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function ModelRoutingRecordList({
  model,
  records,
  onOpenAccount,
  onOpenInvocation,
  onContentHeightChange,
}: {
  model: string;
  records: ModelRoutingTimelineRecord[];
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
  onContentHeightChange?: (height: number) => void;
}) {
  const { t } = useTranslation();
  const [expandedRecords, setExpandedRecords] = useState<Set<string>>(new Set());
  const sectionRef = useRef<HTMLElement>(null);
  const modelRecords = useMemo(
    () =>
      records
        .filter((record) => record.model === model)
        .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt)),
    [model, records],
  );

  useEffect(() => {
    const visibleIds = new Set(modelRecords.map((record) => record.id));
    setExpandedRecords((current) => {
      const next = new Set(Array.from(current).filter((id) => visibleIds.has(id)));
      if (next.size === current.size && Array.from(next).every((id) => current.has(id))) {
        return current;
      }
      return next;
    });
  }, [modelRecords]);

  useLayoutEffect(() => {
    const section = sectionRef.current;
    if (!section || !onContentHeightChange) return;
    const reportHeight = () => {
      const height = Math.ceil(section.getBoundingClientRect().height);
      if (height > 0) onContentHeightChange(height);
    };
    reportHeight();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(reportHeight);
    observer.observe(section);
    return () => observer.disconnect();
  }, [onContentHeightChange]);

  return (
    <section
      ref={sectionRef}
      id={modelRoutingRecordsId(model)}
      className="border-y border-base-300/70 bg-base-100"
      data-testid={`model-routing-model-records-${model}`}
    >
      <div className="flex shrink-0 items-center justify-between gap-3 px-3 py-2">
        <h3 className="truncate text-sm font-semibold text-base-content">
          {t("live.routing.modelRecordsTitle")}
        </h3>
        <span className="shrink-0 text-xs tabular-nums text-base-content/65">
          {t("live.routing.modelRecordsCount", { count: modelRecords.length })}
        </span>
      </div>
      <div>
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
    </section>
  );
}
