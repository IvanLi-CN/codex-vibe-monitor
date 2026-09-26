import { useMemo } from "react";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { Alert } from "../../components/ui/alert";
import { SelectField } from "../../components/ui/select-field";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { ErrorDistributionItem, FailureScope } from "../../lib/api";
import { chartBaseTokens, piePalette } from "../../lib/chartTheme";
import { useTheme } from "../../theme";

interface ErrorReasonDistributionProps {
  items: ErrorDistributionItem[];
  isLoading: boolean;
  error: string | null;
  scope: FailureScope;
  onScopeChange: (scope: FailureScope) => void;
}

export function ErrorReasonDistribution({
  items,
  isLoading,
  error,
  scope,
  onScopeChange,
}: ErrorReasonDistributionProps) {
  const { locale, t } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 0 }),
    [localeTag],
  );
  const chartColors = useMemo(() => chartBaseTokens(themeMode), [themeMode]);
  const colors = useMemo(() => piePalette(themeMode), [themeMode]);
  const scopeOptions = useMemo(
    () =>
      [
        { value: "service", label: t("stats.errors.scope.service") },
        { value: "client", label: t("stats.errors.scope.client") },
        { value: "abort", label: t("stats.errors.scope.abort") },
        { value: "all", label: t("stats.errors.scope.all") },
      ] as const,
    [t],
  );

  return (
    <section
      className="flex min-w-0 flex-col gap-3"
      data-testid="stats-error-distribution"
      aria-busy={isLoading}
    >
      <div className="flex justify-end">
        <SelectField
          label={t("stats.errors.scope.label")}
          className="w-full min-[769px]:max-w-[14rem]"
          options={scopeOptions}
          value={scope}
          onValueChange={(value) => onScopeChange(value as FailureScope)}
          data-testid="stats-error-scope-select-trigger"
          aria-label={t("stats.errors.scope.label")}
        />
      </div>

      {isLoading ? (
        <div className="grid min-h-80 place-items-center" data-testid="error-distribution-loading">
          <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
        </div>
      ) : error ? (
        <Alert variant="error">{error}</Alert>
      ) : items.length === 0 ? (
        <Alert>{t("chart.noDataRange")}</Alert>
      ) : (
        <div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(16rem,1fr)_minmax(0,2fr)]">
          <div
            className="h-72 min-w-0 md:h-80"
            role="img"
            aria-label={t("stats.errors.chartAria")}
            data-testid="error-reason-pie-chart"
          >
            <ResponsiveContainer>
              <PieChart>
                <Tooltip
                  contentStyle={{
                    backgroundColor: chartColors.tooltipBg,
                    borderColor: chartColors.tooltipBorder,
                    borderRadius: 10,
                  }}
                  labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
                  itemStyle={{ color: chartColors.axisText }}
                  formatter={(value) => numberFormatter.format(Number(value))}
                />
                <Pie data={items} dataKey="count" nameKey="reason" outerRadius={110} label={false}>
                  {items.map((item, index) => (
                    <Cell key={item.reason} fill={colors[index % colors.length]} />
                  ))}
                </Pie>
              </PieChart>
            </ResponsiveContainer>
          </div>

          <ul
            className="min-w-0 divide-y divide-base-300/60"
            aria-label={t("stats.errors.title")}
            data-testid="error-reason-list"
          >
            {items.map((item, index) => (
              <li
                key={item.reason}
                className="grid min-w-0 grid-cols-[0.75rem_minmax(0,1fr)_auto] items-start gap-3 py-2.5 first:pt-1.5"
                data-testid="error-reason-row"
              >
                <span
                  aria-hidden="true"
                  className="mt-1.5 size-3 shrink-0 rounded-[2px]"
                  style={{ backgroundColor: colors[index % colors.length] }}
                />
                <span className="min-w-0 break-words text-sm leading-5 text-base-content">
                  {item.reason}
                </span>
                <span className="whitespace-nowrap text-sm font-medium tabular-nums text-base-content/80">
                  <span className="sr-only">{t("stats.errors.countLabel")}: </span>
                  {numberFormatter.format(item.count)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
