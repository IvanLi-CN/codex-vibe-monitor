import { Chip } from "../../components/ui/chip";
import type { UpstreamAccountActivityAccount } from "../../lib/api";
import { cn } from "../../lib/utils";
import { FALLBACK_CELL } from "../invocations/invocation-details-shared";
import type {
  AdaptiveDisplayValueSpec,
  AdaptiveMetricPresentation,
} from "../shared/adaptiveMetricValueSpec";
import {
  buildAdaptiveCurrencyAmountTextSpec,
  buildAdaptiveCurrencyTextSpec,
  buildAdaptiveDurationTextSpec,
  buildAdaptiveNumberTextSpec,
  buildAdaptivePercentTextSpec,
  buildAdaptiveTextSpec,
} from "../shared/adaptiveMetricValueSpec";
import { ACCOUNT_HEADER_BADGE_CLASS_NAME } from "./DashboardWorkingConversationsSection";

export function formatAccountPercentValue(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    style: "percent",
    maximumFractionDigits: 1,
  }).format(value);
}

type InvocationSummaryTone = "primary" | "warning" | "error";

type InvocationSummaryField = {
  label: "Hit" | "Token" | "$";
  text: string;
  tone: InvocationSummaryTone;
  testId: string;
};

const INVOCATION_SUMMARY_TONE_CLASSNAMES: Record<InvocationSummaryTone, string> = {
  primary: "text-base-content/74",
  warning: "text-warning",
  error: "text-error",
};

function resolveInvocationHitTone(hitRate: number | null | undefined): InvocationSummaryTone {
  if (hitRate == null || !Number.isFinite(hitRate)) return "primary";
  if (hitRate < 0.5) return "error";
  if (hitRate < 0.9) return "warning";
  return "primary";
}

function resolveInvocationCostTone(cost: number | null | undefined): InvocationSummaryTone {
  if (cost == null || !Number.isFinite(cost)) return "primary";
  if (cost > 0.5) return "error";
  if (cost > 0.1) return "warning";
  return "primary";
}

function formatCompactInvocationCostValue(costValue: string) {
  return costValue.startsWith("US$") ? `$${costValue.slice(3)}` : costValue;
}

export function buildInvocationSummaryFields({
  cacheHitRate,
  totalTokensValue,
  cost,
  costValue,
  localeTag,
  testIdPrefix,
}: {
  cacheHitRate: number | null | undefined;
  totalTokensValue: string;
  cost: number | null | undefined;
  costValue: string;
  localeTag: string;
  testIdPrefix: string;
}): InvocationSummaryField[] {
  return [
    {
      label: "Hit",
      text: `Hit ${formatAccountPercentValue(cacheHitRate, localeTag)}`,
      tone: resolveInvocationHitTone(cacheHitRate),
      testId: `${testIdPrefix}-hit`,
    },
    {
      label: "Token",
      text: `Token ${totalTokensValue}`,
      tone: "primary",
      testId: `${testIdPrefix}-token`,
    },
    {
      label: "$",
      text: formatCompactInvocationCostValue(costValue),
      tone: resolveInvocationCostTone(cost),
      testId: `${testIdPrefix}-cost`,
    },
  ];
}

export function renderInvocationSummaryFields(
  fields: InvocationSummaryField[],
  separatorClassName = "text-base-content/28",
) {
  return fields.flatMap((field, index) => {
    const content = (
      <span
        key={field.testId}
        data-testid={field.testId}
        data-summary-label={field.label}
        data-summary-tone={field.tone}
        className={INVOCATION_SUMMARY_TONE_CLASSNAMES[field.tone]}
      >
        {field.text}
      </span>
    );

    if (index === 0) return [content];

    return [
      <span key={`${field.testId}-separator`} className={separatorClassName}>
        ·
      </span>,
      content,
    ];
  });
}

export function formatAccountNumberValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 0,
) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    maximumFractionDigits,
  }).format(value);
}

export function formatAccountCurrencyValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 2,
) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);
}

export function formatAccountDurationValue(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  const abs = Math.abs(value);
  if (abs >= 1000) {
    const seconds = value / 1000;
    const maximumFractionDigits = abs >= 100_000 ? 1 : 2;
    return `${formatAccountNumberValue(seconds, localeTag, maximumFractionDigits)} s`;
  }
  return `${formatAccountNumberValue(value, localeTag, abs >= 100 ? 0 : 1)} ms`;
}

