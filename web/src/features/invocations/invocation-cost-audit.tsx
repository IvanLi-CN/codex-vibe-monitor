import type { ReactNode } from "react";
import { Tooltip } from "../../components/ui/tooltip";
import type { TranslationKey } from "../../i18n";
import type { InvocationCostAudit } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";

type Translator = (key: TranslationKey, values?: Record<string, string | number>) => string;

export const INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD = 0.000001;

export function resolveInvocationCostAuditDisplay(
  audit: InvocationCostAudit | null | undefined,
  recordedFallback?: number | null,
) {
  return {
    recordedTotal: audit?.recorded?.total ?? recordedFallback ?? null,
    localTotal: audit?.local?.total ?? null,
    mismatch: audit?.mismatch === true,
    reason: audit?.reason ?? null,
    absoluteDiffUsd: audit?.absoluteDiffUsd ?? null,
    recordedPriceVersion: audit?.recordedPriceVersion ?? null,
    localPriceVersion: audit?.localPriceVersion ?? null,
  };
}

export function hasInvocationCostBucketMismatch(
  recorded: number | null | undefined,
  local: number | null | undefined,
) {
  if (recorded == null || local == null) return false;
  return Math.abs(recorded - local) > INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD;
}

function resolveReasonKey(reason: string | null | undefined): TranslationKey {
  switch (reason) {
    case "price_version_changed":
      return "records.costAudit.reason.priceVersionChanged";
    case "recorded_cost_missing":
      return "records.costAudit.reason.recordedCostMissing";
    case "recorded_price_version_missing":
      return "records.costAudit.reason.recordedPriceVersionMissing";
    case "pricing_mode_unknown":
      return "records.costAudit.reason.pricingModeUnknown";
    case "usage_missing":
      return "records.costAudit.reason.usageMissing";
    case "model_missing":
      return "records.costAudit.reason.modelMissing";
    case "model_pricing_missing":
      return "records.costAudit.reason.modelPricingMissing";
    default:
      return "records.costAudit.reason.totalMismatch";
  }
}

function CostAuditTooltipContent({
  audit,
  t,
  formatCurrency,
}: {
  audit: InvocationCostAudit;
  t: Translator;
  formatCurrency: (value: number | null | undefined) => string;
}) {
  const display = resolveInvocationCostAuditDisplay(audit);
  const reasonKey = resolveReasonKey(display.reason);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-warning">
        <AppIcon name="alert-circle-outline" className="h-4 w-4 shrink-0" aria-hidden />
        <span className="font-semibold">{t("records.costAudit.tooltip.title")}</span>
      </div>
      <p className="leading-5 text-base-content/82">{t(reasonKey)}</p>
      <dl className="grid grid-cols-[minmax(0,5.5rem)_minmax(0,1fr)] items-start gap-x-3 gap-y-1.5 font-mono text-[11px] leading-5 text-base-content/78">
        <dt className="min-w-0">{t("records.costAudit.tooltip.recorded")}</dt>
        <dd className="min-w-0 text-right">{formatCurrency(display.recordedTotal)}</dd>
        <dt className="min-w-0">{t("records.costAudit.tooltip.local")}</dt>
        <dd className="min-w-0 text-right">{formatCurrency(display.localTotal)}</dd>
        <dt className="min-w-0">{t("records.costAudit.tooltip.diff")}</dt>
        <dd className="min-w-0 text-right">{formatCurrency(display.absoluteDiffUsd)}</dd>
      </dl>
      {(display.recordedPriceVersion || display.localPriceVersion) && (
        <dl className="grid grid-cols-[minmax(0,5.5rem)_minmax(0,1fr)] items-start gap-x-3 gap-y-1.5 font-mono text-[11px] leading-5 text-base-content/62">
          <dt className="min-w-0">{t("records.costAudit.tooltip.recordedVersion")}</dt>
          <dd className="min-w-0 break-all text-right">{display.recordedPriceVersion ?? "—"}</dd>
          <dt className="min-w-0">{t("records.costAudit.tooltip.localVersion")}</dt>
          <dd className="min-w-0 break-all text-right">{display.localPriceVersion ?? "—"}</dd>
        </dl>
      )}
    </div>
  );
}

export function renderInvocationCostAuditWarning(
  audit: InvocationCostAudit | null | undefined,
  t: Translator,
  formatCurrency: (value: number | null | undefined) => string,
  options?: {
    className?: string;
    iconClassName?: string;
    testId?: string;
    triggerLabel?: string;
  },
): ReactNode {
  if (!audit?.mismatch) return null;
  return (
    <Tooltip
      side="top"
      sideOffset={8}
      className={cn("inline-flex", options?.className)}
      contentClassName="isolate z-[90] w-[22rem] max-w-[min(22rem,calc(100vw-1rem))] border-warning/45 bg-base-100"
      content={<CostAuditTooltipContent audit={audit} t={t} formatCurrency={formatCurrency} />}
      triggerProps={{
        role: "button",
        tabIndex: 0,
        "aria-label": options?.triggerLabel ?? t("records.costAudit.warningAria"),
      }}
    >
      <span
        data-testid={options?.testId}
        className="inline-flex h-4 w-4 items-center justify-center text-warning"
      >
        <AppIcon
          name="alert-circle-outline"
          className={cn("h-4 w-4", options?.iconClassName)}
          aria-hidden
        />
      </span>
    </Tooltip>
  );
}
