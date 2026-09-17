import { useEffect, useState } from "react";
import { Alert } from "../../components/ui/alert";
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
import { Popover, PopoverArrow, PopoverContent, PopoverTrigger } from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import { AppIcon } from "../../features/shared/AppIcon";
import type { UpstreamAccountDetail, UpstreamAccountDuplicateInfo } from "../../lib/api";
import { upstreamPlanChipRecipe } from "../../lib/upstreamAccountChips";
import { type DuplicateWarningState, formatDateTime } from "./UpstreamAccountCreate.shared";

export function DuplicateWarningPopover({
  duplicateWarning,
  summaryTitle,
  summaryBody,
  openDetailsLabel,
  onOpenDetails,
  side = "top",
}: {
  duplicateWarning: DuplicateWarningState;
  summaryTitle: string;
  summaryBody: string;
  openDetailsLabel: string;
  onOpenDetails: (accountId: number) => void;
  side?: "top" | "right" | "bottom" | "left";
}) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setOpen(true);
  }, []);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex h-5 w-5 shrink-0 items-center justify-center text-warning transition-colors hover:text-warning/90"
          aria-label={summaryTitle}
        >
          <AppIcon name="alert-outline" className="h-5 w-5" aria-hidden />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side={side}
        sideOffset={10}
        onOpenAutoFocus={(event: Event) => event.preventDefault()}
        className="w-[16.5rem] rounded-2xl border border-warning/45 bg-base-100 p-0 shadow-[0_16px_38px_rgba(15,23,42,0.16)]"
      >
        <div className="space-y-3 p-3">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 text-warning">
              <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold leading-5 text-warning">{summaryTitle}</p>
              <p className="text-[11px] leading-5 text-base-content/72">{summaryBody}</p>
            </div>
          </div>
          <div className="flex justify-end">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 rounded-full px-2.5 text-xs font-semibold text-warning hover:bg-warning/10 hover:text-warning"
              onClick={() => {
                setOpen(false);
                onOpenDetails(duplicateWarning.accountId);
              }}
            >
              {openDetailsLabel}
            </Button>
          </div>
        </div>
        <PopoverArrow
          className="fill-base-100 stroke-warning/45 stroke-[0.8]"
          width={16}
          height={8}
        />
      </PopoverContent>
    </Popover>
  );
}

export function DuplicateDetailField({ label, value }: { label: string; value?: string | null }) {
  return (
    <div className="rounded-2xl border border-base-300/70 bg-base-100/82 px-3 py-3">
      <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/45">
        {label}
      </p>
      <p className="mt-1 break-all text-sm text-base-content/82">{value?.trim() ? value : "—"}</p>
    </div>
  );
}

export function accountStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "active") return "success";
  if (status === "syncing") return "warning";
  if (status === "error" || status === "needs_reauth") return "error";
  return "secondary";
}

export function accountKindVariant(kind: string): "secondary" | "success" {
  return kind === "oauth_codex" ? "success" : "secondary";
}

export function DuplicateAccountDetailDialog({
  open,
  detail,
  isLoading,
  onClose,
  title,
  description,
  duplicateLabel,
  closeLabel,
  formatDuplicateReasons,
  statusLabel,
  kindLabel,
  fieldLabels,
}: {
  open: boolean;
  detail: UpstreamAccountDetail | null;
  isLoading: boolean;
  onClose: () => void;
  title: string;
  description: string;
  duplicateLabel: string;
  closeLabel: string;
  formatDuplicateReasons: (duplicateInfo?: UpstreamAccountDuplicateInfo | null) => string;
  statusLabel: (status: string) => string;
  kindLabel: (kind: string) => string;
  fieldLabels: {
    groupName: string;
    email: string;
    accountId: string;
    userId: string;
    lastSuccessSync: string;
  };
}) {
  const planChip = upstreamPlanChipRecipe(detail?.planType ?? null);
  return (
    <Dialog open={open} onOpenChange={(nextOpen: boolean) => !nextOpen && onClose()}>
      <DialogContent className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden p-0 desktop:max-h-[calc(100dvh-2rem)] desktop:w-[min(38rem,calc(100vw-2rem))]">
        <div className="flex shrink-0 items-start justify-between gap-4 border-b border-base-300/70 px-5 py-4">
          <DialogHeader className="min-w-0">
            <DialogTitle className="truncate">{detail?.displayName ?? title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} />
        </div>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-5 pb-[max(env(safe-area-inset-bottom),1rem)]">
          {isLoading ? (
            <div className="flex min-h-44 items-center justify-center">
              <Spinner />
            </div>
          ) : detail ? (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <Chip tone={accountStatusVariant(detail.status)}>{statusLabel(detail.status)}</Chip>
                <Chip tone={accountKindVariant(detail.kind)}>{kindLabel(detail.kind)}</Chip>
                {planChip && detail.planType ? (
                  <Chip tone={planChip.tone}>{detail.planType}</Chip>
                ) : null}
                {detail.duplicateInfo ? <Chip tone="warning">{duplicateLabel}</Chip> : null}
              </div>
              {detail.duplicateInfo ? (
                <Alert variant="warning">
                  <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  <div>
                    <p className="font-semibold text-warning">{duplicateLabel}</p>
                    <p className="mt-1 text-sm text-warning/90">
                      {`命中：${formatDuplicateReasons(detail.duplicateInfo)}。关联账号 ID：${detail.duplicateInfo.peerAccountIds.join(", ") || "—"}。`}
                    </p>
                  </div>
                </Alert>
              ) : null}
              <div className="grid gap-3 md:grid-cols-2">
                <DuplicateDetailField
                  label={fieldLabels.groupName}
                  value={detail.groupName ?? ""}
                />
                <DuplicateDetailField label={fieldLabels.email} value={detail.email ?? ""} />
                <DuplicateDetailField
                  label={fieldLabels.accountId}
                  value={detail.chatgptAccountId ?? detail.maskedApiKey ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.userId}
                  value={detail.chatgptUserId ?? ""}
                />
                <DuplicateDetailField
                  label={fieldLabels.lastSuccessSync}
                  value={formatDateTime(detail.lastSuccessfulSyncAt)}
                />
              </div>
            </>
          ) : (
            <p className="text-sm text-base-content/65">{description}</p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
