import { useMemo } from "react";
import { Cell, Legend, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { Alert } from "../../components/ui/alert";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { ErrorDistributionItem } from "../../lib/api";
import { chartBaseTokens, piePalette } from "../../lib/chartTheme";
import { useTheme } from "../../theme";

interface ErrorReasonPieChartProps {
  items: ErrorDistributionItem[];
  isLoading: boolean;
}

export function ErrorReasonPieChart({ items, isLoading }: ErrorReasonPieChartProps) {
  const { t } = useTranslation();
  const { themeMode } = useTheme();
  const chartColors = useMemo(() => chartBaseTokens(themeMode), [themeMode]);
  const colors = useMemo(() => piePalette(themeMode), [themeMode]);

  const hasData = Array.isArray(items) && items.length > 0;

  if (!hasData) {
    if (isLoading) {
      return (
        <div className="flex justify-center py-10">
          <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
        </div>
      );
    }
    return <Alert>{t("chart.noDataRange")}</Alert>;
  }

  const data = items.map((it) => ({ name: it.reason, value: it.count }));

  return (
    <div className="h-96 w-full">
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
          />
          <Legend wrapperStyle={{ color: chartColors.axisText }} />
          <Pie data={data} dataKey="value" nameKey="name" outerRadius={120} label>
            {data.map((item, index) => (
              <Cell key={item.name} fill={colors[index % colors.length]} />
            ))}
          </Pie>
        </PieChart>
      </ResponsiveContainer>
    </div>
  );
}
