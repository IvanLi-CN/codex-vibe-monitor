import { Alert } from "../../components/ui/alert";
import type { ApiInvocation, ApiInvocationWorkflowDetailResponse } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import {
  FALLBACK_CELL,
  formatNoCandidateReason,
  formatOptionalText,
  formatTimestamp,
} from "./InvocationWorkflowDetailPanel.formatters";
import { IdentityField, SnapshotMetric } from "./InvocationWorkflowDetailPanel.primitives";

type NoCandidateAudit = NonNullable<
  ApiInvocationWorkflowDetailResponse["hero"]["poolRoutingNoCandidateAudit"]
>;

type SnapshotMetricItem = {
  label: string;
  value: string;
  variant: "primary" | "secondary" | "success" | "warning" | "error";
};

type ModelTrailItem = {
  key: string;
  value: string;
};

export type InvocationHeroIdentityCardProps = {
  record: ApiInvocation;
  detail: ApiInvocationWorkflowDetailResponse;
  localeTag: string;
  isZh: boolean;
  isCompact: boolean;
  conversationShortId: string;
  noCandidateAudit: NoCandidateAudit | null | undefined;
  noCandidateReasonCounts: Array<[string, number]>;
  snapshotMetrics: SnapshotMetricItem[];
  modelTrailItems: ModelTrailItem[];
  heroStatusNotes: string[];
};

function NoCandidateAuditSummary({
  audit,
  localeTag,
  isZh,
}: {
  audit: NoCandidateAudit;
  localeTag: string;
  isZh: boolean;
}) {
  return (
    <>
      <div className="font-semibold">
        {isZh ? "未分配上游账号诊断" : "No upstream account diagnostic"}
      </div>
      <div className="mt-1 flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
        <span>
          {isZh ? "原因" : "Reason"}: {formatNoCandidateReason(audit.terminalReasonCode, isZh)}
        </span>
        <code className="max-w-full break-all rounded-md border border-warning/25 bg-warning/10 px-1.5 py-0.5 font-mono text-xs leading-4 text-base-content/72">
          {audit.terminalReasonCode}
        </code>
      </div>
      <dl
        className={cn(
          "mt-3 grid grid-cols-2 gap-x-4 gap-y-2",
          audit.nextEligibleAt ? "sm:grid-cols-4" : "sm:grid-cols-3",
        )}
      >
        {[
          {
            label: isZh ? "候选账号" : "Candidates",
            value: audit.candidateCount.toLocaleString(localeTag),
          },
          {
            label: isZh ? "可用账号" : "Eligible",
            value: audit.eligibleCandidateCount.toLocaleString(localeTag),
          },
          {
            label: isZh ? "容量冲突" : "Capacity conflicts",
            value: audit.reservationConflictCount.toLocaleString(localeTag),
          },
          ...(audit.nextEligibleAt
            ? [
                {
                  label: isZh ? "下一可用时间" : "Next eligible at",
                  value: formatTimestamp(audit.nextEligibleAt, localeTag),
                  testId: "pool-routing-no-candidate-next-eligible-at",
                },
              ]
            : []),
        ].map((metric) => (
          <div key={metric.label} className="min-w-0">
            <dt className="text-xs font-medium text-base-content/58">{metric.label}</dt>
            <dd
              className="mt-0.5 font-mono text-sm font-semibold tabular-nums text-base-content"
              data-testid={metric.testId}
            >
              {metric.value}
            </dd>
          </div>
        ))}
      </dl>
    </>
  );
}

