import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import * as React from "react";
import { cn } from "../../lib/utils";
import { floatingSurfaceArrowStyle, floatingSurfaceStyle } from "./floating-surface";
import { OverlayHostProvider } from "./overlay-host";
import { useOverlayHostElement, useResolvedOverlayContainer } from "./use-overlay-host";
import { usePortaledTheme } from "./use-portaled-theme";

const LONG_PRESS_DELAY_MS = 360;

function tokenList(className?: string) {
  return className?.split(/\s+/).filter(Boolean) ?? [];
}

function hasUtilityOverride(
  className: string | undefined,
  {
    exact = [],
    prefixes = [],
  }: {
    exact?: string[];
    prefixes?: string[];
  },
) {
  return tokenList(className).some((token) => {
    const normalized = token.startsWith("!") ? token.slice(1) : token;
    return exact.includes(normalized) || prefixes.some((prefix) => normalized.startsWith(prefix));
  });
}

function resolveTooltipContentStyle(
  contentClassName: string | undefined,
  theme: ReturnType<typeof usePortaledTheme>,
) {
  const style = { ...floatingSurfaceStyle("neutral", theme) };
  const hasBackgroundOverride = hasUtilityOverride(contentClassName, { prefixes: ["bg-"] });
  if (hasBackgroundOverride) {
    delete style.backgroundColor;
    delete style.backdropFilter;
    delete style.WebkitBackdropFilter;
  }
  if (hasUtilityOverride(contentClassName, { prefixes: ["border-"] })) {
    delete style.borderColor;
  }
  if (hasUtilityOverride(contentClassName, { exact: ["shadow"], prefixes: ["shadow-"] })) {
    delete style.boxShadow;
  }
  if (hasUtilityOverride(contentClassName, { prefixes: ["backdrop-"] })) {
    delete style.backdropFilter;
    delete style.WebkitBackdropFilter;
  }
  return style;
}

function resolveTooltipArrowStyle(
  arrowClassName: string | undefined,
  theme: ReturnType<typeof usePortaledTheme>,
) {
  const style = { ...floatingSurfaceArrowStyle("neutral", theme) };
  if (hasUtilityOverride(arrowClassName, { prefixes: ["fill-"] })) {
    delete style.fill;
  }
  if (hasUtilityOverride(arrowClassName, { prefixes: ["stroke-"] })) {
    delete style.stroke;
    delete style.strokeWidth;
  }
  return style;
}

interface TooltipProps {
  content: React.ReactNode;
  children: React.ReactNode;
  container?: HTMLElement | null;
  className?: string;
  contentClassName?: string;
  arrowClassName?: string;
  side?: "top" | "right" | "bottom" | "left";
  sideOffset?: number;
  open?: boolean;
  clickToOpen?: boolean;
  triggerElement?: "span" | "div";
  triggerProps?: React.HTMLAttributes<HTMLElement>;
}

