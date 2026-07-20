import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";
import { cn } from "../../lib/utils";

const badgeVariants = cva(
  "inline-flex shrink-0 items-center whitespace-nowrap rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 focus:ring-offset-base-100",
  {
    variants: {
      variant: {
        default: "border-primary/40 bg-primary/10 tone-ink-primary",
        accent: "border-accent/35 bg-accent/15 tone-ink-accent",
        secondary: "border-base-300 bg-base-200/70 text-base-content/85",
        success: "border-success/35 bg-success/15 tone-ink-success",
        info: "border-info/35 bg-info/15 tone-ink-info",
        warning: "border-warning/45 bg-warning/12 tone-ink-warning",
        error: "border-error/35 bg-error/15 tone-ink-error",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge };
