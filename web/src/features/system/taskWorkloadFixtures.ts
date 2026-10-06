import type {
  RetentionBacklogTrendPoint,
  TaskWorkloadMetric,
  TaskWorkloadSample,
  TaskWorkloadTrend,
} from "../../lib/api";

const HOUR_MS = 60 * 60 * 1000;
const BACKLOG_HOURS = 7 * 24;
const WORK_SCOPE = "expired_invocations:retention_policy:7days";
const WORK_RANGE = "expired_at < observed_at - retention 7 days";

function stableVariation(index: number, salt: number): number {
  const value = Math.sin(index * 12.9898 + salt * 78.233) * 43_758.5453;
  return (value - Math.floor(value)) * 2 - 1;
}

export interface RetentionWorkloadFixture {
  samples: TaskWorkloadSample[];
  backlog: RetentionBacklogTrendPoint[];
  trend: TaskWorkloadTrend;
}

export interface RetentionWorkloadFixtureOptions {
  taskKey?: string;
  nowMs: number;
  sampleCount: number;
  intervalMs?: number;
  finalPending?: number;
  skippedIndices?: number[];
  zeroCommitFailureIndices?: number[];
  maxProcessedPerRun?: number;
}

function rowMetric(
  value: number,
  unit: string,
  coverage: string,
  observedAt: string,
): TaskWorkloadMetric {
  return {
    value,
    unit,
    scope: WORK_SCOPE,
    range: WORK_RANGE,
    observedAt,
    coverage,
  };
}

function processingForRun(index: number, intervalMs: number): number {
  const hourlyCapacity =
    1_850 +
    330 * Math.sin(index * 0.43) +
    210 * Math.sin(index * 0.17 + 1.2) +
    460 * stableVariation(index, 1) +
    (index % 9 === 0 ? -260 : 0);
  return Math.max(0, Math.round((hourlyCapacity * intervalMs) / HOUR_MS));
}

function arrivalsForRun(index: number, intervalMs: number): number {
  const hourlyArrivals =
    520 +
    170 * Math.sin(index * 0.29 + 0.8) +
    110 * Math.sin(index * 0.71) +
    220 * stableVariation(index, 2);
  return Math.max(0, Math.round((hourlyArrivals * intervalMs) / HOUR_MS));
}

function regressionSlope(samples: TaskWorkloadSample[]): number | null {
  if (samples.length < 2) return null;
  const start = Date.parse(samples[0].pending?.observedAt ?? "");
  if (!Number.isFinite(start)) return null;
  const points = samples.map((sample) => ({
    x: (Date.parse(sample.pending?.observedAt ?? "") - start) / 1000,
    y: sample.pending?.value ?? Number.NaN,
  }));
  if (points.some((point) => !Number.isFinite(point.x) || !Number.isFinite(point.y))) return null;
  const meanX = points.reduce((sum, point) => sum + point.x, 0) / points.length;
  const meanY = points.reduce((sum, point) => sum + point.y, 0) / points.length;
  const numerator = points.reduce((sum, point) => sum + (point.x - meanX) * (point.y - meanY), 0);
  const denominator = points.reduce((sum, point) => sum + (point.x - meanX) ** 2, 0);
  return denominator > 0 ? numerator / denominator : null;
}

