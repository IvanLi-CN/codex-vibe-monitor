import { Fragment, useMemo } from "react";
import { useTranslation } from "../../i18n";
import type {
  ModelRoutingLiveAccount,
  ModelRoutingLiveModelGroup,
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
import { AppIcon } from "../shared/AppIcon";
import { resolveModelIdentityIcon } from "../shared/ModelIdentity";

type RoutingTimelineState = "available" | "degraded" | "cooling_down" | "unknown";

interface RoutingGanttBand {
  state: RoutingTimelineState;
  startMs: number;
  endMs: number;
}

interface RoutingGanttAttempt {
  id: string;
  accountId: number;
  occurredAtMs: number;
  invokeId?: string | null;
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
  state: RoutingTimelineState;
  bands: RoutingGanttBand[];
}

interface RoutingGanttData {
  rangeStartMs: number;
  rangeEndMs: number;
  lanes: RoutingGanttLane[];
  attempts: RoutingGanttAttempt[];
}

export function availableBandOpacity(callCount: number, maxCallCount: number) {
  if (maxCallCount <= 0) return 0.56;
  const ratio = Math.max(0, Math.min(1, callCount / maxCallCount));
  return 0.3 + ratio * 0.7;
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

function routingLaneLabel(accountId: number) {
  return `API Key #${accountId}`;
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
        accountDisplayName: "",
        model,
        state: "unknown",
        priority: "unknown",
        failureCount: 0,
        lastSeenAt: record.occurredAt,
      });
    }
  }

  const lanes = Array.from(accountMap.values())
    .sort((left, right) => left.accountId - right.accountId)
    .map((account) => {
      const laneRecords = modelRecords.filter((record) => record.accountId === account.accountId);
      return {
        accountId: account.accountId,
        label: routingLaneLabel(account.accountId),
        model,
        state: routingState(account.state) ?? "unknown",
        bands: buildLaneBands(account, laneRecords, rangeStartMs, rangeEndMs),
      };
    });
  const attempts = modelRecords.flatMap((record) => {
    if (record.kind !== "attempt") return [];
    const occurredAtMs = parseTimestamp(record.occurredAt);
    if (
      occurredAtMs == null ||
      occurredAtMs < rangeStartMs ||
      occurredAtMs > rangeEndMs ||
      !accountMap.has(record.accountId)
    ) {
      return [];
    }
    return [
      {
        id: record.id,
        accountId: record.accountId,
        occurredAtMs,
        invokeId: record.invokeId,
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

function formatBeijingDate(value: number, localeTag: string) {
  return new Intl.DateTimeFormat(localeTag, {
    timeZone: "Asia/Shanghai",
    month: "2-digit",
    day: "2-digit",
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

function percentAt(value: number, start: number, end: number) {
  if (!(end > start)) return 0;
  return ((clampToRange(value, start, end) - start) / (end - start)) * 100;
}

function bandStyle(band: RoutingGanttBand, rangeStartMs: number, rangeEndMs: number) {
  const left = percentAt(band.startMs, rangeStartMs, rangeEndMs);
  const naturalWidth = percentAt(band.endMs, rangeStartMs, rangeEndMs) - left;
  const width = Math.min(100 - left, Math.max(0.75, naturalWidth));
  return { left: `${left}%`, width: `${width}%` };
}

function edgeClass(index: number, lastIndex: number) {
  if (index === 0) return "left-0";
  if (index === lastIndex) return "-translate-x-full";
  return "-translate-x-1/2";
}

function bandKey(model: string, accountId: number, band: RoutingGanttBand) {
  return `${model}:${accountId}:${band.startMs}:${band.endMs}`;
}

function countAttemptsInBand(
  attempts: RoutingGanttAttempt[],
  band: RoutingGanttBand,
  rangeEndMs: number,
) {
  return attempts.filter(
    (attempt) =>
      attempt.occurredAtMs >= band.startMs &&
      (attempt.occurredAtMs < band.endMs || attempt.occurredAtMs === rangeEndMs),
  ).length;
}

export function ModelRoutingGantt({
  groups,
  records,
  generatedAt,
  window,
  onOpenAccount,
  onOpenInvocation,
}: {
  groups: ModelRoutingLiveModelGroup[];
  records: ModelRoutingTimelineRecord[];
  generatedAt?: string | null;
  window: ModelRoutingLiveWindow;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
}) {
  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const timelines = useMemo(
    () =>
      groups.map((group) => ({
        model: group.model,
        accountCount: group.accounts.length,
        recordCount: records.filter((record) => record.model === group.model).length,
        timeline: buildModelRoutingGanttData({
          model: group.model,
          accounts: group.accounts,
          records,
          generatedAt,
          window,
        }),
      })),
    [generatedAt, groups, records, window],
  );
  const range = timelines[0]?.timeline;
  const colors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    return {
      available: status.success,
      availableLegend: withOpacity(status.success, 0.62),
      degraded: withOpacity(metricAccent("totalCost", themeMode), 0.76),
      cooling_down: withOpacity(metricAccent("totalCost", themeMode), 0.46),
      unknown: withOpacity(base.axisText, 0.58),
      attemptSuccess: status.success,
      attemptFailure: status.failure,
      attemptUnknown: base.axisText,
      grid: base.gridLine,
    };
  }, [themeMode]);
  const stateLabels: Record<RoutingTimelineState, string> = {
    available: t("live.routing.states.available"),
    degraded: t("live.routing.states.degraded"),
    cooling_down: t("live.routing.states.cooling_down"),
    unknown: t("live.routing.states.unknown"),
  };
  const desktopTicks = Array.from({ length: 5 }, (_, index) => index);
  const mobileTicks = [0, 2, 4];
  const gridTicks = desktopTicks.slice(1, -1);
  const availableCallStats = useMemo(() => {
    const counts = new Map<string, number>();
    let max = 0;
    let total = 0;
    for (const { model, timeline } of timelines) {
      for (const lane of timeline.lanes) {
        const attempts = timeline.attempts.filter(
          (attempt) => attempt.accountId === lane.accountId,
        );
        for (const band of lane.bands) {
          if (band.state !== "available") continue;
          const count = countAttemptsInBand(attempts, band, timeline.rangeEndMs);
          counts.set(bandKey(model, lane.accountId, band), count);
          max = Math.max(max, count);
          total += count;
        }
      }
    }
    return { counts, max, total };
  }, [timelines]);

  if (!range || timelines.length === 0) {
    return (
      <p className="px-3 py-4 text-sm text-base-content/70" data-testid="model-routing-gantt-empty">
        {t("live.routing.timeline.empty")}
      </p>
    );
  }

  return (
    <div data-testid="model-routing-gantt">
      <div
        className="flex flex-wrap items-center gap-x-4 gap-y-2 border-y border-base-300/60 px-3 py-2 text-xs text-base-content/70"
        data-testid="model-routing-gantt-legend"
      >
        {(["available", "degraded", "cooling_down", "unknown"] as RoutingTimelineState[]).map(
          (state) => (
            <span key={state} className="inline-flex items-center gap-1.5">
              <span
                className={
                  state === "unknown"
                    ? "h-2.5 w-2.5 rounded-sm border border-dashed"
                    : "h-2.5 w-2.5 rounded-sm"
                }
                style={
                  state === "unknown"
                    ? { borderColor: colors.unknown }
                    : {
                        backgroundColor:
                          state === "available" ? colors.availableLegend : colors[state],
                      }
                }
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
      <section
        className="grid grid-cols-[minmax(8rem,9rem)_minmax(0,1fr)] border-b border-base-300/60 desktop:grid-cols-[minmax(13rem,16rem)_minmax(0,1fr)]"
        data-testid="model-routing-gantt-grid"
        aria-label={t("live.routing.title")}
      >
        <div className="flex h-14 items-center border-r border-base-300/60 px-3 text-xs font-medium text-base-content/60">
          <span>{t("live.routing.timeline.lane")}</span>
        </div>
        <div className="relative h-14 overflow-hidden text-xs font-medium text-base-content/60">
          <span className="absolute left-3 top-1.5">{t("live.routing.timeline.time")}</span>
          <div className="absolute inset-x-0 bottom-1 h-7">
            <div className="desktop:hidden">
              {mobileTicks.map((tick, index) => {
                const left = (tick / 4) * 100;
                const atMs =
                  range.rangeStartMs + ((range.rangeEndMs - range.rangeStartMs) * tick) / 4;
                return (
                  <span
                    key={tick}
                    className={`absolute bottom-0 flex flex-col whitespace-nowrap leading-tight ${edgeClass(index, mobileTicks.length - 1)}`}
                    style={{ left: `${left}%` }}
                  >
                    <span>{formatBeijingDate(atMs, localeTag)}</span>
                    <span>{formatBeijingTime(atMs, localeTag)}</span>
                  </span>
                );
              })}
            </div>
            <div className="hidden desktop:block">
              {desktopTicks.map((tick, index) => {
                const left = (tick / 4) * 100;
                const atMs =
                  range.rangeStartMs + ((range.rangeEndMs - range.rangeStartMs) * tick) / 4;
                return (
                  <span
                    key={tick}
                    className={`absolute bottom-0 flex flex-col whitespace-nowrap leading-tight ${edgeClass(index, desktopTicks.length - 1)}`}
                    style={{ left: `${left}%` }}
                  >
                    <span>{formatBeijingDate(atMs, localeTag)}</span>
                    <span>{formatBeijingTime(atMs, localeTag)}</span>
                  </span>
                );
              })}
            </div>
          </div>
        </div>
        {timelines.map(({ model, accountCount, recordCount, timeline }) => {
          const modelIdentityIcon = resolveModelIdentityIcon(model);
          const attemptsByAccount = new Map<number, RoutingGanttAttempt[]>();
          for (const attempt of timeline.attempts) {
            const current = attemptsByAccount.get(attempt.accountId) ?? [];
            current.push(attempt);
            attemptsByAccount.set(attempt.accountId, current);
          }

          return (
            <Fragment key={model}>
              <div
                className="flex h-9 min-w-0 items-center border-r border-t border-base-300/60 bg-base-200/35 px-3"
                data-testid={`model-routing-model-group-${model}`}
              >
                <span className="flex min-w-0 items-center gap-1.5">
                  {modelIdentityIcon ? (
                    <AppIcon
                      name={modelIdentityIcon}
                      className="h-4 w-4 shrink-0 text-success"
                      aria-hidden
                    />
                  ) : null}
                  <span className="truncate font-mono text-xs font-semibold text-base-content">
                    {model}
                  </span>
                </span>
              </div>
              <div className="flex h-9 items-center justify-end border-t border-base-300/60 bg-base-200/35 px-3 text-xs tabular-nums text-base-content/60">
                <span>
                  {t("live.routing.accountsCount", { count: accountCount })} ·{" "}
                  {t("live.routing.modelRecordsCount", { count: recordCount })}
                </span>
              </div>
              {timeline.lanes.map((lane) => {
                const laneAttempts = attemptsByAccount.get(lane.accountId) ?? [];
                return (
                  <div key={`${model}-${lane.accountId}`} className="contents">
                    <button
                      type="button"
                      className="flex h-12 min-w-0 items-center border-r border-t border-base-300/60 px-3 text-left outline-none transition-colors hover:bg-base-200/50 focus-visible:bg-base-200/70 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
                      aria-label={`${lane.label} · ${stateLabels[lane.state]}`}
                      onClick={() => onOpenAccount(lane.accountId, lane.model)}
                    >
                      <span className="truncate font-mono text-xs font-semibold text-base-content">
                        {lane.label}
                      </span>
                    </button>
                    <div
                      className="relative h-12 overflow-hidden border-t border-base-300/60"
                      data-testid={`model-routing-lane-${model}-${lane.accountId}`}
                    >
                      {gridTicks.map((tick) => (
                        <span
                          key={tick}
                          className="absolute inset-y-0 border-l border-dashed border-base-300/70"
                          style={{ left: `${(tick / 4) * 100}%` }}
                          aria-hidden
                        />
                      ))}
                      {lane.bands.map((band) => {
                        const callCount =
                          band.state === "available"
                            ? (availableCallStats.counts.get(
                                bandKey(model, lane.accountId, band),
                              ) ?? 0)
                            : 0;
                        const allocationPercent =
                          availableCallStats.total > 0
                            ? Math.round((callCount / availableCallStats.total) * 100)
                            : 0;
                        const baseLabel = `${lane.label} · ${stateLabels[band.state]} · ${formatBeijingRange(
                          band.startMs,
                          band.endMs,
                          localeTag,
                        )}`;
                        const label =
                          band.state === "available"
                            ? `${baseLabel} · ${t("live.routing.timeline.availableAllocation", {
                                count: callCount,
                                percent: allocationPercent,
                              })}`
                            : baseLabel;
                        const availableOpacity = availableBandOpacity(
                          callCount,
                          availableCallStats.max,
                        );
                        return (
                          <button
                            key={`${band.state}-${band.startMs}-${band.endMs}`}
                            type="button"
                            className={`absolute top-5 z-10 h-3 cursor-pointer rounded-sm outline-none ring-offset-base-100 focus-visible:ring-2 focus-visible:ring-primary ${
                              band.state === "unknown" ? "border border-dashed" : ""
                            }`}
                            style={
                              band.state === "unknown"
                                ? {
                                    ...bandStyle(band, timeline.rangeStartMs, timeline.rangeEndMs),
                                    borderColor: colors.unknown,
                                  }
                                : {
                                    ...bandStyle(band, timeline.rangeStartMs, timeline.rangeEndMs),
                                    backgroundColor:
                                      band.state === "available"
                                        ? withOpacity(colors.available, availableOpacity)
                                        : colors[band.state],
                                  }
                            }
                            aria-label={label}
                            title={label}
                            onClick={() => onOpenAccount(lane.accountId, lane.model)}
                          >
                            <span className="sr-only">{label}</span>
                          </button>
                        );
                      })}
                      {laneAttempts.map((attempt) => {
                        const successful = attempt.httpStatus != null && attempt.httpStatus < 400;
                        const failed = attempt.httpStatus != null && attempt.httpStatus >= 400;
                        const result = attempt.httpStatus
                          ? `HTTP ${attempt.httpStatus}`
                          : attempt.status || t("live.routing.record.unknown");
                        const latency =
                          attempt.totalLatencyMs != null
                            ? ` · ${Math.round(attempt.totalLatencyMs)} ms`
                            : "";
                        const retry =
                          (attempt.retryIndex ?? 0) > 0
                            ? ` · ${t("live.routing.timeline.retry", { index: attempt.retryIndex ?? 0 })}`
                            : "";
                        const label = `${lane.label} · ${t("live.routing.timeline.attempt")} · ${formatBeijingRange(
                          attempt.occurredAtMs,
                          attempt.occurredAtMs,
                          localeTag,
                        )} · ${result}${latency}${retry}`;
                        const color = successful
                          ? colors.attemptSuccess
                          : failed
                            ? colors.attemptFailure
                            : colors.attemptUnknown;
                        const markerLeft = Math.max(
                          1,
                          Math.min(
                            99,
                            percentAt(
                              attempt.occurredAtMs,
                              timeline.rangeStartMs,
                              timeline.rangeEndMs,
                            ),
                          ),
                        );
                        const marker = (
                          <span
                            key={attempt.id}
                            className="absolute top-5 z-20 h-1.5 w-1.5 -translate-x-1/2 rotate-45 border border-base-100 shadow-sm"
                            style={{ left: `${markerLeft}%`, backgroundColor: color }}
                            aria-hidden
                          />
                        );
                        return attempt.invokeId ? (
                          <button
                            key={attempt.id}
                            type="button"
                            className="absolute inset-y-0 z-30 w-4 -translate-x-1/2 cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary"
                            style={{ left: `${markerLeft}%` }}
                            aria-label={label}
                            title={label}
                            onClick={() => onOpenInvocation(attempt.invokeId ?? "")}
                          >
                            {marker}
                          </button>
                        ) : (
                          <span
                            key={attempt.id}
                            title={label}
                            className="absolute inset-y-0 z-20 w-5 -translate-x-1/2"
                            style={{ left: `${markerLeft}%` }}
                          >
                            {marker}
                          </span>
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </Fragment>
          );
        })}
      </section>
      <p className="px-3 pb-3 pt-2 text-xs text-base-content/60">
        {t("live.routing.timeline.hint")}
      </p>
    </div>
  );
}
