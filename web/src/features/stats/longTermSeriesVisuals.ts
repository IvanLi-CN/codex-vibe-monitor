import type { LongTermSeries, LongTermStatsDimension } from "../../lib/api";
import { longTermSeriesPalette, withOpacity } from "../../lib/chartTheme";
import type { ThemeMode } from "../../theme";
import { formatReasoningEffort } from "../shared/reasoningEffort";

export interface LongTermSeriesVisual {
  color: string;
  fill: string;
  fillOpacity: number;
  label: string;
  stroke: string;
  strokeDasharray?: string;
}

type ReasoningStyle = Omit<LongTermSeriesVisual, "color" | "fill" | "label" | "stroke"> & {
  strokeOpacity: number;
};

const REASONING_STYLES: Record<string, ReasoningStyle> = {
  none: { fillOpacity: 0.38, strokeOpacity: 0.78 },
  minimal: { fillOpacity: 0.2, strokeDasharray: "2 3", strokeOpacity: 0.72 },
  low: { fillOpacity: 0.28, strokeDasharray: "5 3", strokeOpacity: 0.82 },
  medium: { fillOpacity: 0.36, strokeDasharray: "9 3", strokeOpacity: 0.9 },
  high: { fillOpacity: 0.48, strokeDasharray: "13 3", strokeOpacity: 1 },
  xhigh: { fillOpacity: 0.56, strokeDasharray: "1 2", strokeOpacity: 0.96 },
  max: { fillOpacity: 0.64, strokeDasharray: "3 2", strokeOpacity: 1 },
  ultra: { fillOpacity: 0.72, strokeOpacity: 1 },
  unknown: { fillOpacity: 0.24, strokeDasharray: "7 2 1 2", strokeOpacity: 0.76 },
};

function normalizeIdentity(value: string): string {
  return value.trim().toLocaleLowerCase();
}

function resolveReasoningStyle(reasoningEffort: string | null | undefined): ReasoningStyle {
  const key = reasoningEffort ? normalizeIdentity(reasoningEffort) : "none";
  return REASONING_STYLES[key] ?? REASONING_STYLES.unknown;
}

function resolveFamilyKey(item: LongTermSeries, dimension: LongTermStatsDimension): string {
  return dimension === "model" ? normalizeIdentity(item.displayName) : item.seriesKey;
}

export function longTermSeriesLabel(
  item: LongTermSeries,
  dimension: LongTermStatsDimension,
): string {
  if (dimension !== "model") return item.displayName;
  const effort = formatReasoningEffort(item.reasoningEffort);
  return `${item.displayName} · ${effort}`;
}

export function resolveLongTermSeriesVisuals(
  series: readonly LongTermSeries[],
  dimension: LongTermStatsDimension,
  themeMode: ThemeMode,
): Map<string, LongTermSeriesVisual> {
  const familyKeys = [...new Set(series.map((item) => resolveFamilyKey(item, dimension)))].sort(
    (left, right) => left.localeCompare(right),
  );
  const colorsByFamily = new Map(
    familyKeys.map((familyKey, index) => [
      familyKey,
      longTermSeriesPalette(themeMode)[index % longTermSeriesPalette(themeMode).length],
    ]),
  );

  return new Map(
    series.map((item) => {
      const color = colorsByFamily.get(resolveFamilyKey(item, dimension)) ?? "currentColor";
      const reasoningStyle = resolveReasoningStyle(
        dimension === "model" ? item.reasoningEffort : undefined,
      );
      return [
        item.seriesKey,
        {
          color,
          fill: color,
          fillOpacity: reasoningStyle.fillOpacity,
          label: longTermSeriesLabel(item, dimension),
          stroke: withOpacity(color, reasoningStyle.strokeOpacity),
          strokeDasharray: reasoningStyle.strokeDasharray,
        },
      ];
    }),
  );
}