function buildBacklog(nowMs: number, finalPending: number): RetentionBacklogTrendPoint[] {
  const intervals = Array.from({ length: BACKLOG_HOURS - 1 }, (_, index) => {
    const dailyCycle = Math.sin(((index % 24) * Math.PI * 2) / 24);
    const arrivals = Math.round(520 + 145 * dailyCycle + 85 * Math.sin(index * 0.59 + 0.8));
    const processed = Math.round(1_850 + 290 * dailyCycle + 175 * Math.sin(index * 0.47 + 1.2));
    return {
      arrivals,
      processed,
      netDrain: Math.max(0, processed - arrivals),
    };
  });
  let invocationCount = finalPending + intervals.reduce((sum, item) => sum + item.netDrain, 0);
  let maxOverdueSeconds =
    (4 * HOUR_MS) / 1000 +
    intervals.reduce((sum, item) => sum + (item.netDrain / item.processed) * (HOUR_MS / 1000), 0);

  return Array.from({ length: BACKLOG_HOURS }, (_, index) => {
    const bucket = new Date(nowMs - (BACKLOG_HOURS - 1 - index) * HOUR_MS);
    bucket.setUTCMinutes(0, 0, 0);
    const observationAt = bucket.getTime() + 55 * 60_000;
    const missing = index % 23 === 0 || observationAt > nowMs;
    const point: RetentionBacklogTrendPoint = {
      bucketStart: bucket.toISOString(),
      state: missing ? "missing" : "observed",
      observedAt: missing ? null : new Date(observationAt).toISOString(),
      invocationCount: missing ? null : Math.max(0, Math.round(invocationCount)),
      maxOverdueSeconds: missing ? null : Math.max(0, Math.round(maxOverdueSeconds)),
      retentionDays: 7,
      cutoff: new Date(bucket.getTime() - 7 * 24 * HOUR_MS).toISOString(),
      sourceMaxInvocationId: missing ? null : 2_579_364 + index * 1_184,
    };
    if (index < intervals.length) {
      const interval = intervals[index];
      invocationCount = Math.max(0, invocationCount - interval.netDrain);
      maxOverdueSeconds = Math.max(
        0,
        maxOverdueSeconds - (interval.netDrain / interval.processed) * (HOUR_MS / 1000),
      );
    }
    return point;
  });
}

