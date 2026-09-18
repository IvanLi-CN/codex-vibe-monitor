import { Chip } from "../../components/ui/chip";
import { Tooltip } from "../../components/ui/tooltip";
import { useTranslation } from "../../i18n";
import type { ApiInvocationWorkflowTimelineEntry } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import type {
  AttemptSection,
  GenericSection,
  TimelineFact,
  TimelineMetricAction,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  formatTimestamp,
  resolveKindMeta,
  resolveStatusMeta,
} from "./InvocationWorkflowDetailPanel.formatters";
import { TimelineMetricButton } from "./InvocationWorkflowDetailPanel.primitives";
import {
  buildAttemptMetricActions,
  buildGenericMetricActions,
  buildTimelineFacts,
} from "./InvocationWorkflowDetailPanel.timeline-model";

export function TimelineSummaryMetrics({
  entry,
  metricActions,
  isOpen,
  activeSection,
  onSelectSection,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  metricActions: Array<TimelineMetricAction<AttemptSection | GenericSection>>;
  isOpen: boolean;
  activeSection: AttemptSection | GenericSection | null;
  onSelectSection: (section: AttemptSection | GenericSection) => void;
}) {
  if (metricActions.length === 0) return null;
  return (
    <div className="invocation-detail-rail mt-3 overflow-hidden rounded-[0.95rem]">
      <div className="grid gap-px sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
        {metricActions.map((action) => (
          <TimelineMetricButton
            key={`${entry.blockId}-${action.section}`}
            label={action.label}
            tag={action.tag}
            primary={action.primary}
            secondary={action.secondary}
            secondaryTone={action.secondaryTone}
            tertiary={action.tertiary}
            tertiaryChips={action.tertiaryChips}
            tertiaryOverflowCount={action.tertiaryOverflowCount}
            monospace={action.monospace}
            active={isOpen && activeSection === action.section}
            onClick={() => onSelectSection(action.section)}
          />
        ))}
      </div>
    </div>
  );
}

export function TimelineSummaryFacts({
  entry,
  summaryFacts,
  showTitle,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  summaryFacts: TimelineFact[];
  showTitle: boolean;
}) {
  if (summaryFacts.length === 0) return null;
  return (
    <div
      className={cn(
        "flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-base-content/64",
        showTitle ? "mt-1" : "mt-0.5",
      )}
    >
      {summaryFacts.map((fact) =>
        fact.tooltip ? (
          <Tooltip
            key={`${entry.blockId}-${fact.key}`}
            content={fact.tooltip}
            side="top"
            sideOffset={8}
          >
            <Chip size="micro" tone={fact.tone ?? "secondary"} className="min-w-0 break-all px-2">
              {fact.label}
            </Chip>
          </Tooltip>
        ) : (
          <Chip
            size="micro"
            tone={fact.tone ?? "secondary"}
            key={`${entry.blockId}-${fact.key}`}
            className="min-w-0 break-all px-2"
          >
            {fact.label}
          </Chip>
        ),
      )}
    </div>
  );
}

export function TimelineSummary({
  entry,
  localeTag,
  isZh,
  isOpen,
  activeSection,
  onSelectSection,
  attemptIdentityOverride,
  testId,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  isOpen: boolean;
  activeSection: AttemptSection | GenericSection | null;
  onSelectSection: (section: AttemptSection | GenericSection) => void;
  attemptIdentityOverride?: string | null;
  testId?: string;
}) {
  const { t } = useTranslation();
  const kindMeta = resolveKindMeta(entry.kind, isZh);
  const statusMeta = resolveStatusMeta(entry.status, isZh);
  const summaryFacts = buildTimelineFacts(entry, isZh, localeTag);
  const attemptId = attemptIdentityOverride?.trim() || entry.attempt?.attemptId?.trim() || null;
  const showTitle = !entry.attempt;
  const metricActions = entry.attempt
    ? buildAttemptMetricActions(entry, localeTag, isZh, t)
    : buildGenericMetricActions(entry, localeTag, isZh, t);

  return (
    <div
      data-testid={testId}
      data-open={isOpen ? "true" : "false"}
      className={cn(
        "invocation-detail-block w-full min-w-0 max-w-full overflow-hidden rounded-[1rem] px-4 py-3 text-left transition-[background-color,border-color] duration-200",
        !isOpen && "hover:border-[var(--invocation-detail-subsurface-border-active)]",
      )}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Chip tone={kindMeta.variant}>{kindMeta.label}</Chip>
            {entry.status ? <Chip tone={statusMeta.variant}>{statusMeta.label}</Chip> : null}
            {attemptId ? (
              <span className="tone-ink-primary font-mono text-[11px]">{attemptId}</span>
            ) : null}
          </div>
          <div className={cn("min-w-0", showTitle ? "mt-2" : "mt-1.5")}>
            {showTitle ? (
              <div className="text-sm font-semibold text-base-content">{entry.title}</div>
            ) : null}
            <TimelineSummaryFacts entry={entry} summaryFacts={summaryFacts} showTitle={showTitle} />
          </div>
        </div>
        <div className="flex shrink-0 items-start gap-3">
          <div className="text-right text-xs text-base-content/58">
            {formatTimestamp(entry.occurredAt, localeTag)}
          </div>
          {isOpen ? (
            <AppIcon
              name="chevron-down"
              className="mt-0.5 h-4 w-4 text-base-content/52"
              aria-hidden
            />
          ) : null}
        </div>
      </div>
      <TimelineSummaryMetrics
        entry={entry}
        metricActions={metricActions}
        isOpen={isOpen}
        activeSection={activeSection}
        onSelectSection={onSelectSection}
      />
    </div>
  );
}
