import { Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type {
  Formatter,
  NameType,
  ValueType,
} from "recharts/types/component/DefaultTooltipContent";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Chip } from "../../components/ui/chip";
import type {
  RateWindowSnapshot,
  ReserveQuotaSnapshot,
  UpstreamAccountHistoryPoint,
} from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";

const WINDOW_PLACEHOLDER = "-";

interface UpstreamAccountUsageCardProps {
  title: string;
  description: string;
  window?: RateWindowSnapshot | null;
  history: UpstreamAccountHistoryPoint[];
  historyKey: "primaryUsedPercent" | "secondaryUsedPercent";
  emptyLabel: string;
  noteLabel?: string;
  accentClassName?: string;
}

function historyLabel(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

export function UpstreamAccountUsageCard({
  title,
  description,
  window,
  history,
  historyKey,
  emptyLabel,
  noteLabel,
  accentClassName = "text-primary",
}: UpstreamAccountUsageCardProps) {
  const missingWindow = window == null;
  const chartData = history
    .slice(-14)
    .map((point) => ({
      label: historyLabel(point.capturedAt),
      value: point[historyKey] ?? null,
    }))
    .filter((point) => point.value != null);

  const displayChartData = missingWindow ? [] : chartData;
  const chartEmpty = displayChartData.length === 0;
  const usedPercent = Math.max(0, Math.min(window?.usedPercent ?? 0, 100));
  const ringValue = missingWindow ? WINDOW_PLACEHOLDER : `${Math.round(usedPercent)}%`;
  const usedLabel = missingWindow ? WINDOW_PLACEHOLDER : (window?.usedText ?? emptyLabel);
  const limitLabel = missingWindow ? WINDOW_PLACEHOLDER : (window?.limitText ?? emptyLabel);
  const resetLabel = missingWindow
    ? WINDOW_PLACEHOLDER
    : window?.resetsAt
      ? historyLabel(window.resetsAt)
      : emptyLabel;
  const chartEmptyLabel = missingWindow ? WINDOW_PLACEHOLDER : emptyLabel;
  const tooltipFormatter: Formatter<ValueType, NameType> = (value) => {
    const rawValue = Array.isArray(value) ? value[0] : value;
    const numericValue = typeof rawValue === "number" ? rawValue : Number(rawValue ?? 0);
    return [`${Math.round(numericValue)}%`, title];
  };

  return (
    <Card className="border-base-300/80 bg-base-100/75">
      <CardHeader className="gap-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="text-base">{title}</CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          {noteLabel ? <Chip tone="secondary">{noteLabel}</Chip> : null}
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 lg:grid-cols-[auto,minmax(0,1fr)] lg:items-center">
        <div className="flex items-center gap-4">
          <div className="progress-ring" style={{ ["--value" as string]: usedPercent }}>
            <span
              className={
                missingWindow
                  ? "text-lg font-semibold text-base-content/55"
                  : `text-lg font-semibold ${accentClassName}`
              }
            >
              {ringValue}
            </span>
          </div>
          <div
            className={
              missingWindow
                ? "space-y-1 text-sm text-base-content/55"
                : "space-y-1 text-sm text-base-content/75"
            }
          >
            <p
              className={
                missingWindow
                  ? "text-base font-semibold text-base-content/55"
                  : "text-base font-semibold text-base-content"
              }
            >
              {usedLabel}
            </p>
            <p>{limitLabel}</p>
            <p className="inline-flex items-center gap-1">
              <AppIcon
                name="timer-refresh-outline"
                className={
                  missingWindow ? "h-4 w-4 text-base-content/55" : "h-4 w-4 text-base-content/50"
                }
                aria-hidden
              />
              <span>{resetLabel}</span>
            </p>
          </div>
        </div>

        <div className="rounded-2xl border border-base-300/70 bg-base-100/65 p-3">
          {chartEmpty ? (
            <div className="flex h-28 items-center justify-center rounded-xl border border-dashed border-base-300/75 bg-base-200/35 text-sm text-base-content/55">
              {chartEmptyLabel}
            </div>
          ) : (
            <div className="h-28 w-full">
              <ResponsiveContainer>
                <LineChart
                  data={displayChartData}
                  margin={{ top: 8, right: 12, left: 0, bottom: 0 }}
                >
                  <XAxis dataKey="label" hide />
                  <YAxis hide domain={[0, 100]} />
                  <Tooltip
                    cursor={{ stroke: "oklch(var(--color-base-content) / 0.14)", strokeWidth: 1 }}
                    formatter={tooltipFormatter}
                    labelFormatter={(value) => String(value)}
                    contentStyle={{
                      borderRadius: "0.9rem",
                      border:
                        "1px solid color-mix(in oklab, oklch(var(--color-base-content)) 18%, transparent)",
                      background: "oklch(var(--color-base-100) / 0.96)",
                    }}
                  />
                  <Line
                    type="monotone"
                    dataKey="value"
                    stroke="oklch(var(--color-primary))"
                    strokeWidth={2.5}
                    dot={{ r: 2.5, strokeWidth: 0, fill: "oklch(var(--color-primary))" }}
                    activeDot={{ r: 4 }}
                    connectNulls
                  />
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

interface ReserveQuotaCardProps {
  title: string;
  description: string;
  snapshot?: ReserveQuotaSnapshot | null;
  labels: {
    available: string;
    unavailable: string;
    unknown: string;
    used: string;
    availablePercent: string;
    reset: string;
    window: string;
    capturedAt: string;
    unknownValue: string;
    binding?: string;
    meteredFeature?: string;
  };
}

function reserveStatusLabel(
  availability: ReserveQuotaSnapshot["availability"] | undefined,
  labels: ReserveQuotaCardProps["labels"],
) {
  if (availability === "available") return labels.available;
  if (availability === "unavailable") return labels.unavailable;
  return labels.unknown;
}

function reserveStatusTone(availability: ReserveQuotaSnapshot["availability"] | undefined) {
  if (availability === "available") return "success" as const;
  if (availability === "unavailable") return "warning" as const;
  return "secondary" as const;
}

function reservePercent(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return null;
  return Math.max(0, Math.min(value, 100));
}

export function ReserveQuotaCard({ title, description, snapshot, labels }: ReserveQuotaCardProps) {
  const primaryUsedPercent = reservePercent(snapshot?.primaryWindow?.usedPercent);
  const primaryAvailablePercent = reservePercent(snapshot?.primaryWindow?.availablePercent);
  const statusLabel = reserveStatusLabel(snapshot?.availability, labels);
  const statusTone = reserveStatusTone(snapshot?.availability);
  const resetLabel = snapshot?.primaryWindow?.resetsAt
    ? historyLabel(snapshot.primaryWindow.resetsAt)
    : labels.unknownValue;
  const capturedLabel = snapshot?.capturedAt
    ? historyLabel(snapshot.capturedAt)
    : labels.unknownValue;
  const ringValue =
    primaryUsedPercent == null ? WINDOW_PLACEHOLDER : `${Math.round(primaryUsedPercent)}%`;

  return (
    <Card className="border-base-300/80 bg-base-100/75">
      <CardHeader className="gap-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="text-base">{title}</CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          <Chip tone={statusTone}>{statusLabel}</Chip>
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-[auto,minmax(0,1fr)] sm:items-center">
        <div className="flex items-center gap-4">
          <div className="progress-ring" style={{ ["--value" as string]: primaryUsedPercent ?? 0 }}>
            <span
              className={
                primaryUsedPercent == null
                  ? "text-lg font-semibold text-base-content/55"
                  : "text-lg font-semibold text-primary"
              }
            >
              {ringValue}
            </span>
          </div>
          <div className="space-y-1 text-sm text-base-content/75">
            <p className="text-base font-semibold text-base-content">
              {labels.used}{" "}
              {primaryUsedPercent == null
                ? labels.unknownValue
                : `${Math.round(primaryUsedPercent)}%`}
            </p>
            <p>
              {labels.availablePercent}{" "}
              {primaryAvailablePercent == null
                ? labels.unknownValue
                : `${Math.round(primaryAvailablePercent)}%`}
            </p>
            <p className="inline-flex items-center gap-1">
              <AppIcon
                name="timer-refresh-outline"
                className="h-4 w-4 text-base-content/50"
                aria-hidden
              />
              <span>
                {labels.reset} {resetLabel}
              </span>
            </p>
          </div>
        </div>
        <div className="rounded-2xl border border-base-300/70 bg-base-100/65 p-3 text-sm text-base-content/70">
          <p>
            {labels.capturedAt} {capturedLabel}
          </p>
          {snapshot?.normalModelSlug ? (
            <p className="mt-1">
              {labels.binding ?? "Bound model"} {snapshot.normalModelSlug}
            </p>
          ) : null}
          {snapshot?.meteredFeature ? (
            <p className="mt-1">
              {labels.meteredFeature ?? "Metered feature"} {snapshot.meteredFeature}
            </p>
          ) : null}
          {snapshot?.secondaryWindow ? (
            <p className="mt-1">
              {snapshot.secondaryWindow.windowDurationMins} {labels.window} · {labels.used}{" "}
              {reservePercent(snapshot.secondaryWindow.usedPercent) == null
                ? labels.unknownValue
                : `${Math.round(reservePercent(snapshot.secondaryWindow.usedPercent) ?? 0)}%`}
            </p>
          ) : null}
        </div>
      </CardContent>
    </Card>
  );
}
