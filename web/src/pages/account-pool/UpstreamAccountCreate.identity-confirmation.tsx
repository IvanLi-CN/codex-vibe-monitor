import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Spinner } from "../../components/ui/spinner";
import { AppIcon } from "../../features/shared/AppIcon";
import type { LoginSessionStatusResponse } from "../../lib/api";
import { cn } from "../../lib/utils";

type OAuthIdentityConfirmationAlertProps = {
  identityConfirmation: NonNullable<LoginSessionStatusResponse["identityConfirmation"]>;
  fallbackDisplayName: string;
  confirmBusy: boolean;
  confirmDisabled: boolean;
  onConfirm: () => void;
  t: (key: string, values?: Record<string, string | number>) => string;
  compact?: boolean;
};

export function OAuthIdentityConfirmationAlert({
  identityConfirmation,
  fallbackDisplayName,
  confirmBusy,
  confirmDisabled,
  onConfirm,
  t,
  compact = false,
}: OAuthIdentityConfirmationAlertProps) {
  const unavailable = t("accountPool.upstreamAccounts.identityUnavailable");
  const currentName = identityConfirmation.current.displayName ?? fallbackDisplayName;
  const incomingIdentityDetail = [
    identityConfirmation.incoming.chatgptAccountId,
    identityConfirmation.incoming.chatgptUserId,
    identityConfirmation.incoming.planType,
  ]
    .filter(Boolean)
    .join(" / ");

  return (
    <Alert
      variant="warning"
      className="grid gap-3 border-warning/45 bg-warning/10 text-base-content"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div className="flex min-w-0 items-start gap-2">
          <AppIcon
            name="alert-outline"
            className="mt-0.5 h-4 w-4 shrink-0 text-warning"
            aria-hidden
          />
          <div className="min-w-0">
            <p className="text-sm font-semibold">
              {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.title")}
            </p>
            <p className="text-xs leading-5 text-base-content/72">
              {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.body")}
            </p>
          </div>
        </div>
        <Button
          type="button"
          variant="default"
          size={compact ? "sm" : "default"}
          className="shrink-0"
          onClick={onConfirm}
          disabled={confirmDisabled}
        >
          {confirmBusy ? (
            <Spinner size="sm" className="mr-2" />
          ) : (
            <AppIcon name="shield-key-outline" className="mr-2 h-4 w-4" aria-hidden />
          )}
          {t("accountPool.upstreamAccounts.batchOauth.actions.confirmIdentityOverwrite")}
        </Button>
      </div>

      <div className="grid gap-2 text-xs md:grid-cols-2">
        <div className="rounded-lg border border-warning/25 bg-base-100/70 px-3 py-2">
          <p className="font-semibold">
            {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.current")}
          </p>
          <p className="mt-1 break-words">{currentName || unavailable}</p>
          <p className="break-words text-base-content/68">
            {identityConfirmation.current.email ?? unavailable}
          </p>
        </div>
        <div className="rounded-lg border border-warning/25 bg-base-100/70 px-3 py-2">
          <p className="font-semibold">
            {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.incoming")}
          </p>
          <p className="mt-1 break-words">
            {identityConfirmation.incoming.email ??
              identityConfirmation.incoming.verifiedEmail ??
              unavailable}
          </p>
          <p className="break-words text-base-content/68">
            {incomingIdentityDetail || unavailable}
          </p>
        </div>
      </div>

      <div
        className={cn(
          "grid gap-2 rounded-lg border border-base-300/65 bg-base-100/78 px-3 py-2 text-xs text-base-content/72",
          compact ? "md:grid-cols-1" : "md:grid-cols-2",
        )}
      >
        <p>
          <span className="font-semibold text-base-content">
            {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.willUpdate")}
          </span>{" "}
          {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.willUpdateDetail")}
        </p>
        <p>
          <span className="font-semibold text-base-content">
            {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.willKeep")}
          </span>{" "}
          {t("accountPool.upstreamAccounts.batchOauth.identityConfirmation.willKeepDetail")}
        </p>
      </div>
    </Alert>
  );
}
