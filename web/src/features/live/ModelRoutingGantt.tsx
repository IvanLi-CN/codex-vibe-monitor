import { useMemo } from "react";
import {
  Bar,
  CartesianGrid,
  Cell,
  ComposedChart,
  ResponsiveContainer,
  Scatter,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { BarShapeProps } from "recharts/types/cartesian/Bar";
import { useTranslation } from "../../i18n";
import type {
  ModelRoutingLiveAccount,
  ModelRoutingLiveWindow,
  ModelRoutingTimelineRecord,
} from "../../lib/api";
import {
  chartBaseTokens,
  chartStatusTokens,
  metricAccent,
  withOpacity,
} from "../../lib/chartTheme";
import { useTheme } from "../../theme";

type RoutingTimelineState = "available" | "degraded" | "cooling_down" | "unknown";

interface RoutingGanttBand {
  state: RoutingTimelineState;
  startMs: number;
  endMs: number;
}

interface RoutingGanttAttempt {
  id: string;
  x: number;
  y: string;
  invokeId?: string | null;
  accountDisplayName: string;
  occurredAt: string;
  status?: string | null;
  httpStatus?: number | null;
  totalLatencyMs?: number | null;
  retryIndex?: number | null;
}

interface RoutingGanttLane {
  accountId: number;
  label: string;
  model: string;
  bands: RoutingGanttBand[];
}

interface RoutingGanttData {
  rangeStartMs: number;
  rangeEndMs: number;
  lanes: RoutingGanttLane[];
  attempts: RoutingGanttAttempt[];
}

interface RoutingGanttChartDatum {
  lane: string;
  accountId: number;
  bandValues: Record<string, number>;
}

const WINDOW_DURATION_MS: Record<ModelRoutingLiveWindow, number> = {
  "15m": 15 * 60_000,
  "1h": 60 * 60_000,
  "6h": 6 * 60 * 60_000,
  "24h": 24 * 60 * 60_000,
};

const ROUTING_STATES = new Set<RoutingTimelineState>(["available", "degraded", "cooling_down"]);

function parseTimestamp(value?: string | null) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isFinite(epoch) ? epoch : null;
}

function routingState(value?: string | null): RoutingTimelineState | null {
  return value && ROUTING_STATES.has(value as RoutingTimelineState)
    ? (value as RoutingTimelineState)
    : null;
}

function clampToRange(value: number, start: number, end: number) {
  return Math.max(start, Math.min(end, value));
}

function appendBand(
  bands: RoutingGanttBand[],
  state: RoutingTimelineState,
  startMs: number,
  endMs: number,
) {
  if (!(endMs > startMs)) return;
  const previous = bands.at(-1);
  if (previous && previous.state === state && previous.endMs === startMs) {
    previous.endMs = endMs;
    return;
  }
  bands.push({ state, startMs, endMs });
}

interface StatePoint {
  atMs: number;
  state: RoutingTimelineState;
  rank: number;
}

function buildLaneBands(
  account: Pick<ModelRoutingLiveAccount, "state" | "changedAt">,
  records: ModelRoutingTimelineRecord[],
  rangeStartMs: number,
  rangeEndMs: number,
) {
  const points: StatePoint[] = [];
  for (const record of records) {
    const atMs = parseTimestamp(record.occurredAt);
    const state = routingState(record.modelRouteStateAfter);
    if (atMs == null || state == null || atMs < rangeStartMs || atMs > rangeEndMs) continue;
    points.push({ atMs, state, rank: 1 });

    const cooldownUntilMs = parseTimestamp(record.modelRouteCooldownUntil);
    if (state === "cooling_down" && cooldownUntilMs != null && cooldownUntilMs > atMs) {
      points.push({
        atMs: clampToRange(cooldownUntilMs, rangeStartMs, rangeEndMs),
        state: "unknown",
        rank: 0,
      });
    }
  }

  const changedAtMs = parseTimestamp(account.changedAt);
  const currentState = routingState(account.state);
  if (changedAtMs != null && currentState != null && changedAtMs >= rangeStartMs) {
    points.push({
      atMs: clampToRange(changedAtMs, rangeStartMs, rangeEndMs),
      state: currentState,
      rank: 2,
    });
  }

  points.sort((left, right) => left.atMs - right.atMs || left.rank - right.rank);
  const bands: RoutingGanttBand[] = [];
  let cursor = rangeStartMs;
  let active: RoutingTimelineState = "unknown";
  for (const point of points) {
    if (point.atMs > cursor) appendBand(bands, active, cursor, point.atMs);
    active = point.state;
    cursor = Math.max(cursor, point.atMs);
  }
  appendBand(bands, active, cursor, rangeEndMs);
  return bands;
}