export function Tooltip({
  content,
  children,
  container,
  className,
  contentClassName,
  arrowClassName,
  side = "top",
  sideOffset = 10,
  open,
  clickToOpen = false,
  triggerElement = "span",
  triggerProps,
}: TooltipProps) {
  const longPressTimerRef = React.useRef<number | null>(null);
  const [hoverOpen, setHoverOpen] = React.useState(false);
  const [clickOpen, setClickOpen] = React.useState(false);
  const [longPressOpen, setLongPressOpen] = React.useState(false);
  const [rootElement, setRootElement] = React.useState<HTMLElement | null>(null);
  const resolvedContainer = useResolvedOverlayContainer(container);
  const { hostElement, ref: contentRef } = useOverlayHostElement<HTMLDivElement>(undefined);
  const hostValue = hostElement ?? (container === undefined ? resolvedContainer : container);
  const portalTheme = usePortaledTheme(rootElement);
  const contentStyle = React.useMemo(
    () => resolveTooltipContentStyle(contentClassName, portalTheme),
    [contentClassName, portalTheme],
  );
  const arrowStyle = React.useMemo(
    () => resolveTooltipArrowStyle(arrowClassName, portalTheme),
    [arrowClassName, portalTheme],
  );

  const clearLongPressTimer = React.useCallback(() => {
    if (longPressTimerRef.current != null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  React.useEffect(() => () => clearLongPressTimer(), [clearLongPressTimer]);

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (open !== undefined || event.button !== 0) return;
      clearLongPressTimer();
      longPressTimerRef.current = window.setTimeout(() => {
        setLongPressOpen(true);
        longPressTimerRef.current = null;
      }, LONG_PRESS_DELAY_MS);
    },
    [clearLongPressTimer, open],
  );

  const handlePointerRelease = React.useCallback(() => {
    if (open !== undefined) return;
    clearLongPressTimer();
    setLongPressOpen(false);
  }, [clearLongPressTimer, open]);

  const resolvedOpen = open ?? (hoverOpen || clickOpen || longPressOpen);
  const triggerClassName = cn(triggerElement === "div" ? "flex" : "inline-flex", className);
  const triggerEventProps = {
    ref: setRootElement,
    className: triggerClassName,
    ...triggerProps,
    onBlur: (event: React.FocusEvent<HTMLElement>) => {
      triggerProps?.onBlur?.(event);
      if (open !== undefined) return;
      setHoverOpen(false);
      setClickOpen(false);
    },
    onClick: (event: React.MouseEvent<HTMLElement>) => {
      triggerProps?.onClick?.(event);
      if (!clickToOpen || open !== undefined || event.defaultPrevented) return;
      setClickOpen((current) => !current);
    },
    onFocus: (event: React.FocusEvent<HTMLElement>) => {
      triggerProps?.onFocus?.(event);
      if (open === undefined) setHoverOpen(true);
    },
    onMouseEnter: (event: React.MouseEvent<HTMLElement>) => {
      triggerProps?.onMouseEnter?.(event);
      if (open === undefined) setHoverOpen(true);
    },
    onMouseLeave: (event: React.MouseEvent<HTMLElement>) => {
      triggerProps?.onMouseLeave?.(event);
      if (open === undefined) setHoverOpen(false);
    },
    onPointerDownCapture: handlePointerDown,
    onPointerDown: handlePointerDown,
    onPointerUpCapture: handlePointerRelease,
    onPointerUp: handlePointerRelease,
    onPointerCancelCapture: handlePointerRelease,
    onPointerCancel: handlePointerRelease,
    onPointerLeave: handlePointerRelease,
  };

  return (
    <TooltipPrimitive.Provider delayDuration={120}>
      <TooltipPrimitive.Root open={resolvedOpen}>
        <TooltipPrimitive.Trigger asChild>
          {triggerElement === "div" ? (
            <div {...triggerEventProps}>{children}</div>
          ) : (
            <span {...triggerEventProps}>{children}</span>
          )}
        </TooltipPrimitive.Trigger>
        <TooltipPrimitive.Portal container={resolvedContainer ?? undefined}>
          <OverlayHostProvider value={hostValue}>
            <TooltipPrimitive.Content
              data-theme={portalTheme}
              ref={contentRef}
              side={side}
              sideOffset={sideOffset}
              style={contentStyle}
              className={cn(
                "z-50 max-w-[min(20rem,calc(100vw-1rem))] rounded-xl border px-3 py-2",
                "text-left text-xs text-base-content outline-none",
                "data-[state=delayed-open]:animate-in data-[state=closed]:animate-out",
                "data-[state=closed]:fade-out-0 data-[state=delayed-open]:fade-in-0",
                "data-[state=closed]:zoom-out-95 data-[state=delayed-open]:zoom-in-95",
                "data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2",
                "data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2",
                contentClassName,
              )}
            >
              {content}
              <TooltipPrimitive.Arrow
                data-theme={portalTheme}
                style={arrowStyle}
                className={cn(arrowClassName)}
                width={14}
                height={8}
              />
            </TooltipPrimitive.Content>
          </OverlayHostProvider>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}
