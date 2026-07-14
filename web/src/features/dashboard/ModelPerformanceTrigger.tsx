import { type KeyboardEvent, type ReactNode, useEffect, useState } from "react";
import { Dialog, DialogCloseIcon, DialogContent, DialogTitle } from "../../components/ui/dialog";
import { Tooltip } from "../../components/ui/tooltip";
import { useTranslation } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { cn } from "../../lib/utils";
import { ModelPerformanceDetails } from "./ModelPerformanceDetails";

const COMPACT_MEDIA_QUERY = "(max-width: 767px)";

function useCompactPresentation() {
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const media = window.matchMedia(COMPACT_MEDIA_QUERY);
    const sync = () => setCompact(media.matches);
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, []);
  return compact;
}

export function ModelPerformanceTrigger({
  title,
  ariaLabel,
  performance,
  children,
  className,
  contentClassName,
}: {
  title: string;
  ariaLabel: string;
  performance: ModelPerformance;
  children: ReactNode;
  className?: string;
  contentClassName?: string;
}) {
  const { t } = useTranslation();
  const compact = useCompactPresentation();
  const [open, setOpen] = useState(false);
  const details = <ModelPerformanceDetails title={title} performance={performance} />;
  const handleDesktopKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    event.currentTarget.click();
  };

  if (!compact) {
    return (
      <Tooltip
        content={details}
        clickToOpen
        side="bottom"
        sideOffset={8}
        className={className}
        contentClassName={cn(
          "w-[min(48rem,calc(100vw-1rem))] max-w-[min(48rem,calc(100vw-1rem))] px-3.5 py-3",
          contentClassName,
        )}
        triggerProps={{
          role: "button",
          tabIndex: 0,
          "aria-label": ariaLabel,
          onKeyDown: handleDesktopKeyDown,
        }}
      >
        {children}
      </Tooltip>
    );
  }

  return (
    <>
      <button
        type="button"
        className={cn("inline-flex min-w-0 cursor-pointer text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary", className)}
        aria-label={ariaLabel}
        onClick={() => setOpen(true)}
      >
        {children}
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[min(100dvh-1rem,46rem)] overflow-hidden">
          <div className="flex items-start gap-3 border-b border-base-300/70 px-4 py-4 sm:px-5">
            <DialogTitle className="min-w-0 flex-1 text-lg">{title}</DialogTitle>
            <DialogCloseIcon aria-label={t("dashboard.modelPerformance.close")} />
          </div>
          <div className="max-h-[calc(min(100dvh-1rem,46rem)-4.5rem)] overflow-y-auto px-4 py-4 sm:px-5">
            <ModelPerformanceDetails title={title} performance={performance} presentation="drawer" />
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
