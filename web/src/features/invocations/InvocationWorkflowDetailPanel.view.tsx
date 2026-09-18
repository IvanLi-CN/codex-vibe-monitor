import { Chip } from "../../components/ui/chip";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import { cn } from "../../lib/utils";
import { InvocationWorkflowAttemptRecord } from "./InvocationWorkflowDetailPanel.attempt-record";
import {
  type AttemptSection,
  formatRouteMode,
  formatTimestamp,
  type GenericSection,
  type PayloadFetchState,
  resolveKindMeta,
  type resolveStatusMeta,
} from "./InvocationWorkflowDetailPanel.formatters";
import { GenericDetail } from "./InvocationWorkflowDetailPanel.generic-detail";
import { InvocationHeroIdentityCard } from "./InvocationWorkflowDetailPanel.hero-identity";
import { SummaryRows } from "./InvocationWorkflowDetailPanel.primitives";
import { TimelineSummary } from "./InvocationWorkflowDetailPanel.timeline-summary";

type SummaryRow = {
  label: string;
  value: string;
  variant?: "primary" | "secondary" | "success" | "warning" | "error";
  action?: {
    title: string;
    onClick: () => void;
  };
};

type SnapshotMetricItem = {
  label: string;
  value: string;
  variant: "primary" | "secondary" | "success" | "warning" | "error";
};

type ModelTrailItem = {
  key: string;
  value: string;
};

interface InvocationWorkflowDetailPanelViewProps {
  record: ApiInvocation;
  detail: ApiInvocationWorkflowDetailResponse;
  localeTag: string;
  isZh: boolean;
  size: "compact" | "default";
  hideNonShortIds: boolean;
  conversationShortId: string;
  finalStatusMeta: ReturnType<typeof resolveStatusMeta>;
  summaryRows: SummaryRow[];
  heroStatusNotes: string[];
  noCandidateAudit:
    | NonNullable<ApiInvocationWorkflowDetailResponse["hero"]["poolRoutingNoCandidateAudit"]>
    | null
    | undefined;
  noCandidateReasonCounts: Array<[string, number]>;
  snapshotMetrics: SnapshotMetricItem[];
  modelTrailItems: ModelTrailItem[];
  openBlockId: string | null;
  attemptSection: AttemptSection | null;
  genericSection: GenericSection | null;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  toggleAttemptSection: (
    entry: ApiInvocationWorkflowTimelineEntry,
    section: AttemptSection,
  ) => void;
  toggleGenericSection: (
    entry: ApiInvocationWorkflowTimelineEntry,
    section: GenericSection,
  ) => void;
}

type InvocationHeroProps = Pick<
  InvocationWorkflowDetailPanelViewProps,
  | "record"
  | "detail"
  | "localeTag"
  | "isZh"
  | "size"
  | "conversationShortId"
  | "finalStatusMeta"
  | "summaryRows"
  | "heroStatusNotes"
  | "noCandidateAudit"
  | "noCandidateReasonCounts"
  | "snapshotMetrics"
  | "modelTrailItems"
>;

function InvocationHeroMetricsCard({
  detail,
  isZh,
  isCompact,
  summaryRows,
}: {
  detail: ApiInvocationWorkflowDetailResponse;
  isZh: boolean;
  isCompact: boolean;
  summaryRows: SummaryRow[];
}) {
  return (
    <div
      className={cn(
        "invocation-detail-card-surface rounded-[1rem] p-4",
        isCompact && "rounded-none p-0",
      )}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-semibold text-base-content">
          {isZh ? "关键指标" : "Key metrics"}
        </div>
        {detail.hero.failureClass ? (
          <Chip size="compact" tone="error" className="px-2.5 py-1 font-mono text-xs">
            {detail.hero.failureClass}
          </Chip>
        ) : null}
      </div>
      <div className={cn("mt-3", isCompact && "mt-2.5")}>
        <SummaryRows rows={summaryRows} compact={isCompact} />
      </div>
    </div>
  );
}

function InvocationHero({
  detail,
  record,
  localeTag,
  isZh,
  size,
  conversationShortId,
  finalStatusMeta,
  summaryRows,
  heroStatusNotes,
  noCandidateAudit,
  noCandidateReasonCounts,
  snapshotMetrics,
  modelTrailItems,
}: InvocationHeroProps) {
  const hero = detail.hero;
  const isCompact = size === "compact";
  return (
    <section
      className={cn(
        "invocation-detail-hero-surface min-w-0 max-w-full overflow-hidden rounded-[1.2rem] px-4 py-4 sm:px-5 sm:py-5",
        isCompact && "rounded-none px-0 py-0",
      )}
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="tone-ink-primary text-[11px] font-semibold uppercase tracking-[0.22em]">
            {isZh ? "调用详情" : "Invocation Detail"}
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <Chip tone={finalStatusMeta.variant}>{finalStatusMeta.label}</Chip>
            {hero.routeMode ? (
              <Chip tone="secondary">{formatRouteMode(hero.routeMode, isZh)}</Chip>
            ) : null}
            {detail.reconstructed ? (
              <Chip tone="warning">{isZh ? "重建" : "Reconstructed"}</Chip>
            ) : null}
            {detail.partial ? <Chip tone="warning">{isZh ? "部分" : "Partial"}</Chip> : null}
          </div>
        </div>
        <div className="text-right">
          <div className="text-[11px] font-medium text-base-content/54">
            {isZh ? "调用时间" : "Occurred At"}
          </div>
          <div className="mt-1 text-sm font-medium text-base-content/78">
            {formatTimestamp(hero.occurredAt, localeTag)}
          </div>
        </div>
      </div>
      <div
        className={cn(
          "mt-4 grid gap-4",
          isCompact
            ? "gap-3 xl:grid-cols-[minmax(0,1.4fr)_minmax(19rem,0.9fr)]"
            : "xl:grid-cols-[minmax(0,1.45fr)_minmax(22rem,0.95fr)]",
        )}
      >
        <InvocationHeroIdentityCard
          record={record}
          detail={detail}
          localeTag={localeTag}
          isZh={isZh}
          isCompact={isCompact}
          conversationShortId={conversationShortId}
          noCandidateAudit={noCandidateAudit}
          noCandidateReasonCounts={noCandidateReasonCounts}
          snapshotMetrics={snapshotMetrics}
          modelTrailItems={modelTrailItems}
          heroStatusNotes={heroStatusNotes}
        />
        <InvocationHeroMetricsCard
          detail={detail}
          isZh={isZh}
          isCompact={isCompact}
          summaryRows={summaryRows}
        />
      </div>
    </section>
  );
}

