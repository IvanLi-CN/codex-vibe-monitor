import { cn } from "../../lib/utils";

type FloatingFieldErrorPlacement = "input-corner" | "label-inline";

interface FloatingFieldErrorProps {
  message: string;
  className?: string;
  placement?: FloatingFieldErrorPlacement;
}

const bubbleSurfaceClasses =
  "relative rounded-xl border border-error/35 bg-error/12 px-3 py-1.5 text-xs font-medium text-error shadow-lg shadow-error/10 backdrop-blur";

const bubbleArrowClasses =
  "h-2.5 w-2.5 -translate-y-1/2 rotate-45 border-error/35 bg-error/12";

export function FloatingFieldError({
  message,
  className,
  placement = "input-corner",
}: FloatingFieldErrorProps) {
  if (placement === "label-inline") {
    return (
      <div
        role="alert"
        aria-live="polite"
        className={cn("pointer-events-none flex max-w-full justify-start", className)}
      >
        <div className={bubbleSurfaceClasses}>
          <span
            aria-hidden
            className={`absolute left-6 top-full border-b border-r ${bubbleArrowClasses}`}
          />
          {message}
        </div>
      </div>
    );
  }

  return (
    <div
      role="alert"
      aria-live="polite"
      className={cn(
        "pointer-events-none absolute right-3 top-full z-20 mt-2 flex max-w-[min(20rem,calc(100vw-4rem))] justify-end",
        className,
      )}
    >
      <div className={bubbleSurfaceClasses}>
        <span
          aria-hidden
          className={`absolute right-4 top-0 border-l border-t ${bubbleArrowClasses}`}
        />
        {message}
      </div>
    </div>
  );
}
