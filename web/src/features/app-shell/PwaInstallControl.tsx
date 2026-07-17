import { useMemo, useState } from "react";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import type { PwaInstallMode } from "../../hooks/usePwaRuntime";
import { AppIcon } from "../shared/AppIcon";

export interface PwaInstallControlLabels {
  promptButton: string;
  manualButton: string;
  installedButton: string;
  switcherAria: string;
  closeButton: string;
  closeAria: string;
  shellReady: string;
  shellPending: string;
  offlineChip: string;
  manualTitle: string;
  manualDescription: string;
  manualStepOpenShare: string;
  manualStepAdd: string;
  manualStepConfirm: string;
  installedTitle: string;
  installedDescription: string;
  installedHint: string;
}

export interface PwaInstallControlProps {
  mode: Extract<PwaInstallMode, "prompt" | "manual-ios" | "installed">;
  shellReady: boolean;
  isOffline: boolean;
  labels: PwaInstallControlLabels;
  onPromptInstall?: () => Promise<void> | void;
}

export function PwaInstallControl({
  mode,
  shellReady,
  isOffline,
  labels,
  onPromptInstall,
}: PwaInstallControlProps) {
  const [dialogOpen, setDialogOpen] = useState(false);

  const buttonLabel = useMemo(() => {
    switch (mode) {
      case "manual-ios":
        return labels.manualButton;
      case "installed":
        return labels.installedButton;
      default:
        return labels.promptButton;
    }
  }, [labels.installedButton, labels.manualButton, labels.promptButton, mode]);

  const iconName = mode === "installed" ? "check-circle-outline" : "content-save-plus-outline";

  const handleClick = async () => {
    if (mode === "prompt") {
      await onPromptInstall?.();
      return;
    }
    setDialogOpen(true);
  };

  return (
    <>
      <button
        type="button"
        className="control-pill min-w-0"
        aria-label={labels.switcherAria}
        title={buttonLabel}
        onClick={() => {
          void handleClick();
        }}
        data-testid="pwa-install-control"
        data-install-mode={mode}
      >
        <AppIcon name={iconName} className="h-[18px] w-[18px] text-primary" aria-hidden />
        <span className="hidden md:inline">{buttonLabel}</span>
      </button>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="w-[min(32rem,calc(100vw-1rem))] overflow-hidden p-0">
          <div className="surface-panel-body gap-5 p-5">
            <div className="flex items-start gap-3">
              <div className="inline-flex h-11 w-11 flex-shrink-0 items-center justify-center rounded-full border border-primary/25 bg-primary/10 text-primary">
                <AppIcon
                  name={mode === "installed" ? "check-circle-outline" : "content-save-plus-outline"}
                  className="h-6 w-6"
                  aria-hidden
                />
              </div>
              <DialogHeader className="min-w-0 flex-1">
                <DialogTitle className="text-lg">
                  {mode === "installed" ? labels.installedTitle : labels.manualTitle}
                </DialogTitle>
                <DialogDescription>
                  {mode === "installed" ? labels.installedDescription : labels.manualDescription}
                </DialogDescription>
              </DialogHeader>
              <DialogCloseIcon aria-label={labels.closeAria} />
            </div>

            <div className="flex flex-wrap gap-2">
              <span className="inline-flex items-center gap-2 rounded-full border border-base-300/75 bg-base-100/82 px-3 py-1 text-xs font-medium text-base-content/78">
                <AppIcon
                  name={shellReady ? "check-bold" : "timer-refresh-outline"}
                  className="h-4 w-4"
                  aria-hidden
                />
                {shellReady ? labels.shellReady : labels.shellPending}
              </span>
              {isOffline ? (
                <span className="inline-flex items-center gap-2 rounded-full border border-warning/45 bg-warning/12 px-3 py-1 text-xs font-medium text-warning-content">
                  <AppIcon name="alert-circle-outline" className="h-4 w-4" aria-hidden />
                  {labels.offlineChip}
                </span>
              ) : null}
            </div>

            {mode === "manual-ios" ? (
              <ol className="space-y-3 rounded-2xl border border-base-300/80 bg-base-100/86 p-4 text-sm text-base-content/80">
                <li className="flex items-start gap-3">
                  <span className="inline-flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-primary/10 font-semibold text-primary">
                    1
                  </span>
                  <span>{labels.manualStepOpenShare}</span>
                </li>
                <li className="flex items-start gap-3">
                  <span className="inline-flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-primary/10 font-semibold text-primary">
                    2
                  </span>
                  <span>{labels.manualStepAdd}</span>
                </li>
                <li className="flex items-start gap-3">
                  <span className="inline-flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-primary/10 font-semibold text-primary">
                    3
                  </span>
                  <span>{labels.manualStepConfirm}</span>
                </li>
              </ol>
            ) : (
              <div className="rounded-2xl border border-base-300/80 bg-base-100/86 p-4 text-sm text-base-content/78">
                {labels.installedHint}
              </div>
            )}

            <div className="flex justify-end">
              <Button variant="secondary" onClick={() => setDialogOpen(false)}>
                {labels.closeButton}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

export default PwaInstallControl;
