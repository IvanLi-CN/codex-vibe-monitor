import { cn } from "../../lib/utils";

interface FloatingFieldErrorProps {
  message: string;
  className?: string;
}

export function FloatingFieldError({
  message,
  className,
}: FloatingFieldErrorProps) {
  return (
    <div
      role="alert"
      aria-live="polite"
      className={cn(
        "pointer-events-none absolute right-3 top-full z-20 mt-2 flex max-w-[min(20rem,calc(100vw-4rem))] justify-end",
        className,
      )}
    >
      <div className="relative rounded-xl border border-error/55 bg-base-100 px-3 py-1.5 text-xs font-medium text-error shadow-lg shadow-error/10 backdrop-blur">
        <span
          aria-hidden
          className="absolute right-4 top-0 h-2.5 w-2.5 -translate-y-1/2 rotate-45 border-l border-t border-error/55 bg-base-100"
        />
        {message}
      </div>
    </div>
  );
}
