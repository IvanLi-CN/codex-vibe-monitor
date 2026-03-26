import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { AppIcon } from "../AppIcon";
import { Button } from "../ui/button";
import { BubblePopoverContent } from "../ui/bubble-popover";
import { Popover, PopoverAnchor } from "../ui/popover";
import { Spinner } from "../ui/spinner";
import { cn } from "../../lib/utils";

const LONG_PRESS_DELAY_MS = 420;
const HOVER_CLOSE_DELAY_MS = 140;

function selectManualCopyText(target: HTMLDivElement | null) {
  if (!target) return;
  target.focus();
  const selection = target.ownerDocument.getSelection?.();
  if (!selection) return;
  const range = target.ownerDocument.createRange();
  range.selectNodeContents(target);
  selection.removeAllRanges();
  selection.addRange(range);
}

export interface BatchOauthActionButtonProps {
  mode: "generate" | "copy";
  primaryAriaLabel: string;
  regenerateAriaLabel: string;
  popoverTitle: string;
  popoverDescription: string;
  remainingLabel?: string | null;
  expiresAtLabel?: string | null;
  manualCopyTitle: string;
  manualCopyDescription: string;
  manualCopyValue?: string | null;
  busy?: boolean;
  disabled?: boolean;
  regenerateDisabled?: boolean;
  onPrimaryAction: () => void;
  onRegenerate: () => void;
  onManualCopyOpenChange?: (open: boolean) => void;
  className?: string;
}

