import * as PopoverPrimitive from "@radix-ui/react-popover";
import {
  type Dispatch,
  type MouseEvent as ReactMouseEvent,
  type SetStateAction,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import { cn } from "../../lib/utils";
import {
  bubbleArrowClassName,
  bubbleArrowStyle,
  bubbleContentClassName,
  bubbleSurfaceStyle,
} from "./bubble";
import { PopoverArrow, PopoverContent } from "./popover";
import { usePortaledTheme } from "./use-portaled-theme";

interface InfoTooltipProps {
  content: string;
  label: string;
  className?: string;
}

function InfoTooltipTrigger({
  label,
  className,
  open,
  pinned,
  tooltipId,
  rootRef,
  setRootElement,
  clearCloseTimer,
  handlePointerEnter,
  handlePointerLeave,
  setOpen,
  setPinned,
  scheduleClose,
}: Pick<
  InfoTooltipSurfaceProps,
  | "label"
  | "className"
  | "open"
  | "pinned"
  | "tooltipId"
  | "rootRef"
  | "setRootElement"
  | "clearCloseTimer"
  | "handlePointerEnter"
  | "handlePointerLeave"
  | "setOpen"
  | "setPinned"
  | "scheduleClose"
>) {
  return (
    <span
      ref={(node) => {
        rootRef.current = node;
        setRootElement(node);
      }}
      className={cn("inline-flex items-center", className)}
    >
      <PopoverPrimitive.Anchor asChild>
        <button
          type="button"
          aria-label={label}
          aria-describedby={open ? tooltipId : undefined}
          className="inline-flex h-5 w-5 items-center justify-center rounded-full text-[inherit] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          onMouseEnter={handlePointerEnter}
          onMouseLeave={handlePointerLeave}
          onClick={() => {
            clearCloseTimer();
            setPinned((current) => {
              const nextPinned = !current;
              setOpen(nextPinned);
              return nextPinned;
            });
          }}
          onFocus={() => {
            clearCloseTimer();
            setOpen(true);
          }}
          onBlur={() => {
            if (!pinned) scheduleClose();
          }}
        >
          <span
            className="inline-flex h-[18px] w-[18px] items-center justify-center text-[inherit]"
            aria-hidden
          >
            <AppIcon name="help-circle-outline" className="h-full w-full" />
          </span>
        </button>
      </PopoverPrimitive.Anchor>
    </span>
  );
}

function InfoTooltipContent({
  content,
  open,
  tooltipId,
  tooltipRef,
  portalTheme,
  handlePointerEnter,
  handlePointerLeave,
}: Pick<
  InfoTooltipSurfaceProps,
  | "content"
  | "open"
  | "tooltipId"
  | "tooltipRef"
  | "portalTheme"
  | "handlePointerEnter"
  | "handlePointerLeave"
>) {
  return (
    <PopoverContent
      data-theme={portalTheme}
      style={bubbleSurfaceStyle("neutral", portalTheme)}
      forceMount
      ref={tooltipRef}
      id={tooltipId}
      role="tooltip"
      aria-hidden={open ? "false" : "true"}
      onOpenAutoFocus={(event) => event.preventDefault()}
      onCloseAutoFocus={(event) => event.preventDefault()}
      side="top"
      sideOffset={4}
      avoidCollisions
      collisionPadding={8}
      className={cn(
        bubbleContentClassName("neutral"),
        "w-64 max-w-[calc(100vw-1rem)]",
        open ? "pointer-events-auto opacity-100" : "pointer-events-none opacity-0",
      )}
      onMouseEnter={handlePointerEnter}
      onMouseLeave={handlePointerLeave}
    >
      {content}
      <PopoverArrow
        data-theme={portalTheme}
        data-bubble-arrow="true"
        width={14}
        height={7}
        className={bubbleArrowClassName()}
        style={bubbleArrowStyle("neutral", portalTheme)}
      />
    </PopoverContent>
  );
}

interface InfoTooltipSurfaceProps extends InfoTooltipProps {
  open: boolean;
  pinned: boolean;
  setOpen: Dispatch<SetStateAction<boolean>>;
  setPinned: Dispatch<SetStateAction<boolean>>;
  tooltipId: string;
  rootRef: { current: HTMLSpanElement | null };
  tooltipRef: { current: HTMLDivElement | null };
  setRootElement: (element: HTMLSpanElement | null) => void;
  portalTheme: ReturnType<typeof usePortaledTheme>;
  clearCloseTimer: () => void;
  handlePointerEnter: () => void;
  handlePointerLeave: (event: ReactMouseEvent<HTMLElement>) => void;
  scheduleClose: () => void;
}

function InfoTooltipSurface({
  content,
  label,
  className,
  open,
  pinned,
  setOpen,
  setPinned,
  tooltipId,
  rootRef,
  tooltipRef,
  setRootElement,
  portalTheme,
  clearCloseTimer,
  handlePointerEnter,
  handlePointerLeave,
  scheduleClose,
}: InfoTooltipSurfaceProps) {
  return (
    <PopoverPrimitive.Root
      open={open}
      modal={false}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) setPinned(false);
      }}
    >
      <InfoTooltipTrigger
        label={label}
        className={className}
        open={open}
        pinned={pinned}
        tooltipId={tooltipId}
        rootRef={rootRef}
        setRootElement={setRootElement}
        clearCloseTimer={clearCloseTimer}
        handlePointerEnter={handlePointerEnter}
        handlePointerLeave={handlePointerLeave}
        setOpen={setOpen}
        setPinned={setPinned}
        scheduleClose={scheduleClose}
      />
      <InfoTooltipContent
        content={content}
        open={open}
        tooltipId={tooltipId}
        tooltipRef={tooltipRef}
        portalTheme={portalTheme}
        handlePointerEnter={handlePointerEnter}
        handlePointerLeave={handlePointerLeave}
      />
    </PopoverPrimitive.Root>
  );
}

