import { Tooltip } from "./ui/tooltip";
import { cn } from "../lib/utils";

export interface InvocationErrorSummaryProps {
  message: string;
  className?: string;
  textClassName?: string;
  tooltipSide?: "top" | "right" | "bottom" | "left";
}

export function InvocationErrorSummary({
  message,
  className,
  textClassName,
  tooltipSide = "top",
}: InvocationErrorSummaryProps) {
  const normalizedMessage = message.trim();
  if (!normalizedMessage) return null;

  return (
    <Tooltip
      side={tooltipSide}
      sideOffset={8}
      className={cn("inline-flex min-w-0 max-w-full rounded-[0.2rem]", className)}
      contentClassName="max-w-[min(32rem,calc(100vw-1rem))] whitespace-pre-wrap break-words"
      content={normalizedMessage}
      triggerProps={{
        tabIndex: 0,
        "aria-label": normalizedMessage,
      }}
    >
      <span data-testid="invocation-error-summary" className="block min-w-0 max-w-full">
        <span
          data-testid="invocation-error-summary-text"
          className={cn("block min-w-0 max-w-full truncate whitespace-nowrap", textClassName)}
        >
          {normalizedMessage}
        </span>
      </span>
    </Tooltip>
  );
}
