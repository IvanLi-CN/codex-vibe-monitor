import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "../../lib/utils";

const chipVariants = cva(
  "chip inline-flex shrink-0 items-center whitespace-nowrap rounded-full border font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:pointer-events-none disabled:opacity-55",
  {
    variants: {
      tone: {
        neutral: "chip-tone-neutral",
        primary: "chip-tone-primary",
        secondary: "chip-tone-secondary",
        accent: "chip-tone-accent",
        info: "chip-tone-info",
        success: "chip-tone-success",
        warning: "chip-tone-warning",
        error: "chip-tone-error",
        sky: "chip-tone-sky",
        cyan: "chip-tone-cyan",
        blue: "chip-tone-blue",
        indigo: "chip-tone-indigo",
        violet: "chip-tone-violet",
        fuchsia: "chip-tone-fuchsia",
        teal: "chip-tone-teal",
        emerald: "chip-tone-emerald",
        amber: "chip-tone-amber",
        orange: "chip-tone-orange",
      },
      size: {
        micro: "h-4 px-1.5 py-0 text-[8.5px] leading-none",
        compact: "min-h-5 px-2 py-0.5 text-[9px] leading-none",
        default: "px-2.5 py-0.5 text-xs",
        header: "h-6 px-2.5 py-0 text-[11px] leading-none",
        mailbox: "h-7 px-2.5 py-0 text-xs",
        square: "h-5 w-5 justify-center overflow-hidden px-0 py-0 text-[11px] leading-none",
      },
    },
    defaultVariants: {
      tone: "neutral",
      size: "default",
    },
  },
);

export type SemanticChipTone =
  | "neutral"
  | "primary"
  | "secondary"
  | "accent"
  | "info"
  | "success"
  | "warning"
  | "error";

export type CategoricalChipTone =
  | "sky"
  | "cyan"
  | "blue"
  | "indigo"
  | "violet"
  | "fuchsia"
  | "teal"
  | "emerald"
  | "amber"
  | "orange";

export type ChipTone = SemanticChipTone | CategoricalChipTone;
export type ChipSize = NonNullable<VariantProps<typeof chipVariants>["size"]>;

export interface ChipProps
  extends React.HTMLAttributes<HTMLElement>,
    Omit<VariantProps<typeof chipVariants>, "tone" | "size"> {
  tone?: ChipTone;
  size?: ChipSize;
  asChild?: boolean;
}

const Chip = React.forwardRef<HTMLElement, ChipProps>(
  ({ className, tone, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "span";
    return <Comp ref={ref} className={cn(chipVariants({ tone, size }), className)} {...props} />;
  },
);
Chip.displayName = "Chip";

export { Chip, chipVariants };
