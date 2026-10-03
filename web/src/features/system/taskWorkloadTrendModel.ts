import type {
  TaskMeasurementCapabilities,
  TaskWorkloadMetric,
  TaskWorkloadSample,
} from "../../lib/api";

export const WORKLOAD_SERIES = ["pending", "discovered", "processed"] as const;
export type WorkloadSeriesKey = (typeof WORKLOAD_SERIES)[number];

export interface WorkloadPlotDatum {
  index: number;
  label: string;
  sample: TaskWorkloadSample | null;
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
    const data: WorkloadPlotDatum[] = ordered.map((sample, index) => ({
      index,
      label: sample.attemptedAt,
      sample,
    }));
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
