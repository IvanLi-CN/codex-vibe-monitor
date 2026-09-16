import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
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
  laterButton: string;
  manualButton: string;
  installedButton: string;
  switcherAria: string;
  closeButton: string;
  closeAria: string;
  shellReady: string;
  shellPending: string;
  offlineChip: string;
  promptTitle: string;
  promptDescription: string;
  promptHint: string;
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
  open: boolean;
  onOpenChange: (open: boolean) => void;
  shellReady: boolean;
  isOffline: boolean;
  labels: PwaInstallControlLabels;
  onPromptInstall?: () => Promise<void> | void;
  canPromptInstall?: boolean;
}

function PwaInstallDialogStatus({
  shellReady,
  isOffline,
  labels,
}: Pick<PwaInstallControlProps, "shellReady" | "isOffline" | "labels">) {
  return (
    <div className="flex flex-wrap gap-2">
      <Chip size="default" tone="secondary" className="gap-2 px-3 py-1 text-xs">
        <AppIcon
          name={shellReady ? "check-bold" : "timer-refresh-outline"}
          className="h-4 w-4"
          aria-hidden
        />
        {shellReady ? labels.shellReady : labels.shellPending}
      </Chip>
      {isOffline ? (
        <Chip size="default" tone="warning" className="gap-2 px-3 py-1 text-xs font-medium">
          <AppIcon name="alert-circle-outline" className="h-4 w-4" aria-hidden />
          {labels.offlineChip}
        </Chip>
      ) : null}
    </div>
  );
}

function PwaInstallDialogContent({
  mode,
  shellReady,
  isOffline,
  labels,
  onOpenChange,
  onPromptInstall,
  canPromptInstall,
}: Omit<PwaInstallControlProps, "open">) {
  const iconName = mode === "installed" ? "check-circle-outline" : "content-save-plus-outline";
  const title =
    mode === "prompt"
      ? labels.promptTitle
      : mode === "installed"
        ? labels.installedTitle
        : labels.manualTitle;
  const description =
    mode === "prompt"
      ? labels.promptDescription
      : mode === "installed"
        ? labels.installedDescription
        : labels.manualDescription;
  const handlePromptInstall = async () => {
    await onPromptInstall?.();
    onOpenChange(false);
  };

  return (
    <DialogContent
      mobileLayout="centered"
      className="w-[min(32rem,calc(100vw-1rem))] overflow-hidden p-0 desktop:w-[min(32rem,calc(100vw-2rem))]"
      data-testid="pwa-install-dialog"
      data-install-mode={mode}
    >
      <div className="surface-panel-body gap-5 p-5">
        <div className="flex items-start gap-3">
          <div className="inline-flex h-11 w-11 flex-shrink-0 items-center justify-center rounded-full border border-primary/25 bg-primary/10 text-primary">
            <AppIcon name={iconName} className="h-6 w-6" aria-hidden />
          </div>
          <DialogHeader className="min-w-0 flex-1">
            <DialogTitle className="text-lg">{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={labels.closeAria} />
        </div>

        <PwaInstallDialogStatus shellReady={shellReady} isOffline={isOffline} labels={labels} />

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
            {mode === "prompt" ? labels.promptHint : labels.installedHint}
          </div>
        )}

        <div className="flex justify-end gap-3">
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid="pwa-install-close"
          >
            {mode === "prompt" ? labels.laterButton : labels.closeButton}
          </Button>
          {mode === "prompt" && canPromptInstall ? (
            <Button onClick={() => void handlePromptInstall()} data-testid="pwa-install-confirm">
              {labels.promptButton}
            </Button>
          ) : null}
        </div>
      </div>
    </DialogContent>
  );
}
export function PwaInstallControl({
  mode,
  open,
  onOpenChange,
  shellReady,
  isOffline,
  labels,
  onPromptInstall,
  canPromptInstall = true,
}: PwaInstallControlProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <PwaInstallDialogContent
        mode={mode}
        shellReady={shellReady}
        isOffline={isOffline}
        labels={labels}
        onOpenChange={onOpenChange}
        onPromptInstall={onPromptInstall}
        canPromptInstall={canPromptInstall}
      />
    </Dialog>
  );
}

export default PwaInstallControl;