type InvocationTimelineProps = Pick<
  InvocationWorkflowDetailPanelViewProps,
  | "record"
  | "detail"
  | "localeTag"
  | "isZh"
  | "size"
  | "hideNonShortIds"
  | "openBlockId"
  | "attemptSection"
  | "genericSection"
  | "requestBodyState"
  | "toggleAttemptSection"
  | "toggleGenericSection"
>;

function InvocationTimelineEntry({
  record,
  entry,
  localeTag,
  isZh,
  size,
  hideNonShortIds,
  openBlockId,
  attemptSection,
  genericSection,
  requestBodyState,
  toggleAttemptSection,
  toggleGenericSection,
}: Omit<InvocationTimelineProps, "detail"> & {
  entry: ApiInvocationWorkflowTimelineEntry;
}) {
  const isOpen = openBlockId === entry.blockId;
  const kindMeta = resolveKindMeta(entry.kind, isZh);
  return (
    <div className="relative min-w-0">
      <span
        className={cn(
          "invocation-detail-marker absolute left-0 top-5 h-[1.05rem] w-[1.05rem] rounded-full border-2",
          kindMeta.markerClass,
        )}
      />
      <div className={cn("min-w-0", size === "compact" ? "ml-4" : "ml-5")}>
        {entry.attempt ? (
          <InvocationWorkflowAttemptRecord
            record={record}
            entry={entry}
            localeTag={localeTag}
            isZh={isZh}
            isOpen={isOpen}
            activeSection={isOpen ? attemptSection : null}
            onSelectSection={(section) => toggleAttemptSection(entry, section)}
            hideNonShortIds={hideNonShortIds}
          />
        ) : (
          <>
            <TimelineSummary
              entry={entry}
              localeTag={localeTag}
              isZh={isZh}
              isOpen={isOpen}
              activeSection={isOpen ? genericSection : null}
              onSelectSection={(section) => toggleGenericSection(entry, section as GenericSection)}
            />
            {isOpen && genericSection ? (
              <GenericDetail
                entry={entry}
                localeTag={localeTag}
                isZh={isZh}
                activeSection={genericSection}
                requestBodyState={requestBodyState}
              />
            ) : null}
          </>
        )}
      </div>
    </div>
  );
}

function InvocationTimeline({
  record,
  detail,
  localeTag,
  isZh,
  size,
  hideNonShortIds,
  openBlockId,
  attemptSection,
  genericSection,
  requestBodyState,
  toggleAttemptSection,
  toggleGenericSection,
}: InvocationTimelineProps) {
  const timeline = detail.timeline;
  return (
    <section className="invocation-detail-timeline-surface min-w-0 max-w-full overflow-hidden rounded-[1.15rem] px-4 py-4 sm:px-5 sm:py-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-base-content">
            {isZh ? "工作流时间线" : "Workflow Timeline"}
          </h3>
        </div>
        <div className="text-xs text-base-content/58">
          {isZh
            ? `${timeline.length.toLocaleString(localeTag)} 个时间线块`
            : `${timeline.length.toLocaleString(localeTag)} timeline blocks`}
        </div>
      </div>
      <div className={cn("relative min-w-0", size === "compact" ? "pl-4" : "pl-5")}>
        <div
          className={cn(
            "absolute top-3 bottom-3 w-px bg-base-300/72",
            size === "compact" ? "left-[0.45rem]" : "left-[0.55rem]",
          )}
        />
        <div className="space-y-3">
          {timeline.map((entry) => (
            <InvocationTimelineEntry
              key={entry.blockId}
              record={record}
              entry={entry}
              localeTag={localeTag}
              isZh={isZh}
              size={size}
              hideNonShortIds={hideNonShortIds}
              openBlockId={openBlockId}
              attemptSection={attemptSection}
              genericSection={genericSection}
              requestBodyState={requestBodyState}
              toggleAttemptSection={toggleAttemptSection}
              toggleGenericSection={toggleGenericSection}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

export function InvocationWorkflowDetailPanelView(props: InvocationWorkflowDetailPanelViewProps) {
  const { size } = props;
  return (
    <div
      className={cn(
        "min-w-0 max-w-full overflow-hidden space-y-4",
        size === "compact" ? "invocation-detail-mobile-flat text-sm" : "",
      )}
    >
      <InvocationHero {...props} />
      <InvocationTimeline {...props} />
    </div>
  );
}
