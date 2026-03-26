import * as React from "react";
import {
  bubbleArrowClassName,
  bubbleArrowStyle,
  bubbleContentClassName,
  bubbleSurfaceStyle,
  type BubbleVariant,
} from "./bubble";
import { PopoverArrow, PopoverContent } from "./popover";
import { usePortaledTheme } from "./use-portaled-theme";
import { cn } from "../../lib/utils";

interface BubblePopoverContentProps
  extends React.ComponentPropsWithoutRef<typeof PopoverContent> {
  anchorElement?: HTMLElement | null;
  variant?: BubbleVariant;
  arrowWidth?: number;
  arrowHeight?: number;
  arrowClassName?: string;
  hideArrow?: boolean;
}

export const BubblePopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverContent>,
  BubblePopoverContentProps
>(function BubblePopoverContent(
  {
    anchorElement,
    variant = "neutral",
    arrowWidth = 14,
    arrowHeight = 8,
    arrowClassName,
    hideArrow = false,
    className,
    style,
    children,
    ...props
  },
  ref,
) {
  const portalTheme = usePortaledTheme(anchorElement ?? null);

  return (
    <PopoverContent
      ref={ref}
      data-theme={portalTheme}
      style={{
        ...bubbleSurfaceStyle(variant, portalTheme),
        ...style,
      }}
      className={cn(bubbleContentClassName(variant), className)}
      {...props}
    >
      {children}
      {hideArrow ? null : (
        <PopoverArrow
          data-theme={portalTheme}
          data-bubble-arrow="true"
          width={arrowWidth}
          height={arrowHeight}
          className={cn(bubbleArrowClassName(), arrowClassName)}
          style={bubbleArrowStyle(variant, portalTheme)}
        />
      )}
    </PopoverContent>
  );
});
