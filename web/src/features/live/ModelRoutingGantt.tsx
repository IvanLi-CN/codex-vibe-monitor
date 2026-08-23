import Gantt, { type GanttTask, type GanttViewMode } from "frappe-gantt";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
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
import { ModelRoutingRecordList, modelRoutingRecordsId } from "./ModelRoutingRecordList";
import { modelRoutingKey } from "./modelRoutingIds";

type RoutingTimelineState = "available" | "degraded" | "cooling_down" | "unknown";
type RoutingTimelinePriority = "normal" | "demoted" | "excluded" | "unknown";

interface RoutingGanttBand {
  state: RoutingTimelineState;
  priority: RoutingTimelinePriority;
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
  stateBefore?: string | null;
  stateAfter?: string | null;
}

interface RoutingGanttLane {
  accountId: number;
  label: string;
  model: string;
  state: RoutingTimelineState;
  priority: RoutingTimelinePriority;
  bands: RoutingGanttBand[];
}

interface RoutingGanttData {
  rangeStartMs: number;
  rangeEndMs: number;
  lanes: RoutingGanttLane[];
  attempts: RoutingGanttAttempt[];
  recoveryAttempts: RoutingGanttAttempt[];
}

interface RoutingGanttColors {
  priority: Record<RoutingTimelinePriority, string>;
  attemptSuccess: string;
  attemptFailure: string;
  attemptUnknown: string;
}

interface RoutingFrappeTask extends GanttTask {
  kind: "model" | "lane" | "detail";
  accountId?: number;
  model: string;
}

interface RoutingTimelineGroup {
  model: string;
  accountCount: number;
  recordCount: number;
  timeline: RoutingGanttData;
}

interface RoutingViewSpec {
  step: string;
  normalizedStepMs: number;
  minimumColumnWidth: number;
  columns: number;
}

const WINDOW_DURATION_MS: Record<ModelRoutingLiveWindow, number> = {
  "15m": 15 * 60_000,
  "1h": 60 * 60_000,
  "6h": 6 * 60 * 60_000,
  "24h": 24 * 60 * 60_000,
};

const VIEW_SPECS: Record<ModelRoutingLiveWindow, RoutingViewSpec> = {
  "15m": { step: "6h", normalizedStepMs: 6 * 60 * 60_000, minimumColumnWidth: 96, columns: 5 },
  "1h": { step: "6h", normalizedStepMs: 6 * 60 * 60_000, minimumColumnWidth: 80, columns: 5 },
  "6h": { step: "4h", normalizedStepMs: 4 * 60 * 60_000, minimumColumnWidth: 96, columns: 7 },
  "24h": { step: "4h", normalizedStepMs: 4 * 60 * 60_000, minimumColumnWidth: 112, columns: 7 },
};

const ROUTING_STATES = new Set<RoutingTimelineState>(["available", "degraded", "cooling_down"]);
const ROUTING_PRIORITIES = new Set<RoutingTimelinePriority>(["normal", "demoted", "excluded"]);
const SVG_NS = "http://www.w3.org/2000/svg";
const GANTT_HEADER_HEIGHT = 54;
const GANTT_ROW_HEIGHT = 32;
const MODEL_RECORD_DETAIL_ROWS = 8;
// A one-second offset avoids Frappe treating a midnight end as an all-day task.
const NORMALIZED_START_MS = new Date(2000, 0, 1, 0, 0, 1, 0).getTime();
const NORMALIZED_TIMELINE_DURATION_MS = 24 * 60 * 60_000;

export function availableBandOpacity(callCount: number, maxCallCount: number) {
  if (maxCallCount <= 0) return 0.56;
  const ratio = Math.max(0, Math.min(1, callCount / maxCallCount));
  return 0.3 + ratio * 0.7;
}

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

function routingPriority(value?: string | null): RoutingTimelinePriority | null {
  if (value === "deprioritized") return "demoted";
  return value && ROUTING_PRIORITIES.has(value as RoutingTimelinePriority)
    ? (value as RoutingTimelinePriority)
    : null;
}

export function isControlledRecoveryAttempt(record: ModelRoutingTimelineRecord) {
  if (record.kind !== "attempt") return false;
  if (record.modelRouteStateBefore === "cooling_down") return true;
  return [record.reasonCode, record.action, record.routingSource].some((value) => {
    const normalized = value?.toLowerCase();
    return normalized?.includes("probe") || normalized?.includes("recovery");
  });
}

