import type {
  TaskMeasurementCapabilities,
  TaskWorkloadCoverageGap,
  TaskWorkloadMetric,
  TaskWorkloadSample,
} from "../../lib/api";

export const WORKLOAD_SERIES = ["pending", "discovered", "processed"] as const;
export type WorkloadSeriesKey = (typeof WORKLOAD_SERIES)[number];

export interface WorkloadPlotDatum {
  index: number;
  label: string;
  sample: TaskWorkloadSample | null;
  runningMarker: number | null;
  failedMarker: number | null;
  skipMarker: number | null;
  [key: string]: unknown;
}

export interface WorkloadSeriesSegment {
  key: string;
  series: WorkloadSeriesKey;
  scope: string;
  range: string;
}

export interface WorkloadChartModel {
  unit: string;
  data: WorkloadPlotDatum[];
  segments: WorkloadSeriesSegment[];
}

const SERIES_LABELS: Record<WorkloadSeriesKey, string> = {
  pending: "待处理量",
  discovered: "本次发现",
  processed: "本次处理",
};

export function workloadSeriesLabel(series: WorkloadSeriesKey): string {
  return SERIES_LABELS[series];
}

export function visibleWorkloadMarkerValue(
  sample: TaskWorkloadSample,
  unit: string,
  visibleSeries: WorkloadSeriesKey[],
): number | null {
  const measuredValues = visibleSeries.flatMap((series) => {
    const metric = metricFor(sample, series);
    return usableMetric(metric) && metric.unit === unit && metric.coverage !== "unknown"
      ? [metric.value]
      : [];
  });
  if (measuredValues.length > 0) return Math.max(...measuredValues);

  const hasMeasuredValueInUnit = WORKLOAD_SERIES.some((series) => {
    const metric = metricFor(sample, series);
    return usableMetric(metric) && metric.unit === unit && metric.coverage !== "unknown";
  });
  return hasMeasuredValueInUnit ? 0 : null;
}

export function workloadSubsetPartition(
  sample: TaskWorkloadSample,
): { processed: number; discoveredRemainder: number; pendingRemainder: number } | null {
  const pending = sample.pending;
  const discovered = sample.discovered;
  const processed = sample.processed;
  if (
    sample.subsetRelation !== "confirmed" ||
    pending == null ||
    discovered == null ||
    processed == null ||
    !usableMetric(pending) ||
    !usableMetric(discovered) ||
    !usableMetric(processed)
  ) {
    return null;
  }
  if (
    pending.unit !== discovered.unit ||
    discovered.unit !== processed.unit ||
    pending.scope !== discovered.scope ||
    discovered.scope !== processed.scope ||
    pending.range !== discovered.range ||
    discovered.range !== processed.range ||
    processed.value < 0 ||
    discovered.value < processed.value ||
    pending.value < discovered.value
  ) {
    return null;
  }
  return {
    processed: processed.value,
    discoveredRemainder: discovered.value - processed.value,
    pendingRemainder: pending.value - discovered.value,
  };
}

function metricFor(
  sample: TaskWorkloadSample | undefined,
  series: WorkloadSeriesKey,
): TaskWorkloadMetric | null {
  return sample?.[series] ?? null;
}

function usableMetric(metric: TaskWorkloadMetric | null): metric is TaskWorkloadMetric & {
  value: number;
} {
  return metric != null && typeof metric.value === "number" && Number.isFinite(metric.value);
}

function metricIdentity(metric: TaskWorkloadMetric): string {
  return `${metric.scope}\u0000${metric.range}`;
}

export function selectWorkloadRunWindow(
  samples: TaskWorkloadSample[],
  coverageGaps: TaskWorkloadCoverageGap[],
  runWindow: number,
  taskKey: string,
): TaskWorkloadSample[] {
  const orderedSamples = [...samples].sort((left, right) => {
    const timestampOrder = Date.parse(left.attemptedAt) - Date.parse(right.attemptedAt);
    return timestampOrder || left.sampleId.localeCompare(right.sampleId);
  });
  const visibleSamples = orderedSamples.slice(-runWindow);
  if (visibleSamples.length === 0) return coverageGapSamples(coverageGaps, taskKey);

  const firstVisibleAt = Date.parse(visibleSamples[0].attemptedAt);
  const lastVisibleAt = Date.parse(visibleSamples[visibleSamples.length - 1].attemptedAt);
  const visibleGaps = coverageGaps.flatMap((gap) => {
    const gapStart = Date.parse(gap.startedAt);
    const gapEnd = gap.finishedAt == null ? lastVisibleAt : Date.parse(gap.finishedAt);
    if (
      !Number.isFinite(gapStart) ||
      !Number.isFinite(gapEnd) ||
      gapEnd < firstVisibleAt ||
      gapStart > lastVisibleAt
    ) {
      return [];
    }
    return coverageGapSamples(
      [{ ...gap, startedAt: new Date(Math.max(gapStart, firstVisibleAt)).toISOString() }],
      taskKey,
    );
  });

  return [...visibleSamples, ...visibleGaps];
}

