import { useRef } from "react";
import type { useTranslation } from "../../i18n";
import type { DashboardWorkingConversationInvocationModel } from "../../lib/dashboardWorkingConversations";
import {
  buildInvocationCompactTiming,
  formatInvocationCompactTimingValue,
  type InvocationCompactTimingDisplay,
  type InvocationCompactTimingPresentation,
  reconcileInvocationCompactTiming,
} from "../../lib/invocationCompactTiming";
import { isFinitePositiveMilliseconds } from "../../lib/invocationTiming";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "../shared/AppIcon";

type CompactLatencyReading = {
  label: string;
  value: string;
  icon: AppIconName;
  tone: string;
  testId: string;
};

function buildCompactLatencyReadings(
  timing: InvocationCompactTimingDisplay,
  presentation: InvocationCompactTimingPresentation,
  localeTag: string,
  t: ReturnType<typeof useTranslation>["t"],
): CompactLatencyReading[] {
  const requestLabel = t("dashboard.compactTiming.request");
  const ttftLabel = t("dashboard.compactTiming.ttft");
  const responseLabel = t("dashboard.compactTiming.response");
  const estimatedLabel = t("dashboard.compactTiming.estimated");
  const requestAccessibleLabel = presentation.requestProvisional
    ? `${requestLabel} (${estimatedLabel})`
    : requestLabel;
  const ttftAccessibleLabel = presentation.ttftProvisional
    ? `${ttftLabel} (${estimatedLabel})`
    : ttftLabel;
  const requestValue = formatInvocationCompactTimingValue(
    timing.state === "responding" || timing.state === "terminal" ? null : presentation.requestMs,
    localeTag,
  );
  const ttftValue = formatInvocationCompactTimingValue(
    timing.state === "requesting" ? null : presentation.ttftMs,
    localeTag,
  );
  const responseValue = formatInvocationCompactTimingValue(
    timing.state === "awaitingToken" || timing.state === "requesting"
      ? null
      : presentation.responseMs,
    localeTag,
    isFinitePositiveMilliseconds,
  );
  if (timing.state === "requesting") {
    return [
      {
        label: requestAccessibleLabel,
        value: requestValue,
        icon: "clock-outline",
        tone: "text-info",
        testId: "dashboard-compact-latency-request",
      },
    ];
  }
  if (timing.state === "awaitingToken") {
    return [
      {
        label: requestAccessibleLabel,
        value: requestValue,
        icon: "clock-outline",
        tone: "text-info",
        testId: "dashboard-compact-latency-request",
      },
      {
        label: ttftAccessibleLabel,
        value: ttftValue,
        icon: "timer-outline",
        tone: "text-success",
        testId: "dashboard-compact-latency-ttft",
      },
    ];
  }
  return [
    {
      label: ttftAccessibleLabel,
      value: ttftValue,
      icon: "timer-outline",
      tone: "text-success",
      testId: "dashboard-compact-latency-ttft",
    },
    {
      label: responseLabel,
      value: responseValue,
      icon: "speedometer",
      tone: "text-secondary",
      testId: "dashboard-compact-latency-response",
    },
  ];
}

export function CompactLatencyPills({
  invocation,
  nowMs,
  localeTag,
  t,
  className,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  nowMs: number;
  localeTag: string;
  t: ReturnType<typeof useTranslation>["t"];
  className?: string;
}) {
  const presentationRef = useRef<{
    key: string;
    value: InvocationCompactTimingPresentation;
  } | null>(null);
  const timing = buildInvocationCompactTiming({
    record: invocation.record,
    occurredAtEpoch: invocation.occurredAtEpoch,
    isInFlight: invocation.isInFlight,
    nowMs,
  });
  const previous =
    presentationRef.current?.key === invocation.record.invokeId
      ? presentationRef.current.value
      : null;
  const presentation = reconcileInvocationCompactTiming(timing, previous);
  presentationRef.current = { key: invocation.record.invokeId, value: presentation };
  const visibleReadings = buildCompactLatencyReadings(timing, presentation, localeTag, t);

  return (
    <fieldset
      data-testid="dashboard-compact-latency-pills"
      className={cn(
        "inline-flex min-w-0 shrink-0 flex-wrap items-center justify-end gap-1 font-mono text-[11px] font-semibold leading-none text-base-content/86",
        className,
      )}
      aria-label={visibleReadings.map(({ label, value }) => `${label} ${value}`).join("; ")}
      title={visibleReadings.map(({ label, value }) => `${label}: ${value}`).join(" · ")}
    >
      {visibleReadings.map(({ label, value, icon, tone, testId }) => (
        <span
          key={testId}
          data-testid={testId}
          className={cn("inline-flex min-w-0 items-center gap-0.5", tone)}
          title={`${label}: ${value}`}
        >
          <AppIcon
            name={icon}
            className="h-3.5 w-3.5 shrink-0"
            data-compact-latency-icon-name={icon}
            aria-hidden
          />
          <span className="truncate whitespace-nowrap">{value}</span>
        </span>
      ))}
    </fieldset>
  );
}