function clampToRange(value: number, start: number, end: number) {
  return Math.max(start, Math.min(end, value));
}

function appendBand(
  bands: RoutingGanttBand[],
  state: RoutingTimelineState,
  priority: RoutingTimelinePriority,
  startMs: number,
  endMs: number,
) {
  if (!(endMs > startMs)) return;
  const previous = bands.at(-1);
  if (
    previous &&
    previous.state === state &&
    previous.priority === priority &&
    previous.endMs === startMs
  ) {
    previous.endMs = endMs;
    return;
  }
  bands.push({ state, priority, startMs, endMs });
}

interface RoutingPoint {
  atMs: number;
  state?: RoutingTimelineState;
  priority?: RoutingTimelinePriority;
  rank: number;
}

function buildLaneBands(
  account: Pick<ModelRoutingLiveAccount, "state" | "priority" | "changedAt">,
  records: ModelRoutingTimelineRecord[],
  rangeStartMs: number,
  rangeEndMs: number,
) {
  const points: RoutingPoint[] = [];
  const transitions: Array<{
    atMs: number;
    stateBefore: RoutingTimelineState | null;
    stateAfter: RoutingTimelineState | null;
    priorityBefore: RoutingTimelinePriority | null;
    priorityAfter: RoutingTimelinePriority | null;
    cooldownUntilMs: number | null;
  }> = [];
  for (const record of records) {
    const atMs = parseTimestamp(record.occurredAt);
    if (atMs == null || atMs < rangeStartMs || atMs > rangeEndMs) continue;
    const stateAfter = routingState(record.modelRouteStateAfter);
    const priorityAfter = routingPriority(record.modelRoutePriorityAfter);
    if (stateAfter == null && priorityAfter == null) continue;
    transitions.push({
      atMs,
      stateBefore: routingState(record.modelRouteStateBefore),
      stateAfter,
      priorityBefore: routingPriority(record.modelRoutePriorityBefore),
      priorityAfter,
      cooldownUntilMs: parseTimestamp(record.modelRouteCooldownUntil),
    });
  }
  transitions.sort((left, right) => left.atMs - right.atMs);
  const first = transitions[0];
  if (first && (first.stateBefore != null || first.priorityBefore != null)) {
    points.push({
      atMs: rangeStartMs,
      ...(first.stateBefore == null ? {} : { state: first.stateBefore }),
      ...(first.priorityBefore == null ? {} : { priority: first.priorityBefore }),
      rank: 0,
    });
  }
  for (const transition of transitions) {
    points.push({
      atMs: transition.atMs,
      ...(transition.stateAfter == null ? {} : { state: transition.stateAfter }),
      ...(transition.priorityAfter == null ? {} : { priority: transition.priorityAfter }),
      rank: 1,
    });
    if (
      transition.stateAfter === "cooling_down" &&
      transition.cooldownUntilMs != null &&
      transition.cooldownUntilMs > transition.atMs
    ) {
      points.push({
        atMs: clampToRange(transition.cooldownUntilMs, rangeStartMs, rangeEndMs),
        state: "unknown",
        priority: "unknown",
        rank: 2,
      });
    }
  }

  const changedAtMs = parseTimestamp(account.changedAt);
  const currentState = routingState(account.state);
  const currentPriority = routingPriority(account.priority);
  if (changedAtMs != null && (currentState != null || currentPriority != null)) {
    points.push({
      atMs: clampToRange(changedAtMs, rangeStartMs, rangeEndMs),
      ...(currentState == null ? {} : { state: currentState }),
      ...(currentPriority == null ? {} : { priority: currentPriority }),
      rank: 3,
    });
  }

  points.sort((left, right) => left.atMs - right.atMs || left.rank - right.rank);
  const bands: RoutingGanttBand[] = [];
  let cursor = rangeStartMs;
  let activeState: RoutingTimelineState = "unknown";
  let activePriority: RoutingTimelinePriority = "unknown";
  for (const point of points) {
    if (point.atMs > cursor) {
      appendBand(bands, activeState, activePriority, cursor, point.atMs);
    }
    if (point.state != null) activeState = point.state;
    if (point.priority != null) activePriority = point.priority;
    cursor = Math.max(cursor, point.atMs);
  }
  appendBand(bands, activeState, activePriority, cursor, rangeEndMs);
  return bands;
}

function routingLaneLabel(accountId: number, accountDisplayName?: string) {
  return accountDisplayName?.trim() || `API Key #${accountId}`;
}