export function buildModelRoutingGanttData({
  model,
  accounts,
  records,
  generatedAt,
  window,
}: {
  model: string;
  accounts: ModelRoutingLiveAccount[];
  records: ModelRoutingTimelineRecord[];
  generatedAt?: string | null;
  window: ModelRoutingLiveWindow;
}): RoutingGanttData {
  const rangeEndMs = parseTimestamp(generatedAt) ?? Date.now();
  const rangeStartMs = rangeEndMs - WINDOW_DURATION_MS[window];
  const accountMap = new Map(accounts.map((account) => [account.accountId, account]));
  const modelRecords = records.filter((record) => record.model === model);

  for (const record of modelRecords) {
    if (!accountMap.has(record.accountId)) {
      accountMap.set(record.accountId, {
        accountId: record.accountId,
        accountDisplayName: record.accountDisplayName,
        model,
        state: "unknown",
        priority: "unknown",
        failureCount: 0,
        lastSeenAt: record.occurredAt,
      });
    }
  }

  const lanes = Array.from(accountMap.values())
    .sort((left, right) => left.accountDisplayName.localeCompare(right.accountDisplayName))
    .map((account) => {
      const laneRecords = modelRecords.filter((record) => record.accountId === account.accountId);
      return {
        accountId: account.accountId,
        label: account.accountDisplayName,
        model,
        bands: buildLaneBands(account, laneRecords, rangeStartMs, rangeEndMs),
      };
    });
  const laneLabels = new Map(lanes.map((lane) => [lane.accountId, lane.label]));
  const attempts = modelRecords.flatMap((record) => {
    if (record.kind !== "attempt") return [];
    const occurredAtMs = parseTimestamp(record.occurredAt);
    const label = laneLabels.get(record.accountId);
    if (
      occurredAtMs == null ||
      !label ||
      occurredAtMs < rangeStartMs ||
      occurredAtMs > rangeEndMs
    ) {
      return [];
    }
    return [
      {
        id: record.id,
        x: occurredAtMs - rangeStartMs,
        y: label,
        invokeId: record.invokeId,
        accountDisplayName: record.accountDisplayName,
        occurredAt: record.occurredAt,
        status: record.status,
        httpStatus: record.httpStatus,
        totalLatencyMs: record.totalLatencyMs,
        retryIndex: record.sameAccountRetryIndex,
      },
    ];
  });

  return { rangeStartMs, rangeEndMs, lanes, attempts };
}