export function finiteNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function accountCostShare(
  numerator: number | null | undefined,
  total: number | null | undefined,
) {
  const resolvedNumerator = finiteNumber(numerator);
  const resolvedTotal = finiteNumber(total);
  if (resolvedNumerator == null || resolvedTotal == null) return null;
  if (resolvedTotal <= 0) return resolvedNumerator <= 0 ? 0 : null;
  return Math.max(0, resolvedNumerator) / resolvedTotal;
}

export function invocationCacheHitRate({
  cacheInputTokens,
  totalTokens,
}: {
  cacheInputTokens?: number | null;
  totalTokens?: number | null;
}) {
  const resolvedCacheInputTokens = finiteNumber(cacheInputTokens);
  const resolvedTotalTokens = finiteNumber(totalTokens);
  if (resolvedCacheInputTokens == null || resolvedTotalTokens == null) return null;
  if (resolvedTotalTokens <= 0) return resolvedCacheInputTokens <= 0 ? 0 : null;
  return Math.max(0, resolvedCacheInputTokens) / resolvedTotalTokens;
}

export type AccountMetricTone =
  | "neutral"
  | "primary"
  | "secondary"
  | "success"
  | "warning"
  | "error"
  | "info";

export type AccountMetricDetailRow = {
  label: string;
  value: string;
  tone?: AccountMetricTone;
};

export type AccountMetricDetailSection = {
  title: string;
  rows: AccountMetricDetailRow[];
};

export type AccountDisplayValue = {
  spec: AdaptiveDisplayValueSpec;
  fullText: string;
  ariaText: string;
};

export const ACCOUNT_STAT_CARD_PRESENTATION: AdaptiveMetricPresentation = "account-stat-card";

function toAccountDisplayValue(spec: AdaptiveDisplayValueSpec): AccountDisplayValue {
  return {
    spec,
    fullText: spec.fullValue,
    ariaText: spec.fullValue,
  };
}

export function buildAccountPercentDisplayValue(
  value: number | null | undefined,
  localeTag: string,
) {
  return toAccountDisplayValue(
    buildAdaptivePercentTextSpec(value ?? null, localeTag, {
      maximumFractionDigits: 1,
    }),
  );
}

export function buildAccountNumberDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 0,
  presentation: AdaptiveMetricPresentation = "default",
) {
  return toAccountDisplayValue(
    buildAdaptiveNumberTextSpec(value ?? null, localeTag, maximumFractionDigits, { presentation }),
  );
}

export function buildAccountCurrencyDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  presentation: AdaptiveMetricPresentation = "default",
) {
  if (value == null || !Number.isFinite(value)) {
    return toAccountDisplayValue(
      buildAdaptiveTextSpec(FALLBACK_CELL, [
        { key: "placeholder", value: FALLBACK_CELL, priority: 0 },
      ]),
    );
  }

  return toAccountDisplayValue(
    buildAdaptiveCurrencyTextSpec(value, localeTag, {
      maximumFractionDigits: 2,
      minimumFractionDigits: 2,
      presentation,
    }),
  );
}

export function buildAccountCurrencyAmountDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 2,
  presentation: AdaptiveMetricPresentation = "default",
) {
  return toAccountDisplayValue(
    buildAdaptiveCurrencyAmountTextSpec(value ?? null, localeTag, {
      maximumFractionDigits,
      minimumFractionDigits: maximumFractionDigits,
      presentation,
    }),
  );
}

export function buildAccountDurationDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  presentation: AdaptiveMetricPresentation = "default",
) {
  return toAccountDisplayValue(
    buildAdaptiveDurationTextSpec(value ?? null, localeTag, { presentation }),
  );
}

export const ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES: Record<AccountMetricTone, string> = {
  neutral: "text-base-content",
  primary: "text-primary",
  secondary: "text-secondary",
  success: "text-success",
  warning: "text-accent",
  error: "text-error",
  info: "text-info",
};

export const ACCOUNT_METRIC_DOT_TONE_CLASSNAMES: Record<AccountMetricTone, string> = {
  neutral: "bg-base-content/38",
  primary: "bg-primary/90",
  secondary: "bg-secondary/90",
  success: "bg-success/90",
  warning: "bg-warning/90",
  error: "bg-error/90",
  info: "bg-info/90",
};

export const ACCOUNT_INLINE_METRIC_ICON_AND_GAP_PX = 26;
export const ACCOUNT_INLINE_TPM_SPLIT_VALUE_WIDTH_BUDGET_CH = 6;

type AccountAttentionChip = {
  key: string;
  label: string;
  tone: "warning" | "error" | "info";
  title?: string;
};

function normalizeStatusToken(value: string | null | undefined) {
  return value?.trim().toLowerCase().replace(/-/g, "_") ?? "";
}

