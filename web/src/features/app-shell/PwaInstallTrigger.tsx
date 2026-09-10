import type { PwaInstallMode } from "../../hooks/usePwaRuntime";
import { AppIcon } from "../shared/AppIcon";

export interface PwaInstallTriggerProps {
  mode: Extract<PwaInstallMode, "prompt" | "manual-ios">;
  label: string;
  ariaLabel: string;
  compact?: boolean;
  onClick: () => void;
}

export function PwaInstallTrigger({
  mode,
  label,
  ariaLabel,
  compact = false,
  onClick,
}: PwaInstallTriggerProps) {
  return (
    <button
      type="button"
      className="control-pill"
      onClick={onClick}
      aria-label={ariaLabel}
      title={compact ? label : undefined}
      data-testid="pwa-install-trigger"
      data-install-mode={mode}
    >
      <AppIcon
        name="content-save-plus-outline"
        className="h-[18px] w-[18px] text-primary"
        aria-hidden
      />
      <span className={compact ? "sr-only" : "hidden md:inline"}>{label}</span>
    </button>
  );
}

export default PwaInstallTrigger;