function routingTaskId(model: string, accountId: number) {
  return `route-${modelRoutingKey(model)}-${accountId}`;
}

function routingModelTaskId(model: string) {
  return `model-${modelRoutingKey(model)}`;
}

function routingDetailTaskId(model: string, index: number) {
  return `detail-${modelRoutingKey(model)}-${index}`;
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

  // Keep lane order stable by API Key id. Dynamic routing priority is represented only by
  // the time-varying Task bands so vertical position cannot be mistaken for route rank.
  const lanes = Array.from(accountMap.values())
    .sort((left, right) => left.accountId - right.accountId)
    .map((account) => {
      const laneRecords = modelRecords.filter((record) => record.accountId === account.accountId);
      return {
        accountId: account.accountId,
        label: routingLaneLabel(account.accountId, account.accountDisplayName),
        model,
        state: routingState(account.state) ?? "unknown",
        priority: routingPriority(account.priority) ?? "unknown",
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
        stateBefore: record.modelRouteStateBefore,
        stateAfter: record.modelRouteStateAfter,
      },
    ];
  });
  const recoveryAttemptIds = new Set(
    modelRecords.filter(isControlledRecoveryAttempt).map((record) => record.id),
  );
  const recoveryAttempts = attempts.filter((attempt) => recoveryAttemptIds.has(attempt.id));

  return { rangeStartMs, rangeEndMs, lanes, attempts, recoveryAttempts };
}