export function InfoTooltip({ content, label, className }: InfoTooltipProps) {
  const [open, setOpen] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [rootElement, setRootElement] = useState<HTMLSpanElement | null>(null);
  const tooltipId = useId();
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  const portalTheme = usePortaledTheme(rootElement);

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current === null) return;
    window.clearTimeout(closeTimerRef.current);
    closeTimerRef.current = null;
  }, []);

  const isWithinTooltipCluster = (target: EventTarget | null) => {
    if (!(target instanceof Node)) return false;
    return Boolean(rootRef.current?.contains(target) || tooltipRef.current?.contains(target));
  };

  const scheduleClose = () => {
    if (pinned) return;
    clearCloseTimer();
    closeTimerRef.current = window.setTimeout(() => {
      setOpen(false);
      closeTimerRef.current = null;
    }, 100);
  };

  const handlePointerEnter = () => {
    clearCloseTimer();
    if (!pinned) setOpen(true);
  };

  const handlePointerLeave = (event: ReactMouseEvent<HTMLElement>) => {
    if (isWithinTooltipCluster(event.relatedTarget)) {
      clearCloseTimer();
      return;
    }
    scheduleClose();
  };

  useEffect(() => {
    if (open) return;
    setPinned(false);
  }, [open]);

  useEffect(() => () => clearCloseTimer(), [clearCloseTimer]);

  useEffect(() => {
    if (!open) return;

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      const tooltip = document.getElementById(tooltipId);
      if (target && (rootRef.current?.contains(target) || tooltip?.contains(target))) {
        return;
      }
      setPinned(false);
      setOpen(false);
    };

    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [open, tooltipId]);

  return (
    <InfoTooltipSurface
      content={content}
      label={label}
      className={className}
      open={open}
      pinned={pinned}
      setOpen={setOpen}
      setPinned={setPinned}
      tooltipId={tooltipId}
      rootRef={rootRef}
      tooltipRef={tooltipRef}
      setRootElement={setRootElement}
      portalTheme={portalTheme}
      clearCloseTimer={clearCloseTimer}
      handlePointerEnter={handlePointerEnter}
      handlePointerLeave={handlePointerLeave}
      scheduleClose={scheduleClose}
    />
  );
}