function formatBeijingTime(value: number, localeTag: string) {
  return new Intl.DateTimeFormat(localeTag, {
    timeZone: "Asia/Shanghai",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(new Date(value));
}

function formatBeijingRange(startMs: number, endMs: number, localeTag: string) {
  const formatter = new Intl.DateTimeFormat(localeTag, {
    timeZone: "Asia/Shanghai",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${formatter.format(new Date(startMs))} - ${formatter.format(new Date(endMs))}`;
}

function StateBandShape({
  x,
  y,
  width,
  height,
  fill,
  label,
  onOpenAccount,
}: BarShapeProps & { fill: string; label: string; onOpenAccount: () => void }) {
  if (!(width > 0) || !(height > 0)) return null;
  const bandY = y + Math.max(2, height * 0.2);
  const bandHeight = Math.max(4, height * 0.6);
  return (
    <g>
      <title>{label}</title>
      <rect x={x} y={bandY} width={width} height={bandHeight} rx={2} fill={fill} />
      <foreignObject x={x} y={bandY} width={width} height={bandHeight}>
        <button
          type="button"
          className="block h-full w-full cursor-pointer border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-primary"
          aria-label={label}
          onClick={onOpenAccount}
        >
          <span className="sr-only">{label}</span>
        </button>
      </foreignObject>
    </g>
  );
}

function AttemptMarker({
  cx,
  cy,
  payload,
  fill,
  label,
  onOpenInvocation,
}: {
  cx?: number;
  cy?: number;
  payload?: RoutingGanttAttempt;
  fill: string;
  label: string;
  onOpenInvocation: (invokeId: string) => void;
}) {
  if (cx == null || cy == null || !payload) return null;
  const openable = Boolean(payload.invokeId);
  return (
    <g>
      <title>{label}</title>
      <path
        d={`M ${cx} ${cy - 5} L ${cx + 5} ${cy} L ${cx} ${cy + 5} L ${cx - 5} ${cy} Z`}
        fill={fill}
      />
      {openable ? (
        <foreignObject x={cx - 8} y={cy - 8} width={16} height={16}>
          <button
            type="button"
            className="block h-full w-full cursor-pointer border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-primary"
            aria-label={label}
            onClick={() => onOpenInvocation(payload.invokeId ?? "")}
          >
            <span className="sr-only">{label}</span>
          </button>
        </foreignObject>
      ) : null}
    </g>
  );
}

export function ModelRoutingGantt({
  model,
  accounts,
  records,
  generatedAt,
  window,
  onOpenAccount,
  onOpenInvocation,
}: {
  model: string;
  accounts: ModelRoutingLiveAccount[];
  records: ModelRoutingTimelineRecord[];
  generatedAt?: string | null;
  window: ModelRoutingLiveWindow;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
}) {
  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const timeline = useMemo(
    () => buildModelRoutingGanttData({ model, accounts, records, generatedAt, window }),
    [accounts, generatedAt, model, records, window],
  );
  const colors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    return {
      available: withOpacity(status.success, 0.82),
      degraded: withOpacity(metricAccent("totalCost", themeMode), 0.82),
      cooling_down: withOpacity(metricAccent("totalCost", themeMode), 0.48),
      unknown: withOpacity(base.axisText, 0.2),
      attemptSuccess: status.success,
      attemptFailure: status.failure,
      axis: base.axisText,
      grid: base.gridLine,
    };
  }, [themeMode]);
  const chartRows = useMemo<RoutingGanttChartDatum[]>(() => {
    return timeline.lanes.map((lane) => ({
      lane: lane.label,
      accountId: lane.accountId,
      bandValues: Object.fromEntries(
        lane.bands.map((band, index) => [`band-${index}`, band.endMs - band.startMs]),
      ),
    }));
  }, [timeline.lanes]);
  const bandCount = Math.max(0, ...timeline.lanes.map((lane) => lane.bands.length));
  const bandKeys = Array.from({ length: bandCount }, (_, index) => `band-${index}`);
  const chartHeight = Math.max(166, timeline.lanes.length * 48 + 66);
  const tickValues = Array.from(
    { length: 5 },
    (_, index) => ((timeline.rangeEndMs - timeline.rangeStartMs) * index) / 4,
  );
  const stateLabels: Record<RoutingTimelineState, string> = {
    available: t("live.routing.states.available"),
    degraded: t("live.routing.states.degraded"),
    cooling_down: t("live.routing.states.cooling_down"),
    unknown: t("live.routing.states.unknown"),
  };

  if (timeline.lanes.length === 0) {
    return (
      <p
        className="px-3 py-4 text-sm text-base-content/70"
        data-testid={`model-routing-gantt-empty-${model}`}
      >
        {t("live.routing.timeline.empty")}
      </p>
    );
  }

  return (
    <div data-testid={`model-routing-gantt-${model}`}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-y border-base-300/60 px-3 py-2 text-xs text-base-content/70">
        {(["available", "degraded", "cooling_down", "unknown"] as RoutingTimelineState[]).map(
          (state) => (
            <span key={state} className="inline-flex items-center gap-1.5">
              <span
                className="h-2.5 w-2.5 rounded-sm"
                style={{ backgroundColor: colors[state] }}
                aria-hidden
              />
              {stateLabels[state]}
            </span>
          ),
        )}
        <span className="inline-flex items-center gap-1.5">
          <span
            className="h-2.5 w-2.5 rotate-45"
            style={{ backgroundColor: colors.attemptSuccess }}
            aria-hidden
          />
          {t("live.routing.timeline.attempt")}
        </span>
      </div>
      <div className="overflow-x-auto" data-testid={`model-routing-gantt-scroll-${model}`}>
        <div className="min-w-[46rem] px-3 pb-3 pt-2">
          <ResponsiveContainer width="100%" height={chartHeight}>
            <ComposedChart
              layout="vertical"
              data={chartRows.map((row) => ({ ...row, ...row.bandValues }))}
              margin={{ top: 8, right: 14, bottom: 8, left: 8 }}
              barCategoryGap="36%"
              aria-label={t("live.routing.timeline.aria", { model })}
            >
              <CartesianGrid stroke={colors.grid} strokeDasharray="3 4" horizontal={false} />
              <XAxis
                xAxisId="state"
                type="number"
                domain={[0, timeline.rangeEndMs - timeline.rangeStartMs]}
                ticks={tickValues}
                tickFormatter={(value) =>
                  formatBeijingTime(timeline.rangeStartMs + Number(value), localeTag)
                }
                tick={{ fill: colors.axis, fontSize: 11 }}
                axisLine={{ stroke: colors.grid }}
                tickLine={{ stroke: colors.grid }}
                minTickGap={32}
              />
              <YAxis
                yAxisId="state"
                type="category"
                dataKey="lane"
                width={148}
                tick={{ fill: colors.axis, fontSize: 12 }}
                axisLine={false}
                tickLine={false}
              />
              <XAxis
                xAxisId="attempt"
                type="number"
                dataKey="x"
                domain={[0, timeline.rangeEndMs - timeline.rangeStartMs]}
                hide
              />
              <YAxis yAxisId="attempt" type="category" dataKey="y" hide />
              <Tooltip cursor={{ fill: withOpacity(colors.axis, 0.05) }} content={() => null} />
              {bandKeys.map((key, bandIndex) => (
                <Bar
                  key={key}
                  dataKey={key}
                  xAxisId="state"
                  yAxisId="state"
                  stackId="routing-state"
                  isAnimationActive={false}
                  legendType="none"
                  shape={(shapeProps) => {
                    const lane = timeline.lanes[shapeProps.index];
                    const band = lane?.bands[bandIndex];
                    if (!lane || !band) return null;
                    const label = `${lane.label} · ${stateLabels[band.state]} · ${formatBeijingRange(
                      band.startMs,
                      band.endMs,
                      localeTag,
                    )}`;
                    return (
                      <StateBandShape
                        {...shapeProps}
                        fill={colors[band.state]}
                        label={label}
                        onOpenAccount={() => onOpenAccount(lane.accountId, lane.model)}
                      />
                    );
                  }}
                >
                  {chartRows.map((row, rowIndex) => {
                    const state = timeline.lanes[rowIndex]?.bands[bandIndex]?.state ?? "unknown";
                    return <Cell key={`${row.accountId}-${key}`} fill={colors[state]} />;
                  })}
                </Bar>
              ))}
              <Scatter
                xAxisId="attempt"
                yAxisId="attempt"
                data={timeline.attempts}
                isAnimationActive={false}
                legendType="none"
                shape={(shapeProps) => {
                  const attempt = shapeProps.payload as RoutingGanttAttempt | undefined;
                  if (!attempt) return null;
                  const successful = attempt.httpStatus != null && attempt.httpStatus < 400;
                  const result = attempt.httpStatus
                    ? `HTTP ${attempt.httpStatus}`
                    : attempt.status || "-";
                  const latency =
                    attempt.totalLatencyMs != null
                      ? ` · ${Math.round(attempt.totalLatencyMs)} ms`
                      : "";
                  const retry =
                    (attempt.retryIndex ?? 0) > 0
                      ? ` · ${t("live.routing.timeline.retry", { index: attempt.retryIndex ?? 0 })}`
                      : "";
                  const atMs = parseTimestamp(attempt.occurredAt) ?? timeline.rangeStartMs;
                  return (
                    <AttemptMarker
                      cx={shapeProps.cx}
                      cy={shapeProps.cy}
                      payload={attempt}
                      fill={successful ? colors.attemptSuccess : colors.attemptFailure}
                      label={`${attempt.accountDisplayName} · ${t("live.routing.timeline.attempt")} · ${formatBeijingRange(
                        atMs,
                        atMs,
                        localeTag,
                      )} · ${result}${latency}${retry}`}
                      onOpenInvocation={onOpenInvocation}
                    />
                  );
                }}
              />
            </ComposedChart>
          </ResponsiveContainer>
        </div>
      </div>
      <p className="px-3 pb-3 text-xs text-base-content/60">{t("live.routing.timeline.hint")}</p>
    </div>
  );
}
