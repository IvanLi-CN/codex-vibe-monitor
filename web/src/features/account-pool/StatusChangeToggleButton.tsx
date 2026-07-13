import { Button } from "../../components/ui/button";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "../shared/AppIcon";

const activeToggleClassName =
  "border-primary/35 bg-primary/10 text-primary hover:border-primary/45 hover:bg-primary/14";

const inactiveToggleClassName =
  "border-base-300/85 bg-base-100 text-base-content/88 hover:border-base-300 hover:bg-base-200/40";

export function StatusChangeToggleButton({
  pressed,
  disabled,
  interactive = true,
  ariaLabel,
  title,
  description,
  iconName,
  activeLabel = "On",
  inactiveLabel = "Off",
  className,
  onPressedChange,
}: {
  pressed: boolean;
  disabled?: boolean;
  interactive?: boolean;
  ariaLabel: string;
  title: string;
  description?: string;
  iconName?: AppIconName;
  activeLabel?: string;
  inactiveLabel?: string;
  className?: string;
  onPressedChange?: (pressed: boolean) => void;
}) {
  const stateLabel = pressed ? activeLabel : inactiveLabel;
  const hasDescription = Boolean(description);
  const content = (
    <>
      <span className="flex min-w-0 flex-1 flex-col">
        <span
          className={cn(
            "grid min-w-0 flex-1 gap-x-3",
            hasDescription
              ? "grid-cols-[2.5rem_minmax(0,1fr)]"
              : "grid-cols-[2.5rem_minmax(0,1fr)] items-center",
          )}
        >
          {iconName ? (
            <AppIcon
              name={iconName}
              className={cn(
                "h-10 w-10 shrink-0 text-current/72",
                hasDescription ? "mt-0.5 self-start" : "self-center",
              )}
              aria-hidden
            />
          ) : null}
          <span className="min-w-0">
            <span
              className={cn(
                "min-w-0 text-sm font-medium leading-5 text-current",
                hasDescription ? "block" : "flex min-h-10 items-center",
              )}
            >
              {title}
            </span>
          </span>
        </span>
        {description ? (
          <span className="block text-xs leading-[1.1rem] text-current/68">{description}</span>
        ) : null}
      </span>
      <span className="sr-only">{stateLabel}</span>
    </>
  );

  const baseClassName = cn(
    "h-auto min-h-[3.75rem] w-full items-start justify-start rounded-xl border px-3 py-2.5 text-left shadow-none transition-colors whitespace-normal",
    pressed ? activeToggleClassName : inactiveToggleClassName,
    className,
  );

  if (!interactive) {
    return (
      <div data-state={pressed ? "on" : "off"} className={cn(baseClassName, "cursor-default")}>
        {content}
      </div>
    );
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      aria-label={ariaLabel}
      aria-pressed={pressed}
      data-state={pressed ? "on" : "off"}
      disabled={disabled}
      onClick={() => onPressedChange?.(!pressed)}
      className={baseClassName}
    >
      {content}
    </Button>
  );
}