export function BatchOauthActionButton({
  mode,
  primaryAriaLabel,
  regenerateAriaLabel,
  popoverTitle,
  popoverDescription,
  remainingLabel,
  expiresAtLabel,
  manualCopyTitle,
  manualCopyDescription,
  manualCopyValue,
  busy = false,
  disabled = false,
  regenerateDisabled = false,
  onPrimaryAction,
  onRegenerate,
  onManualCopyOpenChange,
  className,
}: BatchOauthActionButtonProps) {
  const longPressTimerRef = useRef<number | null>(null);
  const hoverCloseTimerRef = useRef<number | null>(null);
  const manualCopyValueRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const longPressTriggeredRef = useRef(false);
  const [hoverOpen, setHoverOpen] = useState(false);
  const [pinnedOpen, setPinnedOpen] = useState(false);

  useEffect(() => {
    return () => {
      if (longPressTimerRef.current != null) {
        window.clearTimeout(longPressTimerRef.current);
      }
      if (hoverCloseTimerRef.current != null) {
        window.clearTimeout(hoverCloseTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!manualCopyValue) return;
    setPinnedOpen(true);
    const timerId = window.setTimeout(() => {
      selectManualCopyText(manualCopyValueRef.current);
    }, 0);
    return () => {
      window.clearTimeout(timerId);
    };
  }, [manualCopyValue]);

  const clearLongPressTimer = () => {
    if (longPressTimerRef.current != null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  };

  const clearHoverCloseTimer = () => {
    if (hoverCloseTimerRef.current != null) {
      window.clearTimeout(hoverCloseTimerRef.current);
      hoverCloseTimerRef.current = null;
    }
  };

  const closePopover = () => {
    clearHoverCloseTimer();
    setHoverOpen(false);
    setPinnedOpen(false);
    if (manualCopyValue) {
      onManualCopyOpenChange?.(false);
    }
  };

  const openHoverPopover = () => {
    clearHoverCloseTimer();
    setHoverOpen(true);
  };

  const scheduleHoverPopoverClose = () => {
    clearHoverCloseTimer();
    hoverCloseTimerRef.current = window.setTimeout(() => {
      hoverCloseTimerRef.current = null;
      setHoverOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  };

  const handlePointerDown = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (disabled || busy) return;
    if (event.button !== 0) return;
    if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
    clearLongPressTimer();
    longPressTimerRef.current = window.setTimeout(() => {
      longPressTriggeredRef.current = true;
      setPinnedOpen(true);
      longPressTimerRef.current = null;
    }, LONG_PRESS_DELAY_MS);
  };

  const handlePointerRelease = () => {
    clearLongPressTimer();
  };

  const handlePrimaryClick = () => {
    if (disabled) return;
    if (longPressTriggeredRef.current) {
      longPressTriggeredRef.current = false;
      return;
    }
    if (manualCopyValue) {
      onManualCopyOpenChange?.(false);
    }
    onPrimaryAction();
  };

  const handleRegenerate = () => {
    if (manualCopyValue) {
      onManualCopyOpenChange?.(false);
    }
    setPinnedOpen(false);
    onRegenerate();
  };

  const resolvedOpen = hoverOpen || pinnedOpen || Boolean(manualCopyValue);

  return (
    <Popover
      open={resolvedOpen}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          closePopover();
        }
      }}
    >
      <PopoverAnchor asChild>
        <Button
          ref={triggerRef}
          type="button"
          size="icon"
          variant={mode === "copy" ? "secondary" : "default"}
          className={cn("h-9 w-9 shrink-0 rounded-full", className)}
          aria-label={primaryAriaLabel}
          title={primaryAriaLabel}
          disabled={disabled}
          onMouseEnter={openHoverPopover}
          onMouseLeave={() => {
            if (!pinnedOpen && !manualCopyValue) {
              scheduleHoverPopoverClose();
            }
          }}
          onFocus={openHoverPopover}
          onBlur={() => {
            if (!pinnedOpen && !manualCopyValue) {
              scheduleHoverPopoverClose();
            }
          }}
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerRelease}
          onPointerCancel={handlePointerRelease}
          onPointerLeave={handlePointerRelease}
          onContextMenu={(event) => {
            event.preventDefault();
            clearLongPressTimer();
            setPinnedOpen(true);
          }}
          onClick={handlePrimaryClick}
        >
          {busy ? (
            <Spinner size="sm" />
          ) : (
            <AppIcon
              name={mode === "copy" ? "content-copy" : "link-variant-plus"}
              className="h-4 w-4"
              aria-hidden
            />
          )}
        </Button>
      </PopoverAnchor>
      <BubblePopoverContent
        anchorElement={triggerRef.current}
        align="start"
        side="top"
        sideOffset={10}
        collisionPadding={12}
        sticky="partial"
        className="w-[min(28rem,calc(100vw-1rem))] rounded-2xl px-4 py-4 shadow-xl"
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onMouseEnter={openHoverPopover}
        onMouseLeave={() => {
          if (!pinnedOpen && !manualCopyValue) {
            scheduleHoverPopoverClose();
          }
        }}
        onFocusCapture={openHoverPopover}
        onBlurCapture={(event) => {
          if (
            !event.currentTarget.contains(event.relatedTarget as Node | null) &&
            !pinnedOpen &&
            !manualCopyValue
          ) {
            scheduleHoverPopoverClose();
          }
        }}
      >
        <div className="space-y-3">
          <div className="space-y-1">
            <p className="text-sm font-semibold text-base-content">
              {popoverTitle}
            </p>
            <p className="text-sm leading-5 text-base-content/65">
              {popoverDescription}
            </p>
          </div>
          {remainingLabel || expiresAtLabel ? (
            <div className="grid gap-2 rounded-2xl border border-base-300/70 bg-base-200/45 px-3 py-2">
              {remainingLabel ? (
                <p className="text-xs font-medium text-base-content/78">
                  {remainingLabel}
                </p>
              ) : null}
              {expiresAtLabel ? (
                <p className="text-xs text-base-content/62">{expiresAtLabel}</p>
              ) : null}
            </div>
          ) : null}
          {manualCopyValue ? (
            <div className="space-y-2">
              <div className="space-y-1">
                <p className="text-sm font-semibold text-base-content">
                  {manualCopyTitle}
                </p>
                <p className="text-sm text-base-content/65">
                  {manualCopyDescription}
                </p>
              </div>
              <div
                ref={manualCopyValueRef}
                role="textbox"
                aria-readonly="true"
                tabIndex={0}
                translate="no"
                spellCheck={false}
                data-lpignore="true"
                data-1p-ignore="true"
                data-form-type="other"
                className="max-h-28 overflow-auto rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-warning/40"
                onFocus={(event) => selectManualCopyText(event.currentTarget)}
                onClick={(event) => selectManualCopyText(event.currentTarget)}
              >
                <span className="break-all">{manualCopyValue}</span>
              </div>
            </div>
          ) : null}
          {mode === "copy" ? (
            <div className="flex justify-end">
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="rounded-full"
                aria-label={regenerateAriaLabel}
                title={regenerateAriaLabel}
                disabled={regenerateDisabled}
                onClick={handleRegenerate}
              >
                <AppIcon
                  name="refresh"
                  className="mr-1.5 h-3.5 w-3.5"
                  aria-hidden
                />
                {regenerateAriaLabel}
              </Button>
            </div>
          ) : null}
        </div>
      </BubblePopoverContent>
    </Popover>
  );
}
