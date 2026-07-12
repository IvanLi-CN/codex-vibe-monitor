import { Button } from "../../components/ui/button";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

type ListBodyStateVariant = "loading" | "error" | "empty";

interface ListBodyStateProps {
  variant: ListBodyStateVariant;
  title: string;
  description?: string | null;
  icon?: AppIconName;
  retryLabel?: string;
  onRetry?: () => void;
  className?: string;
  testId?: string;
  skeletonRows?: number;
}

const variantClasses: Record<ListBodyStateVariant, string> = {
  loading: "border-base-300/80 bg-base-100/60 text-primary",
  error: "border-error/35 bg-error/10 text-error",
  empty: "border-base-300/80 bg-base-100/45 text-primary",
};

const defaultIcons: Record<ListBodyStateVariant, AppIconName> = {
  loading: "loading",
  error: "alert-circle-outline",
  empty: "server-network-outline",
};

export function ListBodyState({
  variant,
  title,
  description,
  icon,
  retryLabel,
  onRetry,
  className,
  testId,
  skeletonRows = 4,
}: ListBodyStateProps) {
  const resolvedIcon = icon ?? defaultIcons[variant];
  const showSkeleton = variant === "loading";

  return (
    <div
      data-testid={testId}
      className={cn(
        "flex min-h-[14rem] flex-col items-center justify-center rounded-[1rem] border border-dashed px-6 py-8 text-center",
        variantClasses[variant],
        className,
      )}
      role={variant === "error" ? "alert" : "status"}
      aria-live="polite"
      aria-busy={variant === "loading" ? "true" : undefined}
      aria-label={variant === "loading" ? title : undefined}
    >
      <div
        className={cn(
          "mb-4 flex h-12 w-12 items-center justify-center rounded-full",
          variant === "error" ? "bg-error/10 text-error" : "bg-primary/10 text-primary",
        )}
      >
        <AppIcon
          name={resolvedIcon}
          className={cn("h-6 w-6", variant === "loading" && "animate-spin")}
          aria-hidden
        />
      </div>
      <h3 className="text-base font-semibold text-base-content">{title}</h3>
      {description ? (
        <p className="mt-2 max-w-md text-sm leading-6 text-base-content/70">{description}</p>
      ) : null}
      {showSkeleton ? (
        <div className="mt-6 w-full max-w-2xl space-y-3" aria-hidden>
          {Array.from({ length: Math.max(1, skeletonRows) }, (_, index) => (
            <div
              key={index}
              className="grid grid-cols-[minmax(7rem,0.8fr)_minmax(10rem,1.4fr)_minmax(6rem,0.6fr)] gap-3 rounded-xl border border-base-300/50 bg-base-100/65 p-3"
            >
              <div className="h-3 rounded-full bg-base-content/10" />
              <div className="h-3 rounded-full bg-base-content/10" />
              <div className="h-3 rounded-full bg-base-content/10" />
            </div>
          ))}
        </div>
      ) : null}
      {variant === "error" && onRetry && retryLabel ? (
        <Button type="button" variant="secondary" className="mt-4" onClick={onRetry}>
          <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
          {retryLabel}
        </Button>
      ) : null}
    </div>
  );
}
