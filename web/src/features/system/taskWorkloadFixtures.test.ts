import { describe, expect, it } from "vitest";
import { buildRetentionWorkloadFixture } from "./taskWorkloadFixtures";

const HOUR_MS = 60 * 60 * 1000;

describe("buildRetentionWorkloadFixture", () => {
  it("keeps backlog, discoveries, commits, and skipped attempts consistent", () => {
    const nowMs = Date.parse("2026-10-04T12:00:00.000Z");
    const fixture = buildRetentionWorkloadFixture({
      nowMs,
      sampleCount: 100,
      intervalMs: HOUR_MS,
      finalPending: 2_544,
      skippedIndices: [47, 82],
      zeroCommitFailureIndices: [46, 81],
    });
    const { samples } = fixture;

    expect(samples.at(-1)?.pending?.value).toBe(2_544);
    expect(samples[46].status).toBe("failed");
    expect(samples[46].discovered?.value).toBeGreaterThan(0);
    expect(samples[46].processed?.value).toBe(0);
    expect(samples[47].status).toBe("skipped");
    expect(samples[47].pending).toBeNull();
    expect(samples[48].pending?.value).toBeGreaterThan(samples[46].pending?.value ?? 0);
    expect(samples[0].status).toBe("failed");
    expect(samples[0].processed?.value).toBeGreaterThan(0);

    for (const sample of samples) {
      if (sample.pending?.value == null || sample.discovered?.value == null) continue;
      expect(sample.processed?.value).toBeLessThanOrEqual(sample.discovered.value);
      expect(sample.discovered.value).toBeLessThanOrEqual(sample.pending.value);
    }

    const lastBacklogPoint = [...fixture.backlog]
      .reverse()
      .find((point) => point.state === "observed");
    expect(lastBacklogPoint?.invocationCount).toBeGreaterThan(2_544);
    expect(lastBacklogPoint?.invocationCount).toBeLessThan(6_000);
    expect(Date.parse(lastBacklogPoint?.observedAt ?? "")).toBeLessThanOrEqual(nowMs);
    expect(fixture.backlog.some((point) => point.state === "missing")).toBe(true);
    expect(fixture.trend.clearanceEta).not.toBeNull();
  });

  it("derives the displayed processing rate from completed run counts and elapsed time", () => {
    const fixture = buildRetentionWorkloadFixture({
      nowMs: Date.parse("2026-10-04T12:00:00.000Z"),
      sampleCount: 30,
      intervalMs: HOUR_MS,
      finalPending: 8_000,
    });
    const completed = fixture.samples
      .filter(
        (sample) =>
          sample.actualStartedAt != null &&
          sample.finishedAt != null &&
          sample.processed?.coverage === "window",
      )
      .slice(-20);
    const total = completed.reduce((sum, sample) => sum + (sample.processed?.value ?? 0), 0);
    const spanMs =
      Date.parse(completed.at(-1)?.finishedAt ?? "") -
      Date.parse(completed[0]?.actualStartedAt ?? "");

    expect(fixture.trend.processingRatePerSecond).toBeCloseTo(total / (spanMs / 1000));
    expect(fixture.trend.processingRateWindow).toContain("20 次");
  });
});