export function buildRetentionWorkloadFixture({
  taskKey = "retention_archive",
  nowMs,
  sampleCount,
  intervalMs = HOUR_MS,
  finalPending = 2_544,
  skippedIndices = [],
  zeroCommitFailureIndices = [],
  maxProcessedPerRun = Number.POSITIVE_INFINITY,
}: RetentionWorkloadFixtureOptions): RetentionWorkloadFixture {
  const skipped = new Set(skippedIndices);
  const zeroCommitFailures = new Set(zeroCommitFailureIndices);
  const arrivalsByRun = Array.from({ length: sampleCount }, (_, index) =>
    arrivalsForRun(index, intervalMs),
  );
  const processedCounts = Array.from({ length: sampleCount }, (_, index) => {
    if (skipped.has(index) || zeroCommitFailures.has(index)) return 0;
    const processed = Math.min(processingForRun(index, intervalMs), maxProcessedPerRun);
    return index % 19 === 0 ? Math.floor(processed * 0.55) : processed;
  });
  const pendingBeforeFirst = Math.max(
    finalPending,
    finalPending +
      processedCounts.slice(0, -1).reduce((sum, count) => sum + count, 0) -
      arrivalsByRun.slice(0, -1).reduce((sum, count) => sum + count, 0),
  );
  let pendingPopulation = pendingBeforeFirst;

  const samples = Array.from({ length: sampleCount }, (_, index) => {
    const attemptedAtMs = nowMs - 52_000 - (sampleCount - 1 - index) * intervalMs;
    if (index > 0) pendingPopulation += arrivalsByRun[index - 1];
    const attemptedAt = new Date(attemptedAtMs).toISOString();
    const isSkipped = skipped.has(index);
    const zeroCommitFailure = zeroCommitFailures.has(index);
    const failed = zeroCommitFailure || index % 19 === 0;
    const processed = Math.min(processedCounts[index], pendingPopulation);
    const discovered = Math.min(
      pendingPopulation,
      Math.max(
        processed,
        Math.round(processingForRun(index, intervalMs) * (1.08 + stableVariation(index, 3) * 0.24)),
      ),
    );
    const sample: TaskWorkloadSample = {
      sampleId: `fixture:${taskKey}:${attemptedAtMs}`,
      executionUid: `fixture-execution-${taskKey}-${attemptedAtMs}`,
      managedRunId: index + 1,
      taskKey,
      triggerKind: index % 25 === 0 ? "manual" : "interval",
      attemptedAt,
      actualStartedAt: isSkipped ? null : new Date(attemptedAtMs + 4_000).toISOString(),
      finishedAt: isSkipped ? null : new Date(attemptedAtMs + 52_000).toISOString(),
      durationMs: isSkipped ? null : 48_000,
      status: isSkipped ? "skipped" : failed ? "failed" : "success",
      reason: isSkipped
        ? "资源准入确认跳过"
        : zeroCommitFailure
          ? "本次没有提交归档批次"
          : failed
            ? "部分归档提交后遇到可恢复错误"
            : null,
      sequence: 2,
      pending: null,
      discovered: null,
      processed: null,
      subsetRelation: "unknown",
    };

    if (!isSkipped) {
      sample.pending = rowMetric(pendingPopulation, "invocation rows", "exact", attemptedAt);
      sample.discovered =
        index % 29 === 6 ? null : rowMetric(discovered, "invocation rows", "window", attemptedAt);
      sample.processed = rowMetric(processed, "invocation rows", "window", attemptedAt);
      sample.subsetRelation = sample.discovered ? "confirmed" : "unknown";
      pendingPopulation = Math.max(0, pendingPopulation - processed);
    }
    return sample;
  });

  const latestPendingSample = [...samples]
    .reverse()
    .find((sample) => sample.pending?.value != null && sample.pending.coverage === "exact");
  const latestProcessedSample = [...samples]
    .reverse()
    .find((sample) => sample.processed?.value != null && sample.finishedAt != null);
  const latestPending = latestPendingSample?.pending ?? null;
  const latestProcessed = latestProcessedSample?.processed ?? null;
  const completeAttempts = samples
    .filter(
      (sample) =>
        sample.actualStartedAt != null &&
        sample.finishedAt != null &&
        sample.processed?.value != null &&
        sample.processed.coverage === "window",
    )
    .slice(-20);
  const processingSpanMs =
    completeAttempts.length > 1
      ? Date.parse(completeAttempts.at(-1)?.finishedAt ?? "") -
        Date.parse(completeAttempts[0].actualStartedAt ?? "")
      : 0;
  const processingTotal = completeAttempts.reduce(
    (sum, sample) => sum + (sample.processed?.value ?? 0),
    0,
  );
  const processingRatePerSecond =
    processingSpanMs > 0 ? processingTotal / (processingSpanMs / 1000) : null;

  const latestObservationMs = Date.parse(latestPending?.observedAt ?? "");
  const recentPendingSamples = latestPendingSample
    ? samples
        .filter((sample) => {
          const observedAt = Date.parse(sample.pending?.observedAt ?? "");
          return (
            sample.pending?.value != null &&
            sample.pending.coverage === "exact" &&
            sample.pending.unit === latestPending?.unit &&
            sample.pending.scope === latestPending?.scope &&
            sample.pending.range === latestPending?.range &&
            observedAt >= latestObservationMs - 24 * HOUR_MS
          );
        })
        .slice(-20)
    : [];
  const slope = regressionSlope(recentPendingSamples);
  const latestIsFresh =
    Number.isFinite(latestObservationMs) &&
    nowMs - latestObservationMs <= Math.max(intervalMs * 2, 120_000);
  const slopeSpanMs =
    recentPendingSamples.length > 1
      ? Date.parse(recentPendingSamples.at(-1)?.pending?.observedAt ?? "") -
        Date.parse(recentPendingSamples[0].pending?.observedAt ?? "")
      : 0;
  let clearanceEstimateReason = "insufficient_samples";
  let clearanceEta: string | null = null;
  if (!latestIsFresh) {
    clearanceEstimateReason = "stale_observation";
  } else if (latestPending?.value === 0) {
    clearanceEstimateReason = "cleared";
  } else if (recentPendingSamples.length < 5) {
    clearanceEstimateReason = "insufficient_samples";
  } else if (slopeSpanMs < 60_000) {
    clearanceEstimateReason = "insufficient_span";
  } else if (slope == null || slope >= 0 || latestPending?.value == null) {
    clearanceEstimateReason = "no_net_backlog_decline";
  } else {
    clearanceEstimateReason = "estimated";
    clearanceEta = new Date(
      latestObservationMs + (latestPending.value / -slope) * 1000,
    ).toISOString();
  }

  const trend: TaskWorkloadTrend = {
    revision: sampleCount,
    coverage: "recorded",
    samples,
    latestPending,
    latestProcessed,
    latestObservedAt: latestPending?.observedAt ?? latestProcessed?.observedAt ?? null,
    processingRatePerSecond,
    processingRateWindow:
      completeAttempts.length > 1
        ? `${completeAttempts.length} 次 · 跨 ${Math.round(processingSpanMs / HOUR_MS)} 小时`
        : null,
    clearanceEta,
    clearanceEstimateWindow: recentPendingSamples.length > 0 ? "最近 24 小时" : null,
    clearanceEstimateCoverage:
      recentPendingSamples.length > 0 ? `${recentPendingSamples.length} 个准确快照` : null,
    clearanceEstimateReason,
  };
  return {
    samples,
    backlog: buildBacklog(nowMs, finalPending),
    trend,
  };
}
