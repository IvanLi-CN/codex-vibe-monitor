import type { useTranslation } from "../../i18n";
import type { UpstreamAccountActivityAccount } from "../../lib/api";
import { FALLBACK_CELL } from "../invocations/invocation-details-shared";
import {
  ACCOUNT_STAT_CARD_PRESENTATION,
  type AccountMetricDetailRow,
  type AccountMetricDetailSection,
  accountCostShare,
  buildAccountCurrencyAmountDisplayValue,
  buildAccountCurrencyDisplayValue,
  buildAccountDurationDisplayValue,
  buildAccountNumberDisplayValue,
  buildAccountPercentDisplayValue,
  finiteNumber,
  formatAccountCurrencyValue,
  formatAccountDurationValue,
  formatAccountNumberValue,
  formatAccountPercentValue,
} from "./DashboardWorkingAccountMetrics";

type Translate = ReturnType<typeof useTranslation>["t"];

function buildRequestSummarySegments(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
) {
  return [
    {
      label: locale === "zh" ? "成功" : "Success",
      value: buildAccountNumberDisplayValue(
        account.successCount,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "success" as const,
    },
    {
      label: locale === "zh" ? "失败" : "Failure",
      value: buildAccountNumberDisplayValue(
        account.failureCount,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "error" as const,
    },
    {
      label: locale === "zh" ? "其他" : "Other",
      value: buildAccountNumberDisplayValue(
        Math.max(0, account.nonSuccessCount - account.failureCount),
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "warning" as const,
    },
  ];
}

function buildCostSummarySegments(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
) {
  return [
    {
      label: locale === "zh" ? "失败" : "Failure",
      value: buildAccountCurrencyDisplayValue(
        account.failureCost,
        localeTag,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "error" as const,
    },
    {
      label: locale === "zh" ? "失败成本比率" : "Failure cost ratio",
      value: buildAccountPercentDisplayValue(
        accountCostShare(account.failureCost, account.totalCost),
        localeTag,
      ),
      tone: "error" as const,
    },
  ];
}

function buildTokenSummarySegments(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
) {
  return [
    {
      label: locale === "zh" ? "缓存命中率" : "Cache hit",
      value: buildAccountPercentDisplayValue(account.cacheHitRate, localeTag),
      tone: "secondary" as const,
    },
    {
      label: locale === "zh" ? "失败" : "Failure",
      value: buildAccountNumberDisplayValue(
        account.failureTokens,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "error" as const,
    },
  ];
}

function buildRecentBridgeSegments(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
) {
  const segments = [];
  if (account.failureCount > 0) {
    segments.push({
      label: locale === "zh" ? "失败" : "Failure",
      value: buildAccountNumberDisplayValue(
        account.failureCount,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "error" as const,
      iconName: "alert-circle-outline" as const,
    });
  }
  if (account.nonSuccessCount > account.failureCount) {
    segments.push({
      label: locale === "zh" ? "非成功" : "Non-success",
      value: buildAccountNumberDisplayValue(
        account.nonSuccessCount - account.failureCount,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "warning" as const,
      iconName: "alert-outline" as const,
    });
  }
  if (account.successCount > 0) {
    segments.push({
      label: locale === "zh" ? "成功" : "Success",
      value: buildAccountNumberDisplayValue(
        account.successCount,
        localeTag,
        0,
        ACCOUNT_STAT_CARD_PRESENTATION,
      ),
      tone: "success" as const,
      iconName: "check-circle-outline" as const,
    });
  }
  return segments;
}

function buildUsageBreakdownModel(locale: "zh" | "en", localeTag: string, t: Translate) {
  const usageDetailsLabel = t("dashboard.usageBreakdown.title");
  const usageBreakdownLabels =
    locale === "zh"
      ? {
          total: "总计",
          cacheWrite: "缓存写入",
          cacheRead: "缓存读取",
          cacheHitTokens: "缓存读取",
          cacheHitRate: "缓存命中率",
          output: "输出",
          model: "模型",
          input: "输入",
          reasoning: "推理",
          unknown: "未知",
          unavailable: "成本分项未提供",
          tokenUnavailable: "Token 分项未提供",
          unknownModel: "未标识模型",
          reasoningEffort: "思考等级",
        }
      : {
          total: "Total",
          cacheWrite: "Cache write",
          cacheRead: "Cache read",
          cacheHitTokens: "Cache read",
          cacheHitRate: "Cache hit rate",
          output: "Output",
          model: "Model",
          input: "Input",
          reasoning: "Reasoning",
          unknown: "Unknown",
          unavailable: "Cost breakdown unavailable",
          tokenUnavailable: "Token breakdown unavailable",
          unknownModel: "Unidentified model",
          reasoningEffort: "Reasoning effort",
        };
  return {
    usageDetailsLabel,
    usageBreakdownLabels,
    formatBreakdownNumber: (value: number) => formatAccountNumberValue(value, localeTag, 0),
    formatBreakdownRatio: (value: number | null) =>
      value == null ? FALLBACK_CELL : formatAccountPercentValue(value, localeTag),
    formatBreakdownCurrency: (value: number) => formatAccountCurrencyValue(value, localeTag, 4),
  };
}

function buildLatencyDetailSections({
  account,
  locale,
  localeTag,
  t,
  currentFirstByteValue,
  currentResponseDurationValue,
  rangeFirstByteValue,
  rangeResponseDurationValue,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  localeTag: string;
  t: Translate;
  currentFirstByteValue: string;
  currentResponseDurationValue: string;
  rangeFirstByteValue: string;
  rangeResponseDurationValue: string;
}): AccountMetricDetailSection[] {
  const currentFirstByteMs = finiteNumber(account.currentFirstTokenAvgMs);
  const firstByteMs = finiteNumber(account.firstTokenAvgMs);
  const stageFirstByteMs = finiteNumber(account.firstByteAvgMs);
  const currentResponseDurationMs = finiteNumber(account.currentAvgResponseMs);
  const relatedRows: AccountMetricDetailRow[] = [];
  if (
    stageFirstByteMs != null &&
    firstByteMs != null &&
    Math.abs(stageFirstByteMs - firstByteMs) >= 0.5
  ) {
    relatedRows.push({
      label: locale === "zh" ? "阶段首字节" : "Stage first byte",
      value: formatAccountDurationValue(stageFirstByteMs, localeTag),
      tone: "secondary",
    });
  }
  if (
    currentFirstByteMs != null &&
    currentResponseDurationMs != null &&
    currentResponseDurationMs > 0 &&
    currentFirstByteMs <= currentResponseDurationMs
  ) {
    relatedRows.push({
      label: locale === "zh" ? "首字占比" : "First-byte share",
      value: formatAccountPercentValue(currentFirstByteMs / currentResponseDurationMs, localeTag),
      tone: "secondary",
    });
  }
  return [
    {
      title: locale === "zh" ? "当前显示值" : "Current display",
      rows: [
        {
          label: t("dashboard.today.firstResponseTime"),
          value: currentFirstByteValue,
          tone: currentFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary",
        },
        {
          label: t("dashboard.today.responseTime"),
          value: currentResponseDurationValue,
          tone: currentResponseDurationValue === FALLBACK_CELL ? "neutral" : "primary",
        },
      ],
    },
    {
      title: locale === "zh" ? "当前范围统计" : "Current range stats",
      rows: [
        {
          label: t("dashboard.today.firstResponseTime"),
          value: rangeFirstByteValue,
          tone: rangeFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary",
        },
        {
          label: t("dashboard.today.responseTime"),
          value: rangeResponseDurationValue,
          tone: rangeResponseDurationValue === FALLBACK_CELL ? "neutral" : "primary",
        },
      ],
    },
    ...(relatedRows.length > 0
      ? [{ title: locale === "zh" ? "相关数据" : "Related data", rows: relatedRows }]
      : []),
  ];
}

function buildRequestDetailSections(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
  totalRequestValue: string,
): AccountMetricDetailSection[] {
  const otherCount = Math.max(0, account.nonSuccessCount - account.failureCount);
  const successRate = account.requestCount > 0 ? account.successCount / account.requestCount : null;
  const nonSuccessRate =
    account.requestCount > 0 ? account.nonSuccessCount / account.requestCount : null;
  return [
    {
      title: locale === "zh" ? "当前字段" : "Current fields",
      rows: [
        {
          label: locale === "zh" ? "请求数" : "Requests",
          value: totalRequestValue,
          tone: "neutral",
        },
        {
          label: locale === "zh" ? "成功" : "Success",
          value: formatAccountNumberValue(account.successCount, localeTag, 0),
          tone: "success",
        },
        {
          label: locale === "zh" ? "失败" : "Failure",
          value: formatAccountNumberValue(account.failureCount, localeTag, 0),
          tone: "error",
        },
        {
          label: locale === "zh" ? "其他" : "Other",
          value: formatAccountNumberValue(otherCount, localeTag, 0),
          tone: "warning",
        },
      ],
    },
    {
      title: locale === "zh" ? "相关数据" : "Related data",
      rows: [
        {
          label: locale === "zh" ? "成功率" : "Success rate",
          value: formatAccountPercentValue(successRate, localeTag),
          tone: "success",
        },
        {
          label: locale === "zh" ? "非成功率" : "Non-success rate",
          value: formatAccountPercentValue(nonSuccessRate, localeTag),
          tone: "warning",
        },
      ],
    },
  ];
}

function buildCostDetailSections(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
  totalCostValue: string,
): AccountMetricDetailSection[] {
  const failureCostShare = accountCostShare(account.failureCost, account.totalCost);
  const nonFailureCost = account.totalCost - account.failureCost;
  const averageCost = account.requestCount > 0 ? account.totalCost / account.requestCount : null;
  return [
    {
      title: locale === "zh" ? "当前字段" : "Current fields",
      rows: [
        { label: locale === "zh" ? "成本" : "Cost", value: totalCostValue, tone: "warning" },
        {
          label: locale === "zh" ? "失败成本" : "Failure cost",
          value: formatAccountCurrencyValue(account.failureCost, localeTag, 2),
          tone: "error",
        },
        {
          label: locale === "zh" ? "失败成本比率" : "Failure cost ratio",
          value: formatAccountPercentValue(failureCostShare, localeTag),
          tone: "error",
        },
      ],
    },
    {
      title: locale === "zh" ? "相关数据" : "Related data",
      rows: [
        {
          label: locale === "zh" ? "成功/其他成本" : "Success/other cost",
          value: formatAccountCurrencyValue(nonFailureCost, localeTag, 2),
          tone: "warning",
        },
        {
          label: locale === "zh" ? "单次均价" : "Average per request",
          value: formatAccountCurrencyValue(averageCost, localeTag, 4),
          tone: "warning",
        },
      ],
    },
  ];
}

function buildTokenDetailSections(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
  localeTag: string,
  totalTokenValue: string,
): AccountMetricDetailSection[] {
  const averageTokens =
    account.requestCount > 0 ? account.totalTokens / account.requestCount : null;
  return [
    {
      title: locale === "zh" ? "当前字段" : "Current fields",
      rows: [
        { label: "Token", value: totalTokenValue, tone: "success" },
        {
          label: locale === "zh" ? "缓存命中率" : "Cache hit",
          value: formatAccountPercentValue(account.cacheHitRate, localeTag),
          tone: "secondary",
        },
        {
          label: locale === "zh" ? "失败 Token" : "Failure tokens",
          value: formatAccountNumberValue(account.failureTokens, localeTag, 0),
          tone: "error",
        },
      ],
    },
    {
      title: locale === "zh" ? "相关数据" : "Related data",
      rows: [
        {
          label: locale === "zh" ? "成功 Token" : "Success tokens",
          value: formatAccountNumberValue(account.successTokens, localeTag, 0),
          tone: "success",
        },
        {
          label: locale === "zh" ? "非成功 Token" : "Non-success tokens",
          value: formatAccountNumberValue(account.nonSuccessTokens, localeTag, 0),
          tone: "warning",
        },
        {
          label: locale === "zh" ? "单请求 Token" : "Tokens per request",
          value: formatAccountNumberValue(averageTokens, localeTag, 1),
          tone: "success",
        },
      ],
    },
  ];
}

function buildAccountDisplayValues(account: UpstreamAccountActivityAccount, localeTag: string) {
  const currentFirstByteDisplayValue = buildAccountDurationDisplayValue(
    account.currentFirstTokenAvgMs,
    localeTag,
    ACCOUNT_STAT_CARD_PRESENTATION,
  );
  const currentResponseDurationDisplayValue = buildAccountDurationDisplayValue(
    account.currentAvgResponseMs,
    localeTag,
    ACCOUNT_STAT_CARD_PRESENTATION,
  );
  const totalRequestDisplayValue = buildAccountNumberDisplayValue(
    account.requestCount,
    localeTag,
    0,
    ACCOUNT_STAT_CARD_PRESENTATION,
  );
  const totalCostDisplayValue = buildAccountCurrencyAmountDisplayValue(
    account.totalCost,
    localeTag,
    2,
    ACCOUNT_STAT_CARD_PRESENTATION,
  );
  const totalTokenDisplayValue = buildAccountNumberDisplayValue(
    account.totalTokens,
    localeTag,
    0,
    ACCOUNT_STAT_CARD_PRESENTATION,
  );
  return {
    currentFirstByteDisplayValue,
    currentResponseDurationDisplayValue,
    rangeFirstByteValue: formatAccountDurationValue(account.firstTokenAvgMs, localeTag),
    rangeResponseDurationValue: formatAccountDurationValue(
      account.modelPerformance?.total.avgResponseMs,
      localeTag,
    ),
    totalRequestDisplayValue,
    totalCostDisplayValue,
    totalTokenDisplayValue,
    currentFirstByteValue: currentFirstByteDisplayValue.fullText,
    currentResponseDurationValue: currentResponseDurationDisplayValue.fullText,
    totalRequestValue: totalRequestDisplayValue.fullText,
    totalCostValue: totalCostDisplayValue.fullText,
    totalTokenValue: totalTokenDisplayValue.fullText,
  };
}

export function buildDashboardWorkingAccountMetricModel({
  account,
  locale,
  localeTag,
  t,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  localeTag: string;
  t: Translate;
}) {
  const values = buildAccountDisplayValues(account, localeTag);
  const usage = buildUsageBreakdownModel(locale, localeTag, t);
  return {
    ...values,
    ...usage,
    requestSummarySegments: buildRequestSummarySegments(account, locale, localeTag),
    costSummarySegments: buildCostSummarySegments(account, locale, localeTag),
    tokenSummarySegments: buildTokenSummarySegments(account, locale, localeTag),
    recentBridgeSegments: buildRecentBridgeSegments(account, locale, localeTag),
    latencyDetailSections: buildLatencyDetailSections({
      account,
      locale,
      localeTag,
      t,
      currentFirstByteValue: values.currentFirstByteValue,
      currentResponseDurationValue: values.currentResponseDurationValue,
      rangeFirstByteValue: values.rangeFirstByteValue,
      rangeResponseDurationValue: values.rangeResponseDurationValue,
    }),
    requestDetailSections: buildRequestDetailSections(
      account,
      locale,
      localeTag,
      values.totalRequestValue,
    ),
    costDetailSections: buildCostDetailSections(account, locale, localeTag, values.totalCostValue),
    tokenDetailSections: buildTokenDetailSections(
      account,
      locale,
      localeTag,
      values.totalTokenValue,
    ),
    inProgressDisplayValue: buildAccountNumberDisplayValue(
      account.inProgressInvocationCount,
      localeTag,
      0,
    ),
    tokensPerMinuteDisplayValue: buildAccountNumberDisplayValue(
      account.tokensPerMinute,
      localeTag,
      0,
    ),
    spendRateDisplayValue: buildAccountCurrencyAmountDisplayValue(account.spendRate, localeTag, 2),
    modelPerformanceTitle: `${account.displayName} · ${t("dashboard.modelPerformance.title")}`,
  };
}

export type DashboardWorkingAccountMetricModel = ReturnType<
  typeof buildDashboardWorkingAccountMetricModel
>;
