import { describe, expect, it } from "vitest";
import type {
  TaskMeasurementCapabilities,
  TaskWorkloadMetric,
  TaskWorkloadSample,
} from "../../lib/api";
import { buildWorkloadChartModels, workloadSubsetPartition } from "./taskWorkloadTrendModel";

function metric(value: number, range = "source<=10", unit = "rows"): TaskWorkloadMetric {
  return {
    value,
    unit,
    scope: "source",
    range,
    observedAt: "2026-10-03T00:00:00.000Z",
    coverage: "exact",
  };
}

function sample(index: number, overrides: Partial<TaskWorkloadSample> = {}): TaskWorkloadSample {
  return {
    sampleId: `sample-${index}`,
    executionUid: `execution-${index}`,
    managedRunId: index,
    taskKey: "retention_archive",
    triggerKind: "interval",
    attemptedAt: new Date(Date.parse("2026-10-03T00:00:00.000Z") + index * 60_000).toISOString(),
    actualStartedAt: "2026-10-03T00:00:01.000Z",
    finishedAt: "2026-10-03T00:00:10.000Z",
    status: "success",
    reason: null,
    sequence: 1,
    pending: metric(1_000),
    discovered: metric(100),
    processed: metric(60),
    subsetRelation: "confirmed",
    ...overrides,
  };
}

describe("buildWorkloadChartModels", () => {
  it("keeps P/D/C as overlapping raw values on one zero-based unit chart", () => {
    const model = buildWorkloadChartModels([sample(0)])[0];
    expect(model.unit).toBe("rows");
    const values = Object.fromEntries(
      model.segments.map((segment) => [segment.key, model.data[0][segment.key]]),
    );
    expect(Object.values(values)).toEqual([1_000, 100, 60]);
    expect(model.segments.map((segment) => segment.series)).toEqual([
      "pending",
      "discovered",
      "processed",
    ]);
  });

  it("preserves a lone real zero and values retained from failed attempts", () => {
    const zero = sample(0, {
      pending: { ...metric(0), coverage: "exact" },
      discovered: null,
      processed: { ...metric(0), coverage: "window" },
      status: "failed",
    });
    const model = buildWorkloadChartModels([zero])[0];
    expect(model.segments.map((segment) => segment.series)).toEqual(["pending", "processed"]);
    expect(model.data[0][model.segments[0].key]).toBe(0);
    expect(model.data[0][model.segments[1].key]).toBe(0);
  });

  it("draws a dashed bridge only over confirmed skipped attempts", () => {
    const start = sample(0);
    const skipped = sample(1, {
      status: "skipped",
      actualStartedAt: null,
      finishedAt: null,
      pending: null,
      discovered: null,
      processed: null,
    });
    const end = sample(2, {
      pending: metric(900),
      discovered: metric(90),
      processed: metric(50),
    });
    const withSkip = buildWorkloadChartModels([start, skipped, end])[0];
    const bridgeKey = Object.keys(withSkip.data[0]).find((key) =>
      key.startsWith("bridge_pending_"),
    );
    expect(bridgeKey).toBeDefined();
    expect(withSkip.data[0][bridgeKey as string]).toBe(1_000);
    expect(withSkip.data[1][bridgeKey as string]).toBeUndefined();
    expect(withSkip.data[2][bridgeKey as string]).toBe(900);

    const ordinaryGap = sample(1, { pending: null });
    const withGap = buildWorkloadChartModels([start, ordinaryGap, end])[0];
    expect(Object.keys(withGap.data[0]).some((key) => key.startsWith("bridge_pending_"))).toBe(
      false,
    );

    const changedRange = sample(2, {
      pending: metric(900, "source<=20"),
      discovered: metric(90, "source<=20"),
      processed: metric(50, "source<=20"),
    });
    const withRangeChange = buildWorkloadChartModels([start, skipped, changedRange])[0];
    expect(
      Object.keys(withRangeChange.data[0]).some((key) => key.startsWith("bridge_pending_")),
    ).toBe(false);

    const leadingSkip = buildWorkloadChartModels([
      { ...skipped, attemptedAt: "2026-10-02T23:59:00.000Z" },
      start,
      end,
    ])[0];
    const trailingSkip = buildWorkloadChartModels([start, skipped])[0];
    expect(Object.keys(leadingSkip.data[0]).some((key) => key.startsWith("bridge_pending_"))).toBe(
      false,
    );
    expect(Object.keys(trailingSkip.data[0]).some((key) => key.startsWith("bridge_pending_"))).toBe(
      false,
    );
  });

  it("separates incompatible units and ends each series at a scope boundary", () => {
    const first = sample(0);
    const second = sample(1, {
      pending: metric(50, "source<=10", "files"),
      discovered: null,
      processed: null,
    });
    const third = sample(2, { pending: metric(900, "source<=20") });
    const models = buildWorkloadChartModels([first, second, third]);
    expect(models.map((model) => model.unit)).toEqual(["rows", "files"]);
    const rowModel = models[0];
    expect(rowModel.segments.filter((segment) => segment.series === "pending")).toHaveLength(2);
    expect(rowModel.data[0][rowModel.segments[0].key]).toBe(1_000);
    expect(rowModel.data[2][rowModel.segments[1].key]).toBe(900);
  });

  it("reserves unit charts from declared capability before the first sample exists", () => {
    const capabilities: TaskMeasurementCapabilities = {
      pending: { supported: true, unit: "rows", scope: "source" },
      discovered: { supported: true, unit: "files", scope: "source" },
      processed: { supported: false, unit: null, scope: null },
    };
    const models = buildWorkloadChartModels([], capabilities);
    expect(models.map((model) => model.unit)).toEqual(["rows", "files"]);
    expect(models.every((model) => model.segments.length === 0)).toBe(true);
  });
});

describe("workloadSubsetPartition", () => {
  it("reports proven partitions only for matching scopes, ranges, units, and nested counts", () => {
    expect(workloadSubsetPartition(sample(0))).toEqual({
      processed: 60,
      discoveredRemainder: 40,
      pendingRemainder: 900,
    });
    expect(
      workloadSubsetPartition(
        sample(0, { discovered: metric(100, "source<=20"), processed: metric(60, "source<=20") }),
      ),
    ).toBeNull();
    expect(workloadSubsetPartition(sample(0, { processed: metric(101) }))).toBeNull();
    expect(
      workloadSubsetPartition(sample(0, { discovered: metric(100, "source<=10", "files") })),
    ).toBeNull();
  });
});