function formatBeijing(value: number, localeTag: string, options: Intl.DateTimeFormatOptions) {
  return new Intl.DateTimeFormat(localeTag, {
    timeZone: "Asia/Shanghai",
    hour12: false,
    ...options,
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

function bandKey(model: string, accountId: number, band: RoutingGanttBand) {
  return JSON.stringify([modelRoutingKey(model), accountId, band.startMs, band.endMs]);
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

export function buildFrappeRoutingTasks(
  timeline: RoutingGanttData,
  normalizedStartMs = NORMALIZED_START_MS,
): RoutingFrappeTask[] {
  const normalizedEndMs = normalizedStartMs + NORMALIZED_TIMELINE_DURATION_MS;
  return timeline.lanes.map((lane) => ({
    id: routingTaskId(lane.model, lane.accountId),
    name: lane.label,
    start: new Date(normalizedStartMs),
    end: new Date(normalizedEndMs),
    progress: 0,
    custom_class: "model-routing-task",
    kind: "lane",
    accountId: lane.accountId,
    model: lane.model,
  }));
}

export function buildFrappeSystemRoutingTasks(
  timelines: RoutingTimelineGroup[],
  normalizedStartMs = NORMALIZED_START_MS,
  expandedModel: string | null = null,
): RoutingFrappeTask[] {
  return timelines.flatMap((group) => {
    const normalizedEndMs = normalizedStartMs + NORMALIZED_TIMELINE_DURATION_MS;
    const modelTask: RoutingFrappeTask = {
      id: routingModelTaskId(group.model),
      name: group.model,
      start: new Date(normalizedStartMs),
      end: new Date(normalizedEndMs),
      progress: 0,
      custom_class: "model-routing-model-task",
      kind: "model",
      model: group.model,
    };
    const laneTasks = buildFrappeRoutingTasks(group.timeline, normalizedStartMs);
    const detailTasks =
      group.model === expandedModel
        ? Array.from(
            { length: MODEL_RECORD_DETAIL_ROWS },
            (_, index): RoutingFrappeTask => ({
              id: routingDetailTaskId(group.model, index),
              name: "\u00a0",
              start: new Date(normalizedStartMs),
              end: new Date(normalizedEndMs),
              progress: 0,
              custom_class: "model-routing-detail-task",
              kind: "detail",
              model: group.model,
            }),
          )
        : [];
    return [modelTask, ...laneTasks, ...detailTasks];
  });
}

function modelRecordDetailTop(timelines: RoutingTimelineGroup[], expandedModel: string | null) {
  if (!expandedModel) return null;
  let completedRows = 0;
  for (const group of timelines) {
    completedRows += 1 + group.timeline.lanes.length;
    if (group.model === expandedModel) {
      return GANTT_HEADER_HEIGHT + completedRows * GANTT_ROW_HEIGHT;
    }
  }
  return null;
}

function svgElement<K extends keyof SVGElementTagNameMap>(
  name: K,
  attributes: Record<string, string>,
) {
  const element = document.createElementNS(SVG_NS, name);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, value);
  return element;
}

function appendSvgTitle(element: SVGElement, label: string) {
  const title = svgElement("title", {});
  title.textContent = label;
  element.appendChild(title);
}

function truncateSvgLaneLabel(label: string, measuredWidth: number, availableWidth: number) {
  if (measuredWidth <= availableWidth || availableWidth <= 0) return label;
  const characterBudget = Math.max(6, Math.floor(label.length * (availableWidth / measuredWidth)));
  return `${label.slice(0, Math.max(1, characterBudget - 1)).trimEnd()}…`;
}

function bindSvgAction(element: SVGElement, action: () => void) {
  element.setAttribute("role", "button");
  element.setAttribute("tabindex", "0");
  element.addEventListener("click", (event) => {
    event.stopPropagation();
    action();
  });
  element.addEventListener("keydown", (event) => {
    if (event instanceof KeyboardEvent && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      event.stopPropagation();
      action();
    }
  });
}

function decorateTimelineSvg({
  host,
  timeline,
  stateLabels,
  colors,
  availableCounts,
  maxAvailableCount,
  totalAvailableCount,
  localeTag,
  attemptLabel,
  retryLabel,
  unknownLabel,
  allocationLabel,
  priorityLabels,
  onOpenAccount,
  onOpenInvocation,
}: {
  host: HTMLElement;
  timeline: RoutingGanttData;
  stateLabels: Record<RoutingTimelineState, string>;
  colors: RoutingGanttColors;
  availableCounts: Map<string, number>;
  maxAvailableCount: number;
  totalAvailableCount: number;
  localeTag: string;
  attemptLabel: string;
  retryLabel: (index: number) => string;
  unknownLabel: string;
  allocationLabel: (count: number, percent: number) => string;
  priorityLabels: Record<RoutingTimelinePriority, string>;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
}) {
  const svg = host.querySelector<SVGSVGElement>("svg.gantt");
  if (!svg) return;
  svg.setAttribute("data-testid", `model-routing-svg-${timeline.lanes[0]?.model ?? "empty"}`);
  svg.setAttribute("aria-label", timeline.lanes[0]?.model ?? "model routing");

  const rangeDuration = timeline.rangeEndMs - timeline.rangeStartMs;
  for (const lane of timeline.lanes) {
    const taskId = routingTaskId(lane.model, lane.accountId);
    const wrapper = Array.from(svg.querySelectorAll<SVGGElement>(".bar-wrapper")).find(
      (candidate) => candidate.getAttribute("data-id") === taskId,
    );
    const baseBar = wrapper?.querySelector<SVGRectElement>(".bar");
    const barGroup = wrapper?.querySelector<SVGGElement>(".bar-group");
    const label = wrapper?.querySelector<SVGTextElement>(".bar-label");
    if (!wrapper || !baseBar || !barGroup || !label || rangeDuration <= 0) continue;

    const x = Number(baseBar.getAttribute("x"));
    const y = Number(baseBar.getAttribute("y"));
    const width = Number(baseBar.getAttribute("width"));
    const height = Number(baseBar.getAttribute("height"));
    baseBar.setAttribute("fill", "transparent");
    baseBar.setAttribute("stroke", "transparent");
    label.setAttribute("x", "8");
    label.setAttribute("text-anchor", "start");
    const fullLabel = lane.label;
    const availableLabelWidth = Math.max(0, x - 16);
    const measuredLabelWidth = label.getComputedTextLength();
    const visibleLabel = truncateSvgLaneLabel(fullLabel, measuredLabelWidth, availableLabelWidth);
    if (visibleLabel !== fullLabel) {
      label.textContent = visibleLabel;
      appendSvgTitle(label, fullLabel);
    }
    wrapper.setAttribute("data-testid", `model-routing-lane-${lane.model}-${lane.accountId}`);

    const segmentGroup = svgElement("g", { class: "model-routing-segments" });
    for (const band of lane.bands) {
      const startRatio = (band.startMs - timeline.rangeStartMs) / rangeDuration;
      const endRatio = (band.endMs - timeline.rangeStartMs) / rangeDuration;
      const bandX = x + Math.max(0, startRatio) * width;
      const bandWidth = Math.max(1.5, (Math.min(1, endRatio) - Math.max(0, startRatio)) * width);
      const callCount =
        band.state === "available"
          ? (availableCounts.get(bandKey(lane.model, lane.accountId, band)) ?? 0)
          : 0;
      const allocationPercent =
        totalAvailableCount > 0 ? Math.round((callCount / totalAvailableCount) * 100) : 0;
      const baseText = `${lane.label} · ${priorityLabels[band.priority]} · ${
        stateLabels[band.state]
      } · ${formatBeijingRange(band.startMs, band.endMs, localeTag)}`;
      const accessibleText =
        band.state === "available"
          ? `${baseText} · ${allocationLabel(callCount, allocationPercent)}`
          : baseText;
      const rect = svgElement("rect", {
        x: String(bandX),
        y: String(y),
        width: String(bandWidth),
        height: String(height),
        rx: "2",
        class: `model-routing-band model-routing-band--${
          band.state === "unknown" || band.priority === "unknown" ? "unknown" : band.state
        }`,
        fill: colors.priority[band.priority],
        "aria-label": accessibleText,
        "data-routing-state": band.state,
        "data-routing-priority": band.priority,
      });
      if (band.state === "available") {
        rect.setAttribute(
          "fill-opacity",
          String(availableBandOpacity(callCount, maxAvailableCount)),
        );
      }
      if (band.state === "unknown" || band.priority === "unknown") {
        rect.setAttribute("stroke-dasharray", "3 2");
      }
      appendSvgTitle(rect, accessibleText);
      bindSvgAction(rect, () => onOpenAccount(lane.accountId, lane.model));
      segmentGroup.appendChild(rect);
    }

    const laneAttempts = timeline.recoveryAttempts.filter(
      (attempt) => attempt.accountId === lane.accountId,
    );
    for (const attempt of laneAttempts) {
      const ratio = (attempt.occurredAtMs - timeline.rangeStartMs) / rangeDuration;
      const centerX = x + Math.max(0.006, Math.min(0.994, ratio)) * width;
      const centerY = y - 4;
      const successful = attempt.httpStatus != null && attempt.httpStatus < 400;
      const failed = attempt.httpStatus != null && attempt.httpStatus >= 400;
      const result = attempt.httpStatus
        ? `HTTP ${attempt.httpStatus}`
        : attempt.status || unknownLabel;
      const latency =
        attempt.totalLatencyMs != null ? ` · ${Math.round(attempt.totalLatencyMs)} ms` : "";
      const retry =
        (attempt.retryIndex ?? 0) > 0 ? ` · ${retryLabel(attempt.retryIndex ?? 0)}` : "";
      const accessibleText = `${lane.label} · ${attemptLabel} · ${formatBeijingRange(
        attempt.occurredAtMs,
        attempt.occurredAtMs,
        localeTag,
      )} · ${result}${latency}${retry}`;
      const color = successful
        ? colors.attemptSuccess
        : failed
          ? colors.attemptFailure
          : colors.attemptUnknown;
      const marker = svgElement("polygon", {
        points: `${centerX},${centerY - 3} ${centerX + 3},${centerY} ${centerX},${centerY + 3} ${centerX - 3},${centerY}`,
        class: "model-routing-attempt",
        fill: color,
        stroke: "var(--g-header-background)",
        "stroke-width": "1",
        "aria-label": accessibleText,
        "data-attempt-id": attempt.id,
      });
      appendSvgTitle(marker, accessibleText);
      if (attempt.invokeId) bindSvgAction(marker, () => onOpenInvocation(attempt.invokeId ?? ""));
      segmentGroup.appendChild(marker);
    }

    barGroup.insertBefore(segmentGroup, label);
  }
}

function decorateSystemRoutingSvg({
  host,
  timelines,
  modelSummaryLabel,
  modelToggleLabel,
  expandedModel,
  laneHeaderLabel,
  onToggleModelRecords,
  ...options
}: Omit<Parameters<typeof decorateTimelineSvg>[0], "timeline"> & {
  timelines: RoutingTimelineGroup[];
  modelSummaryLabel: (model: string, accountCount: number, recordCount: number) => string;
  modelToggleLabel: (model: string) => string;
  expandedModel: string | null;
  laneHeaderLabel: string;
  onToggleModelRecords: (model: string) => void;
}) {
  const svg = host.querySelector<SVGSVGElement>("svg.gantt");
  if (!svg) return;

  for (const group of timelines) {
    decorateTimelineSvg({ host, timeline: group.timeline, ...options });
    const wrapper = Array.from(svg.querySelectorAll<SVGGElement>(".bar-wrapper")).find(
      (candidate) => candidate.getAttribute("data-id") === routingModelTaskId(group.model),
    );
    const baseBar = wrapper?.querySelector<SVGRectElement>(".bar");
    const barGroup = wrapper?.querySelector<SVGGElement>(".bar-group");
    const label = wrapper?.querySelector<SVGTextElement>(".bar-label");
    if (!wrapper || !baseBar || !barGroup || !label) continue;

    const x = Number(baseBar.getAttribute("x"));
    const y = Number(baseBar.getAttribute("y"));
    const width = Number(baseBar.getAttribute("width"));
    const height = Number(baseBar.getAttribute("height"));
    wrapper.setAttribute("data-testid", `model-routing-model-group-${group.model}`);
    baseBar.setAttribute("x", "0");
    baseBar.setAttribute("width", String(x + width));
    label.setAttribute("x", "8");
    label.setAttribute("text-anchor", "start");
    const toggleLabel = modelToggleLabel(group.model);
    wrapper.setAttribute("aria-label", toggleLabel);
    wrapper.setAttribute("aria-expanded", String(expandedModel === group.model));
    if (expandedModel === group.model) {
      wrapper.setAttribute("aria-controls", modelRoutingRecordsId(group.model));
    } else {
      wrapper.removeAttribute("aria-controls");
    }
    appendSvgTitle(wrapper, toggleLabel);
    bindSvgAction(wrapper, () => onToggleModelRecords(group.model));
    const countLabel = svgElement("text", {
      x: String(x + width - 24),
      y: String(y + height / 2),
      class: "model-routing-model-count",
      "text-anchor": "end",
    });
    countLabel.textContent = modelSummaryLabel(group.model, group.accountCount, group.recordCount);
    barGroup.appendChild(countLabel);
  }

  const laneHeader = svgElement("text", {
    x: "8",
    y: "34",
    class: "model-routing-lane-header",
  });
  laneHeader.textContent = laneHeaderLabel;
  svg.appendChild(laneHeader);

  svg.setAttribute("data-testid", "model-routing-svg-system");
  svg.setAttribute("aria-label", "model routing");
}

function ModelRoutingSvgChart({
  timelines,
  records,
  window,
  localeTag,
  stateLabels,
  colors,
  availableCounts,
  maxAvailableCount,
  totalAvailableCount,
  attemptLabel,
  retryLabel,
  unknownLabel,
  allocationLabel,
  priorityLabels,
  modelSummaryLabel,
  modelToggleLabel,
  expandedModel,
  laneHeaderLabel,
  onOpenAccount,
  onOpenInvocation,
  onToggleModelRecords,
}: {
  timelines: RoutingTimelineGroup[];
  records: ModelRoutingTimelineRecord[];
  window: ModelRoutingLiveWindow;
  localeTag: string;
  stateLabels: Record<RoutingTimelineState, string>;
  colors: RoutingGanttColors;
  availableCounts: Map<string, number>;
  maxAvailableCount: number;
  totalAvailableCount: number;
  attemptLabel: string;
  retryLabel: (index: number) => string;
  unknownLabel: string;
  allocationLabel: (count: number, percent: number) => string;
  modelSummaryLabel: (model: string, accountCount: number, recordCount: number) => string;
  modelToggleLabel: (model: string) => string;
  expandedModel: string | null;
  laneHeaderLabel: string;
  priorityLabels: Record<RoutingTimelinePriority, string>;
  onOpenAccount: (accountId: number, model: string) => void;
  onOpenInvocation: (invokeId: string) => void;
  onToggleModelRecords: (model: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [detailPortalTarget, setDetailPortalTarget] = useState<HTMLDivElement | null>(null);
  const [hostWidth, setHostWidth] = useState(0);
  const range = timelines[0]?.timeline;
  const detailTop = modelRecordDetailTop(timelines, expandedModel);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const updateWidth = () => setHostWidth(host.clientWidth);
    updateWidth();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateWidth);
    observer.observe(host);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || hostWidth <= 0 || timelines.length === 0 || !range) return;
    setDetailPortalTarget(null);
    host.replaceChildren();
    const spec = VIEW_SPECS[window];
    const compact = host.clientWidth < 640;
    const timelineIntervals = Math.max(1, spec.columns - 1);
    const labelIntervals = compact ? 2 : 1;
    const fittedColumnWidth = Math.floor(hostWidth / (timelineIntervals + labelIntervals));
    const columnWidth = compact
      ? Math.max(44, fittedColumnWidth)
      : Math.max(spec.minimumColumnWidth, fittedColumnWidth);
    const normalizedTimelineStartMs = NORMALIZED_START_MS + spec.normalizedStepMs * labelIntervals;
    const tasks = buildFrappeSystemRoutingTasks(
      timelines,
      normalizedTimelineStartMs,
      expandedModel,
    );
    const compactLabelStride = window === "24h" ? 3 : window === "6h" || window === "1h" ? 2 : 1;
    const toObservedMs = (normalizedDate: Date) =>
      range.rangeStartMs +
      ((normalizedDate.getTime() - normalizedTimelineStartMs) / NORMALIZED_TIMELINE_DURATION_MS) *
        WINDOW_DURATION_MS[window];
    const viewMode: GanttViewMode = {
      name: `routing-${window}`,
      padding: ["0h", "0h"],
      step: spec.step,
      date_format: "YYYY-MM-DD HH:mm",
      lower_text: (date) => {
        if (date.getTime() < normalizedTimelineStartMs) return "";
        const normalizedIndex = Math.round(
          (date.getTime() - normalizedTimelineStartMs) /
            (NORMALIZED_TIMELINE_DURATION_MS / (spec.columns - 1)),
        );
        if (compact && normalizedIndex % compactLabelStride !== 0) return "";
        return formatBeijing(toObservedMs(date), localeTag, {
          hour: "2-digit",
          minute: "2-digit",
        });
      },
      upper_text: (date, previous) => {
        if (date.getTime() < normalizedTimelineStartMs) return "\u00a0";
        const currentMs = toObservedMs(date);
        const previousMs = previous ? toObservedMs(previous) : null;
        const currentDate = formatBeijing(currentMs, localeTag, {
          month: "2-digit",
          day: "2-digit",
        });
        const previousDate =
          previousMs == null
            ? null
            : formatBeijing(previousMs, localeTag, {
                month: "2-digit",
                day: "2-digit",
              });
        return currentDate === previousDate ? "" : currentDate;
      },
      upper_text_frequency: Math.max(1, Math.round(24 / Number.parseInt(spec.step, 10))),
    };
    new Gantt(host, tasks, {
      view_mode: viewMode.name,
      view_modes: [viewMode],
      column_width: columnWidth,
      bar_height: 22,
      bar_corner_radius: 2,
      padding: 10,
      upper_header_height: 20,
      lower_header_height: 24,
      container_height: "auto",
      infinite_padding: false,
      lines: "both",
      holidays: {},
      readonly: true,
      today_button: false,
      scroll_to: "start",
      popup: false,
      on_click: (task) => {
        const routingTask = task as RoutingFrappeTask;
        if (routingTask.kind === "lane" && routingTask.accountId != null) {
          onOpenAccount(routingTask.accountId, routingTask.model);
        }
      },
    });
    let portalTarget: HTMLDivElement | null = null;
    if (expandedModel && detailTop != null) {
      const container = host.querySelector<HTMLElement>(".gantt-container");
      if (container) {
        portalTarget = document.createElement("div");
        portalTarget.className = "model-routing-records-slot";
        portalTarget.style.top = `${detailTop}px`;
        portalTarget.style.height = `${MODEL_RECORD_DETAIL_ROWS * GANTT_ROW_HEIGHT}px`;
        container.appendChild(portalTarget);
        setDetailPortalTarget(portalTarget);
      }
    }
    const frame = requestAnimationFrame(() =>
      decorateSystemRoutingSvg({
        host,
        timelines,
        stateLabels,
        colors,
        availableCounts,
        maxAvailableCount,
        totalAvailableCount,
        localeTag,
        attemptLabel,
        retryLabel,
        unknownLabel,
        allocationLabel,
        priorityLabels,
        modelSummaryLabel,
        modelToggleLabel,
        expandedModel,
        laneHeaderLabel,
        onOpenAccount,
        onOpenInvocation,
        onToggleModelRecords,
      }),
    );
    return () => {
      cancelAnimationFrame(frame);
      portalTarget?.remove();
      host.replaceChildren();
    };
  }, [
    allocationLabel,
    attemptLabel,
    availableCounts,
    colors,
    detailTop,
    expandedModel,
    laneHeaderLabel,
    localeTag,
    maxAvailableCount,
    modelToggleLabel,
    modelSummaryLabel,
    onOpenAccount,
    onOpenInvocation,
    onToggleModelRecords,
    priorityLabels,
    range,
    retryLabel,
    stateLabels,
    timelines,
    totalAvailableCount,
    unknownLabel,
    window,
    hostWidth,
  ]);

  return (
    <>
      <div
        ref={hostRef}
        className="model-routing-frappe-gantt"
        data-testid="model-routing-gantt-chart-system"
      />
      {detailPortalTarget && expandedModel
        ? createPortal(
            <ModelRoutingRecordList
              model={expandedModel}
              records={records}
              onOpenAccount={onOpenAccount}
              onOpenInvocation={onOpenInvocation}
            />,
            detailPortalTarget,
          )
        : null}
    </>
  );
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
  const [expandedModel, setExpandedModel] = useState<string | null>(null);
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
  const colors = useMemo<RoutingGanttColors>(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    return {
      priority: {
        normal: status.success,
        demoted: metricAccent("totalCost", themeMode),
        excluded: status.failure,
        unknown: withOpacity(base.axisText, 0.25),
      },
      attemptSuccess: status.success,
      attemptFailure: status.failure,
      attemptUnknown: base.axisText,
    };
  }, [themeMode]);
  const stateLabels = useMemo<Record<RoutingTimelineState, string>>(
    () => ({
      available: t("live.routing.states.available"),
      degraded: t("live.routing.states.degraded"),
      cooling_down: t("live.routing.states.cooling_down"),
      unknown: t("live.routing.states.unknown"),
    }),
    [t],
  );
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
  const toggleModelRecords = useCallback((model: string) => {
    setExpandedModel((current) => (current === model ? null : model));
  }, []);

  useEffect(() => {
    if (expandedModel && !timelines.some((timeline) => timeline.model === expandedModel)) {
      setExpandedModel(null);
    }
  }, [expandedModel, timelines]);

  if (timelines.length === 0) {
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
        {(["normal", "demoted", "excluded", "unknown"] as RoutingTimelinePriority[]).map(
          (priority) => (
            <span key={priority} className="inline-flex items-center gap-1.5">
              <span
                className={`h-2.5 w-2.5 rounded-sm ${priority === "unknown" ? "border border-dashed" : ""}`}
                style={
                  priority === "unknown"
                    ? { borderColor: colors.attemptUnknown }
                    : { backgroundColor: colors.priority[priority] }
                }
                aria-hidden
              />
              {t(`live.routing.modelPriority.${priority}`)}
            </span>
          ),
        )}
        <span className="inline-flex items-center gap-1.5">
          <span
            className="h-2 w-2 rotate-45 border border-base-100"
            style={{ backgroundColor: colors.attemptSuccess }}
            aria-hidden
          />
          {t("live.routing.timeline.attempt")}
        </span>
      </div>
      <ModelRoutingSvgChart
        timelines={timelines}
        records={records}
        window={window}
        localeTag={localeTag}
        stateLabels={stateLabels}
        colors={colors}
        availableCounts={availableCallStats.counts}
        maxAvailableCount={availableCallStats.max}
        totalAvailableCount={availableCallStats.total}
        attemptLabel={t("live.routing.timeline.attempt")}
        retryLabel={(index) => t("live.routing.timeline.retry", { index })}
        unknownLabel={t("live.routing.record.unknown")}
        allocationLabel={(count, percent) =>
          t("live.routing.timeline.availableAllocation", { count, percent })
        }
        priorityLabels={{
          normal: t("live.routing.modelPriority.normal"),
          demoted: t("live.routing.modelPriority.demoted"),
          excluded: t("live.routing.modelPriority.excluded"),
          unknown: t("live.routing.modelPriority.unknown"),
        }}
        modelSummaryLabel={(selectedModel, accountCount, recordCount) =>
          `${t("live.routing.accountsCount", { count: accountCount })} · ${t(
            "live.routing.modelRecordsCount",
            { count: recordCount },
          )} · ${t(
            expandedModel === selectedModel
              ? "live.routing.model.collapse"
              : "live.routing.model.expand",
          )}`
        }
        modelToggleLabel={(selectedModel) =>
          t(
            expandedModel === selectedModel
              ? "live.routing.model.collapseLabel"
              : "live.routing.model.expandLabel",
            { model: selectedModel },
          )
        }
        expandedModel={expandedModel}
        laneHeaderLabel={t("live.routing.timeline.lane")}
        onOpenAccount={onOpenAccount}
        onOpenInvocation={onOpenInvocation}
        onToggleModelRecords={toggleModelRecords}
      />
    </div>
  );
}
