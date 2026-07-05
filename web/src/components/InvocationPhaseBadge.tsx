import type { InvocationLivePhase, InvocationPhaseCounts } from "../lib/api";
import { useTranslation } from "../i18n";
import {
  getInvocationPhaseDisplay,
  normalizeInvocationPhaseCounts,
} from "../lib/invocationPhase";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";

interface InvocationPhaseBadgeProps {
  phase: InvocationLivePhase;
  className?: string;
}

export function InvocationPhaseBadge({ phase, className }: InvocationPhaseBadgeProps) {
  const { t } = useTranslation();
  const display = getInvocationPhaseDisplay(phase);
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
      <span
        className={cn(
          "size-1.5 rounded-full",
          phase === "queued" && "bg-warning",
          phase === "requesting" && "bg-info",
          phase === "responding" && "bg-teal-500",
        )}
        aria-hidden="true"
      />
      {t(display.labelKey)}
    </Badge>
  );
}

interface InvocationPhaseSegmentsProps {
  counts: InvocationPhaseCounts | null | undefined;
  className?: string;
  itemClassName?: string;
  showZero?: boolean;
}

export function InvocationPhaseSegments({
  counts,
  className,
  itemClassName,
  showZero = true,
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
            <span
              className={cn(
                "size-1.5 rounded-full",
                item.phase === "queued" && "bg-warning",
                item.phase === "requesting" && "bg-info",
                item.phase === "responding" && "bg-teal-500",
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
