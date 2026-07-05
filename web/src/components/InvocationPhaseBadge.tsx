import type { InvocationLivePhase, InvocationPhaseCounts } from "../lib/api";
import { useTranslation } from "../i18n";
import {
  getInvocationPhaseDisplay,
  normalizeInvocationPhaseCounts,
} from "../lib/invocationPhase";
import { cn } from "../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";
import { Badge } from "./ui/badge";

interface InvocationPhaseBadgeProps {
  phase: InvocationLivePhase;
  className?: string;
  appearance?: "badge" | "inline";
}

const PHASE_ICON_NAMES: Record<InvocationLivePhase, AppIconName> = {
  queued: "timer-refresh-outline",
  requesting: "send",
  responding: "loading",
};

const PHASE_TEXT_CLASSNAMES: Record<InvocationLivePhase, string> = {
  queued: "text-warning",
  requesting: "text-info",
  responding: "text-teal-600 dark:text-teal-300",
};

export function InvocationPhaseBadge({
  phase,
  className,
  appearance = "badge",
}: InvocationPhaseBadgeProps) {
  const { t } = useTranslation();
  const display = getInvocationPhaseDisplay(phase);
  const icon = (
    <AppIcon
      name={PHASE_ICON_NAMES[phase]}
      className={cn(
        appearance === "inline" ? "h-3.5 w-3.5" : "h-3 w-3",
        "shrink-0",
        appearance === "inline" && PHASE_TEXT_CLASSNAMES[phase],
        phase === "responding" && "animate-spin",
      )}
      aria-hidden="true"
    />
  );

  if (appearance === "inline") {
    return (
      <span
        data-testid="invocation-phase-badge"
        className={cn(
          "inline-flex items-center gap-1 whitespace-nowrap text-[11px] font-semibold leading-none",
          PHASE_TEXT_CLASSNAMES[phase],
          className,
        )}
      >
        {icon}
        <span>{t(display.labelKey)}</span>
      </span>
    );
  }

  return (
    <Badge
      variant={display.badgeVariant}
      data-testid="invocation-phase-badge"
      className={cn(
        "gap-1.5",
        phase === "responding" &&
          "border-teal-500/40 bg-teal-500/10 text-teal-700 dark:text-teal-300",
        className,
      )}
    >
      {icon}
      {t(display.labelKey)}
    </Badge>
  );
}

interface InvocationPhaseSegmentsProps {
  counts: InvocationPhaseCounts | null | undefined;
  className?: string;
  itemClassName?: string;
  showZero?: boolean;
  appearance?: "badge" | "inline";
}

export function InvocationPhaseSegments({
  counts,
  className,
  itemClassName,
  showZero = true,
  appearance = "badge",
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
      <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1.5", className)}>
        {items.map((item) => {
          const display = getInvocationPhaseDisplay(item.phase);
          return (
            <span
              key={item.phase}
              className={cn(
                "inline-flex items-center gap-1.5 whitespace-nowrap text-[11px] font-semibold leading-none tabular-nums text-base-content/68",
                itemClassName,
              )}
            >
              <AppIcon
                name={PHASE_ICON_NAMES[item.phase]}
                className={cn(
                  "h-3.5 w-3.5 shrink-0",
                  PHASE_TEXT_CLASSNAMES[item.phase],
                  item.phase === "responding" && "animate-spin",
                )}
                aria-hidden="true"
              />
              <span>{t(display.labelKey)}</span>
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
          <Badge
            key={item.phase}
            variant={display.badgeVariant}
            className={cn(
              "gap-1.5 tabular-nums",
              item.phase === "responding" &&
                "border-teal-500/40 bg-teal-500/10 text-teal-700 dark:text-teal-300",
              itemClassName,
            )}
          >
            <AppIcon
              name={PHASE_ICON_NAMES[item.phase]}
              className={cn(
                "h-3 w-3 shrink-0",
                item.phase === "responding" && "animate-spin",
              )}
              aria-hidden="true"
            />
            <span>{t(display.labelKey)}</span>
            <span className="font-mono font-semibold">{item.value}</span>
          </Badge>
        );
      })}
    </div>
  );
}