function resolveAccountAttentionChips(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
): AccountAttentionChip[] {
  const labels = {
    disabled: locale === "zh" ? "禁用" : "Disabled",
    syncing: locale === "zh" ? "同步中" : "Syncing",
    upstreamRejected: locale === "zh" ? "上游拒绝" : "Rejected",
    upstreamUnavailable: locale === "zh" ? "上游不可达" : "Unavailable",
    needsReauth: locale === "zh" ? "需重登" : "Reauth",
    rateLimited: locale === "zh" ? "限流" : "Limited",
    degraded: locale === "zh" ? "降级" : "Degraded",
    otherError: locale === "zh" ? "其它异常" : "Other error",
    unavailable: locale === "zh" ? "不可用" : "Unavailable",
  };
  const detail = account.lastActionReasonMessage || account.lastError || undefined;
  const badges: AccountAttentionChip[] = [];
  const seen = new Set<string>();
  const add = (badge: AccountAttentionChip) => {
    if (seen.has(badge.key)) return;
    seen.add(badge.key);
    badges.push(badge);
  };
  const enableStatus = normalizeStatusToken(account.enableStatus);
  const displayStatus = normalizeStatusToken(account.displayStatus);
  const healthStatus = normalizeStatusToken(account.healthStatus);
  const syncState = normalizeStatusToken(account.syncState);
  const workStatus = normalizeStatusToken(account.workStatus);

  if (account.enabled === false || enableStatus === "disabled" || displayStatus === "disabled") {
    add({
      key: "disabled",
      label: labels.disabled,
      tone: "warning",
      title: detail,
    });
  }
  if (syncState === "syncing" || displayStatus === "syncing") {
    add({ key: "syncing", label: labels.syncing, tone: "info", title: detail });
  }

  const healthSource = healthStatus || displayStatus;
  if (healthSource === "upstream_rejected") {
    add({
      key: "upstream_rejected",
      label: labels.upstreamRejected,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "upstream_unavailable") {
    add({
      key: "upstream_unavailable",
      label: labels.upstreamUnavailable,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "needs_reauth") {
    add({
      key: "needs_reauth",
      label: labels.needsReauth,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "error_other" || healthSource === "error") {
    add({
      key: "error_other",
      label: labels.otherError,
      tone: "error",
      title: detail,
    });
  }

  if (workStatus === "rate_limited" || displayStatus === "rate_limited") {
    add({
      key: "rate_limited",
      label: labels.rateLimited,
      tone: "warning",
      title: detail,
    });
  }
  if (workStatus === "degraded" || displayStatus === "degraded") {
    add({
      key: "degraded",
      label: labels.degraded,
      tone: "warning",
      title: detail,
    });
  }
  if (workStatus === "unavailable" && badges.length === 0) {
    add({
      key: "unavailable",
      label: labels.unavailable,
      tone: "error",
      title: detail,
    });
  }
  return badges;
}

export function AccountAttentionChips({
  account,
  locale,
  clickable,
  onClick,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  clickable: boolean;
  onClick?: () => void;
}) {
  const badges = resolveAccountAttentionChips(account, locale);
  if (badges.length === 0) return null;
  const title = badges.map((badge) => badge.label).join(" · ");
  const openLabel = locale === "zh" ? "打开账号健康事件" : "Open health events";
  return (
    <fieldset
      data-testid="dashboard-upstream-account-attention-badges"
      className="inline-flex min-h-6 max-w-full flex-wrap items-center gap-1.5"
      title={title}
      aria-label={title}
    >
      {badges.map((badge) =>
        clickable ? (
          <Chip
            asChild
            size="header"
            tone={badge.tone}
            key={badge.key}
            data-testid="dashboard-upstream-account-attention-badge"
            title={badge.title ?? badge.label}
            aria-label={`${badge.label} · ${openLabel}`}
            className={cn(
              ACCOUNT_HEADER_BADGE_CLASS_NAME,
              "appearance-none transition-opacity duration-200 hover:opacity-80",
            )}
          >
            <button
              type="button"
              className="appearance-none"
              onClick={(event) => {
                event.stopPropagation();
                onClick?.();
              }}
              onKeyDown={(event) => event.stopPropagation()}
            >
              {badge.label}
            </button>
          </Chip>
        ) : (
          <Chip
            size="header"
            tone={badge.tone}
            key={badge.key}
            data-testid="dashboard-upstream-account-attention-badge"
            title={badge.title ?? badge.label}
            className={ACCOUNT_HEADER_BADGE_CLASS_NAME}
          >
            {badge.label}
          </Chip>
        ),
      )}
    </fieldset>
  );
}