function NoCandidateAuditDetails({
  audit,
  reasonCounts,
  isZh,
}: {
  audit: NoCandidateAudit;
  reasonCounts: Array<[string, number]>;
  isZh: boolean;
}) {
  return (
    <>
      {reasonCounts.length > 0 ? (
        <div
          className="mt-3 border-t border-warning/25 pt-3"
          data-testid="pool-routing-no-candidate-reason-counts"
        >
          <div className="text-xs font-medium text-base-content/58">
            {isZh ? "排除原因汇总" : "Exclusion summary"}
          </div>
          <ul className="mt-2 grid min-w-0 gap-x-4 gap-y-2 sm:grid-cols-2">
            {reasonCounts.map(([reasonCode, count]) => (
              <li
                key={reasonCode}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-x-2 gap-y-0.5"
                data-testid={`pool-routing-no-candidate-reason-count-${reasonCode}`}
              >
                <span className="break-words text-xs text-base-content/76">
                  {formatNoCandidateReason(reasonCode, isZh)}
                </span>
                <span className="font-mono text-xs font-semibold tabular-nums text-base-content/84">
                  {count.toLocaleString()}
                </span>
                <code className="col-span-2 max-w-full break-all font-mono text-xs leading-4 text-base-content/58">
                  {reasonCode}
                </code>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {audit.candidates.length > 0 ? (
        <div className="mt-3 border-t border-warning/25 pt-3">
          <div className="text-xs font-medium text-base-content/58">
            {isZh ? "候选明细" : "Candidate details"}
          </div>
          <ul className="mt-2 grid min-w-0 gap-2 sm:grid-cols-2">
            {audit.candidates.map((candidate) => (
              <li key={`${candidate.accountId}-${candidate.reasonCode}`} className="min-w-0">
                <div className="break-words text-xs font-semibold text-base-content/84">
                  {candidate.accountName}
                </div>
                <div className="mt-0.5 break-words text-xs text-base-content/70">
                  {formatNoCandidateReason(candidate.reasonCode, isZh)}
                </div>
                <code className="mt-0.5 block max-w-full break-all font-mono text-xs leading-4 text-base-content/58">
                  {candidate.reasonCode}
                </code>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </>
  );
}

function NoCandidateAudit({
  audit,
  reasonCounts,
  localeTag,
  isZh,
  isCompact,
}: {
  audit: NoCandidateAudit;
  reasonCounts: Array<[string, number]>;
  localeTag: string;
  isZh: boolean;
  isCompact: boolean;
}) {
  return (
    <Alert
      variant="warning"
      className={cn("mt-4", isCompact && "mt-3")}
      data-testid="pool-routing-no-candidate-audit"
    >
      <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1 text-sm leading-5">
        <NoCandidateAuditSummary audit={audit} localeTag={localeTag} isZh={isZh} />
        <NoCandidateAuditDetails audit={audit} reasonCounts={reasonCounts} isZh={isZh} />
      </div>
    </Alert>
  );
}

function InvocationHeroIdentityHeader({
  hero,
  record,
  conversationShortId,
  isZh,
  isCompact,
}: {
  hero: ApiInvocationWorkflowDetailResponse["hero"];
  record: ApiInvocation;
  conversationShortId: string;
  isZh: boolean;
  isCompact: boolean;
}) {
  return (
    <div
      className={cn(
        "grid gap-4 lg:grid-cols-[minmax(0,1.1fr)_minmax(14rem,0.9fr)]",
        isCompact && "gap-3",
      )}
    >
      <div className="min-w-0">
        <div className="text-xs font-medium text-base-content/56">
          {isZh ? "调用 ID" : "Call ID"}
        </div>
        <div className="mt-1 break-all font-mono text-[1.08rem] font-semibold tracking-[-0.02em] text-base-content sm:text-[1.22rem]">
          {hero.invokeId ?? record.invokeId}
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
        <IdentityField label={isZh ? "对话 ID" : "Conversation ID"} value={conversationShortId} />
        <IdentityField
          label={isZh ? "原始 Prompt Cache Key" : "Raw Prompt Cache Key"}
          value={hero.promptCacheKey ?? FALLBACK_CELL}
          monospace
        />
      </div>
    </div>
  );
}

function InvocationHeroIdentityCard({
  record,
  detail,
  localeTag,
  isZh,
  isCompact,
  conversationShortId,
  noCandidateAudit,
  noCandidateReasonCounts,
  snapshotMetrics,
  modelTrailItems,
  heroStatusNotes,
}: InvocationHeroIdentityCardProps) {
  const hero = detail.hero;
  return (
    <div
      className={cn(
        "invocation-detail-card-surface rounded-[1rem] p-4",
        isCompact && "rounded-none p-0",
      )}
    >
      <InvocationHeroIdentityHeader
        hero={hero}
        record={record}
        conversationShortId={conversationShortId}
        isZh={isZh}
        isCompact={isCompact}
      />
      <div
        className={cn(
          "mt-4 grid grid-cols-2 gap-2.5 sm:grid-cols-4 sm:gap-3",
          isCompact &&
            (isZh
              ? "mt-3 grid-cols-4 gap-2 sm:gap-2.5"
              : "mt-3 gap-2 min-[496px]:grid-cols-4 sm:gap-2.5"),
        )}
      >
        {snapshotMetrics.map((metric) => (
          <SnapshotMetric
            key={metric.label}
            label={metric.label}
            value={metric.value}
            variant={metric.variant}
            compact={isCompact}
          />
        ))}
      </div>
      {noCandidateAudit ? (
        <NoCandidateAudit
          audit={noCandidateAudit}
          reasonCounts={noCandidateReasonCounts}
          localeTag={localeTag}
          isZh={isZh}
          isCompact={isCompact}
        />
      ) : null}
      <div
        className={cn(
          "mt-4 grid gap-4 border-t border-base-300/65 pt-4 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,0.95fr)]",
          isCompact && "mt-3 gap-3 pt-3",
        )}
      >
        <div className="min-w-0">
          <div className="text-[11px] font-medium text-base-content/56">
            {isZh ? "模型与端点" : "Models and Endpoint"}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-base-content/84">
            {modelTrailItems.length > 0 ? (
              <>
                {modelTrailItems.map((modelItem) => (
                  <span key={modelItem.key} className="min-w-0 break-all">
                    {modelItem.value}
                  </span>
                ))}
                <span className="text-base-content/46">·</span>
              </>
            ) : null}
            <span className="break-all">{formatOptionalText(hero.endpoint)}</span>
          </div>
        </div>
        {heroStatusNotes.length > 0 ? (
          <div className="min-w-0">
            <div className="text-xs font-medium text-base-content/56">
              {isZh ? "状态" : "Status"}
            </div>
            <div className="mt-1 space-y-1 text-sm text-base-content/72">
              {heroStatusNotes.map((note) => (
                <div key={note}>{note}</div>
              ))}
            </div>
          </div>
        ) : (
          <div className="min-w-0" />
        )}
      </div>
    </div>
  );
}

export { InvocationHeroIdentityCard };
