import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type { InvocationLivePhase, InvocationPhaseCounts } from "../../lib/api";
import {
  getInvocationPhaseDisplay,
  normalizeInvocationPhaseCounts,
} from "../../lib/invocationPhase";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "../shared/AppIcon";

export type InvocationPhaseMotion = "static" | "dynamic";

interface InvocationPhaseChipProps {
  phase: InvocationLivePhase;
  className?: string;
  appearance?: "badge" | "inline";
  showLabel?: boolean;
  motion?: InvocationPhaseMotion;
}

const PHASE_ICON_NAMES: Record<InvocationLivePhase, AppIconName> = {
  queued: "timer-refresh-outline",
  requesting: "navigation-variant",
  responding: "sync",
};

const STATIC_PHASE_ICON_NAMES: Record<InvocationLivePhase, AppIconName> = {
  ...PHASE_ICON_NAMES,
  responding: "chat-processing-outline",
};

const PHASE_TEXT_CLASSNAMES: Record<InvocationLivePhase, string> = {
  queued: "text-warning",
  requesting: "text-info",
  responding: "text-teal-600 dark:text-teal-300",
};

function phaseMotionClassName(phase: InvocationLivePhase, motion: InvocationPhaseMotion) {
  if (motion !== "dynamic") return null;
  if (phase === "requesting") return "animate-invocation-phase-requesting";
  if (phase === "responding") return "animate-spin";
  return null;
}

function phaseIconName(phase: InvocationLivePhase, motion: InvocationPhaseMotion) {
  return motion === "static" ? STATIC_PHASE_ICON_NAMES[phase] : PHASE_ICON_NAMES[phase];
}

export function InvocationPhaseChip({
  phase,
  className,
  appearance = "badge",
  showLabel = true,
  motion = "dynamic",
}: InvocationPhaseChipProps) {
  const { t } = useTranslation();
  const display = getInvocationPhaseDisplay(phase);
  const label = t(display.labelKey);
  const motionClassName = phaseMotionClassName(phase, motion);
  const iconName = phaseIconName(phase, motion);
  const icon = (
    <AppIcon
      name={iconName}
      data-testid="invocation-phase-icon"
      data-phase-icon-name={iconName}
      className={cn(
        appearance === "inline" ? "h-3.5 w-3.5" : "h-3 w-3",
        "shrink-0",
        appearance === "inline" && PHASE_TEXT_CLASSNAMES[phase],
        motionClassName,
      )}
      aria-hidden="true"
    />
  );

  if (appearance === "inline") {
    return (
      <span
        data-testid="invocation-phase-badge"
        data-phase={phase}
        data-phase-label-visible={showLabel ? "true" : "false"}
        data-phase-motion={motion}
        className={cn(
          showLabel
            ? "inline-flex items-center gap-1 whitespace-nowrap text-[11px] font-semibold leading-none"
            : "inline-flex h-5 w-5 items-center justify-center rounded-full bg-base-100/12",
          PHASE_TEXT_CLASSNAMES[phase],
          className,
        )}
        aria-label={label}
        title={showLabel ? undefined : label}
        role="img"
      >
        {icon}
        {showLabel ? <span>{label}</span> : null}
      </span>
    );
  }

  return (
    <Chip
      tone={display.chipTone}
      data-testid="invocation-phase-badge"
      data-phase={phase}
      data-phase-label-visible={showLabel ? "true" : "false"}
      data-phase-motion={motion}
      className={cn(showLabel ? "gap-1.5" : "h-6 w-6 justify-center px-0", className)}
      aria-label={showLabel ? undefined : label}
      title={showLabel ? undefined : label}
      role={showLabel ? undefined : "img"}
    >
      {icon}
      {showLabel ? label : null}
    </Chip>
  );
}

interface InvocationPhaseSegmentsProps {
  counts: InvocationPhaseCounts | null | undefined;
  className?: string;
  itemClassName?: string;
  showZero?: boolean;
  appearance?: "badge" | "inline";
  motion?: InvocationPhaseMotion;
  showLabel?: boolean;
  countVisibility?: "always" | "multipleOnly";
}

export function InvocationPhaseSegments({
  counts,
  className,
  itemClassName,
  showZero = true,
  appearance = "badge",
  motion = "static",
  showLabel = true,
  countVisibility = "always",
}: InvocationPhaseSegmentsProps) {
  const { t } = useTranslation();
  if (counts == null) return null;

  const normalized = normalizeInvocationPhaseCounts(counts);
  const phaseItems: Array<{ phase: InvocationLivePhase; value: number }> = [
    { phase: "queued", value: normalized.queued },
    { phase: "requesting", value: normalized.requesting },
    { phase: "responding", value: normalized.responding },
  ];
  const items = phaseItems.filter((item) => showZero || item.value > 0);

  if (items.length === 0) return null;

  if (appearance === "inline") {
    return (
      <div
        className={cn(
          "flex flex-wrap items-center gap-y-1.5",
          showLabel ? "gap-x-3" : "gap-x-2.5",
          className,
        )}
      >
        {items.map((item) => {
          const display = getInvocationPhaseDisplay(item.phase);
          const label = t(display.labelKey);
          const sharedClassName = cn(
            "inline-flex items-center whitespace-nowrap text-[11px] font-semibold leading-none tabular-nums text-base-content/68",
            showLabel ? "gap-1.5" : "gap-1",
            itemClassName,
          );
          const icon = (
            <AppIcon
              name={phaseIconName(item.phase, motion)}
              data-testid="invocation-phase-icon"
              data-phase-icon-name={phaseIconName(item.phase, motion)}
              className={cn(
                "h-3.5 w-3.5 shrink-0",
                PHASE_TEXT_CLASSNAMES[item.phase],
                phaseMotionClassName(item.phase, motion),
              )}
              aria-hidden="true"
            />
          );

          if (!showLabel) {
            return (
              <span
                key={item.phase}
                data-testid="invocation-phase-segment"
                data-phase={item.phase}
                data-phase-motion={motion}
                data-phase-label-visible="false"
                role="img"
                aria-label={`${label} ${item.value}`}
                title={`${label} ${item.value}`}
                className={sharedClassName}
              >
                {icon}
                {countVisibility === "always" || item.value > 1 ? (
                  <span className="font-mono text-base-content/86">{item.value}</span>
                ) : null}
              </span>
            );
          }

          return (
            <span
              key={item.phase}
              data-testid="invocation-phase-segment"
              data-phase={item.phase}
              data-phase-motion={motion}
              data-phase-label-visible="true"
              className={sharedClassName}
            >
              {icon}
              <span>{label}</span>
              <span className="font-mono text-base-content/86">{item.value}</span>
            </span>
          );
        })}
      </div>
    );
  }

  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      {items.map((item) => {
        const display = getInvocationPhaseDisplay(item.phase);
        return (
          <Chip
            key={item.phase}
            tone={display.chipTone}
            data-testid="invocation-phase-segment"
            data-phase={item.phase}
            data-phase-motion={motion}
            className={cn("gap-1.5 tabular-nums", itemClassName)}
          >
            <AppIcon
              name={phaseIconName(item.phase, motion)}
              data-testid="invocation-phase-icon"
              data-phase-icon-name={phaseIconName(item.phase, motion)}
              className={cn("h-3 w-3 shrink-0", phaseMotionClassName(item.phase, motion))}
              aria-hidden="true"
            />
            <span>{t(display.labelKey)}</span>
            <span className="font-mono font-semibold">{item.value}</span>
          </Chip>
        );
      })}
    </div>
  );
}