function coverageGapSamples(
  gaps: TaskWorkloadCoverageGap[],
  taskKey: string,
): TaskWorkloadSample[] {
  return gaps.map((gap) => ({
    sampleId: `coverage-gap:${gap.id}`,
    executionUid: `coverage-gap:${gap.id}`,
    managedRunId: null,
    taskKey,
    triggerKind: "观测覆盖缺口",
    attemptedAt: gap.startedAt,
    actualStartedAt: null,
    finishedAt: gap.finishedAt ?? null,
    status: "unknown",
    reason: gap.reason ?? "工作量观测缺失",
    sequence: 0,
    pending: null,
    discovered: null,
    processed: null,
    subsetRelation: "unknown",
  }));
}

function isConfirmedSkip(sample: TaskWorkloadSample): boolean {
  return sample.status === "skipped" && sample.actualStartedAt == null;
}

function addSkipBridges(
  data: WorkloadPlotDatum[],
  samples: TaskWorkloadSample[],
  unit: string,
  series: WorkloadSeriesKey,
): void {
  let leftIndex = 0;
  while (leftIndex < samples.length) {
    const leftMetric = metricFor(samples[leftIndex], series);
    if (!usableMetric(leftMetric) || leftMetric.unit !== unit) {
      leftIndex += 1;
      continue;
    }
    let rightIndex = leftIndex + 1;
    while (rightIndex < samples.length && isConfirmedSkip(samples[rightIndex])) {
      rightIndex += 1;
    }
    if (rightIndex === leftIndex + 1) {
      leftIndex += 1;
      continue;
    }
    const rightMetric = metricFor(samples[rightIndex], series);
    if (
      usableMetric(rightMetric) &&
      rightMetric.unit === unit &&
      metricIdentity(rightMetric) === metricIdentity(leftMetric)
    ) {
      const key = `bridge_${series}_${leftIndex}_${rightIndex}`;
      data[leftIndex][key] = leftMetric.value;
      data[rightIndex][key] = rightMetric.value;
      leftIndex = rightIndex;
    } else {
      leftIndex = rightIndex;
    }
  }
}

export function buildWorkloadChartModels(
  samples: TaskWorkloadSample[],
  capabilities?: TaskMeasurementCapabilities,
): WorkloadChartModel[] {
  const ordered = [...samples].sort((left, right) => {
    const timestampOrder = Date.parse(left.attemptedAt) - Date.parse(right.attemptedAt);
    return timestampOrder || left.sampleId.localeCompare(right.sampleId);
  });
  const units = Array.from(
    new Set([
      ...ordered.flatMap((sample) =>
        WORKLOAD_SERIES.flatMap((series) => {
          const metric = metricFor(sample, series);
          return metric?.unit ? [metric.unit] : [];
        }),
      ),
      ...WORKLOAD_SERIES.flatMap((series) => {
        const capability = capabilities?.[series];
        return capability?.supported && capability.unit ? [capability.unit] : [];
      }),
    ]),
  );
  if (units.length === 0) units.push("单位未知");

  return units.map((unit) => {
    const data: WorkloadPlotDatum[] = ordered.map((sample, index) => {
      const measuredValues = WORKLOAD_SERIES.flatMap((series) => {
        const metric = metricFor(sample, series);
        return usableMetric(metric) && metric.unit === unit && metric.coverage !== "unknown"
          ? [metric.value]
          : [];
      });
      const statusValue = measuredValues.length > 0 ? Math.max(...measuredValues) : null;
      return {
        index,
        label: sample.attemptedAt,
        sample,
        runningMarker:
          sample.status !== "running" ? null : (statusValue ?? (unit === units[0] ? 0 : null)),
        failedMarker: sample.status === "failed" ? statusValue : null,
        skipMarker: isConfirmedSkip(sample) ? 0 : null,
      };
    });
    const segments: WorkloadSeriesSegment[] = [];
    let nextSegment = 0;

    for (const series of WORKLOAD_SERIES) {
      let activeKey: string | null = null;
      let activeIdentity: string | null = null;
      const closeSegment = () => {
        activeKey = null;
        activeIdentity = null;
      };
      for (let index = 0; index < ordered.length; index += 1) {
        const metric = metricFor(ordered[index], series);
        if (!usableMetric(metric) || metric.unit !== unit || metric.coverage === "unknown") {
          closeSegment();
          continue;
        }
        const identity = metricIdentity(metric);
        if (activeKey == null || activeIdentity !== identity) {
          activeKey = `area_${series}_${nextSegment++}`;
          activeIdentity = identity;
          segments.push({ key: activeKey, series, scope: metric.scope, range: metric.range });
        }
        data[index][activeKey] = metric.value;
      }
    }

    for (const series of WORKLOAD_SERIES) addSkipBridges(data, ordered, unit, series);
    return { unit, data, segments };
  });
}
